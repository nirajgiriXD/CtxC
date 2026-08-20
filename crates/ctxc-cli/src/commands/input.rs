//! Resolving what a command should read.
//!
//! Every content command accepts the same three things: a path, `-` for
//! standard input, or nothing at all — which also means standard input, so that
//! `git status | ctxc optimize` works without ceremony. Reading a terminal is
//! refused rather than left hanging, because a command that silently waits
//! forever is the worst possible failure mode.

use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Context as _, Result};

use ctxc_core::Context;

use crate::error::CliError;

/// The marker that means "standard input" wherever a path is accepted.
pub const STDIN_MARKER: &str = "-";

/// Load one input into a context.
pub fn load(input: Option<&Path>) -> Result<Context> {
    match input {
        _ if reads_stdin(input) => load_stdin(),
        Some(path) => ctxc_context::ingest::from_path(path)
            .with_context(|| format!("failed to read {}", path.display())),
        None => load_stdin(),
    }
}

/// Whether an input specification means standard input.
pub fn reads_stdin(input: Option<&Path>) -> bool {
    match input {
        None => true,
        Some(path) => path.as_os_str() == STDIN_MARKER,
    }
}

/// Load one input, attributing it to the command the user says produced it.
///
/// The attribution is what lets a tool-specific optimizer claim piped input:
/// CtxC cannot tell that bytes on standard input came from `git status`, but
/// the person running the pipeline can say so.
pub fn load_from(input: Option<&Path>, from: Option<&str>) -> Result<Context> {
    let context = load(input)?;
    let Some(command) = from else {
        return Ok(context);
    };

    Ok(ctxc_context::ingest::from_text(
        ctxc_core::ContextSource::Command {
            command: command.to_owned(),
        },
        &context.content,
        None,
    ))
}

fn load_stdin() -> Result<Context> {
    if std::io::stdin().is_terminal() {
        return Err(
            CliError::new("no input given, and standard input is a terminal")
                .with_hint("pass a file, or pipe data in:  git status | ctxc optimize")
                .into(),
        );
    }

    ctxc_context::ingest::from_stdin().context("failed to read standard input")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_missing_file_is_reported_with_its_path() {
        let error = load(Some(&PathBuf::from("no-such-input.txt"))).unwrap_err();
        assert!(error.to_string().contains("no-such-input.txt"));
    }

    #[test]
    fn the_stdin_marker_and_no_input_both_mean_stdin() {
        // Deliberately tests the decision rather than the read: a unit test
        // that actually reads standard input hangs whenever the harness holds
        // the pipe open, which is most of the time.
        assert!(reads_stdin(None));
        assert!(reads_stdin(Some(&PathBuf::from(STDIN_MARKER))));
        assert!(!reads_stdin(Some(&PathBuf::from("notes.txt"))));
        assert!(!reads_stdin(Some(&PathBuf::from("./-"))));
    }
}
