//! Asking about, and stopping, a daemon.
//!
//! These are the operations a client performs on a daemon it did not start.
//! They live here rather than in the CLI so that the rules — what "running"
//! means, what a stale lockfile implies — have one definition.

use std::path::Path;

use ctxc_api::client::Client;
use ctxc_api::DaemonStatus;

use crate::error::{DaemonError, Result};
use crate::lock::{self, Lock, Occupancy};

/// What is known about the daemon for one data directory.
#[derive(Debug)]
pub enum DaemonState {
    /// Nothing is running and nothing was left behind.
    Stopped,
    /// A daemon is answering.
    Running {
        lock: Box<Lock>,
        status: Box<DaemonStatus>,
    },
    /// A lockfile exists, but nothing answers on its port.
    ///
    /// Reported rather than hidden: it is how a crash becomes visible.
    Stale { lock: Box<Lock> },
}

impl DaemonState {
    pub fn is_running(&self) -> bool {
        matches!(self, DaemonState::Running { .. })
    }
}

/// Look up the daemon for a data directory.
pub fn status(data_dir: &Path) -> Result<DaemonState> {
    match lock::occupancy(data_dir)? {
        Occupancy::Free => Ok(DaemonState::Stopped),
        Occupancy::Stale(lock) => Ok(DaemonState::Stale { lock }),
        Occupancy::Running(lock) => {
            let status = lock.client().get::<DaemonStatus>("/v1/status")?;
            Ok(DaemonState::Running {
                lock,
                status: Box::new(status),
            })
        }
    }
}

/// Stop the daemon, and wait for it to actually be gone.
///
/// Stopping is a request over the API rather than a signal: Windows has no
/// portable equivalent of SIGTERM, and going through the API means the daemon
/// finishes what it is doing and removes its own lockfile.
pub fn stop(data_dir: &Path) -> Result<Lock> {
    let lock = match lock::occupancy(data_dir)? {
        Occupancy::Running(lock) => lock,
        // Clearing a stale lockfile is exactly what someone running `stop`
        // after a crash wants, so it counts as success.
        Occupancy::Stale(lock) => {
            Lock::remove(data_dir)?;
            return Ok(*lock);
        }
        Occupancy::Free => return Err(DaemonError::NotRunning),
    };

    lock.client()
        .post_empty::<serde_json::Value>("/v1/shutdown")?;
    wait_until_stopped(&lock.client());

    // The daemon removes its own lockfile on the way out; this covers the case
    // where it died before finishing.
    Lock::remove(data_dir)?;
    Ok(*lock)
}

/// Poll until the daemon stops answering, or long enough that something is
/// clearly wrong.
fn wait_until_stopped(client: &Client) {
    const ATTEMPTS: u32 = 50;
    const PAUSE: std::time::Duration = std::time::Duration::from_millis(100);

    for _ in 0..ATTEMPTS {
        if !client.is_alive() {
            return;
        }
        std::thread::sleep(PAUSE);
    }
    tracing::warn!("the daemon is still answering after being asked to stop");
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_api::AccessToken;
    use std::path::PathBuf;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let path = std::env::temp_dir()
                .join("ctxc-lifecycle-tests")
                .join(format!("{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Fixture(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn nothing_running_reports_stopped() {
        let fixture = Fixture::new("stopped");
        assert!(matches!(status(&fixture.0).unwrap(), DaemonState::Stopped));
        assert!(!status(&fixture.0).unwrap().is_running());
    }

    #[test]
    fn stopping_nothing_says_so() {
        let fixture = Fixture::new("stop-nothing");
        let error = stop(&fixture.0).unwrap_err();

        assert!(matches!(error, DaemonError::NotRunning));
        assert!(error.hint().unwrap().contains("ctxc start"));
    }

    #[test]
    fn a_stale_lockfile_is_reported_not_hidden() {
        let fixture = Fixture::new("stale");
        let mut lock = Lock::describe(7717, "127.0.0.1", &AccessToken::generate());
        lock.port = 1; // nothing answers here
        lock.write(&fixture.0).unwrap();

        assert!(matches!(
            status(&fixture.0).unwrap(),
            DaemonState::Stale { .. }
        ));
    }

    #[test]
    fn stopping_clears_a_stale_lockfile() {
        let fixture = Fixture::new("stop-stale");
        let mut lock = Lock::describe(7717, "127.0.0.1", &AccessToken::generate());
        lock.port = 1;
        lock.write(&fixture.0).unwrap();

        stop(&fixture.0).unwrap();
        assert!(Lock::read(&fixture.0).unwrap().is_none());
        assert!(matches!(status(&fixture.0).unwrap(), DaemonState::Stopped));
    }
}
