//! Diagnostic logging.
//!
//! Logs always go to stderr: stdout belongs to command output, which may be
//! machine readable. `CTXC_LOG` (falling back to `RUST_LOG`) overrides the
//! level chosen by the flags and accepts the usual `tracing` filter syntax.

use std::io::IsTerminal;

use tracing_subscriber::EnvFilter;

use crate::cli::Verbosity;

/// Environment variable checked before `RUST_LOG`.
pub const LOG_ENV: &str = "CTXC_LOG";

/// Install the process-wide subscriber.
pub fn init(verbosity: Verbosity) {
    let filter = EnvFilter::try_from_env(LOG_ENV)
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new(default_directive(verbosity)));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_target(matches!(verbosity, Verbosity::Debug))
        .without_time()
        .init();
}

/// Filter directive implied by the verbosity flags.
///
/// Third-party crates stay at `warn` even in debug mode, so `--debug` shows
/// CtxC's own reasoning rather than a wall of dependency output.
fn default_directive(verbosity: Verbosity) -> String {
    match verbosity {
        Verbosity::Warn => "warn".into(),
        Verbosity::Info => "warn,ctxc=info,ctxc_core=info,ctxc_store=info".into(),
        Verbosity::Debug => "warn,ctxc=debug,ctxc_core=debug,ctxc_store=debug".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directives_are_valid_filters() {
        for verbosity in [Verbosity::Warn, Verbosity::Info, Verbosity::Debug] {
            let directive = default_directive(verbosity);
            EnvFilter::try_new(&directive)
                .unwrap_or_else(|err| panic!("invalid filter {directive:?}: {err}"));
        }
    }

    #[test]
    fn quiet_by_default() {
        assert_eq!(default_directive(Verbosity::Warn), "warn");
    }
}
