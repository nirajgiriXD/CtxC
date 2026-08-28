//! `ctxc update`
//!
//! Updating a CtxC built from source means building it again: fast-forward the
//! checkout to the latest `main`, build the release binary, and put it where
//! the running one is.
//!
//! A CtxC installed from a release has no checkout to build, so there is
//! nothing here for it to do. Rather than failing at it, this says which
//! install command replaces it — the same one that put it there.
//!
//! Two things make this more than a shell one-liner. The executable being
//! replaced is the one running the update, which is handled by renaming it
//! aside — every platform allows that even while the file is open. And a build
//! that carries the dashboard has to keep carrying it, so the web UI is rebuilt
//! whenever the running binary has one.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result};
use serde::Serialize;

use ctxc_daemon::DaemonState;

use crate::app::App;
use crate::cli::UpdateOptions;
use crate::error::CliError;
use crate::output::{Printer, Render};

/// The remote a source checkout is updated from.
const REMOTE: &str = "origin";

/// Where the source path is remembered, inside the data directory.
const SOURCE_FILE: &str = "source-path";

/// The one-line install for this platform, which is also how a released binary
/// is updated: the script downloads the newest release, checks it against the
/// published checksums, and replaces what is there.
#[cfg(windows)]
const INSTALL_COMMAND: &str =
    "irm https://raw.githubusercontent.com/nirajgirixd/ctxc/main/install.ps1 | iex";

#[cfg(not(windows))]
const INSTALL_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/nirajgirixd/ctxc/main/install.sh | sh";

/// What the web UI did, or did not do, during an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DashboardBuild {
    /// The running binary carries no dashboard, so none was built for the new
    /// one. An update keeps what you had; it does not add things.
    Skipped,
    /// The web UI was rebuilt, so the new binary keeps its dashboard.
    Rebuilt,
    /// The dashboard should have been rebuilt, but npm could not be found.
    NoNpm,
}

impl DashboardBuild {
    fn describe(self) -> Option<&'static str> {
        match self {
            DashboardBuild::Skipped => None,
            DashboardBuild::Rebuilt => Some("web UI rebuilt"),
            DashboardBuild::NoNpm => {
                Some("not rebuilt (npm was not found), so this build has none")
            }
        }
    }
}

/// What an update found, and what it did about it.
#[derive(Debug, Serialize)]
pub struct UpdateReport {
    pub source: PathBuf,
    pub remote: &'static str,
    pub branch: String,
    /// The commit the checkout was on, abbreviated.
    pub from: String,
    /// The commit the branch is on now.
    pub to: String,
    /// How many commits separated them.
    pub behind: usize,
    pub up_to_date: bool,
    /// `--check` was passed, so nothing was changed.
    pub checked: bool,
    pub built: bool,
    pub dashboard: DashboardBuild,
    /// The executable that now holds the new build.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed: Option<PathBuf>,
    /// The version the new binary reports, once it has been asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Whether a daemon this command stopped came back. `None` when none was
    /// running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon_restarted: Option<bool>,
}

