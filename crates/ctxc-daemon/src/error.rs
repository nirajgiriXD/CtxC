//! Errors raised by the daemon.

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T, E = DaemonError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("a daemon is already running (pid {pid}, port {port})")]
    AlreadyRunning { pid: u32, port: u16 },

    #[error("no daemon is running")]
    NotRunning,

    #[error("`{value}` is not an address to bind to")]
    BadBindAddress { value: String },

    #[error("failed to listen on {address}")]
    Bind {
        address: String,
        #[source]
        source: std::io::Error,
    },

    #[error("the daemon stopped unexpectedly")]
    Serve {
        #[source]
        source: std::io::Error,
    },

    #[error("failed to start the async runtime")]
    Runtime {
        #[source]
        source: std::io::Error,
    },

    #[error("failed to {action}: {}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to open the database")]
    Store(#[from] ctxc_store::StoreError),

    #[error("failed to read the project registry")]
    Project(#[from] ctxc_project::ProjectError),

    #[error("failed to update the index")]
    Engine(#[from] ctxc_engine::EngineError),

    #[error(transparent)]
    Core(ctxc_core::Error),

    #[error("failed to reach the daemon")]
    Client(#[from] ctxc_api::client::ClientError),
}

impl DaemonError {
    pub fn hint(&self) -> Option<String> {
        match self {
            DaemonError::AlreadyRunning { .. } => {
                Some("run `ctxc daemon status`, or stop it with `ctxc stop`".into())
            }
            DaemonError::NotRunning => Some("start it with `ctxc start`".into()),
            DaemonError::BadBindAddress { .. } => {
                Some("set daemon.bind to an IP address such as 127.0.0.1".into())
            }
            DaemonError::Bind { source, .. } if source.kind() == std::io::ErrorKind::AddrInUse => {
                Some("another program holds that port; change daemon.port".into())
            }
            DaemonError::Bind { .. } => Some("check daemon.bind and daemon.port".into()),
            DaemonError::Store(err) => err.hint(),
            DaemonError::Project(err) => err.hint(),
            DaemonError::Engine(err) => err.hint(),
            DaemonError::Core(err) => err.hint(),
            DaemonError::Client(err) => err.hint(),
            DaemonError::Serve { .. } | DaemonError::Runtime { .. } | DaemonError::Io { .. } => {
                None
            }
        }
    }
}
