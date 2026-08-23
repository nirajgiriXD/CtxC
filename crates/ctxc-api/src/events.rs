//! The live event stream.
//!
//! The dashboard has to show what CtxC is doing as it happens, and polling a
//! metrics endpoint every second would be both wasteful and always slightly
//! wrong. So the daemon publishes what it does, and clients subscribe.
//!
//! ```text
//! supervisor / handler -> publish() -> broadcast -> WS /v1/events
//! ```
//!
//! Almost every event is a [`MetricEvent`] that was going to be recorded
//! anyway: one call at the point where something happens both counts it and
//! announces it, so the stream and the metrics can never tell different
//! stories.
//!
//! Subscribers are deliberately lossy. A client that stops reading must not be
//! able to make the daemon buffer without limit, and a dashboard that missed
//! nine of the last ten file changes has lost nothing that matters — it can
//! always re-read the current state. Slow clients are told what they missed
//! rather than silently shown a gap.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use ctxc_core::Timestamp;
use ctxc_metrics::MetricEvent;

use crate::state::WatchReport;

/// Events held for a subscriber that is not keeping up.
///
/// Roughly a few seconds of a busy daemon. Past this the oldest are dropped and
/// the subscriber is told how many it lost.
const CAPACITY: usize = 256;

/// Something the daemon did, as published to clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// An operation completed: optimization, indexing, a settled watch batch.
    /// Carries the same record the metrics subsystem stored.
    Operation { event: Box<MetricEvent> },

    /// A project was added, paused, resumed or removed.
    Project {
        id: String,
        name: String,
        /// `active`, `paused`, or `removed`.
        status: String,
    },

    /// What the supervisor is observing changed — including falling back to
    /// polling, which is the kind of degradation a person needs to see.
    Watching { projects: Vec<WatchReport> },

    /// The configuration file changed, and which keys moved.
    ///
    /// Sent so that a settings screen open in another tab stops showing values
    /// that are no longer what the file says.
    Config { changed: Vec<String> },

    /// Something worth saying that is not an operation.
    Notice {
        /// `info`, `warn`, or `error`.
        level: String,
        message: String,
    },

    /// Sent to one subscriber that fell behind, in place of the events it
    /// missed. Never hidden: a gap the client does not know about is worse
    /// than one it does.
    Lagged { missed: u64 },
}

/// One event, with when it was published.
///
/// The timestamp is added here rather than inside each variant so that every
/// frame a client receives has one, including the ones the daemon synthesises.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub at: Timestamp,
    #[serde(flatten)]
    pub event: StreamEvent,
}

impl Envelope {
    pub fn now(event: StreamEvent) -> Self {
        Envelope {
            at: Timestamp::now(),
            event,
        }
    }
}

/// Publishes events to whoever is listening.
///
/// Cloneable and cheap: it is a handle to one channel, and publishing with no
/// subscribers costs nothing.
#[derive(Debug, Clone)]
pub struct Broadcaster {
    sender: broadcast::Sender<Envelope>,
}

impl Default for Broadcaster {
    fn default() -> Self {
        Broadcaster::new()
    }
}

impl Broadcaster {
    pub fn new() -> Self {
        Broadcaster {
            sender: broadcast::channel(CAPACITY).0,
        }
    }

    /// Announce an event.
    ///
    /// Never fails and never blocks. No subscribers is the normal case — most
    /// of the time nobody has the dashboard open — and it is not an error.
    pub fn publish(&self, event: StreamEvent) {
        let _ = self.sender.send(Envelope::now(event));
    }

    /// Announce an operation.
    pub fn publish_operation(&self, event: MetricEvent) {
        self.publish(StreamEvent::Operation {
            event: Box::new(event),
        });
    }

    /// Listen from now on. Events published before this returns are not
    /// replayed: the stream is what is happening, not a log.
    pub fn subscribe(&self) -> Subscriber {
        Subscriber {
            receiver: self.sender.subscribe(),
        }
    }

    /// How many clients are listening.
    pub fn listeners(&self) -> usize {
        self.sender.receiver_count()
    }
}

/// One client's view of the stream.
pub struct Subscriber {
    receiver: broadcast::Receiver<Envelope>,
}

impl Subscriber {
    /// The next event, or `None` once the daemon has stopped publishing.
    ///
    /// A subscriber that fell behind gets a [`StreamEvent::Lagged`] telling it
    /// how many it missed, and then carries on from the oldest event still
    /// held.
    pub async fn next(&mut self) -> Option<Envelope> {
        match self.receiver.recv().await {
            Ok(envelope) => Some(envelope),
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                Some(Envelope::now(StreamEvent::Lagged { missed }))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_metrics::Operation;

    fn operation(source: &str) -> MetricEvent {
        MetricEvent::new(Operation::Optimize, source)
    }

    #[tokio::test]
    async fn subscribers_receive_what_is_published() {
        let broadcaster = Broadcaster::new();
        let mut subscriber = broadcaster.subscribe();

        broadcaster.publish_operation(operation("stdin"));

        let envelope = subscriber.next().await.unwrap();
        assert!(envelope.at.as_millis() > 0);
        match envelope.event {
            StreamEvent::Operation { event } => assert_eq!(event.source, "stdin"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn publishing_with_nobody_listening_is_fine() {
        let broadcaster = Broadcaster::new();
        assert_eq!(broadcaster.listeners(), 0);
        broadcaster.publish_operation(operation("stdin"));

        // And a subscriber joining afterwards starts from now, not from the
        // beginning of time.
        let mut subscriber = broadcaster.subscribe();
        broadcaster.publish(StreamEvent::Notice {
            level: "info".into(),
            message: "second".into(),
        });

        match subscriber.next().await.unwrap().event {
            StreamEvent::Notice { message, .. } => assert_eq!(message, "second"),
            other => panic!("a late subscriber must not be replayed history: {other:?}"),
        }
    }

    #[tokio::test]
    async fn every_listener_sees_every_event() {
        let broadcaster = Broadcaster::new();
        let mut first = broadcaster.subscribe();
        let mut second = broadcaster.subscribe();
        assert_eq!(broadcaster.listeners(), 2);

        broadcaster.publish_operation(operation("shared"));

        for subscriber in [&mut first, &mut second] {
            match subscriber.next().await.unwrap().event {
                StreamEvent::Operation { event } => assert_eq!(event.source, "shared"),
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_slow_client_is_told_what_it_missed() {
        let broadcaster = Broadcaster::new();
        let mut subscriber = broadcaster.subscribe();

        for index in 0..(CAPACITY + 10) {
            broadcaster.publish_operation(operation(&format!("event-{index}")));
        }

        match subscriber.next().await.unwrap().event {
            StreamEvent::Lagged { missed } => assert_eq!(missed, 10),
            other => panic!("a gap must be reported, not hidden: {other:?}"),
        }

        // And the stream continues rather than ending.
        match subscriber.next().await.unwrap().event {
            StreamEvent::Operation { event } => assert_eq!(event.source, "event-10"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_stream_ends_when_the_daemon_stops_publishing() {
        let broadcaster = Broadcaster::new();
        let mut subscriber = broadcaster.subscribe();
        drop(broadcaster);

        assert!(subscriber.next().await.is_none());
    }

    #[test]
    fn events_are_tagged_so_a_client_can_tell_them_apart() {
        let json = serde_json::to_value(Envelope::now(StreamEvent::Project {
            id: "abc".into(),
            name: "acme-web".into(),
            status: "active".into(),
        }))
        .unwrap();

        assert_eq!(json["type"], "project");
        assert_eq!(json["name"], "acme-web");
        assert!(json["at"].is_number());
    }
}
