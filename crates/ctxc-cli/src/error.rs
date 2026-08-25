//! CLI-level errors and the shared failure output.

use std::fmt;

use ctxc_context::ContextError;
use ctxc_engine::EngineError;
use ctxc_store::StoreError;

/// An error raised by the CLI itself, carrying the suggestion to print under
/// `Try:`.
#[derive(Debug)]
pub struct CliError {
    message: String,
    hint: Option<String>,
}

impl CliError {
    pub fn new(message: impl Into<String>) -> Self {
        CliError {
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

/// Render a failure as "what failed / why / what to try".
///
/// Hints can come from any crate in the chain: core errors carry their own,
/// store errors are asked for one, and the CLI can attach one directly.
pub fn report(error: &anyhow::Error) -> String {
    let fallback = error.chain().find_map(hint_of);
    ctxc_core::report_with(error.as_ref(), fallback)
}

/// The hint one link in the chain can offer, whichever crate defined it.
fn hint_of(cause: &(dyn std::error::Error + 'static)) -> Option<String> {
    if let Some(cli) = cause.downcast_ref::<CliError>() {
        return cli.hint().map(str::to_owned);
    }
    if let Some(context) = cause.downcast_ref::<ContextError>() {
        return context.hint();
    }
    if let Some(engine) = cause.downcast_ref::<EngineError>() {
        return engine.hint();
    }
    if let Some(project) = cause.downcast_ref::<ctxc_project::ProjectError>() {
        return project.hint();
    }
    if let Some(daemon) = cause.downcast_ref::<ctxc_daemon::DaemonError>() {
        return daemon.hint();
    }
    if let Some(client) = cause.downcast_ref::<ctxc_api::client::ClientError>() {
        return client.hint();
    }
    if let Some(integration) = cause.downcast_ref::<ctxc_integrations::IntegrationError>() {
        return integration.hint();
    }
    cause
        .downcast_ref::<StoreError>()
        .and_then(StoreError::hint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_hints_are_reported() {
        let error = anyhow::Error::new(
            CliError::new("configuration file already exists")
                .with_hint("ctxc config init --force"),
        );
        let text = report(&error);
        assert!(
            text.starts_with("configuration file already exists"),
            "{text}"
        );
        assert!(text.contains("Try:\n  ctxc config init --force"), "{text}");
    }

    #[test]
    fn integration_hints_are_reported() {
        let error = anyhow::Error::new(ctxc_integrations::IntegrationError::Unknown {
            name: "emacs".into(),
        });

        let text = report(&error);
        assert!(text.starts_with("no integration named `emacs`"), "{text}");
        assert!(
            text.contains(
                "Try:
  run `ctxc config agents list`"
            ),
            "{text}"
        );
    }

    #[test]
    fn core_hints_survive_wrapping() {
        let error = anyhow::Error::new(ctxc_core::Error::ConfigValue {
            key: "daemon.port".into(),
            reason: "must be between 1 and 65535".into(),
        })
        .context("failed to load configuration");

        let text = report(&error);
        assert!(text.starts_with("failed to load configuration"), "{text}");
        assert!(
            text.contains("Reason:\n  invalid value for `daemon.port`"),
            "{text}"
        );
        assert!(text.contains("Try:\n  run `ctxc config show`"), "{text}");
    }
}