impl Render for UpdateReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        if self.up_to_date && !self.built {
            writeln!(
                out,
                "CtxC is up to date ({}/{}, {}).",
                self.remote, self.branch, self.from
            )?;
            writeln!(out, "Source:     {}", self.source.display())?;
            return Ok(());
        }

        if self.checked {
            writeln!(out, "An update is available.")?;
            writeln!(out)?;
            writeln!(out, "Source:     {}", self.source.display())?;
            writeln!(out, "Branch:     {}/{}", self.remote, self.branch)?;
            writeln!(out, "Current:    {}", self.from)?;
            writeln!(
                out,
                "Latest:     {}  ({} ahead)",
                self.to,
                commits(self.behind)
            )?;
            writeln!(out)?;
            writeln!(out, "Run `ctxc update` to build and install it.")?;
            return Ok(());
        }

        match &self.version {
            Some(version) => writeln!(out, "Updated CtxC to {version}.")?,
            None => writeln!(out, "Updated CtxC.")?,
        }
        writeln!(out)?;
        writeln!(out, "Source:     {}", self.source.display())?;
        writeln!(out, "Branch:     {}/{}", self.remote, self.branch)?;
        writeln!(
            out,
            "Commit:     {} -> {}{}",
            self.from,
            self.to,
            match self.behind {
                0 => "  (rebuilt, nothing new)".to_string(),
                behind => format!("  ({})", commits(behind)),
            }
        )?;
        if let Some(dashboard) = self.dashboard.describe() {
            writeln!(out, "Dashboard:  {dashboard}")?;
        }
        if let Some(path) = &self.installed {
            writeln!(out, "Installed:  {}", path.display())?;
        }
        match self.daemon_restarted {
            Some(true) => writeln!(out, "Daemon:     restarted")?,
            Some(false) => {
                writeln!(out, "Daemon:     stopped, and did not come back")?;
                writeln!(out)?;
                writeln!(out, "Start it again with `ctxc start --detach`.")?;
            }
            None => {}
        }
        Ok(())
    }
}

/// Count commits the way a sentence needs them.
fn commits(count: usize) -> String {
    match count {
        1 => "1 commit".to_string(),
        other => format!("{other} commits"),
    }
}

pub fn run<W: Write>(app: &App, options: &UpdateOptions, printer: &mut Printer<W>) -> Result<()> {
    let executable = std::env::current_exe().context("failed to find the CtxC executable")?;
    let executable = tidy(
        executable
            .canonicalize()
            .unwrap_or_else(|_| executable.clone()),
    );

    // A previous update could not delete the binary it replaced, because that
    // binary was still running. Now it is not.
    let _ = fs::remove_file(backup_path(&executable));
    refuse_development_build(&executable)?;

    let source = resolve_source(app, options.source.as_deref(), &executable)?;
    let target_ref = format!("{REMOTE}/{}", options.branch);

    let checked_out = git(&source, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if checked_out != options.branch && !options.force {
        return Err(CliError::new(format!(
            "the source checkout is on `{checked_out}`, not `{}`",
            options.branch
        ))
        .with_hint(format!(
            "switch it with `git -C {} switch {}`, or pass `--branch {checked_out}`",
            source.display(),
            options.branch
        ))
        .into());
    }

    if !git(&source, &["status", "--porcelain"])?.is_empty() && !options.force {
        return Err(
            CliError::new(format!("{} has uncommitted changes", source.display()))
                .with_hint("commit or stash them, or pass `--force`")
                .into(),
        );
    }

    git(&source, &["fetch", REMOTE, &options.branch])?;

    let from = git(&source, &["rev-parse", "--short", "HEAD"])?;
    let to = git(&source, &["rev-parse", "--short", &target_ref])?;
    let behind = git(
        &source,
        &["rev-list", "--count", &format!("HEAD..{target_ref}")],
    )?
    .parse()
    .unwrap_or(0);

    let mut report = UpdateReport {
        source: source.clone(),
        remote: REMOTE,
        branch: options.branch.clone(),
        from,
        to,
        behind,
        up_to_date: behind == 0,
        checked: options.check,
        built: false,
        dashboard: DashboardBuild::Skipped,
        installed: None,
        version: None,
        daemon_restarted: None,
    };

    // Nothing past this point happens unless it was asked for: `--check` only
    // reports, and a checkout with nothing new is left exactly as it is.
    if options.check || (report.up_to_date && !options.force) {
        printer.emit(&report)?;
        return Ok(());
    }

    if behind > 0 {
        git(&source, &["merge", "--ff-only", &target_ref])?;
    }

    report.dashboard = build_dashboard(&source, options.no_dashboard)?;

    let fresh = built_binary(&source);
    // Cargo cannot write over the binary this command is running from, so when
    // that is where the build output goes, step out of the way first — and step
    // back if the build fails, or the update has cost them their CtxC.
    let aside = if same_file(&executable, &fresh) {
        Some(step_aside(&executable)?)
    } else {
        None
    };

    match build(&source) {
        Ok(()) => {
            if let Some(backup) = &aside {
                let _ = fs::remove_file(backup);
            }
        }
        Err(err) => {
            if let Some(backup) = &aside {
                let _ = fs::rename(backup, &executable);
            }
            return Err(err);
        }
    }

    if !fresh.is_file() {
        return Err(CliError::new(format!(
            "the build finished but left no binary at {}",
            fresh.display()
        ))
        .with_hint("check CARGO_TARGET_DIR, then build in the checkout by hand")
        .into());
    }
    report.built = true;

    // The daemon is stopped as late as possible, so it is only down for the
    // copy rather than for the whole build.
    let daemon_was_running = matches!(
        ctxc_daemon::status(app.paths().data_dir()),
        Ok(DaemonState::Running { .. })
    );
    if daemon_was_running {
        let _ = ctxc_daemon::stop(app.paths().data_dir());
    }

    install(&fresh, &executable)?;
    report.installed = Some(executable.clone());
    report.version = installed_version(&executable);

    if daemon_was_running {
        let restarted = restart_daemon(app, &executable);
        if !restarted {
            tracing::warn!("the daemon did not come back after the update");
        }
        report.daemon_restarted = Some(restarted);
    }

    remember_source(app, &source);
    printer.emit(&report)?;
    Ok(())
}

/// Refuse to touch a binary that cargo is managing for a developer.
///
/// `cargo run -- update` would otherwise drop a release build on top of the
/// debug one, which is not what anyone hacking on CtxC meant to ask for.
fn refuse_development_build(executable: &Path) -> Result<()> {
    let in_debug_profile = executable.parent().is_some_and(|profile| {
        profile.file_name().is_some_and(|name| name == "debug")
            && profile
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == "target")
    });

    if in_debug_profile {
        return Err(
            CliError::new("this is a development build, not an installed one")
                .with_hint("build the checkout yourself with `cargo build --release`")
                .into(),
        );
    }
    Ok(())
}

