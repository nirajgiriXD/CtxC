//! The daemon's local HTTP API.
//!
//! Bound to loopback, versioned under `/v1`, and behind a token the daemon
//! generates at startup. Everything above the engine — the CLI when a daemon is
//! running, and the dashboard later — talks to CtxC through this, with no
//! privileged path around it.

pub mod client;
pub mod events;
pub mod routes;
pub mod state;

pub use events::{Broadcaster, Envelope, StreamEvent};
pub use routes::{router, ApiError, DaemonStatus, Health, ProjectView};
pub use state::{AccessToken, ApiState, WatchReport};

/// Serve the API on an already-bound listener until `shutdown` resolves.
///
/// Axum lives behind this function so that the daemon — and anything else that
/// hosts the API later — does not have to depend on the web framework directly.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: ApiState,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}
