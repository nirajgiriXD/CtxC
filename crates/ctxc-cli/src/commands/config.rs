//! `ctxc config [show|path|init]`

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde::Serialize;

use ctxc_core::config::{Config, LayerInfo, LayerKind};
use ctxc_core::Paths;

use crate::app::App;
use crate::cli::ConfigAction;
use crate::error::CliError;
use crate::output::{Printer, Render};

/// The effective configuration, plus where it came from.
#[derive(Debug, Serialize)]
pub struct ConfigReport {
    pub config: Config,
    /// Rendered TOML, so JSON consumers see exactly what a file would contain.
    pub toml: String,
}

impl Render for ConfigReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        write!(out, "{}", self.toml)
    }
}

/// The layer stack, lowest precedence first.
#[derive(Debug, Serialize)]
pub struct LayersReport {
    pub layers: Vec<LayerInfo>,
}

impl Render for LayersReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out, "Configuration layers (lowest precedence first)")?;
        writeln!(out)?;
        for layer in &self.layers {
            let name = match layer.kind {
                LayerKind::Defaults => "defaults",
                LayerKind::File => "file",
                LayerKind::Environment => "environment",
                LayerKind::Overrides => "command line",
            };
            let location = match &layer.path {
                Some(path) => path.display().to_string(),
                None => "-".into(),
            };
            let state = if layer.applied { "" } else { "  (not present)" };
            writeln!(out, "  {name:<13}{location}{state}")?;
        }
        Ok(())
    }
}

/// Result of writing a configuration file.
#[derive(Debug, Serialize)]
pub struct InitReport {
    pub path: PathBuf,
    pub written: bool,
}

impl Render for InitReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out, "wrote {}", self.path.display())
    }
}

pub fn run<W: Write>(
    app: &App,
    action: Option<&ConfigAction>,
    printer: &mut Printer<W>,
) -> Result<()> {
    match action.unwrap_or(&ConfigAction::Show) {
        ConfigAction::Show => {
            let report = ConfigReport {
                toml: app.config.to_toml(),
                config: app.config.clone(),
            };
            printer.emit(&report)?;
        }
        ConfigAction::Path => {
            printer.emit(&LayersReport {
                layers: app.layers.clone(),
            })?;
        }
        ConfigAction::Init { force } => {
            let report = init(app, *force)?;
            printer.emit(&report)?;
        }
    }
    Ok(())
}

/// Write the built-in defaults to the configuration file.
///
/// Defaults rather than the effective configuration: the file is a starting
/// point to edit, and baking in whatever environment variables happened to be
/// set would surprise the next run.
fn init(app: &App, force: bool) -> Result<InitReport> {
    let path = &app.config_file;
    if path.exists() && !force {
        return Err(CliError::new(format!(
            "configuration file already exists: {}",
            path.display()
        ))
        .with_hint("ctxc config init --force  (overwrites the existing file)")
        .into());
    }

    if let Some(parent) = path.parent() {
        Paths::ensure_dir(parent)?;
    }

    let contents = format!(
        "# CtxC configuration.\n\
         # Values shown are the built-in defaults; edit what you need.\n\
         # Environment variables (CTXC_<SECTION>_<KEY>) override this file.\n\n{}",
        Config::default().to_toml()
    );
    std::fs::write(path, contents)
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(InitReport {
        path: path.clone(),
        written: true,
    })
}
