//! Running a command and capturing its output.
//!
//! Commands are executed directly — no shell is involved, on any platform. That
//! is both a portability decision (never assume bash exists, and never depend on
//! PowerShell quoting rules) and a safety one: with no shell in the loop, the
//! arguments a user typed cannot turn into a pipeline, a redirect, or a second
//! command.
//!
//! Execution only ever happens because the user asked for it by running
//! `ctxc capture`. Nothing in an optimized context can cause a command to run.

use std::process::{Command, Stdio};

use crate::error::CliError;

/// What a captured command produced.
#[derive(Debug)]
pub struct Captured {
    /// The command as the user wrote it, for display and for routing.
    pub command_line: String,
    /// Exit code, or `None` if the process was killed by a signal.
    pub exit_code: Option<i32>,
    /// Standard output followed by standard error.
    ///
    /// The two streams are interleaved in a terminal but arrive here
    /// separately; concatenating them keeps every line, and diagnostics are
    /// what matter most, so they go last where they are easiest to find.
    pub output: String,
}

/// Run `command` with `args`, capturing both output streams.
pub fn capture(command: &str, args: &[String]) -> Result<Captured, CliError> {
    let command_line = describe(command, args);

    let output = run(command, args).map_err(|err| match err.kind() {
        std::io::ErrorKind::NotFound => CliError::new(format!("command not found: {command}"))
            .with_hint("check the name, or use the full path to the executable"),
        std::io::ErrorKind::PermissionDenied => {
            CliError::new(format!("not allowed to run {command}"))
                .with_hint("check the file's permissions")
        }
        _ => CliError::new(format!("failed to run {command_line}: {err}")),
    })?;

    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&stderr);
    }

    Ok(Captured {
        command_line,
        exit_code: output.status.code(),
        output: text,
    })
}

/// Spawn the process, with a Windows fallback for script-based tools.
fn run(command: &str, args: &[String]) -> std::io::Result<std::process::Output> {
    let direct = Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .output();

    match direct {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            fallback(command, args).unwrap_or(Err(err))
        }
        other => other,
    }
}

/// On Windows many tools ship as `.cmd` or `.bat` shims — `npm`, `yarn`, `tsc`.
/// Those cannot be started directly, so the command processor has to run them.
///
/// The fallback is used only after confirming that such a script really exists
/// on the PATH. Retrying through `cmd` unconditionally would turn "no such
/// command" into a successful capture of cmd's complaint, which is a far worse
/// answer than an honest error.
#[cfg(target_os = "windows")]
fn fallback(command: &str, args: &[String]) -> Option<std::io::Result<std::process::Output>> {
    let script = find_script(command)?;
    Some(
        Command::new("cmd")
            .arg("/C")
            .arg(script)
            .args(args)
            .stdin(Stdio::null())
            .output(),
    )
}

/// Locate `command` on the PATH as a batch script, if that is what it is.
#[cfg(target_os = "windows")]
fn find_script(command: &str) -> Option<std::path::PathBuf> {
    const SCRIPT_EXTENSIONS: [&str; 2] = ["cmd", "bat"];

    let named_directly = std::path::Path::new(command);
    if named_directly.components().count() > 1 {
        return has_script_extension(named_directly).then(|| named_directly.to_path_buf());
    }

    for directory in std::env::split_paths(&std::env::var_os("PATH")?) {
        for extension in SCRIPT_EXTENSIONS {
            let candidate = directory.join(format!("{command}.{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn has_script_extension(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
}

#[cfg(not(target_os = "windows"))]
fn fallback(_command: &str, _args: &[String]) -> Option<std::io::Result<std::process::Output>> {
    None
}

/// Render a command line for display, quoting arguments that need it.
fn describe(command: &str, args: &[String]) -> String {
    let mut out = String::from(command);
    for arg in args {
        out.push(' ');
        if arg.is_empty() || arg.chars().any(char::is_whitespace) {
            out.push('"');
            out.push_str(arg);
            out.push('"');
        } else {
            out.push_str(arg);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines_are_readable() {
        assert_eq!(describe("git", &["status".into()]), "git status");
        assert_eq!(
            describe("grep", &["two words".into(), "file.txt".into()]),
            "grep \"two words\" file.txt"
        );
        assert_eq!(describe("tool", &[String::new()]), "tool \"\"");
    }

    #[test]
    fn a_missing_command_is_reported_with_a_hint() {
        let error = capture("ctxc-definitely-not-a-real-command", &[]).unwrap_err();
        assert!(error.to_string().contains("command not found"));
        assert!(error.hint().unwrap().contains("full path"));
    }

    // Capturing a real process is exercised end to end in `tests/cli.rs`,
    // where the built `ctxc` binary is available to run as the subject.
}