/// Find the checkout to build from.
///
/// The flag wins, then the environment, then what a previous update recorded,
/// then the directories above the running binary and above the current one.
/// The answer is remembered, so the search only has to succeed once.
fn resolve_source(app: &App, requested: Option<&Path>, executable: &Path) -> Result<PathBuf> {
    if let Some(path) = requested {
        return accept(path).ok_or_else(|| not_a_checkout(path));
    }

    if let Some(path) = std::env::var_os("CTXC_SOURCE").map(PathBuf::from) {
        return accept(&path).ok_or_else(|| not_a_checkout(&path));
    }

    if let Some(path) = remembered_source(app).as_deref().and_then(accept) {
        return Ok(path);
    }

    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for start in [executable, current_dir.as_path()] {
        if let Some(found) = start.ancestors().find(|dir| is_checkout(dir)) {
            return accept(found).ok_or_else(|| not_a_checkout(found));
        }
    }

    // No checkout anywhere is what an installed binary looks like, and it is
    // the normal case now that releases are published. The installer replaces
    // it in place, verifying the download on the way — which is the update.
    Err(
        CliError::new("this CtxC was not built from a source checkout")
            .with_hint(format!(
                "install the latest release over it:\n  {}\n\nor, to build \
                 from source instead, clone \
                 https://github.com/nirajgirixd/ctxc and run \
                 `ctxc update --source <PATH>` once; the path is remembered",
                INSTALL_COMMAND
            ))
            .into(),
    )
}

/// Resolve a candidate to a real checkout, or reject it.
fn accept(path: &Path) -> Option<PathBuf> {
    let path = tidy(path.canonicalize().ok()?);
    is_checkout(&path).then_some(path)
}

