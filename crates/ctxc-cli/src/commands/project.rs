//! `ctxc project add|list|remove|pause|resume|status|open`.
//!
//! The registry is what turns CtxC from a command you run into something that
//! knows about your work. These commands go straight to the database rather
//! than through the daemon, so they behave identically whether or not one is
//! running — the daemon picks up the change on its next pass.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::Serialize;

use ctxc_engine::index::{IndexOptions, Indexer};
use ctxc_project::{Project, Registry};
use ctxc_store::{Database, IndexStore, SqliteIndexStore, SqliteProjectStore};

use crate::app::App;
use crate::cli::ProjectAction;
use crate::error::CliError;
use crate::output::{human_count, Printer, Render};

/// A project as the CLI reports it.
#[derive(Debug, Serialize)]
pub struct ProjectReport {
    pub id: String,
    pub name: String,
    pub path: String,
    pub status: String,
    /// False when the directory has been moved or deleted.
    pub exists: bool,
    pub indexed_files: u64,
    pub symbols: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub frameworks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<String>,
    pub git: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_indexed_at: Option<String>,
}

impl ProjectReport {
    fn new(project: Project, store: &dyn IndexStore) -> ProjectReport {
        let counts = store.counts(&project.path).unwrap_or_default();
        ProjectReport {
            id: project.id.to_string(),
            name: project.name,
            status: project.status.as_str().to_string(),
            exists: Path::new(&project.path).is_dir(),
            indexed_files: counts.files,
            symbols: counts.symbols,
            languages: project.detection.languages,
            frameworks: project.detection.frameworks,
            package_manager: project.detection.package_manager,
            git: project.detection.git,
            last_indexed_at: project.last_indexed_at.map(|at| at.to_rfc3339()),
            path: project.path,
        }
    }
}

impl Render for ProjectReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out, "{}", self.name)?;
        writeln!(out)?;
        writeln!(out, "Id:         {}", self.id)?;
        writeln!(
            out,
            "Path:       {}{}",
            self.path,
            if self.exists { "" } else { "  (missing)" }
        )?;
        writeln!(out, "Status:     {}", self.status)?;
        writeln!(
            out,
            "Indexed:    {} files, {} symbols",
            human_count(self.indexed_files as u32),
            human_count(self.symbols as u32)
        )?;
        if !self.languages.is_empty() {
            writeln!(out, "Languages:  {}", self.languages.join(", "))?;
        }
        if !self.frameworks.is_empty() {
            writeln!(out, "Frameworks: {}", self.frameworks.join(", "))?;
        }
        if let Some(manager) = &self.package_manager {
            writeln!(out, "Packages:   {manager}")?;
        }
        writeln!(out, "Git:        {}", if self.git { "yes" } else { "no" })?;
        match &self.last_indexed_at {
            Some(at) => writeln!(out, "Last index: {at}"),
            None => writeln!(out, "Last index: never"),
        }
    }
}

/// Every registered project.
#[derive(Debug, Serialize)]
pub struct ProjectList {
    pub projects: Vec<ProjectReport>,
}

impl Render for ProjectList {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        if self.projects.is_empty() {
            return writeln!(
                out,
                "No projects registered. Add one with `ctxc project add <path>`."
            );
        }

        writeln!(out, "{:<24}{:<10}{:<10}PATH", "PROJECT", "STATUS", "INDEX")?;
        for project in &self.projects {
            let index = match (project.exists, project.indexed_files) {
                (false, _) => "missing".to_string(),
                (true, 0) => "pending".to_string(),
                (true, files) => format!("{} files", human_count(files as u32)),
            };
            writeln!(
                out,
                "{:<24}{:<10}{:<10}{}",
                truncate(&project.name, 23),
                project.status,
                index,
                project.path
            )?;
        }
        Ok(())
    }
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_string();
    }
    let kept: String = value.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}\u{2026}")
}

