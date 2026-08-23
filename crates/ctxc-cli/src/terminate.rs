//! Ending a process this one did not start.
//!
//! Seeing which CtxC processes exist is [`ctxc_core::processes`]'s job, and the
//! daemon does that too so a dashboard can report them. Ending one is only ever
//! done here, by `ctxc stop`, which is why the two halves live apart.
//!
//! The standard library can only kill a child, so each platform's own tool does
//! the work: `taskkill` on Windows, `kill` everywhere else. They are run
//! directly, never through a shell, and the only thing interpolated into them
//! is a number.
//!
//! A pid on its own is never enough. Operating systems reuse them, so a record
//! left behind by a process that has since exited can name something entirely
//! unrelated. Every pid is checked against the executable it is supposed to be
//! running before anything is sent to it, and a pid that fails the check is
//! treated as gone rather than as a target.

use std::process::{Command, Stdio};
use std::time::Duration;

use ctxc_core::processes::is_running;

/// What happened to one process.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It was running, and it is not any more.
    Terminated,
    /// It had already exited, or the pid now belongs to something else.
    Gone,
    /// It is still running, and why the attempt did not take.
    Failed(String),
}

/// How long to wait for a process to disappear after being asked to stop.
const PATIENCE: Duration = Duration::from_millis(100);
const ATTEMPTS: u32 = 50;

/// End the process `pid`, if it is still the CtxC process it claims to be.
pub fn terminate(pid: u32, executable: &str) -> Outcome {
    if !is_running(pid, executable) {
        return Outcome::Gone;
    }

    if let Err(reason) = kill(pid, executable) {
        // The process may have exited between the check and the kill, which is
        // the outcome we wanted anyway.
        if !is_running(pid, executable) {
            return Outcome::Gone;
        }
        return Outcome::Failed(reason);
    }

    for _ in 0..ATTEMPTS {
        if !is_running(pid, executable) {
            return Outcome::Terminated;
        }
        std::thread::sleep(PATIENCE);
    }
    Outcome::Failed("it is still running".to_string())
}

/// `taskkill /T` takes the process's children with it — something an MCP server
/// spawned must not outlive the process that owns it.
#[cfg(windows)]
fn kill(pid: u32, _executable: &str) -> Result<(), String> {
    let killed = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| format!("taskkill could not be run: {err}"))?;

    if killed.status.success() {
        return Ok(());
    }
    Err(first_line(&String::from_utf8_lossy(&killed.stderr))
        .unwrap_or_else(|| "taskkill refused".to_string()))
}

/// Ask first, insist second: a daemon asked to stop writes out what it holds.
#[cfg(not(windows))]
fn kill(pid: u32, executable: &str) -> Result<(), String> {
    let pid_text = pid.to_string();
    if signal("TERM", &pid_text).is_err() {
        return signal("KILL", &pid_text);
    }

    for _ in 0..ATTEMPTS {
        if !is_running(pid, executable) {
            return Ok(());
        }
        std::thread::sleep(PATIENCE);
    }
    signal("KILL", &pid_text)
}

#[cfg(not(windows))]
fn signal(name: &str, pid: &str) -> Result<(), String> {
    let sent = Command::new("kill")
        .args([format!("-{name}").as_str(), pid])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| format!("kill could not be run: {err}"))?;

    if sent.status.success() {
        return Ok(());
    }
    Err(first_line(&String::from_utf8_lossy(&sent.stderr))
        .unwrap_or_else(|| format!("SIG{name} was refused")))
}

/// The first non-empty line of a tool's diagnostics.
fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_reduced_to_one_line() {
        assert_eq!(first_line("\n\n  boom  \nmore\n").unwrap(), "boom");
        assert_eq!(first_line("   \n"), None);
    }

    #[test]
    fn a_pid_that_cannot_be_ours_is_reported_as_gone() {
        // This process is real, so the pid resolves; the executable does not
        // match, which is exactly the reused-pid case that must not be killed.
        assert_eq!(
            terminate(std::process::id(), "ctxc-not-a-real-binary"),
            Outcome::Gone
        );
    }
}
