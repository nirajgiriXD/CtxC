//! `ctxc stop`
//!
//! Ends everything CtxC is running for this data directory: the daemon first,
//! then the processes it does not own — the MCP servers an agent spawned, a
//! daemon left running in another terminal — and finally any records left
//! behind by processes that are already gone.
//!
//! The daemon is asked over its API rather than killed, so it finishes what it
//! is doing and removes its own lockfile. Everything else has no such channel,
//! so it is terminated, having first been checked to still be the CtxC process
//! its record claims.
//!
//! `ctxc daemon stop` remains the narrow one: it stops the daemon and nothing
//! else. `--all` widens this the other way, past the data directory to every
//! CtxC process on the machine.

use std::io::{self, Write};

use anyhow::Result;
use serde::Serialize;

use ctxc_core::processes as registry;
use ctxc_daemon::DaemonError;

use crate::app::App;
use crate::output::{Printer, Render};
use crate::terminate::{self, Outcome};

/// One process this command ended.
#[derive(Debug, Serialize)]
pub struct Ended {
    pub pid: u32,
    /// The command it was running, where a record said so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// One process this command could not end.
#[derive(Debug, Serialize)]
pub struct Survived {
    pub pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub reason: String,
}

/// What `ctxc stop` did.
#[derive(Debug, Default, Serialize)]
pub struct StopReport {
    /// The daemon that was shut down, if one was running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon: Option<u32>,
    /// True when all that was found was a lockfile left behind by a crash.
    pub stale_lock: bool,
    pub terminated: Vec<Ended>,
    /// Records cleared for processes that had already exited.
    pub cleared: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub survived: Vec<Survived>,
}

impl StopReport {
    /// Whether anything at all was running.
    fn found_something(&self) -> bool {
        self.daemon.is_some()
            || self.stale_lock
            || self.cleared > 0
            || !self.terminated.is_empty()
            || !self.survived.is_empty()
    }
}

impl Render for StopReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        match (self.daemon, self.stale_lock) {
            (Some(pid), true) => writeln!(out, "Cleared a lockfile left behind (pid {pid})")?,
            (Some(pid), false) => writeln!(out, "Daemon stopped (pid {pid})")?,
            (None, _) => writeln!(out, "Daemon:     not running")?,
        }

        if !self.terminated.is_empty() {
            writeln!(
                out,
                "Stopped {} other CtxC process(es)",
                self.terminated.len()
            )?;
            for process in &self.terminated {
                writeln!(
                    out,
                    "            pid {}{}",
                    process.pid,
                    described(&process.command)
                )?;
            }
        }

        if self.cleared > 0 {
            writeln!(
                out,
                "Cleared {} record(s) of processes already gone",
                self.cleared
            )?;
        }

        for process in &self.survived {
            writeln!(
                out,
                "Could not stop pid {}{}: {}",
                process.pid,
                described(&process.command),
                process.reason
            )?;
        }
        Ok(())
    }
}

/// The command a process was running, rendered for a list.
fn described(command: &Option<String>) -> String {
    command
        .as_deref()
        .map(|command| format!("  ctxc {command}"))
        .unwrap_or_default()
}

pub fn run<W: Write>(app: &App, all: bool, printer: &mut Printer<W>) -> Result<()> {
    let data_dir = app.paths().data_dir();
    let executable = registry::executable_name();
    let mut report = StopReport::default();

    // The daemon gets a clean shutdown; a lockfile with nothing behind it gets
    // cleared, which is what someone running `stop` after a crash wants.
    match ctxc_daemon::status(data_dir)? {
        ctxc_daemon::DaemonState::Stopped => {}
        ctxc_daemon::DaemonState::Stale { .. } => {
            report.daemon = Some(ctxc_daemon::stop(data_dir)?.pid);
            report.stale_lock = true;
        }
        ctxc_daemon::DaemonState::Running { .. } => {
            report.daemon = Some(ctxc_daemon::stop(data_dir)?.pid);
        }
    }

    // Never this process: `stop` has to survive long enough to say what it did.
    // Never the daemon either — it has already exited on its own terms, and its
    // pid may have been handed to something else by now.
    let mut handled: Vec<u32> = vec![std::process::id()];
    handled.extend(report.daemon);

    for entry in registry::entries(data_dir) {
        if handled.contains(&entry.pid) {
            registry::forget(data_dir, entry.pid);
            continue;
        }
        handled.push(entry.pid);

        match terminate::terminate(entry.pid, &entry.executable) {
            Outcome::Terminated => {
                registry::forget(data_dir, entry.pid);
                report.terminated.push(Ended {
                    pid: entry.pid,
                    command: Some(entry.command),
                });
            }
            Outcome::Gone => {
                registry::forget(data_dir, entry.pid);
                report.cleared += 1;
            }
            // The record stays, so a second `stop` can try again.
            Outcome::Failed(reason) => report.survived.push(Survived {
                pid: entry.pid,
                command: Some(entry.command),
                reason,
            }),
        }
    }

    if all {
        sweep(&executable, &mut handled, &mut report);
    }

    // Saying "stopped" when nothing was running would be a lie, and the hint is
    // the useful half of the answer.
    if !report.found_something() {
        return Err(DaemonError::NotRunning.into());
    }

    printer.emit(&report)?;
    Ok(())
}

/// Terminate every CtxC process on the machine, whatever data directory it
/// belongs to and whether or not it ever recorded itself.
///
/// This is the `--all` escape hatch, for processes that were killed hard enough
/// to leave no record, or that belong to an installation this one cannot name.
fn sweep(executable: &str, handled: &mut Vec<u32>, report: &mut StopReport) {
    for pid in registry::running(executable) {
        if handled.contains(&pid) {
            continue;
        }
        handled.push(pid);

        match terminate::terminate(pid, executable) {
            Outcome::Terminated => report.terminated.push(Ended { pid, command: None }),
            Outcome::Gone => {}
            Outcome::Failed(reason) => report.survived.push(Survived {
                pid,
                command: None,
                reason,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_running_is_not_a_stop() {
        assert!(!StopReport::default().found_something());
    }

    #[test]
    fn a_cleared_record_counts_as_having_found_something() {
        let report = StopReport {
            cleared: 1,
            ..StopReport::default()
        };
        assert!(report.found_something());
    }

    #[test]
    fn a_process_is_listed_with_the_command_it_was_running() {
        assert_eq!(described(&Some("mcp".to_string())), "  ctxc mcp");
        assert_eq!(described(&None), "");
    }
}
