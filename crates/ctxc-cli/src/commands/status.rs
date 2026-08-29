//! `ctxc status`

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::Serialize;

use ctxc_core::platform::{Os, Platform};
use ctxc_store::{ContextStore, Database, IndexStore, SqliteContextStore, SqliteIndexStore};

use crate::app::App;
use crate::output::{human_bytes, Printer, Render};
use crate::style::Palette;

/// What this installation looks like right now.
#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    pub config_file: PathBuf,
    pub config_file_exists: bool,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub database: DatabaseStatus,
    /// Projects that have been indexed at least once.
    pub indexed_roots: usize,
    /// What the daemon is doing, if anything.
    pub daemon: DaemonSummary,
}

#[derive(Debug, Serialize)]
pub struct DatabaseStatus {
    pub path: PathBuf,
    pub schema_version: u32,
    pub size_bytes: Option<u64>,
    pub contexts: u64,
}

/// The daemon, as `ctxc status` sees it.
#[derive(Debug, Serialize)]
pub struct DaemonSummary {
    pub running: bool,
    /// Projects being watched, and any that fell back to scanning.
    pub watching: usize,
    pub degraded: usize,
    /// A lockfile was left behind by a daemon that is gone.
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

impl DaemonSummary {
    fn look_up(data_dir: &std::path::Path) -> DaemonSummary {
        // A daemon that cannot be reached is reported as not running rather
        // than as an error: `status` should always answer.
        match ctxc_daemon::status(data_dir) {
            Ok(ctxc_daemon::DaemonState::Running { lock, status }) => DaemonSummary {
                running: true,
                stale: false,
                watching: status.watching,
                degraded: status.degraded,
                pid: Some(lock.pid),
                port: Some(lock.port),
            },
            Ok(ctxc_daemon::DaemonState::Stale { lock }) => DaemonSummary {
                running: false,
                stale: true,
                watching: 0,
                degraded: 0,
                pid: Some(lock.pid),
                port: Some(lock.port),
            },
            Ok(ctxc_daemon::DaemonState::Stopped) | Err(_) => DaemonSummary {
                running: false,
                stale: false,
                watching: 0,
                degraded: 0,
                pid: None,
                port: None,
            },
        }
    }
}

impl StatusReport {
    /// Inspect the installation, opening (and creating) the database.
    pub fn collect(app: &App) -> Result<Self> {
        let database_path = app.database_path();
        let database = Database::open(&database_path).with_context(|| {
            format!(
                "failed to open context database ({})",
                database_path.display()
            )
        })?;

        let store = SqliteContextStore::new(&database);
        let contexts = store
            .count_contexts()
            .context("failed to read the context table")?;

        let index = SqliteIndexStore::new(&database);
        let indexed_roots = index
            .roots()
            .context("failed to read the index state")?
            .len();

        Ok(StatusReport {
            version: env!("CARGO_PKG_VERSION"),
            os: Os::current().as_str(),
            arch: std::env::consts::ARCH,
            config_file: app.config_file.clone(),
            config_file_exists: app.config_file.exists(),
            data_dir: app.platform.paths().data_dir().to_path_buf(),
            cache_dir: app.platform.paths().cache_dir().to_path_buf(),
            database: DatabaseStatus {
                schema_version: database.schema_version()?,
                size_bytes: database.size_on_disk(),
                path: database_path,
                contexts,
            },
            indexed_roots,
            daemon: DaemonSummary::look_up(app.paths().data_dir()),
        })
    }
}

impl Render for StatusReport {
    fn render_human(&self, out: &mut dyn Write, palette: Palette) -> io::Result<()> {
        writeln!(
            out,
            "{} {}",
            palette.heading("CtxC"),
            palette.number(&self.version)
        )?;
        writeln!(out)?;
        writeln!(
            out,
            "{} {} {}",
            palette.label("Platform:  "),
            self.os,
            palette.dim(format!("({})", self.arch))
        )?;
        writeln!(
            out,
            "{} {}{}",
            palette.label("Config:    "),
            palette.path(self.config_file.display()),
            if self.config_file_exists {
                String::new()
            } else {
                format!("  {}", palette.dim("(not created, using defaults)"))
            }
        )?;
        writeln!(
            out,
            "{} {}",
            palette.label("Data:      "),
            palette.path(self.data_dir.display())
        )?;
        writeln!(
            out,
            "{} {}",
            palette.label("Cache:     "),
            palette.path(self.cache_dir.display())
        )?;
        writeln!(
            out,
            "{} {}  {}",
            palette.label("Database:  "),
            palette.path(self.database.path.display()),
            palette.dim(format!(
                "(schema {}{})",
                self.database.schema_version,
                match self.database.size_bytes {
                    Some(bytes) => format!(", {}", human_bytes(bytes)),
                    None => String::new(),
                }
            ))
        )?;
        writeln!(
            out,
            "{} {}",
            palette.label("Contexts:  "),
            palette.number(self.database.contexts)
        )?;
        writeln!(
            out,
            "{} {} {}",
            palette.label("Indexed:   "),
            palette.number(self.indexed_roots),
            palette.dim("project(s)")
        )?;
        match (&self.daemon.running, &self.daemon.stale) {
            (true, _) => {
                writeln!(
                    out,
                    "{} {} {}",
                    palette.label("Daemon:    "),
                    palette.good("running"),
                    palette.dim(format!(
                        "(pid {}, port {})",
                        self.daemon.pid.unwrap_or_default(),
                        self.daemon.port.unwrap_or_default()
                    ))
                )?;
                writeln!(
                    out,
                    "{} {} {}{}",
                    palette.label("Watching:  "),
                    palette.number(self.daemon.watching),
                    palette.dim("project(s)"),
                    match self.daemon.degraded {
                        0 => String::new(),
                        degraded => format!(
                            "  {}",
                            palette.warn(format!("({degraded} scanning instead)"))
                        ),
                    }
                )
            }

            (false, true) => writeln!(
                out,
                "{} {} {}",
                palette.label("Daemon:    "),
                palette.warn("not running"),
                palette.dim("(a lockfile was left behind)")
            ),
            (false, false) => writeln!(
                out,
                "{} {}",
                palette.label("Daemon:    "),
                palette.dim("not running")
            ),
        }
    }
}

pub fn run<W: Write>(app: &App, printer: &mut Printer<W>) -> Result<()> {
    let report = StatusReport::collect(app)?;
    let empty = report.indexed_roots == 0;
    printer.emit(&report)?;

    if empty {
        printer.hint(&["No project is indexed yet. Set one up with:", "  ctxc init"]);
    }
    Ok(())
}
