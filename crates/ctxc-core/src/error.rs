//! Typed errors for the core crate, plus the shared "actionable error" report
//! format used by every CtxC binary.

use std::path::PathBuf;

use thiserror::Error;

/// Convenience alias used throughout the core crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors produced by configuration loading and platform path resolution.
#[derive(Debug, Error)]
pub enum Error {
    /// A filesystem operation failed. `action` reads as a verb phrase, e.g.
    /// "read configuration file".
    #[error("failed to {action}: {}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A TOML document could not be parsed or contained unknown keys.
    #[error("invalid configuration file: {}", path.display())]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },

    /// A configuration value was syntactically valid but semantically wrong.
    #[error("invalid value for `{key}`: {reason}")]
    ConfigValue { key: String, reason: String },

    /// A platform directory could not be resolved from the environment.
    #[error("cannot determine the {kind} directory for this platform")]
    MissingDirectory {
        kind: &'static str,
        /// Environment variables that would have answered the question.
        variables: &'static str,
    },

    /// A string was not a well-formed context identifier.
    #[error("not a valid context id: {0}")]
    InvalidContextId(String),
}

impl Error {
    /// A short, concrete next step for the user. Shown under `Try:`.
    pub fn hint(&self) -> Option<String> {
        match self {
            Error::Io { path, source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied => {
                Some(format!("check the permissions on {}", path.display()))
            }
            Error::Io { .. } => None,
            Error::ConfigParse { path, .. } => Some(format!(
                "run `ctxc config show` after fixing {}, or delete the file to fall back to defaults",
                path.display()
            )),
            Error::ConfigValue { key, .. } => {
                Some(format!("run `ctxc config show` and correct `{key}`"))
            }
            Error::MissingDirectory { variables, .. } => Some(format!(
                "set CTXC_HOME, or ensure one of these is set: {variables}"
            )),
            Error::InvalidContextId(_) => {
                Some("context ids are 32 lowercase hex characters, e.g. ctxc://context/<id>".into())
            }
        }
    }
}

/// Render an error the way CtxC presents failures to users: what failed, why,
/// and what to do about it.
///
/// ```text
/// Failed to open context database: /path/to/ctxc.db
///
/// Reason:
///   database is locked
///
/// Try:
///   ctxc status
/// ```
pub fn report(error: &(dyn std::error::Error + 'static)) -> String {
    report_with(error, None)
}

/// Like [`report`], with a hint supplied by a crate that defines its own error
/// type. The chain is still searched first, so a core hint wins when both are
/// available.
pub fn report_with(
    error: &(dyn std::error::Error + 'static),
    fallback_hint: Option<String>,
) -> String {
    let mut out = error.to_string();

    let mut reasons = Vec::new();
    let mut source = error.source();
    while let Some(current) = source {
        reasons.push(current.to_string());
        source = current.source();
    }
    if !reasons.is_empty() {
        out.push_str("\n\nReason:");
        for reason in reasons {
            out.push_str("\n  ");
            out.push_str(&reason);
        }
    }

    if let Some(hint) = find_hint(error).or(fallback_hint) {
        out.push_str("\n\nTry:\n  ");
        out.push_str(&hint);
    }

    out
}

/// Walk the source chain looking for a core error that carries a hint.
fn find_hint(error: &(dyn std::error::Error + 'static)) -> Option<String> {
    let mut current = Some(error);
    while let Some(err) = current {
        if let Some(core) = err.downcast_ref::<Error>() {
            if let Some(hint) = core.hint() {
                return Some(hint);
            }
        }
        current = err.source();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Error)]
    #[error("failed to open context database")]
    struct Wrapper(#[source] Error);

    #[test]
    fn report_includes_reason_and_hint() {
        let inner = Error::InvalidContextId("nope".into());
        let text = report(&Wrapper(inner));

        assert!(text.starts_with("failed to open context database"));
        assert!(text.contains("Reason:\n  not a valid context id: nope"));
        assert!(text.contains("Try:\n  context ids are 32 lowercase hex"));
    }

    #[test]
    fn report_without_source_is_single_line() {
        let text = report(&Error::ConfigValue {
            key: "daemon.port".into(),
            reason: "must be between 1 and 65535".into(),
        });
        assert_eq!(
            text,
            "invalid value for `daemon.port`: must be between 1 and 65535\n\nTry:\n  run `ctxc config show` and correct `daemon.port`"
        );
    }
}
