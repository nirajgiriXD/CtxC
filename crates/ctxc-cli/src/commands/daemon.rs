//! `ctxc start`, `ctxc stop` and `ctxc status --daemon`.
//!
//! `start` runs the daemon in this process by default, which is what makes it
//! usable under a service manager, in a container, or in a terminal you can
//! watch. `--detach` hands it to the operating system instead, for the case
//! where someone just wants it running.

use std::io::{self, Write};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result};
use serde::Serialize;

use ctxc_daemon::{DaemonOptions, DaemonState};

use crate::app::App;
use crate::error::CliError;
use crate::output::{human_count, Printer, Render};

/// What `ctxc status --daemon` reports.
#[derive(Debug, Serialize)]
pub struct DaemonReport {
    pub running: bool,
    /// True when a lockfile was left behind by a daemon that is gone.
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_projects: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_files: Option<u64>,
    /// Projects being watched, and any that fell back to periodic scanning.
    pub watching: usize,
    pub degraded: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub watch: Vec<ctxc_api::WatchReport>,
}

impl DaemonReport {
    fn from_state(state: &DaemonState) -> DaemonReport {
        match state {
            DaemonState::Stopped => DaemonReport {
                running: false,
                stale: false,
                pid: None,
                port: None,
                uptime_ms: None,
                projects: None,
                active_projects: None,
                indexed_files: None,
                watching: 0,
                degraded: 0,
                watch: Vec::new(),
            },
            DaemonState::Stale { lock } => DaemonReport {
                running: false,
                stale: true,
                pid: Some(lock.pid),
                port: Some(lock.port),
                uptime_ms: None,
                projects: None,
                active_projects: None,
                indexed_files: None,
                watching: 0,
                degraded: 0,
                watch: Vec::new(),
            },
            DaemonState::Running { lock, status } => DaemonReport {
                running: true,
                stale: false,
                pid: Some(lock.pid),
                port: Some(lock.port),
                uptime_ms: Some(status.uptime_ms),
                projects: Some(status.projects),
                active_projects: Some(status.active_projects),
                indexed_files: Some(status.indexed_files),
                watching: status.watching,
                degraded: status.degraded,
                watch: status.watch.clone(),
            },
        }
    }
}

impl Render for DaemonReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        if !self.running {
            writeln!(
                out,
                "Daemon:     {}",
                if self.stale {
                    "not running (a lockfile was left behind)"
                } else {
                    "not running"
                }
            )?;
            if self.stale {
                writeln!(out, "            run `ctxc stop` to clear it")?;
            }
            return Ok(());
        }

        writeln!(out, "Daemon:     running")?;
        writeln!(out, "PID:        {}", self.pid.unwrap_or_default())?;
        writeln!(out, "Port:       {}", self.port.unwrap_or_default())?;
        writeln!(out, "Uptime:     {}", uptime(self.uptime_ms.unwrap_or(0)))?;
        writeln!(
            out,
            "Projects:   {} ({} active)",
            human_count(self.projects.unwrap_or(0) as u32),
            human_count(self.active_projects.unwrap_or(0) as u32)
        )?;
        writeln!(
            out,
            "Indexed:    {} files",
            human_count(self.indexed_files.unwrap_or(0) as u32)
        )?;
        writeln!(
            out,
            "Watching:   {} project(s){}",
            self.watching,
            match self.degraded {
                0 => String::new(),
                degraded => format!("  ({degraded} scanning instead)"),
            }
        )?;
        for report in self.watch.iter().filter(|report| !report.watching) {
            if let Some(reason) = &report.degraded_reason {
                writeln!(out, "            {}: {reason}", report.project)?;
            }
        }
        Ok(())
    }
}

/// Render a duration the way a person reads one.
fn uptime(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3_599 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3_600, (seconds % 3_600) / 60),
    }
}

/// Result of starting a daemon in the background.
#[derive(Debug, Serialize)]
pub struct StartedReport {
    pub started: bool,
    pub pid: u32,
    pub port: u16,
}

impl Render for StartedReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out, "Daemon started (pid {}, port {})", self.pid, self.port)
    }
}

pub fn start<W: Write>(app: &App, detach: bool, printer: &mut Printer<W>) -> Result<()> {
    if detach {
        return start_detached(app, printer);
    }

    let options =
        DaemonOptions::from_config(&app.config, app.paths()).with_config_file(&app.config_file);

    // The daemon serves the dashboard, and the dashboard has a page describing
    // what `ctxc` can do. It is described here, from this binary's own command
    // tree, so the page can never list a command this build does not have.
    ctxc_api::commands::install(crate::commands::catalog::build());

    // Recorded so `ctxc stop` can find this process even though nobody here
    // spawned it; the guard clears the record when the daemon returns.
    let _recorded = crate::commands::record(app, "start");

    // Running in the foreground is the honest default: the process the user
    // started is the process doing the work.
    ctxc_daemon::run(app.config.clone(), options).context("the daemon stopped")?;
    Ok(())
}

