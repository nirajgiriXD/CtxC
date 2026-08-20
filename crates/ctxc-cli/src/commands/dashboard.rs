//! `ctxc dashboard`
//!
//! Opens the local dashboard in the user's browser. The dashboard is served by
//! the daemon, from the same port as the API, so this command's whole job is:
//! make sure a daemon is running, build a URL that carries the token, and hand
//! it to the browser.
//!
//! The token is in the URL because a browser cannot be given a header to send.
//! It never leaves loopback, and anyone who can read the URL can already read
//! the lockfile it came from.

use std::io::{self, Write};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result};
use serde::Serialize;

use ctxc_core::Os;
use ctxc_daemon::DaemonState;

use crate::app::App;
use crate::error::CliError;
use crate::output::{human_bytes, Printer, Render};

/// Where the dashboard is, and whether it was opened.
#[derive(Debug, Serialize)]
pub struct DashboardReport {
    pub url: String,
    pub port: u16,
    /// False when `--no-open` was passed, or no browser could be launched.
    pub opened: bool,
    /// Whether this build carries the dashboard at all.
    pub bundled: bool,
    /// Size of the embedded dashboard.
    pub bytes: usize,
    /// True when this command started the daemon.
    pub started_daemon: bool,
}

impl Render for DashboardReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        if !self.bundled {
            writeln!(out, "This build of CtxC does not include the dashboard.")?;
            writeln!(out)?;
            writeln!(
                out,
                "Everything else works; only the web interface is missing."
            )?;
            return Ok(());
        }

        if self.started_daemon {
            writeln!(out, "Started the daemon on port {}.", self.port)?;
        }
        writeln!(out, "Dashboard:  {}", self.url)?;
        writeln!(out, "Size:       {}", human_bytes(self.bytes as u64))?;

        if !self.opened {
            writeln!(out)?;
            writeln!(out, "Open that URL in a browser. It carries the access")?;
            writeln!(out, "token, so keep it to yourself.")?;
        }
        Ok(())
    }
}

pub fn run<W: Write>(app: &App, open_browser: bool, printer: &mut Printer<W>) -> Result<()> {
    if !app.config.dashboard.enabled {
        return Err(CliError::new("the dashboard is disabled in configuration")
            .with_hint("set dashboard.enabled = true, or run `ctxc metrics` instead")
            .into());
    }

    // Report a build with no dashboard before starting anything: launching a
    // daemon to serve a page that does not exist helps nobody.
    if !ctxc_dashboard::is_bundled() {
        printer.emit(&DashboardReport {
            url: String::new(),
            port: 0,
            opened: false,
            bundled: false,
            bytes: 0,
            started_daemon: false,
        })?;
        return Ok(());
    }

    let (lock, started_daemon) = running_daemon(app)?;
    let token = lock
        .token()
        .ok_or_else(|| CliError::new("the daemon's lockfile has no access token"))?;
    let url = format!("http://127.0.0.1:{}/?token={}", lock.port, token.as_str());

    let opened = if open_browser {
        match open(&url) {
            Ok(()) => true,
            // Headless machines, containers, and locked-down desktops are all
            // normal. The URL is still the answer, so print it and carry on.
            Err(err) => {
                tracing::debug!(error = %err, "could not launch a browser");
                false
            }
        }
    } else {
        false
    };

    printer.emit(&DashboardReport {
        url,
        port: lock.port,
        opened,
        bundled: true,
        bytes: ctxc_dashboard::total_bytes(),
        started_daemon,
    })?;
    Ok(())
}

/// The lockfile of a running daemon, starting one if there is none.
///
/// Someone asking for the dashboard wants to look at it, not to be told to run
/// another command first. The bool says whether we started it, so the report
/// can mention it.
fn running_daemon(app: &App) -> Result<(ctxc_daemon::lock::Lock, bool)> {
    if let DaemonState::Running { lock, .. } = ctxc_daemon::status(app.paths().data_dir())? {
        return Ok((*lock, false));
    }

    if !app.config.daemon.enabled {
        return Err(
            CliError::new("the daemon is disabled, so nothing can serve the dashboard")
                .with_hint("set daemon.enabled = true in your configuration")
                .into(),
        );
    }

    tracing::info!("starting a daemon to serve the dashboard");
    crate::commands::daemon::start_for_dashboard(app)?;

    match ctxc_daemon::status(app.paths().data_dir())? {
        DaemonState::Running { lock, .. } => Ok((*lock, true)),
        _ => Err(CliError::new("the daemon did not start")
            .with_hint("run `ctxc start` and look at what it reports")
            .into()),
    }
}

/// Hand a URL to the desktop's default browser.
///
/// Every platform has its own launcher and none of them is worth a dependency.
/// The URL is built by this process from a port and a token, so nothing a user
/// typed reaches a shell.
fn open(url: &str) -> Result<()> {
    let mut command = match Os::current() {
        // `start` is a cmd builtin rather than a program, and its first
        // quoted argument is taken as a window title — hence the empty one.
        Os::Windows => {
            let mut command = Command::new("cmd");
            command.args(["/C", "start", "", url]);
            command
        }
        Os::MacOs => {
            let mut command = Command::new("open");
            command.arg(url);
            command
        }
        Os::Unix => {
            let mut command = Command::new("xdg-open");
            command.arg(url);
            command
        }
    };

    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to launch a browser")?;

    if !status.success() {
        return Err(CliError::new("the browser launcher reported a failure").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputFormat;

    fn render(report: &DashboardReport) -> String {
        let mut buffer = Vec::new();
        Printer::new(OutputFormat::Human, &mut buffer)
            .emit(report)
            .unwrap();
        String::from_utf8(buffer).unwrap()
    }

    fn report() -> DashboardReport {
        DashboardReport {
            url: "http://127.0.0.1:7717/?token=abc".into(),
            port: 7717,
            opened: true,
            bundled: true,
            bytes: 2048,
            started_daemon: false,
        }
    }

    #[test]
    fn a_build_without_a_dashboard_says_so_rather_than_printing_a_url() {
        let text = render(&DashboardReport {
            bundled: false,
            ..report()
        });
        assert!(text.contains("does not include the dashboard"), "{text}");
        assert!(
            !text.contains("127.0.0.1"),
            "a URL that cannot work must not be offered: {text}"
        );
    }

    #[test]
    fn a_url_that_could_not_be_opened_is_printed_with_instructions() {
        let text = render(&DashboardReport {
            opened: false,
            ..report()
        });
        assert!(text.contains("http://127.0.0.1:7717/?token=abc"), "{text}");
        assert!(text.contains("Open that URL"), "{text}");
    }

    #[test]
    fn starting_the_daemon_is_mentioned_rather_than_done_silently() {
        let text = render(&DashboardReport {
            started_daemon: true,
            ..report()
        });
        assert!(text.contains("Started the daemon on port 7717"), "{text}");
    }
}
