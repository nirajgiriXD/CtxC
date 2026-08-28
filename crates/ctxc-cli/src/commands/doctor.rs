//! `ctxc doctor`.
//!
//! `ctxc status` says what this installation *is*. This says whether anything
//! about it is *wrong*, and what to type about it.
//!
//! Every failing check carries the command that fixes it. A diagnostic that
//! names a problem and leaves the reader to search the documentation for the
//! cure has done half a job.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use ctxc_core::Timestamp;
use ctxc_project::Registry;
use ctxc_store::{Database, SqliteProjectStore};

use crate::app::App;
use crate::error::CliError;
use crate::output::{Printer, Render};

/// A project indexed longer ago than this is worth re-reading. Long enough
/// that a week of not touching a project is not a complaint, short enough that
/// a stale index is caught before someone wonders why search is wrong.
const STALE_INDEX_DAYS: i64 = 30;

/// How one check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    /// Nothing to do.
    Pass,
    /// Works, but something is worth attending to.
    Warn,
    /// Broken, and something will not work until it is fixed.
    Fail,
}

impl Health {
    fn marker(self) -> &'static str {
        match self {
            Health::Pass => "ok  ",
            Health::Warn => "warn",
            Health::Fail => "FAIL",
        }
    }
}

/// One thing that was checked.
#[derive(Debug, Serialize)]
pub struct Check {
    pub name: String,
    pub health: Health,
    pub detail: String,
    /// The command that fixes it, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl Check {
    fn pass(name: impl Into<String>, detail: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            health: Health::Pass,
            detail: detail.into(),
            fix: None,
        }
    }

    fn warn(name: impl Into<String>, detail: impl Into<String>, fix: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            health: Health::Warn,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }

    fn fail(name: impl Into<String>, detail: impl Into<String>, fix: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            health: Health::Fail,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}

/// Everything `ctxc doctor` looked at.
#[derive(Debug, Serialize)]
pub struct Diagnosis {
    pub checks: Vec<Check>,
    pub passed: usize,
    pub warnings: usize,
    pub failures: usize,
}

impl Diagnosis {
    fn new(checks: Vec<Check>) -> Diagnosis {
        let count = |health| checks.iter().filter(|check| check.health == health).count();
        Diagnosis {
            passed: count(Health::Pass),
            warnings: count(Health::Warn),
            failures: count(Health::Fail),
            checks,
        }
    }
}

impl Render for Diagnosis {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        for check in &self.checks {
            writeln!(
                out,
                "{}  {:<22}{}",
                check.health.marker(),
                check.name,
                check.detail
            )?;
            if let Some(fix) = &check.fix {
                writeln!(out, "      {fix}")?;
            }
        }

        writeln!(out)?;
        if self.failures == 0 && self.warnings == 0 {
            return writeln!(out, "{} check(s) passed. Nothing to fix.", self.passed);
        }
        writeln!(
            out,
            "{} passed, {} warning(s), {} failure(s).",
            self.passed, self.warnings, self.failures
        )
    }
}

pub fn run<W: Write>(app: &App, printer: &mut Printer<W>) -> Result<()> {
    let mut checks = Vec::new();

    let database = check_database(app, &mut checks);
    checks.push(check_data_dir(app));
    checks.push(check_daemon(app));
    if let Some(database) = &database {
        check_projects(database, &mut checks);
    }
    checks.push(check_agents(app));
    checks.push(check_dashboard());

    let diagnosis = Diagnosis::new(checks);
    let failures = diagnosis.failures;
    let first_fix = diagnosis
        .checks
        .iter()
        .find(|check| check.health == Health::Fail)
        .and_then(|check| check.fix.clone());
    printer.emit(&diagnosis)?;

    // A non-zero exit is what makes this usable from a script, and the report
    // above has already said what is wrong in full.
    if failures > 0 {
        let mut error = CliError::new(format!("{failures} check(s) failed"));
        if let Some(fix) = first_fix {
            error = error.with_hint(fix);
        }
        return Err(error.into());
    }
    Ok(())
}