pub fn run<W: Write>(app: &App, action: &ProjectAction, printer: &mut Printer<W>) -> Result<()> {
    // Reading a project's code is about a directory, not about the registry,
    // so these two run before a registry connection is opened at all.
    match action {
        ProjectAction::Index { path, force } => {
            return super::index::index(
                app,
                &super::index::root_or_current(path.as_ref()),
                *force,
                printer,
            );
        }
        ProjectAction::Graph { path, file, limit } => {
            return super::index::graph(
                app,
                &super::index::root_or_current(path.as_ref()),
                file.as_deref(),
                *limit,
                printer,
            );
        }
        _ => {}
    }

    let database = app.open_database()?;

    match action {
        ProjectAction::Index { .. } | ProjectAction::Graph { .. } => unreachable!("handled above"),
        ProjectAction::Add { path, index } => add(app, &database, path, *index, printer),
        ProjectAction::List => list(&database, printer),
        ProjectAction::Status { project } => status(&database, project, printer),
        ProjectAction::Remove { project } => {
            let project = with_registry(&database, |registry| registry.remove(project))?;
            emit_one(&database, project, printer)
        }
        ProjectAction::Pause { project } => {
            let project = with_registry(&database, |registry| registry.pause(project))?;
            emit_one(&database, project, printer)
        }
        ProjectAction::Resume { project } => {
            let project = with_registry(&database, |registry| registry.resume(project))?;
            emit_one(&database, project, printer)
        }
        ProjectAction::Open { project } => open(&database, project, printer),
    }
}

/// Run one registry operation.
fn with_registry<T>(
    database: &Database,
    work: impl FnOnce(&Registry<'_>) -> ctxc_project::Result<T>,
) -> Result<T> {
    let store = SqliteProjectStore::new(database);
    Ok(work(&Registry::new(&store))?)
}

fn add<W: Write>(
    app: &App,
    database: &Database,
    path: &Path,
    index_now: bool,
    printer: &mut Printer<W>,
) -> Result<()> {
    let registration = with_registry(database, |registry| registry.add(path))?;
    let mut project = registration.project;

    if index_now {
        // Indexing on add makes the project immediately useful, which is what
        // someone who just registered it is about to want.
        let store = SqliteIndexStore::new(database);
        let root = PathBuf::from(&project.path);
        database
            .transaction(|| Indexer::new(&store).index(&root, &IndexOptions::default()))
            .with_context(|| format!("failed to index {}", project.path))?;
        let at = ctxc_core::Timestamp::now();
        with_registry(database, |registry| {
            registry.record_indexed(&project.id, at)
        })?;
        project.last_indexed_at = Some(at);
    } else if !index_now && ctxc_daemon::status(app.paths().data_dir())?.is_running() {
        tracing::info!("the daemon will index this project on its next pass");
    }

    emit_one(database, project, printer)
}

fn list<W: Write>(database: &Database, printer: &mut Printer<W>) -> Result<()> {
    let projects = with_registry(database, |registry| registry.list())?;
    let store = SqliteIndexStore::new(database);

    printer.emit(&ProjectList {
        projects: projects
            .into_iter()
            .map(|project| ProjectReport::new(project, &store))
            .collect(),
    })?;
    Ok(())
}

fn status<W: Write>(database: &Database, reference: &str, printer: &mut Printer<W>) -> Result<()> {
    let project = with_registry(database, |registry| registry.refresh_detection(reference))?;
    emit_one(database, project, printer)
}

/// Show where a project is, for a shell to act on.
///
/// CtxC prints the path rather than launching anything: what "open" means is
/// the user's business, and guessing at an editor or a file manager would be
/// both platform specific and presumptuous.
fn open<W: Write>(database: &Database, reference: &str, printer: &mut Printer<W>) -> Result<()> {
    let project = with_registry(database, |registry| registry.resolve(reference))?;
    if !Path::new(&project.path).is_dir() {
        return Err(CliError::new(format!(
            "{} no longer exists at {}",
            project.name, project.path
        ))
        .with_hint("remove it with `ctxc project remove`, or restore the directory")
        .into());
    }

    printer.write_content(&project.path)?;
    Ok(())
}

fn emit_one<W: Write>(
    database: &Database,
    project: Project,
    printer: &mut Printer<W>,
) -> Result<()> {
    let store = SqliteIndexStore::new(database);
    printer.emit(&ProjectReport::new(project, &store))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_names_are_truncated_with_an_ellipsis() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(
            truncate("a-very-long-project-name", 10),
            "a-very-lo\u{2026}"
        );
    }
}
