//! `ctxc integrations`
//!
//! Coding agents do not have an API for "here is a tool you can use" — they
//! read a file. So installing an integration means writing one marked block
//! into that file, and uninstalling means taking exactly that block out again.
//!
//! Every command here names the file it touched. Writing into somebody's
//! `CLAUDE.md` without saying so would be rude at best.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use ctxc_integrations::{AgentIntegration, IntegrationStatus, Removal};

use crate::app::App;
use crate::cli::IntegrationAction;
use crate::output::{Printer, Render};

/// What CtxC knows about every integration for this project.
#[derive(Debug, Serialize)]
pub struct IntegrationsReport {
    pub project: String,
    pub root: String,
    pub integrations: Vec<IntegrationStatus>,
}

impl Render for IntegrationsReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out, "{}", self.root)?;
        writeln!(out)?;

        for status in &self.integrations {
            writeln!(
                out,
                "  {:<14}{:<14}{}",
                status.name,
                status.summary(),
                relative(&self.root, &status.path)
            )?;
        }

        let installed = self
            .integrations
            .iter()
            .filter(|status| status.installed)
            .count();
        let available = self
            .integrations
            .iter()
            .filter(|status| status.detected && !status.installed)
            .count();

        writeln!(out)?;
        if installed == 0 && available == 0 {
            writeln!(out, "No agents detected here.")?;
            writeln!(
                out,
                "Install one anyway with `ctxc integrations install <NAME>`."
            )?;
        } else if available > 0 {
            writeln!(
                out,
                "{available} detected agent(s) have no CtxC guidance yet."
            )?;
            writeln!(out, "Add it with `ctxc integrations install --detected`.")?;
        }
        Ok(())
    }
}

/// What one install or uninstall did.
#[derive(Debug, Serialize)]
pub struct ChangeReport {
    pub action: &'static str,
    pub changes: Vec<AppliedChange>,
    /// True when this directory is not a registered project.
    ///
    /// The guidance CtxC writes tells an agent to search this project. That
    /// only works once something has indexed it, so an install into an
    /// unregistered directory should say so rather than leave the agent to
    /// discover it.
    pub registered: bool,
}

#[derive(Debug, Serialize)]
pub struct AppliedChange {
    pub name: String,
    pub outcome: String,
    pub path: String,
}

impl Render for ChangeReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        if self.changes.is_empty() {
            return writeln!(out, "No agents detected here, so nothing was changed.");
        }

        for change in &self.changes {
            writeln!(
                out,
                "  {:<14}{:<14}{}",
                change.name, change.outcome, change.path
            )?;
        }

        if self.action == "install" && !self.registered {
            writeln!(out)?;
            writeln!(out, "This directory is not a registered project, so there")?;
            writeln!(out, "is nothing for an agent to search yet.")?;
            writeln!(out, "Run `ctxc project add .` or `ctxc index .` first.")?;
        }
        Ok(())
    }
}