/// Start a daemon in the background, for a command that needs one running.
///
/// `ctxc dashboard` should not make someone run `ctxc start` first, but it also
/// has its own thing to report, so this starts the daemon and says nothing.
pub fn start_for_dashboard(app: &App) -> Result<()> {
    let mut quiet = Printer::new(crate::output::OutputFormat::Quiet, io::sink());
    start_detached(app, &mut quiet)
}

/// Start a daemon as a separate process and wait for it to answer.
fn start_detached<W: Write>(app: &App, printer: &mut Printer<W>) -> Result<()> {
    if let DaemonState::Running { lock, .. } = ctxc_daemon::status(app.paths().data_dir())? {
        return Err(CliError::new(format!(
            "a daemon is already running (pid {}, port {})",
            lock.pid, lock.port
        ))
        .with_hint("run `ctxc status --daemon`, or stop it with `ctxc stop`")
        .into());
    }

    let executable = std::env::current_exe().context("failed to find the CtxC executable")?;
    let mut command = Command::new(executable);
    command
        .arg("start")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // The child must find the same data directory this process resolved,
    // whether it came from configuration or from the environment.
    command.env("CTXC_HOME", app.paths().data_dir());

    let child = spawn_detached(command)?;

    let started = wait_until_running(app)?;
    printer.emit(&StartedReport {
        started: true,
        pid: child.id(),
        port: started,
    })?;
    Ok(())
}

/// Start the daemon so that it outlives this process cleanly.
///
/// On Windows a child inherits the console by default, which keeps the parent's
/// pipes open for as long as the daemon runs — long enough to hang whatever
/// started it. `DETACHED_PROCESS` gives it no console at all, and its own
/// process group means a Ctrl-C in this terminal does not reach it.
#[cfg(windows)]
fn spawn_detached(mut command: Command) -> Result<std::process::Child> {
    use std::os::windows::process::CommandExt;

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    stop_leaking_our_handles();

    command
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .context("failed to start the daemon")
}

/// Keep the daemon from inheriting this process's standard handles.
///
/// `CreateProcess` inherits *every* inheritable handle, not only the three it
/// is told to use. So when `ctxc start --detach` is itself run with its output
/// captured — by a script, by CI, by `ctxc dashboard` under a test — the
/// daemon inherits the write end of that pipe and holds it open for its whole
/// life. The caller waits for end-of-file that only arrives when the daemon
/// exits, which is to say: it hangs.
///
/// Giving the daemon null stdio is not enough to prevent this; the handles have
/// to stop being inheritable before the spawn. Clearing the flag is safe here
/// because the daemon is being given no stdio at all, and best effort because a
/// console handle will simply decline.
#[cfg(windows)]
fn stop_leaking_our_handles() {
    use std::os::windows::io::AsRawHandle;

    extern "system" {
        fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
    }

    const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;

    let handles = [
        io::stdin().as_raw_handle(),
        io::stdout().as_raw_handle(),
        io::stderr().as_raw_handle(),
    ];

    for handle in handles {
        // SAFETY: each handle comes from the standard streams of this process,
        // which are valid for as long as the process is, and clearing an
        // inheritance flag cannot invalidate one.
        unsafe {
            SetHandleInformation(handle.cast(), HANDLE_FLAG_INHERIT, 0);
        }
    }
}

#[cfg(not(windows))]
fn spawn_detached(mut command: Command) -> Result<std::process::Child> {
    command.spawn().context("failed to start the daemon")
}

/// Wait for a freshly started daemon to answer, and report its port.
fn wait_until_running(app: &App) -> Result<u16> {
    const ATTEMPTS: u32 = 100;
    const PAUSE: std::time::Duration = std::time::Duration::from_millis(100);

    for _ in 0..ATTEMPTS {
        if let DaemonState::Running { lock, .. } = ctxc_daemon::status(app.paths().data_dir())? {
            return Ok(lock.port);
        }
        std::thread::sleep(PAUSE);
    }

    Err(CliError::new("the daemon did not start")
        .with_hint("run `ctxc start` in the foreground to see why")
        .into())
}

pub fn status<W: Write>(app: &App, printer: &mut Printer<W>) -> Result<()> {
    let state = ctxc_daemon::status(app.paths().data_dir())?;
    printer.emit(&DaemonReport::from_state(&state))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptimes_are_readable() {
        assert_eq!(uptime(0), "0s");
        assert_eq!(uptime(45_000), "45s");
        assert_eq!(uptime(90_000), "1m 30s");
        assert_eq!(uptime(7_320_000), "2h 2m");
    }
}
