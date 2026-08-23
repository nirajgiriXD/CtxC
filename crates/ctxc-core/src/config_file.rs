//! Editing the configuration file.
//!
//! [`Config::load`] reads layers; this writes one of them. It exists so that a
//! settings screen and `ctxc config` change a file the same way, through one
//! piece of code that knows the rules:
//!
//! * only the keys an edit names are touched, so setting one value never
//!   freezes every other default into the file;
//! * the result is validated before anything is written, so a rejected edit
//!   leaves the file exactly as it was;
//! * comments, key order and spacing survive, because the file belongs to
//!   whoever wrote it and an edit is not an excuse to reformat their notes.
//!
//! Only the file layer is editable. Values coming from `CTXC_*` variables win
//! over it, and pretending otherwise would let a settings screen report a
//! change the next run ignores — [`Edit::shadowed`] names those keys instead.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, Value};

use crate::config::{Config, PartialConfig};
use crate::error::{Error, Result};
use crate::platform::{Environment, Paths};

/// What an edit did.
#[derive(Debug, Clone)]
pub struct Edit {
    pub path: PathBuf,
    /// The file layer as it now stands.
    pub layer: PartialConfig,
    /// Keys whose value in the file changed, including ones that were reset.
    pub changed: Vec<String>,
    /// Keys this edit set that an environment variable still overrides.
    pub shadowed: Vec<String>,
    /// True when the file did not exist before.
    pub created: bool,
}

/// An edit to apply: values to set, and keys to hand back to the defaults.
#[derive(Debug, Default, Clone)]
pub struct Change {
    pub set: PartialConfig,
    /// Dotted keys to remove from the file, e.g. `daemon.port`.
    pub reset: Vec<String>,
}

impl Change {
    pub fn set(values: PartialConfig) -> Change {
        Change {
            set: values,
            reset: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.reset.is_empty()
    }
}

/// Apply `change` to the configuration file at `path`.
///
/// `env` is read only to report which keys the environment will keep
/// overriding; it never affects what is written.
pub fn apply(path: &Path, change: Change, env: &dyn Environment) -> Result<Edit> {
    let (text, created) = match std::fs::read_to_string(path) {
        Ok(text) => (text, false),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => (String::new(), true),
        Err(source) => {
            return Err(Error::Io {
                action: "read configuration file",
                path: path.to_path_buf(),
                source,
            })
        }
    };

    // Parsing through PartialConfig first means a file this build cannot
    // understand is refused with the same message `ctxc config show` gives,
    // rather than being quietly rewritten into something it is not.
    let before = PartialConfig::from_toml(&text, path)?;

    let mut layer = before.clone();
    layer.overlay(change.set.clone());
    for key in &change.reset {
        if !layer.clear(key) {
            return Err(Error::ConfigValue {
                key: key.clone(),
                reason: "not a configuration key".into(),
            });
        }
    }

    // Validate the file layer on its own, on top of the defaults. Folding in
    // the environment would let a machine-specific variable fail an otherwise
    // fine edit, or hide one that is broken.
    let mut effective = Config::default();
    effective.merge(layer.clone());
    effective.validate()?;

    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|source| Error::ConfigValue {
            // Unreachable in practice: `PartialConfig::from_toml` has already
            // accepted this text with the same parser underneath. Reported
            // rather than unwrapped, because a daemon must not die of one.
            key: path.display().to_string(),
            reason: source.to_string(),
        })?;

    let changed = write_into(&mut document, &change, &before);

    if let Some(parent) = path.parent() {
        Paths::ensure_dir(parent)?;
    }

    let mut rendered = document.to_string();
    if created {
        rendered.insert_str(0, HEADER);
    }
    std::fs::write(path, rendered).map_err(|source| Error::Io {
        action: "write configuration file",
        path: path.to_path_buf(),
        source,
    })?;

    let overridden = PartialConfig::from_env(env)?.set_keys();
    let shadowed = changed
        .iter()
        .filter(|key| overridden.contains(&key.as_str()))
        .cloned()
        .collect();

    Ok(Edit {
        path: path.to_path_buf(),
        layer,
        changed,
        shadowed,
        created,
    })
}

/// The comment a file created by an edit opens with.
const HEADER: &str = "# CtxC configuration.\n\
                      # Only settings that differ from the built-in defaults are written here;\n\
                      # `ctxc config show` prints the full effective configuration.\n\n";

