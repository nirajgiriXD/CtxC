//! Diagnostic logging.
//!
//! Logs always go to stderr: stdout belongs to command output, which may be
//! machine readable. `CTXC_LOG` (falling back to `RUST_LOG`) overrides the
//! level chosen by the flags and accepts the usual `tracing` filter syntax.
//!
//! A second destination sits beside stderr: a bounded in-memory buffer the
//! daemon serves over `/v1/logs`. It carries its own filter, because the two
//! answer different questions. Stderr shows what the person running the command
//! asked to see, and defaults to near-silence. The buffer is what someone opens
//! a diagnostics panel to read *after* something went wrong, on a daemon
//! started with `--detach` that has no terminal at all — so it keeps CtxC's own
//! informational records whatever the flags said.

use std::io::IsTerminal;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::cli::Verbosity;

/// Environment variable checked before `RUST_LOG`.
pub const LOG_ENV: &str = "CTXC_LOG";

/// Install the process-wide subscriber.
pub fn init(verbosity: Verbosity) {
    let filter = EnvFilter::try_from_env(LOG_ENV)
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new(default_directive(verbosity)));

    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_target(matches!(verbosity, Verbosity::Debug))
        .without_time()
        .with_filter(filter);

    tracing_subscriber::registry()
        .with(stderr)
        .with(ctxc_api::logs::layer::capture().with_filter(kept_in_memory()))
        .init();
}

/// What the in-memory buffer keeps, regardless of the verbosity flags.
///
/// CtxC's own crates at `info`, everything else at `warn`: enough to see the
/// daemon start, index, watch and fall back to polling, without a diagnostics
/// panel filling up with somebody else's debug output.
fn kept_in_memory() -> EnvFilter {
    EnvFilter::new(
        "warn,ctxc=info,ctxc_api=info,ctxc_core=info,ctxc_daemon=info,ctxc_engine=info,ctxc_store=info,ctxc_watcher=info",
    )
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

    #[test]
    fn the_in_memory_filter_is_valid_and_keeps_ctxc_at_info() {
        let filter = kept_in_memory().to_string();
        assert!(filter.contains("ctxc_daemon=info"), "{filter}");
        EnvFilter::try_new(&filter).expect("the in-memory filter must parse");
    }
}
