//! The daemon runtime.
//!
//! Start the API on loopback, keep registered projects indexed in the
//! background, and stop cleanly when asked. Everything it does is work the CLI
//! can also do on its own — the daemon exists so that the work is already done
//! by the time someone asks.
//!
//! Observation runs on its own thread: filesystem watchers where they work,
//! periodic scanning where they do not, and settled batches of changes applied
//! incrementally to the index.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ctxc_api::state::{AccessToken, ApiState, Locations};
use ctxc_core::{Config, Paths};
use ctxc_store::Database;

use crate::error::{DaemonError, Result};
use crate::lock::{self, Lock, Occupancy};
use crate::supervisor::{Supervisor, SupervisorOptions};

/// How the daemon should run.
#[derive(Debug, Clone)]
pub struct DaemonOptions {
    pub data_dir: PathBuf,
    pub database: PathBuf,
    pub bind: String,
    pub port: u16,
    /// How the daemon observes projects.
    pub supervisor: SupervisorOptions,
    /// Where this installation keeps its files, so the API can report them and
    /// edit the right configuration file.
    pub locations: Locations,
}

impl DaemonOptions {
    /// Read the options out of configuration and the platform paths.
    pub fn from_config(config: &Config, paths: &Paths) -> Self {
        DaemonOptions {
            data_dir: paths.data_dir().to_path_buf(),
            database: config.database_path(paths),
            bind: config.daemon.bind.clone(),
            port: config.daemon.port,
            supervisor: SupervisorOptions {
                debounce: Duration::from_millis(config.watch.debounce_ms as u64),
                poll_interval: Duration::from_millis(config.watch.poll_interval_ms as u64),
                watch_enabled: config.watch.enabled,
                ..SupervisorOptions::default()
            },
            locations: Locations::resolve(config, paths),
        }
    }

    /// Use a configuration file other than the platform default.
    ///
    /// `ctxc --config` picks one for the run; the daemon has to edit that file
    /// rather than the one it would have chosen on its own.
    pub fn with_config_file(mut self, path: impl AsRef<std::path::Path>) -> DaemonOptions {
        self.locations = self.locations.with_config_file(path);
        self
    }
}

/// What a started daemon is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub port: u16,
    pub pid: u32,
}

/// Run the daemon until it is asked to stop.
///
/// Blocks the calling thread. The runtime is created here rather than by the
/// caller so that the CLI stays free of async.
pub fn run(config: Config, options: DaemonOptions) -> Result<()> {
    match lock::occupancy(&options.data_dir)? {
        Occupancy::Running(existing) => {
            return Err(DaemonError::AlreadyRunning {
                pid: existing.pid,
                port: existing.port,
            })
        }
        // A daemon that crashed leaves its lockfile behind. Taking it over is
        // the recovery path, and it must not need a human.
        Occupancy::Stale(stale) => {
            tracing::info!(pid = stale.pid, "replacing a stale daemon lockfile");
            Lock::remove(&options.data_dir)?;
        }
        Occupancy::Free => {}
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| DaemonError::Runtime { source })?;

    runtime.block_on(serve(config, options))
}

/// Bind, serve, and index, until a shutdown is requested.
async fn serve(config: Config, options: DaemonOptions) -> Result<()> {
    let database = Database::open(&options.database).map_err(DaemonError::Store)?;
    let token = AccessToken::generate();
    let state = ApiState::new(database, config, token.clone(), options.locations.clone());

    let address: SocketAddr = SocketAddr::new(
        options
            .bind
            .parse::<IpAddr>()
            .map_err(|_| DaemonError::BadBindAddress {
                value: options.bind.clone(),
            })?,
        options.port,
    );

    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|source| DaemonError::Bind {
            address: address.to_string(),
            source,
        })?;

    // The port is only known for certain after binding, which matters when the
    // configured port is 0 and the operating system chooses one.
    let bound = listener.local_addr().map_err(|source| DaemonError::Bind {
        address: address.to_string(),
        source,
    })?;

    Lock::describe(bound.port(), &options.bind, &token).write(&options.data_dir)?;
    tracing::info!(
        port = bound.port(),
        pid = std::process::id(),
        "daemon started"
    );

    // Continuous mode runs on its own thread: the work is blocking, and a
    // dedicated thread is easier to stop cleanly than a pool of tasks.
    let stop = Arc::new(AtomicBool::new(false));
    let supervisor = Supervisor::new(state.clone(), options.supervisor.clone());
    let observing = {
        let stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("ctxc-supervisor".into())
            .spawn(move || supervisor.run(stop))
            .map_err(|source| DaemonError::Runtime { source })?
    };

    let shutdown = shutdown_signal(state.clone());
    let served = ctxc_api::serve(listener, state, shutdown).await;

    stop.store(true, Ordering::Relaxed);
    if observing.join().is_err() {
        tracing::warn!("the observation thread ended unexpectedly");
    }

    // The lockfile goes even if serving ended badly: leaving one behind would
    // make the next start think a daemon is still here.
    Lock::remove(&options.data_dir)?;
    tracing::info!("daemon stopped");

    served.map_err(|source| DaemonError::Serve { source })
}

/// Resolve when the daemon should stop: either a client asked, or the terminal
/// did.
async fn shutdown_signal(state: ApiState) {
    let requested = state.shutdown_requested();
    let interrupt = tokio::signal::ctrl_c();

    tokio::select! {
        _ = requested => tracing::info!("shutdown requested through the API"),
        result = interrupt => match result {
            Ok(()) => tracing::info!("interrupted"),
            Err(err) => tracing::warn!(error = %err, "could not listen for interrupts"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_come_from_configuration() {
        let config = Config::default();
        let paths = Paths::new(
            PathBuf::from("/cfg"),
            PathBuf::from("/data"),
            PathBuf::from("/cache"),
        );

        let options = DaemonOptions::from_config(&config, &paths);
        assert_eq!(options.port, 7717);
        assert_eq!(options.bind, "127.0.0.1");
        assert_eq!(options.data_dir, PathBuf::from("/data"));
        assert_eq!(options.database, paths.database_file());
        assert!(options.supervisor.watch_enabled);
        assert_eq!(options.supervisor.debounce, Duration::from_millis(300));
        assert_eq!(options.supervisor.poll_interval, Duration::from_secs(30));
    }

    #[test]
    fn a_bind_address_that_is_not_an_address_is_refused() {
        let mut config = Config::default();
        config.daemon.bind = "not-an-address".into();

        let paths = Paths::new(
            std::env::temp_dir(),
            std::env::temp_dir().join(format!("ctxc-daemon-bind-{}", std::process::id())),
            std::env::temp_dir(),
        );
        let options = DaemonOptions::from_config(&config, &paths);

        let error = run(config, options).unwrap_err();
        assert!(matches!(error, DaemonError::BadBindAddress { .. }));
    }
}