/// Apply the change to the document, returning the keys that actually moved.
fn write_into(document: &mut DocumentMut, change: &Change, before: &PartialConfig) -> Vec<String> {
    let mut changed = Vec::new();

    macro_rules! sync {
        ($($section:ident . $field:ident),+ $(,)?) => {
            $(if let Some(value) = &change.set.$section.$field {
                if before.$section.$field.as_ref() != Some(value) {
                    changed.push(concat!(stringify!($section), ".", stringify!($field)).to_string());
                }
                set(
                    document,
                    stringify!($section),
                    stringify!($field),
                    value.to_toml(),
                );
            })+
        };
    }

    sync!(
        core.mode,
        optimization.enabled,
        optimization.target_reduction,
        retrieval.enabled,
        ranking.keyword,
        ranking.semantic,
        ranking.symbol,
        ranking.graph,
        ranking.recency,
        ranking.hop_decay,
        ranking.expansion_depth,
        ranking.recency_half_life_days,
        graph.enabled,
        storage.path,
        budget.default,
        daemon.enabled,
        daemon.auto_start,
        daemon.bind,
        daemon.port,
        watch.enabled,
        watch.debounce_ms,
        watch.poll_interval_ms,
        semantic.enabled,
        semantic.provider,
        semantic.dimensions,
        semantic.redundancy_threshold,
        semantic.diversity,
        metrics.enabled,
        metrics.raw_retention_days,
        metrics.hourly_retention_days,
        metrics.cost_model,
        metrics.cost_per_million_input_tokens,
        metrics.cost_currency,
        dashboard.enabled,
        dashboard.port,
        telemetry.enabled,
    );

    for key in &change.reset {
        if let Some((section, field)) = key.split_once('.') {
            if remove(document, section, field) {
                changed.push(key.clone());
            }
        }
    }

    changed
}

/// How a configuration value is spelled in a TOML document.
///
/// `toml_edit` converts the types TOML has; CtxC also stores port and count
/// values as `u16`/`u32`, which are integers once written down.
trait ToToml {
    fn to_toml(&self) -> Value;
}

impl ToToml for String {
    fn to_toml(&self) -> Value {
        Value::from(self.as_str())
    }
}

impl ToToml for bool {
    fn to_toml(&self) -> Value {
        Value::from(*self)
    }
}

impl ToToml for f64 {
    fn to_toml(&self) -> Value {
        Value::from(*self)
    }
}

impl ToToml for u32 {
    fn to_toml(&self) -> Value {
        Value::from(i64::from(*self))
    }
}

impl ToToml for u16 {
    fn to_toml(&self) -> Value {
        Value::from(i64::from(*self))
    }
}

/// Set one key, creating its table if the file has not got one yet.
fn set(document: &mut DocumentMut, section: &str, key: &str, value: Value) {
    let entry = document.entry(section).or_insert(Item::Table(Table::new()));

    if let Item::Table(table) = entry {
        // A table CtxC created should read like one a person wrote: a header on
        // its own line rather than an inline `{ ... }` blob.
        table.set_implicit(false);
        table[key] = Item::Value(value);
    }
}

/// Remove one key, and the table with it when nothing is left.
fn remove(document: &mut DocumentMut, section: &str, key: &str) -> bool {
    let Some(Item::Table(table)) = document.get_mut(section) else {
        return false;
    };

    let removed = table.remove(key).is_some();
    if table.is_empty() {
        document.remove(section);
    }
    removed
}

/// Every dotted key a file layer can hold.
///
/// The settings surface and the reset request are checked against this, so a
/// key that exists in one place exists in both.
pub fn keys() -> Vec<&'static str> {
    let mut every = PartialConfig::default();
    fill(&mut every);
    every.set_keys()
}