/// Canonicalizing on Windows produces a `\\?\` path: correct, and unreadable
/// in a message. Only plain drive paths are unwrapped — `\\?\UNC\...` is not
/// a path Windows takes back without its prefix.
#[cfg(windows)]
fn tidy(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(plain) if !plain.starts_with("UNC\\") => PathBuf::from(plain),
        _ => path,
    }
}

#[cfg(not(windows))]
fn tidy(path: PathBuf) -> PathBuf {
    path
}

/// Whether a directory is a git clone of CtxC.
fn is_checkout(directory: &Path) -> bool {
    directory.join(".git").exists()
        && directory
            .join("crates")
            .join("ctxc-cli")
            .join("Cargo.toml")
            .is_file()
}

fn not_a_checkout(path: &Path) -> anyhow::Error {
    CliError::new(format!("{} is not a CtxC source checkout", path.display()))
        .with_hint("point `--source` at a clone of https://github.com/nirajgirixd/ctxc")
        .into()
}

/// The checkout a previous update used.
fn remembered_source(app: &App) -> Option<PathBuf> {
    let recorded = fs::read_to_string(app.paths().data_dir().join(SOURCE_FILE)).ok()?;
    let trimmed = recorded.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

/// Record the checkout, so the next update can be run from anywhere.
///
/// Best effort: an update that worked must not fail over a note to self.
fn remember_source(app: &App, source: &Path) {
    let data_dir = app.paths().data_dir();
    if fs::create_dir_all(data_dir).is_err() {
        return;
    }
    if let Err(err) = fs::write(
        data_dir.join(SOURCE_FILE),
        source.to_string_lossy().as_bytes(),
    ) {
        tracing::debug!(error = %err, "could not record the source path");
    }
}

/// Rebuild the web UI, so a binary that had a dashboard still has one.
fn build_dashboard(source: &Path, skip: bool) -> Result<DashboardBuild> {
    let ui = source.join("crates").join("ctxc-dashboard").join("ui");
    if skip || !ctxc_dashboard::is_bundled() || !ui.join("package.json").is_file() {
        return Ok(DashboardBuild::Skipped);
    }

    if !has_npm() {
        tracing::warn!("npm was not found, so the new build will carry no dashboard");
        return Ok(DashboardBuild::NoNpm);
    }

    run_tool("npm", &["ci"], &ui)?;
    run_tool("npm", &["run", "build"], &ui)?;
    Ok(DashboardBuild::Rebuilt)
}

fn has_npm() -> bool {
    tool_command("npm")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Build the release binary.
///
/// `--locked` because the lockfile is committed: an update should build what
/// the branch says, not quietly pick up newer dependencies along the way.
fn build(source: &Path) -> Result<()> {
    run_tool(
        "cargo",
        &["build", "--release", "--locked", "--package", "ctxc-cli"],
        source,
    )
}

/// Where `cargo build --release` puts the binary.
fn built_binary(source: &Path) -> PathBuf {
    // `join` on an absolute path replaces, so this handles both an absolute
    // CARGO_TARGET_DIR and one relative to the checkout.
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => source.join(dir),
        None => source.join("target"),
    };
    target
        .join("release")
        .join(if cfg!(windows) { "ctxc.exe" } else { "ctxc" })
}

/// Put the fresh binary where the running one lives.
fn install(fresh: &Path, executable: &Path) -> Result<()> {
    // The build already wrote it in place: this is a checkout being run from
    // its own `target/release`.
    if same_file(fresh, executable) {
        return Ok(());
    }

    let backup = step_aside(executable)?;
    match fs::copy(fresh, executable) {
        Ok(_) => {
            // Windows will not delete a running program's file; the next
            // update clears it.
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(err) => {
            let _ = fs::rename(&backup, executable);
            Err(CliError::new(format!(
                "failed to install the new binary at {}: {err}",
                executable.display()
            ))
            .with_hint("check that you can write to that directory")
            .into())
        }
    }
}

/// Move the running executable aside so a new one can take its name.
///
/// An open executable cannot be overwritten on Windows, but it can be renamed:
/// the handle follows the file, not the name. Unix would allow the overwrite,
/// and renaming first is harmless there, so one path serves both.
fn step_aside(executable: &Path) -> Result<PathBuf> {
    let backup = backup_path(executable);
    let _ = fs::remove_file(&backup);
    fs::rename(executable, &backup).with_context(|| {
        format!(
            "failed to move {} aside to make room for the new build",
            executable.display()
        )
    })?;
    Ok(backup)
}

/// Where the outgoing executable waits until it can be deleted.
fn backup_path(executable: &Path) -> PathBuf {
    let mut name = executable.file_name().unwrap_or_default().to_os_string();
    name.push(".ctxc-old");
    executable.with_file_name(name)
}

fn same_file(one: &Path, other: &Path) -> bool {
    let resolve = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    resolve(one) == resolve(other)
}

/// Ask the binary that was just installed what it is.
///
/// This is the proof that the update worked: a build can succeed and still
/// leave behind something that will not start.
fn installed_version(executable: &Path) -> Option<String> {
    let output = Command::new(executable)
        .args(["version", "--format", "json"])
        .stdin(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    report.get("version")?.as_str().map(str::to_owned)
}

/// Start the daemon again, from the binary that was just installed.
///
/// Explicitly not `current_exe`: on Linux that resolves through the file we
/// renamed aside, which would bring the old build straight back.
fn restart_daemon(app: &App, executable: &Path) -> bool {
    Command::new(executable)
        .args(["start", "--detach"])
        .env("CTXC_HOME", app.paths().data_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Run a build tool, letting its progress reach the terminal.
fn run_tool(program: &str, args: &[&str], cwd: &Path) -> Result<()> {
    let status = tool_command(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        // Build chatter goes to stderr, where diagnostics live; stdout stays
        // reserved for this command's own report.
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => CliError::new(format!("{program} is not installed"))
                .with_hint(format!("install {program}, then run `ctxc update` again")),
            _ => CliError::new(format!("failed to run {program}: {err}")),
        })?;

    if !status.success() {
        return Err(
            CliError::new(format!("`{program} {}` failed", args.join(" ")))
                .with_hint("the build output above says why")
                .into(),
        );
    }
    Ok(())
}

/// On Windows `npm` ships as a `.cmd` shim, which cannot be started directly;
/// the command processor has to run it. `cargo` is a real executable, and
/// starting it directly gives a far better error when it is missing.
#[cfg(windows)]
fn tool_command(program: &str) -> Command {
    if program == "npm" {
        let mut command = Command::new("cmd");
        command.args(["/C", "npm"]);
        return command;
    }
    Command::new(program)
}

#[cfg(not(windows))]
fn tool_command(program: &str) -> Command {
    Command::new(program)
}

/// Run git in the checkout, with its output captured.
fn git(source: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => CliError::new("git is not installed")
                .with_hint("install git, or update by building the checkout yourself"),
            _ => CliError::new(format!("failed to run git: {err}")),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("it printed nothing")
            .trim();
        return Err(
            CliError::new(format!("`git {}` failed: {reason}", args.join(" ")))
                .with_hint(format!("run it yourself in {}", source.display()))
                .into(),
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputFormat;

    fn render(report: &UpdateReport) -> String {
        let mut buffer = Vec::new();
        Printer::new(OutputFormat::Human, &mut buffer)
            .emit(report)
            .unwrap();
        String::from_utf8(buffer).unwrap()
    }

    fn report() -> UpdateReport {
        UpdateReport {
            source: PathBuf::from("/src/ctxc"),
            remote: REMOTE,
            branch: "main".into(),
            from: "8f3a1c2".into(),
            to: "1d4e9ab".into(),
            behind: 12,
            up_to_date: false,
            checked: false,
            built: true,
            dashboard: DashboardBuild::Rebuilt,
            installed: Some(PathBuf::from("/usr/local/bin/ctxc")),
            version: Some("0.2.0".into()),
            daemon_restarted: Some(true),
        }
    }

    #[test]
    fn an_unchanged_checkout_reports_no_work() {
        let text = render(&UpdateReport {
            behind: 0,
            up_to_date: true,
            built: false,
            dashboard: DashboardBuild::Skipped,
            installed: None,
            version: None,
            daemon_restarted: None,
            ..report()
        });
        assert!(text.contains("up to date (origin/main, 8f3a1c2)"), "{text}");
        assert!(!text.contains("Installed"), "{text}");
    }

    #[test]
    fn check_describes_an_update_without_claiming_to_have_done_it() {
        let text = render(&UpdateReport {
            checked: true,
            built: false,
            installed: None,
            version: None,
            daemon_restarted: None,
            ..report()
        });
        assert!(text.contains("An update is available"), "{text}");
        assert!(text.contains("12 commits ahead"), "{text}");
        assert!(!text.contains("Updated CtxC"), "{text}");
    }

    #[test]
    fn a_finished_update_reports_the_new_version_and_where_it_went() {
        let text = render(&report());
        assert!(text.contains("Updated CtxC to 0.2.0"), "{text}");
        assert!(text.contains("8f3a1c2 -> 1d4e9ab  (12 commits)"), "{text}");
        assert!(text.contains("Installed:  /usr/local/bin/ctxc"), "{text}");
        assert!(text.contains("Daemon:     restarted"), "{text}");
    }

    #[test]
    fn a_daemon_that_did_not_come_back_is_not_hidden() {
        let text = render(&UpdateReport {
            daemon_restarted: Some(false),
            ..report()
        });
        assert!(text.contains("did not come back"), "{text}");
        assert!(text.contains("ctxc start --detach"), "{text}");
    }

    #[test]
    fn a_forced_rebuild_says_that_nothing_new_arrived() {
        let text = render(&UpdateReport {
            behind: 0,
            up_to_date: true,
            to: "8f3a1c2".into(),
            ..report()
        });
        assert!(text.contains("rebuilt, nothing new"), "{text}");
    }

    #[test]
    fn one_commit_is_singular() {
        assert_eq!(commits(1), "1 commit");
        assert_eq!(commits(0), "0 commits");
        assert_eq!(commits(2), "2 commits");
    }

    #[test]
    fn the_outgoing_binary_waits_beside_the_new_one() {
        assert_eq!(
            backup_path(Path::new("/usr/local/bin/ctxc")),
            PathBuf::from("/usr/local/bin/ctxc.ctxc-old")
        );
        assert_eq!(
            backup_path(Path::new("C:\\bin\\ctxc.exe"))
                .file_name()
                .unwrap(),
            "ctxc.exe.ctxc-old"
        );
    }

    #[test]
    fn only_a_git_clone_of_ctxc_counts_as_a_checkout() {
        let root = std::env::temp_dir()
            .join("ctxc-update-tests")
            .join(format!("{}-checkout", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let manifest = root.join("crates").join("ctxc-cli").join("Cargo.toml");
        fs::create_dir_all(manifest.parent().unwrap()).unwrap();

        assert!(!is_checkout(&root), "no .git, no manifest");

        fs::write(&manifest, "").unwrap();
        assert!(!is_checkout(&root), "a source tree without .git is not one");

        fs::create_dir_all(root.join(".git")).unwrap();
        assert!(is_checkout(&root));
        assert!(
            !is_checkout(&root.join("crates")),
            "a subdirectory is not the checkout"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_development_build_is_refused() {
        assert!(refuse_development_build(Path::new("/src/ctxc/target/debug/ctxc")).is_err());
        assert!(refuse_development_build(Path::new("/src/ctxc/target/release/ctxc")).is_ok());
        assert!(refuse_development_build(Path::new("/usr/local/bin/ctxc")).is_ok());
    }
}