/// Show a path relative to the project when it is inside it.
fn relative(root: &str, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub fn run<W: Write>(
    app: &App,
    action: Option<&IntegrationAction>,
    printer: &mut Printer<W>,
) -> Result<()> {
    match action {
        None | Some(IntegrationAction::List { .. }) => {
            let path = match action {
                Some(IntegrationAction::List { path }) => path.as_ref(),
                _ => None,
            };
            list(app, path, printer)
        }
        Some(IntegrationAction::Install {
            name,
            path,
            detected,
        }) => apply(
            app,
            name.as_deref(),
            path.as_ref(),
            *detected,
            true,
            printer,
        ),
        Some(IntegrationAction::Uninstall {
            name,
            path,
            detected,
        }) => apply(
            app,
            name.as_deref(),
            path.as_ref(),
            *detected,
            false,
            printer,
        ),
    }
}

fn list<W: Write>(app: &App, path: Option<&PathBuf>, printer: &mut Printer<W>) -> Result<()> {
    let (root, project) = target(app, path)?;

    let mut integrations = Vec::new();
    for integration in ctxc_integrations::all(&root, &project) {
        integrations.push(integration.status()?);
    }

    printer.emit(&IntegrationsReport {
        project,
        root: root.display().to_string(),
        integrations,
    })?;
    Ok(())
}

/// Install or uninstall, for one agent or for every detected one.
fn apply<W: Write>(
    app: &App,
    name: Option<&str>,
    path: Option<&PathBuf>,
    detected_only: bool,
    installing: bool,
    printer: &mut Printer<W>,
) -> Result<()> {
    let (root, project) = target(app, path)?;
    let registered = registered_name(app, &root).is_some();

    // A named agent is installed whether or not it was detected: setting one up
    // before its first run is entirely reasonable, and refusing would be
    // second-guessing someone who typed the name.
    let chosen: Vec<Box<dyn AgentIntegration>> = match name {
        Some(name) => vec![ctxc_integrations::get(name, &root, &project)?],
        None => ctxc_integrations::all(&root, &project)
            .into_iter()
            .filter(|integration| !detected_only || integration.detect())
            .collect(),
    };

    let mut changes = Vec::new();
    for integration in chosen {
        let status = integration.status()?;
        let outcome = if installing {
            integration.install()?.as_str().to_string()
        } else {
            match integration.uninstall()? {
                Removal::Removed => "removed".to_string(),
                Removal::NothingToDo => "nothing to do".to_string(),
            }
        };

        changes.push(AppliedChange {
            name: status.name.to_string(),
            outcome,
            path: relative(&root.display().to_string(), &status.path),
        });
    }

    printer.emit(&ChangeReport {
        action: if installing { "install" } else { "uninstall" },
        changes,
        registered,
    })?;
    Ok(())
}

/// The project directory to work on, and what to call it.
///
/// The name comes from the registry when the directory is registered, so the
/// guidance CtxC writes says the same thing `ctxc project list` does. For an
/// unregistered directory the folder name is the honest answer.
fn target(app: &App, path: Option<&PathBuf>) -> Result<(PathBuf, String)> {
    let root = super::index::root_or_current(path);
    // Canonicalize so that `.` and a relative path report the same place, then
    // drop the extended-length prefix Windows adds along the way.
    let root = match root.canonicalize() {
        Ok(canonical) => strip_verbatim(canonical),
        Err(_) => root,
    };

    if !root.is_dir() {
        return Err(ctxc_integrations::IntegrationError::NotADirectory { path: root }.into());
    }

    let name = registered_name(app, &root).unwrap_or_else(|| {
        root.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "this project".to_string())
    });

    Ok((root, name))
}

/// Remove the extended-length prefix Windows' `canonicalize` adds.
///
/// `\\?\C:\work` and `C:\work` are the same directory, and only one of them is
/// something a person wants to read. A no-op on every other platform.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(plain) => PathBuf::from(plain),
        None => path,
    }
}

/// The registry's name for this directory, when it has one.
///
/// Not being registered is normal — integrations work on any directory — so a
/// lookup failure costs the nicer name and nothing else.
fn registered_name(app: &App, root: &Path) -> Option<String> {
    let database = app.open_database().ok()?;
    let key = ctxc_engine::index::root_key(root);

    let store = ctxc_store::SqliteProjectStore::new(&database);
    match ctxc_project::ProjectStore::project_by_path(&store, &key) {
        Ok(project) => project.map(|project| project.name),
        Err(err) => {
            tracing::debug!(error = %err, "could not read the project registry");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_verbatim_prefix_is_not_shown_to_anyone() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\work\app")),
            PathBuf::from(r"C:\work\app")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from("/work/app")),
            PathBuf::from("/work/app")
        );
    }

    #[test]
    fn paths_inside_the_project_are_shown_relative_to_it() {
        let root = if cfg!(windows) {
            r"C:\work\app"
        } else {
            "/work/app"
        };
        let path = Path::new(root).join("CLAUDE.md");

        assert_eq!(relative(root, &path), "CLAUDE.md");
    }

    #[test]
    fn a_path_outside_the_project_is_shown_in_full() {
        let root = if cfg!(windows) {
            r"C:\work\app"
        } else {
            "/work/app"
        };
        let elsewhere = if cfg!(windows) {
            Path::new(r"C:\elsewhere\CLAUDE.md")
        } else {
            Path::new("/elsewhere/CLAUDE.md")
        };

        assert_eq!(
            relative(root, elsewhere),
            elsewhere.display().to_string(),
            "a path that is not under the project must not be shown as if it were"
        );
    }
}