/// The database opens, is migrated, and accepts a write.
///
/// Returns the handle so the project checks can reuse it: a second open would
/// be a second connection competing for the same lock.
fn check_database(app: &App, checks: &mut Vec<Check>) -> Option<Database> {
    let path = app.database_path();

    let database = match Database::open(&path) {
        Ok(database) => database,
        Err(err) => {
            checks.push(Check::fail(
                "database",
                format!("cannot open {}: {err}", path.display()),
                "check the path in `ctxc config show`, and that the file is readable",
            ));
            return None;
        }
    };

    let version = match database.schema_version() {
        Ok(version) => version,
        Err(err) => {
            checks.push(Check::fail(
                "database",
                format!("cannot read the schema version: {err}"),
                "the file may not be a CtxC database; move it aside and let CtxC recreate it",
            ));
            return None;
        }
    };

    let latest = ctxc_store::latest_schema_version();
    if version != latest {
        checks.push(Check::fail(
            "database",
            format!("schema {version}, but this build expects {latest}"),
            "opening the database migrates it; if this persists the file is from a newer CtxC",
        ));
        return Some(database);
    }

    // Opening proves it is readable. Only a write proves nothing else is
    // holding the lock, which is the failure people actually hit.
    match database.transaction::<_, ctxc_store::StoreError>(|| Ok(())) {
        Ok(()) => checks.push(Check::pass(
            "database",
            format!("schema {version}, writable"),
        )),
        Err(err) => checks.push(Check::fail(
            "database",
            format!("locked by another process: {err}"),
            "run `ctxc stop` to release it, or wait for the other command to finish",
        )),
    }
    Some(database)
}

/// The data directory exists and takes a write.
fn check_data_dir(app: &App) -> Check {
    let dir = app.paths().data_dir().to_path_buf();
    if !dir.is_dir() {
        return Check::fail(
            "data directory",
            format!("{} does not exist", dir.display()),
            "run `ctxc status`, which creates it",
        );
    }

    // Actually write, rather than reading permissions: on Windows the
    // permission bits and what a write is allowed to do are different
    // questions, and only one of them is the one that matters.
    let probe = dir.join(format!(".ctxc-doctor-{}", std::process::id()));
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Check::pass("data directory", format!("{} writable", dir.display()))
        }
        Err(err) => Check::fail(
            "data directory",
            format!("{} is not writable: {err}", dir.display()),
            "check its permissions, or set `CTXC_HOME` to a directory you own",
        ),
    }
}

/// The daemon is answering, absent, or has left a lockfile behind.
fn check_daemon(app: &App) -> Check {
    match ctxc_daemon::status(app.paths().data_dir()) {
        Ok(ctxc_daemon::DaemonState::Running { lock, .. }) => Check::pass(
            "daemon",
            format!("running (pid {}, port {})", lock.pid, lock.port),
        ),
        // Not running is a choice, not a fault: every command works without it.
        Ok(ctxc_daemon::DaemonState::Stopped) => Check::pass("daemon", "not running"),
        Ok(ctxc_daemon::DaemonState::Stale { lock }) => Check::fail(
            "daemon",
            format!("a lockfile from pid {} was left behind", lock.pid),
            "ctxc stop",
        ),
        Err(err) => Check::fail(
            "daemon",
            format!("cannot be asked: {err}"),
            "ctxc stop, then ctxc start --detach",
        ),
    }
}

/// Registered projects still exist, and their indexes are not ancient.
fn check_projects(database: &Database, checks: &mut Vec<Check>) {
    let store = SqliteProjectStore::new(database);
    let projects = match Registry::new(&store).list() {
        Ok(projects) => projects,
        Err(err) => {
            checks.push(Check::fail(
                "projects",
                format!("the registry cannot be read: {err}"),
                "ctxc doctor after `ctxc stop`, in case a daemon is mid-write",
            ));
            return;
        }
    };

    if projects.is_empty() {
        checks.push(Check::warn("projects", "none registered", "ctxc init"));
        return;
    }

    let now = Timestamp::now().as_millis();
    let mut healthy = 0;

    for project in &projects {
        if !Path::new(&project.path).is_dir() {
            checks.push(Check::fail(
                format!("project {}", project.name),
                format!("{} no longer exists", project.path),
                format!("ctxc project remove {}", project.name),
            ));
            continue;
        }

        match project.last_indexed_at {
            None => checks.push(Check::warn(
                format!("project {}", project.name),
                "never indexed",
                format!("ctxc project index {}", project.path),
            )),
            Some(at) => {
                let days = (now - at.as_millis()) / (1000 * 60 * 60 * 24);
                if days >= STALE_INDEX_DAYS {
                    checks.push(Check::warn(
                        format!("project {}", project.name),
                        format!("last indexed {days} days ago"),
                        format!("ctxc project index {}", project.path),
                    ));
                } else {
                    healthy += 1;
                }
            }
        }
    }

    if healthy > 0 {
        checks.push(Check::pass(
            "projects",
            format!("{healthy} of {} indexed and present", projects.len()),
        ));
    }
}

