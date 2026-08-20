//! Process-wide state assembled once, before a command runs.

use std::path::PathBuf;

use anyhow::{Context as _, Result};

use ctxc_core::config::{Config, LayerInfo, LoadOptions};
use ctxc_core::platform::{HostPlatform, Platform, SystemEnvironment};
use ctxc_metrics::{Collector, MetricEvent};

use crate::cli::Cli;

/// Resolved platform, configuration, and the layers that produced it.
pub struct App {
    pub platform: HostPlatform,
    pub config: Config,
    pub layers: Vec<LayerInfo>,
    /// The configuration file consulted, whether or not it exists.
    pub config_file: PathBuf,
    /// Measurements taken while the command ran, written once it is over.
    pub metrics: Collector,
}

impl App {
    /// Resolve directories, then load configuration on top of them.
    pub fn bootstrap(cli: &Cli) -> Result<App> {
        let platform = HostPlatform::detect()
            .context("failed to resolve CtxC directories for this platform")?;

        let config_file = cli
            .config
            .clone()
            .unwrap_or_else(|| platform.paths().config_file());

        let env = SystemEnvironment;
        let loaded = Config::load(LoadOptions::new(&env).with_file(&config_file))
            .with_context(|| format!("failed to load configuration ({})", config_file.display()))?;

        Ok(App {
            metrics: Collector::from_config(&loaded.config),
            platform,
            config: loaded.config,
            layers: loaded.layers,
            config_file,
        })
    }

    /// Measure an operation. Cheap, and never fails: a command's own result
    /// must not depend on whether it could be counted.
    pub fn record(&self, event: MetricEvent) {
        self.metrics.record(event);
    }

    /// Write what this command measured.
    ///
    /// Called after the command has already produced its output, so a metrics
    /// failure costs a warning on stderr and nothing else. Nothing was recorded
    /// means nothing is opened: `ctxc version` must not create a database.
    pub fn flush_metrics(&self) {
        if self.metrics.pending() == 0 {
            return;
        }

        match self.open_database() {
            Ok(database) => {
                let store = ctxc_store::SqliteMetricsStore::new(&database);
                self.metrics.flush_quietly(&store);
            }
            Err(err) => {
                tracing::warn!(error = %err, "could not open the database to write metrics")
            }
        }
    }

    /// The platform directories this run resolved.
    pub fn paths(&self) -> &ctxc_core::Paths {
        self.platform.paths()
    }

    /// Path of the SQLite database this installation uses.
    pub fn database_path(&self) -> PathBuf {
        self.config.database_path(self.platform.paths())
    }

    /// Open (creating and migrating if needed) the context database.
    pub fn open_database(&self) -> Result<ctxc_store::Database> {
        let path = self.database_path();
        ctxc_store::Database::open(&path)
            .with_context(|| format!("failed to open context database ({})", path.display()))
    }
}