/// Set every key to something, so [`PartialConfig::set_keys`] enumerates them.
fn fill(config: &mut PartialConfig) {
    config.core.mode = Some(String::new());
    config.optimization.enabled = Some(false);
    config.optimization.target_reduction = Some(0.0);
    config.retrieval.enabled = Some(false);
    config.ranking.keyword = Some(0.0);
    config.ranking.semantic = Some(0.0);
    config.ranking.symbol = Some(0.0);
    config.ranking.graph = Some(0.0);
    config.ranking.recency = Some(0.0);
    config.ranking.hop_decay = Some(0.0);
    config.ranking.expansion_depth = Some(0);
    config.ranking.recency_half_life_days = Some(0.0);
    config.graph.enabled = Some(false);
    config.storage.path = Some(String::new());
    config.budget.default = Some(0);
    config.daemon.enabled = Some(false);
    config.daemon.auto_start = Some(false);
    config.daemon.bind = Some(String::new());
    config.daemon.port = Some(0);
    config.watch.enabled = Some(false);
    config.watch.debounce_ms = Some(0);
    config.watch.poll_interval_ms = Some(0);
    config.semantic.enabled = Some(false);
    config.semantic.provider = Some(String::new());
    config.semantic.dimensions = Some(0);
    config.semantic.redundancy_threshold = Some(0.0);
    config.semantic.diversity = Some(0.0);
    config.metrics.enabled = Some(false);
    config.metrics.raw_retention_days = Some(0);
    config.metrics.hourly_retention_days = Some(0);
    config.metrics.cost_model = Some(String::new());
    config.metrics.cost_per_million_input_tokens = Some(0.0);
    config.metrics.cost_currency = Some(String::new());
    config.dashboard.enabled = Some(false);
    config.dashboard.port = Some(0);
    config.telemetry.enabled = Some(false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::MapEnvironment;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!(
                "ctxc-config-file-{}-{name}-{}",
                std::process::id(),
                crate::Timestamp::now().as_millis()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("config.toml")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn change(build: impl FnOnce(&mut PartialConfig)) -> Change {
        let mut values = PartialConfig::default();
        build(&mut values);
        Change::set(values)
    }

    #[test]
    fn an_edit_creates_the_file_and_writes_only_what_it_was_given() {
        let scratch = Scratch::new("create");
        let env = MapEnvironment::default();

        let edit = apply(
            &scratch.file(),
            change(|values| values.daemon.port = Some(9000)),
            &env,
        )
        .unwrap();

        assert!(edit.created);
        assert_eq!(edit.changed, vec!["daemon.port"]);

        let text = std::fs::read_to_string(scratch.file()).unwrap();
        assert!(text.contains("port = 9000"), "{text}");
        assert!(
            !text.contains("debounce_ms"),
            "an edit must not write settings nobody asked about:\n{text}"
        );
    }

    #[test]
    fn comments_and_untouched_keys_survive_an_edit() {
        let scratch = Scratch::new("comments");
        let env = MapEnvironment::default();
        std::fs::write(
            scratch.file(),
            "# my notes\n[watch]\n# quiet period\ndebounce_ms = 500\n\n[daemon]\nport = 7000\n",
        )
        .unwrap();

        apply(
            &scratch.file(),
            change(|values| values.daemon.port = Some(7001)),
            &env,
        )
        .unwrap();

        let text = std::fs::read_to_string(scratch.file()).unwrap();
        assert!(text.contains("# my notes"), "{text}");
        assert!(text.contains("# quiet period"), "{text}");
        assert!(text.contains("debounce_ms = 500"), "{text}");
        assert!(text.contains("port = 7001"), "{text}");
    }

    #[test]
    fn a_reset_removes_the_key_and_the_table_it_emptied() {
        let scratch = Scratch::new("reset");
        let env = MapEnvironment::default();
        std::fs::write(scratch.file(), "[daemon]\nport = 7000\n").unwrap();

        let edit = apply(
            &scratch.file(),
            Change {
                set: PartialConfig::default(),
                reset: vec!["daemon.port".into()],
            },
            &env,
        )
        .unwrap();

        assert_eq!(edit.changed, vec!["daemon.port"]);
        let text = std::fs::read_to_string(scratch.file()).unwrap();
        assert!(!text.contains("daemon"), "{text}");
        assert!(edit.layer.daemon.port.is_none());
    }

    #[test]
    fn resetting_a_key_that_is_not_one_is_refused() {
        let scratch = Scratch::new("unknown-reset");
        let env = MapEnvironment::default();

        let error = apply(
            &scratch.file(),
            Change {
                set: PartialConfig::default(),
                reset: vec!["daemon.prot".into()],
            },
            &env,
        )
        .unwrap_err();

        assert!(matches!(error, Error::ConfigValue { ref key, .. } if key == "daemon.prot"));
    }

    #[test]
    fn a_rejected_edit_leaves_the_file_alone() {
        let scratch = Scratch::new("invalid");
        let env = MapEnvironment::default();
        std::fs::write(scratch.file(), "[budget]\ndefault = 1000\n").unwrap();

        let error = apply(
            &scratch.file(),
            change(|values| values.budget.default = Some(0)),
            &env,
        )
        .unwrap_err();

        assert!(matches!(error, Error::ConfigValue { ref key, .. } if key == "budget.default"));
        assert_eq!(
            std::fs::read_to_string(scratch.file()).unwrap(),
            "[budget]\ndefault = 1000\n",
            "nothing should have been written"
        );
    }

    #[test]
    fn an_edit_the_environment_overrides_says_so() {
        let scratch = Scratch::new("shadowed");
        let env = MapEnvironment::new([("CTXC_DAEMON_PORT", "7100")]);

        let edit = apply(
            &scratch.file(),
            change(|values| values.daemon.port = Some(9000)),
            &env,
        )
        .unwrap();

        assert_eq!(edit.shadowed, vec!["daemon.port"]);
    }

    #[test]
    fn writing_the_same_value_twice_changes_nothing() {
        let scratch = Scratch::new("idempotent");
        let env = MapEnvironment::default();

        apply(
            &scratch.file(),
            change(|values| values.watch.debounce_ms = Some(400)),
            &env,
        )
        .unwrap();
        let edit = apply(
            &scratch.file(),
            change(|values| values.watch.debounce_ms = Some(400)),
            &env,
        )
        .unwrap();

        assert!(edit.changed.is_empty());
        assert!(!edit.created);
    }

    #[test]
    fn a_created_file_explains_itself() {
        let scratch = Scratch::new("header");
        let env = MapEnvironment::default();

        apply(
            &scratch.file(),
            change(|values| values.telemetry.enabled = Some(false)),
            &env,
        )
        .unwrap();

        let text = std::fs::read_to_string(scratch.file()).unwrap();
        assert!(text.starts_with("# CtxC configuration."), "{text}");
    }

    #[test]
    fn every_key_can_be_named_for_a_reset() {
        let names = keys();
        assert!(names.contains(&"daemon.port"));
        assert!(names.contains(&"metrics.cost_currency"));
        assert_eq!(names.len(), 36, "keys() must cover the whole file layer");
    }
}
