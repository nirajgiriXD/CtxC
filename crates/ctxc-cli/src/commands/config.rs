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
use crate::style::Palette;

/// The effective configuration, plus where it came from.
#[derive(Debug, Serialize)]
pub struct ConfigReport {
    pub config: Config,
    /// Rendered TOML, so JSON consumers see exactly what a file would contain.
    pub toml: String,
}

impl Render for ConfigReport {
    fn render_human(&self, out: &mut dyn Write, _palette: Palette) -> io::Result<()> {
        // Left exactly as written: this is TOML someone will copy into a file,
        // and escape sequences in it would be pasted along with the rest.
        write!(out, "{}", self.toml)
    }
}

/// The layer stack, lowest precedence first.
#[derive(Debug, Serialize)]
pub struct LayersReport {
    pub layers: Vec<LayerInfo>,
}

impl Render for LayersReport {
    fn render_human(&self, out: &mut dyn Write, palette: Palette) -> io::Result<()> {
        writeln!(
            out,
            "{}",
            palette.heading("Configuration layers (lowest precedence first)")
        )?;
        writeln!(out)?;
        for layer in &self.layers {
            let name = match layer.kind {
                LayerKind::Defaults => "defaults",
                LayerKind::File => "file",
                LayerKind::Environment => "environment",
                LayerKind::Overrides => "command line",
            };
            let location = match &layer.path {
                Some(path) => palette.path(path.display().to_string()),
                None => palette.dim("-".to_string()),
            };
            let state = if layer.applied {
                String::new()
            } else {
                format!("  {}", palette.dim("(not present)"))
            };
            writeln!(out, "  {:<13}{location}{state}", palette.label(name))?;
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
    fn render_human(&self, out: &mut dyn Write, palette: Palette) -> io::Result<()> {
        writeln!(out, "wrote {}", palette.path(self.path.display()))
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
        // Agent guidance is configuration too — it is just written into files
        // the agents read rather than into CtxC's own.
        ConfigAction::Agents { action } => {
            return super::integrations::run(app, action.as_ref(), printer);
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