/// Agent guidance points at a `ctxc` the agent can actually run.
///
/// The block CtxC writes tells an agent to run `ctxc`. Installed from a binary
/// that is not on `PATH` — a `cargo build` output run by its full path, most
/// often — the agent reads the instruction and cannot follow it.
fn check_agents(app: &App) -> Check {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "this project".to_string());

    let installed = ctxc_integrations::all(&root, &name)
        .into_iter()
        .filter_map(|integration| integration.status().ok())
        .filter(|status| status.installed)
        .count();

    if installed == 0 {
        return Check::pass("agents", "no guidance installed here");
    }

    let _ = app;
    match on_path("ctxc") {
        Some(found) => Check::pass(
            "agents",
            format!("{installed} installed; ctxc found at {}", found.display()),
        ),
        None => Check::warn(
            "agents",
            format!("{installed} installed, but `ctxc` is not on PATH"),
            "put the ctxc binary on PATH, or the agents cannot run what they were told to",
        ),
    }
}

/// Whether this build carries the web interface.
fn check_dashboard() -> Check {
    if ctxc_dashboard::is_bundled() {
        Check::pass("dashboard", "compiled into this build")
    } else {
        Check::warn(
            "dashboard",
            "not compiled into this build",
            "cd crates/ctxc-dashboard/ui && npm ci && npm run build, then rebuild ctxc",
        )
    }
}

/// Where `PATH` would find an executable, if anywhere.
fn on_path(program: &str) -> Option<PathBuf> {
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string())
            .split(';')
            .map(|extension| extension.to_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };

    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
        extensions.iter().find_map(|extension| {
            let candidate = directory.join(format!("{program}{extension}"));
            candidate.is_file().then_some(candidate)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputFormat;

    fn render(checks: Vec<Check>) -> String {
        let mut buffer = Vec::new();
        Printer::new(OutputFormat::Human, &mut buffer)
            .emit(&Diagnosis::new(checks))
            .unwrap();
        String::from_utf8(buffer).unwrap()
    }

    #[test]
    fn a_clean_installation_says_there_is_nothing_to_fix() {
        let text = render(vec![Check::pass("database", "schema 4, writable")]);
        assert!(text.contains("Nothing to fix"), "{text}");
    }

    #[test]
    fn a_failure_is_printed_with_the_command_that_fixes_it() {
        let text = render(vec![
            Check::pass("database", "schema 4, writable"),
            Check::fail(
                "project app",
                "/gone no longer exists",
                "ctxc project remove app",
            ),
        ]);

        assert!(text.contains("FAIL"), "{text}");
        assert!(text.contains("ctxc project remove app"), "{text}");
        assert!(
            text.contains("1 passed, 0 warning(s), 1 failure(s)"),
            "{text}"
        );
    }

    #[test]
    fn the_running_binary_is_findable_when_its_directory_is_on_the_path() {
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_stem().unwrap().to_string_lossy().into_owned();

        let original = std::env::var_os("PATH");
        let mut directories = vec![exe.parent().unwrap().to_path_buf()];
        if let Some(original) = &original {
            directories.extend(std::env::split_paths(original));
        }
        std::env::set_var("PATH", std::env::join_paths(directories).unwrap());

        let found = on_path(&name);

        match original {
            Some(original) => std::env::set_var("PATH", original),
            None => std::env::remove_var("PATH"),
        }
        assert!(found.is_some(), "{name} was not found on PATH");
        assert!(on_path("ctxc-no-such-program-anywhere").is_none());
    }
}
