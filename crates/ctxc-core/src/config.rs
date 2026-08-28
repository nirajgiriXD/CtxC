//! Layered configuration.
//!
//! Precedence, lowest to highest:
//!
//! ```text
//! built-in defaults -> config files (in order) -> environment -> explicit overrides
//! ```
//!
//! Every layer is a [`PartialConfig`]: a document where each key is optional.
//! Layers are merged key by key, so a project file that sets one value does not
//! reset the rest. Files are TOML and reject unknown keys, which turns a typo
//! into an actionable error instead of a silently ignored setting.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::platform::{Environment, Paths};
use crate::token::TokenBudget;

/// Prefix for configuration environment variables: `CTXC_<SECTION>_<KEY>`.
pub const ENV_PREFIX: &str = "CTXC";

/// Effective configuration after all layers have been merged.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Config {
    pub core: CoreConfig,
    pub optimization: OptimizationConfig,
    pub retrieval: RetrievalConfig,
    pub ranking: RankingConfig,
    pub graph: GraphConfig,
    pub storage: StorageConfig,
    pub budget: BudgetConfig,
    pub daemon: DaemonConfig,
    pub watch: WatchConfig,
    pub semantic: SemanticConfig,
    pub metrics: MetricsConfig,
    pub dashboard: DashboardConfig,
    pub telemetry: TelemetryConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoreConfig {
    /// Processing mode. Only `local` exists today; the key exists so that
    /// enabling remote assistance is a configuration change, not a redesign.
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OptimizationConfig {
    pub enabled: bool,
    /// Reduction the engine aims for, as a fraction of the input.
    pub target_reduction: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RetrievalConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct RankingConfig {
    /// Weight of full-text relevance.
    pub keyword: f64,
    /// Weight of embedding similarity. Zero — the default — leaves ranking
    /// exactly as it is when embeddings are off.
    pub semantic: f64,
    /// Weight of a matching symbol name.
    pub symbol: f64,
    /// Weight of how much the project depends on a file.
    pub graph: f64,
    /// Weight of how recently a file changed.
    pub recency: f64,
    /// Score multiplier per dependency-graph hop from a direct match.
    pub hop_decay: f64,
    /// How far to follow the graph from a match. Zero disables expansion.
    pub expansion_depth: u32,
    /// Days after which a file counts as half as recent.
    pub recency_half_life_days: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StorageConfig {
    /// `auto` resolves to the platform data directory; anything else is used
    /// as the database path verbatim.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BudgetConfig {
    /// Token budget applied when a caller does not supply one.
    pub default: u32,
    /// Which tokenizer counts against that budget.
    ///
    /// `"heuristic"` is the estimator CtxC ships: no vocabulary, no download,
    /// and honest about being approximate. `"cl100k"` is the exact byte-pair
    /// encoder GPT-4 and friends use, available only when this build was
    /// compiled with the `cl100k` feature — asking for it otherwise is a
    /// configuration error rather than a silent fall back to the estimate.
    pub tokenizer: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DaemonConfig {
    pub enabled: bool,
    pub auto_start: bool,
    pub bind: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WatchConfig {
    pub enabled: bool,
    /// Quiet period before a changed file is acted on.
    pub debounce_ms: u32,
    /// How often a project that cannot be watched is scanned instead.
    pub poll_interval_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticConfig {
    /// Whether CtxC computes embeddings at all.
    ///
    /// Off by default. Embedding costs index time and database size, and every
    /// deterministic path — search, ranking, optimization — works without it.
    pub enabled: bool,
    /// Which embedder to use. `hashed` needs no model and no network.
    pub provider: String,
    pub dimensions: u32,
    /// Similarity at which two pieces of text count as saying the same thing.
    pub redundancy_threshold: f64,
    /// How much relevance to trade for coverage when selecting results.
    pub diversity: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetricsConfig {
    pub enabled: bool,
    /// Days of raw per-operation events to keep. Zero keeps them forever.
    pub raw_retention_days: u32,
    /// Days of hourly aggregates to keep. Daily aggregates are the long-term
    /// record and are never pruned.
    pub hourly_retention_days: u32,
    /// The model cost estimates assume, shown next to every figure.
    pub cost_model: String,
    /// Price of a million input tokens. Zero — the default — means CtxC has no
    /// rate to work from and reports no cost at all, which is the only honest
    /// answer to a question nobody has configured.
    pub cost_per_million_input_tokens: f64,
    pub cost_currency: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DashboardConfig {
    pub enabled: bool,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TelemetryConfig {
    /// External transmission. Off by default and unrelated to local metrics.
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            core: CoreConfig {
                mode: "local".into(),
            },
            optimization: OptimizationConfig {
                enabled: true,
                target_reduction: 0.5,
            },
            retrieval: RetrievalConfig { enabled: true },
            ranking: RankingConfig {
                keyword: 1.0,
                semantic: 0.0,
                symbol: 1.5,
                graph: 0.5,
                recency: 0.3,
                hop_decay: 0.4,
                expansion_depth: 1,
                recency_half_life_days: 30.0,
            },
            graph: GraphConfig { enabled: true },
            storage: StorageConfig {
                path: "auto".into(),
            },
            budget: BudgetConfig {
                default: 32_000,
                tokenizer: "heuristic".into(),
            },
            daemon: DaemonConfig {
                enabled: true,
                auto_start: true,
                bind: "127.0.0.1".into(),
                port: 7717,
            },
            watch: WatchConfig {
                enabled: true,
                debounce_ms: 300,
                poll_interval_ms: 30_000,
            },
            semantic: SemanticConfig {
                enabled: false,
                provider: "hashed".into(),
                dimensions: 256,
                redundancy_threshold: 0.92,
                diversity: 0.25,
            },
            metrics: MetricsConfig {
                enabled: true,
                raw_retention_days: 30,
                hourly_retention_days: 90,
                cost_model: "unspecified".into(),
                cost_per_million_input_tokens: 0.0,
                cost_currency: "USD".into(),
            },
            dashboard: DashboardConfig {
                enabled: true,
                port: 7718,
            },
            telemetry: TelemetryConfig { enabled: false },
        }
    }
}

/// One configuration layer: every key optional.
///
/// Deserialized from a file or built from the environment, and serialized back
/// when something edits the file: a layer records the keys someone actually set,
/// so writing one out never turns a default into a decision.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialConfig {
    #[serde(default, skip_serializing_if = "PartialCore::is_empty")]
    pub core: PartialCore,
    #[serde(default, skip_serializing_if = "PartialOptimization::is_empty")]
    pub optimization: PartialOptimization,
    #[serde(default, skip_serializing_if = "PartialToggle::is_empty")]
    pub retrieval: PartialToggle,
    #[serde(default, skip_serializing_if = "PartialRanking::is_empty")]
    pub ranking: PartialRanking,
    #[serde(default, skip_serializing_if = "PartialToggle::is_empty")]
    pub graph: PartialToggle,
    #[serde(default, skip_serializing_if = "PartialStorage::is_empty")]
    pub storage: PartialStorage,
    #[serde(default, skip_serializing_if = "PartialBudget::is_empty")]
    pub budget: PartialBudget,
    #[serde(default, skip_serializing_if = "PartialDaemon::is_empty")]
    pub daemon: PartialDaemon,
    #[serde(default, skip_serializing_if = "PartialWatch::is_empty")]
    pub watch: PartialWatch,
    #[serde(default, skip_serializing_if = "PartialSemantic::is_empty")]
    pub semantic: PartialSemantic,
    #[serde(default, skip_serializing_if = "PartialMetrics::is_empty")]
    pub metrics: PartialMetrics,
    #[serde(default, skip_serializing_if = "PartialDashboard::is_empty")]
    pub dashboard: PartialDashboard,
    #[serde(default, skip_serializing_if = "PartialToggle::is_empty")]
    pub telemetry: PartialToggle,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialCore {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialOptimization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_reduction: Option<f64>,
}

/// Sections whose only knob is `enabled`.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialToggle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialRanking {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recency: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hop_decay: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expansion_depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recency_half_life_days: Option<f64>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialStorage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialBudget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokenizer: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialDaemon {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_start: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialWatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debounce_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_interval_ms: Option<u32>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialSemantic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redundancy_threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diversity: Option<f64>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_retention_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hourly_retention_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_per_million_input_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_currency: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartialDashboard {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// Whether a section carries no decisions at all.
///
/// Used to keep empty tables out of a written file: a `[semantic]` header with
/// nothing under it says something was configured when nothing was.
macro_rules! empty_when_unset {
    ($type:ty, $($field:ident),+ $(,)?) => {
        impl $type {
            pub fn is_empty(&self) -> bool {
                $(self.$field.is_none())&&+
            }
        }
    };
}

empty_when_unset!(PartialCore, mode);
empty_when_unset!(PartialOptimization, enabled, target_reduction);
empty_when_unset!(PartialToggle, enabled);
empty_when_unset!(
    PartialRanking,
    keyword,
    semantic,
    symbol,
    graph,
    recency,
    hop_decay,
    expansion_depth,
    recency_half_life_days,
);
empty_when_unset!(PartialStorage, path);
empty_when_unset!(PartialBudget, default);
empty_when_unset!(PartialDaemon, enabled, auto_start, bind, port);
empty_when_unset!(PartialWatch, enabled, debounce_ms, poll_interval_ms);
empty_when_unset!(
    PartialSemantic,
    enabled,
    provider,
    dimensions,
    redundancy_threshold,
    diversity,
);
empty_when_unset!(
    PartialMetrics,
    enabled,
    raw_retention_days,
    hourly_retention_days,
    cost_model,
    cost_per_million_input_tokens,
    cost_currency,
);
empty_when_unset!(PartialDashboard, enabled, port);

impl PartialConfig {
    /// Whether this layer decides nothing.
    pub fn is_empty(&self) -> bool {
        self.set_keys().is_empty()
    }

    /// The dotted keys this layer sets, lowest section first.
    ///
    /// A settings screen needs this to say *why* a value is what it is: a key
    /// set by the environment cannot be changed by editing the file, and
    /// showing it as editable would be a lie.
    pub fn set_keys(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        let mut note = |set: bool, key: &'static str| {
            if set {
                keys.push(key);
            }
        };

        note(self.core.mode.is_some(), "core.mode");
        note(self.optimization.enabled.is_some(), "optimization.enabled");
        note(
            self.optimization.target_reduction.is_some(),
            "optimization.target_reduction",
        );
        note(self.retrieval.enabled.is_some(), "retrieval.enabled");
        note(self.ranking.keyword.is_some(), "ranking.keyword");
        note(self.ranking.semantic.is_some(), "ranking.semantic");
        note(self.ranking.symbol.is_some(), "ranking.symbol");
        note(self.ranking.graph.is_some(), "ranking.graph");
        note(self.ranking.recency.is_some(), "ranking.recency");
        note(self.ranking.hop_decay.is_some(), "ranking.hop_decay");
        note(
            self.ranking.expansion_depth.is_some(),
            "ranking.expansion_depth",
        );
        note(
            self.ranking.recency_half_life_days.is_some(),
            "ranking.recency_half_life_days",
        );
        note(self.graph.enabled.is_some(), "graph.enabled");
        note(self.storage.path.is_some(), "storage.path");
        note(self.budget.default.is_some(), "budget.default");
        note(self.daemon.enabled.is_some(), "daemon.enabled");
        note(self.daemon.auto_start.is_some(), "daemon.auto_start");
        note(self.daemon.bind.is_some(), "daemon.bind");
        note(self.daemon.port.is_some(), "daemon.port");
        note(self.watch.enabled.is_some(), "watch.enabled");
        note(self.watch.debounce_ms.is_some(), "watch.debounce_ms");
        note(
            self.watch.poll_interval_ms.is_some(),
            "watch.poll_interval_ms",
        );
        note(self.semantic.enabled.is_some(), "semantic.enabled");
        note(self.semantic.provider.is_some(), "semantic.provider");
        note(self.semantic.dimensions.is_some(), "semantic.dimensions");
        note(
            self.semantic.redundancy_threshold.is_some(),
            "semantic.redundancy_threshold",
        );
        note(self.semantic.diversity.is_some(), "semantic.diversity");
        note(self.metrics.enabled.is_some(), "metrics.enabled");
        note(
            self.metrics.raw_retention_days.is_some(),
            "metrics.raw_retention_days",
        );
        note(
            self.metrics.hourly_retention_days.is_some(),
            "metrics.hourly_retention_days",
        );
        note(self.metrics.cost_model.is_some(), "metrics.cost_model");
        note(
            self.metrics.cost_per_million_input_tokens.is_some(),
            "metrics.cost_per_million_input_tokens",
        );
        note(
            self.metrics.cost_currency.is_some(),
            "metrics.cost_currency",
        );
        note(self.dashboard.enabled.is_some(), "dashboard.enabled");
        note(self.dashboard.port.is_some(), "dashboard.port");
        note(self.telemetry.enabled.is_some(), "telemetry.enabled");

        keys
    }

    /// Lay `update` on top of this layer, key by key.
    ///
    /// Only keys `update` actually sets are touched, for the same reason
    /// [`Config::merge`] works that way: an edit to one setting must not quietly
    /// promote every other default into a written decision.
    pub fn overlay(&mut self, update: PartialConfig) {
        macro_rules! take {
            ($($section:ident . $field:ident),+ $(,)?) => {
                $(if let Some(value) = update.$section.$field {
                    self.$section.$field = Some(value);
                })+
            };
        }

        take!(
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
            budget.tokenizer,
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
    }

    /// Forget one key, so the layer below decides it again.
    ///
    /// Returns false for a name that is not a configuration key, which is how a
    /// typed-in reset request is refused rather than silently ignored.
    pub fn clear(&mut self, key: &str) -> bool {
        macro_rules! clear {
            ($($section:ident . $field:ident),+ $(,)?) => {
                match key {
                    $(concat!(stringify!($section), ".", stringify!($field)) => {
                        self.$section.$field = None;
                        true
                    })+
                    _ => false,
                }
            };
        }

        clear!(
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
            budget.tokenizer,
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
        )
    }

    /// Render this layer as TOML, ready to be written to a file.
    pub fn to_toml(&self) -> String {
        // Every top-level field is a table and every value is a primitive, so
        // serialization cannot fail and cannot produce a value-after-table
        // document that TOML would reject.
        toml::to_string_pretty(self).expect("a configuration layer is always serializable")
    }

    /// Parse a layer from TOML text. `path` is used for error messages only.
    pub fn from_toml(text: &str, path: &Path) -> Result<Self> {
        toml::from_str(text).map_err(|source| Error::ConfigParse {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
    }

    /// Read a layer from disk. Returns `None` when the file does not exist,
    /// which is the normal case for a fresh install.
    pub fn from_file(path: &Path) -> Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(Error::Io {
                    action: "read configuration file",
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        PartialConfig::from_toml(&text, path).map(Some)
    }

    /// Build a layer from `CTXC_*` environment variables.
    pub fn from_env(env: &dyn Environment) -> Result<Self> {
        let mut partial = PartialConfig::default();

        partial.core.mode = string_var(env, "CORE_MODE");
        partial.optimization.enabled = bool_var(env, "OPTIMIZATION_ENABLED")?;
        partial.optimization.target_reduction = parse_var(env, "OPTIMIZATION_TARGET_REDUCTION")?;
        partial.retrieval.enabled = bool_var(env, "RETRIEVAL_ENABLED")?;
        partial.ranking.keyword = parse_var(env, "RANKING_KEYWORD")?;
        partial.ranking.semantic = parse_var(env, "RANKING_SEMANTIC")?;
        partial.ranking.symbol = parse_var(env, "RANKING_SYMBOL")?;
        partial.ranking.graph = parse_var(env, "RANKING_GRAPH")?;
        partial.ranking.recency = parse_var(env, "RANKING_RECENCY")?;
        partial.ranking.hop_decay = parse_var(env, "RANKING_HOP_DECAY")?;
        partial.ranking.expansion_depth = parse_var(env, "RANKING_EXPANSION_DEPTH")?;
        partial.ranking.recency_half_life_days = parse_var(env, "RANKING_RECENCY_HALF_LIFE_DAYS")?;
        partial.graph.enabled = bool_var(env, "GRAPH_ENABLED")?;
        partial.storage.path = string_var(env, "STORAGE_PATH");
        partial.budget.default = parse_var(env, "BUDGET_DEFAULT")?;
        partial.budget.tokenizer = string_var(env, "BUDGET_TOKENIZER");
        partial.daemon.enabled = bool_var(env, "DAEMON_ENABLED")?;
        partial.daemon.auto_start = bool_var(env, "DAEMON_AUTO_START")?;
        partial.daemon.bind = string_var(env, "DAEMON_BIND");
        partial.daemon.port = parse_var(env, "DAEMON_PORT")?;
        partial.watch.enabled = bool_var(env, "WATCH_ENABLED")?;
        partial.watch.debounce_ms = parse_var(env, "WATCH_DEBOUNCE_MS")?;
        partial.watch.poll_interval_ms = parse_var(env, "WATCH_POLL_INTERVAL_MS")?;
        partial.semantic.enabled = bool_var(env, "SEMANTIC_ENABLED")?;
        partial.semantic.provider = string_var(env, "SEMANTIC_PROVIDER");
        partial.semantic.dimensions = parse_var(env, "SEMANTIC_DIMENSIONS")?;
        partial.semantic.redundancy_threshold = parse_var(env, "SEMANTIC_REDUNDANCY_THRESHOLD")?;
        partial.semantic.diversity = parse_var(env, "SEMANTIC_DIVERSITY")?;
        partial.metrics.enabled = bool_var(env, "METRICS_ENABLED")?;
        partial.metrics.raw_retention_days = parse_var(env, "METRICS_RAW_RETENTION_DAYS")?;
        partial.metrics.hourly_retention_days = parse_var(env, "METRICS_HOURLY_RETENTION_DAYS")?;
        partial.metrics.cost_model = string_var(env, "METRICS_COST_MODEL");
        partial.metrics.cost_per_million_input_tokens =
            parse_var(env, "METRICS_COST_PER_MILLION_INPUT_TOKENS")?;
        partial.metrics.cost_currency = string_var(env, "METRICS_COST_CURRENCY");
        partial.dashboard.enabled = bool_var(env, "DASHBOARD_ENABLED")?;
        partial.dashboard.port = parse_var(env, "DASHBOARD_PORT")?;
        partial.telemetry.enabled = bool_var(env, "TELEMETRY_ENABLED")?;

        Ok(partial)
    }
}

/// Where a layer came from, for `ctxc config path` and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Defaults,
    File,
    Environment,
    Overrides,
}

/// One entry in the resolved layer stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayerInfo {
    pub kind: LayerKind,
    pub path: Option<PathBuf>,
    /// False for a configuration file that does not exist yet.
    pub applied: bool,
}

/// Inputs to [`Config::load`], in increasing order of precedence.
pub struct LoadOptions<'a> {
    /// Configuration files, lowest precedence first.
    pub files: Vec<PathBuf>,
    /// Environment to read `CTXC_*` variables from.
    pub env: &'a dyn Environment,
    /// Values supplied on the command line.
    pub overrides: PartialConfig,
}

impl<'a> LoadOptions<'a> {
    pub fn new(env: &'a dyn Environment) -> Self {
        LoadOptions {
            files: Vec::new(),
            env,
            overrides: PartialConfig::default(),
        }
    }

    pub fn with_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.files.push(path.into());
        self
    }
}

/// A loaded configuration together with the layers that produced it.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: Config,
    pub layers: Vec<LayerInfo>,
}

impl Config {
    /// Merge every layer and validate the result.
    pub fn load(options: LoadOptions<'_>) -> Result<LoadedConfig> {
        let mut config = Config::default();
        let mut layers = vec![LayerInfo {
            kind: LayerKind::Defaults,
            path: None,
            applied: true,
        }];

        for path in &options.files {
            let layer = PartialConfig::from_file(path)?;
            layers.push(LayerInfo {
                kind: LayerKind::File,
                path: Some(path.clone()),
                applied: layer.is_some(),
            });
            if let Some(layer) = layer {
                config.merge(layer);
            }
        }

        let env_layer = PartialConfig::from_env(options.env)?;
        layers.push(LayerInfo {
            kind: LayerKind::Environment,
            path: None,
            applied: true,
        });
        config.merge(env_layer);

        layers.push(LayerInfo {
            kind: LayerKind::Overrides,
            path: None,
            applied: true,
        });
        config.merge(options.overrides);

        config.validate()?;
        Ok(LoadedConfig { config, layers })
    }

    /// Apply the set keys of `layer` on top of this configuration.
    pub fn merge(&mut self, layer: PartialConfig) {
        if let Some(mode) = layer.core.mode {
            self.core.mode = mode;
        }
        if let Some(enabled) = layer.optimization.enabled {
            self.optimization.enabled = enabled;
        }
        if let Some(target) = layer.optimization.target_reduction {
            self.optimization.target_reduction = target;
        }
        if let Some(enabled) = layer.retrieval.enabled {
            self.retrieval.enabled = enabled;
        }
        if let Some(keyword) = layer.ranking.keyword {
            self.ranking.keyword = keyword;
        }
        if let Some(semantic) = layer.ranking.semantic {
            self.ranking.semantic = semantic;
        }
        if let Some(symbol) = layer.ranking.symbol {
            self.ranking.symbol = symbol;
        }
        if let Some(graph) = layer.ranking.graph {
            self.ranking.graph = graph;
        }
        if let Some(recency) = layer.ranking.recency {
            self.ranking.recency = recency;
        }
        if let Some(decay) = layer.ranking.hop_decay {
            self.ranking.hop_decay = decay;
        }
        if let Some(depth) = layer.ranking.expansion_depth {
            self.ranking.expansion_depth = depth;
        }
        if let Some(days) = layer.ranking.recency_half_life_days {
            self.ranking.recency_half_life_days = days;
        }
        if let Some(enabled) = layer.graph.enabled {
            self.graph.enabled = enabled;
        }
        if let Some(path) = layer.storage.path {
            self.storage.path = path;
        }
        if let Some(default) = layer.budget.default {
            self.budget.default = default;
        }
        if let Some(tokenizer) = layer.budget.tokenizer {
            self.budget.tokenizer = tokenizer;
        }
        if let Some(enabled) = layer.daemon.enabled {
            self.daemon.enabled = enabled;
        }
        if let Some(auto_start) = layer.daemon.auto_start {
            self.daemon.auto_start = auto_start;
        }
        if let Some(bind) = layer.daemon.bind {
            self.daemon.bind = bind;
        }
        if let Some(port) = layer.daemon.port {
            self.daemon.port = port;
        }
        if let Some(enabled) = layer.watch.enabled {
            self.watch.enabled = enabled;
        }
        if let Some(debounce) = layer.watch.debounce_ms {
            self.watch.debounce_ms = debounce;
        }
        if let Some(interval) = layer.watch.poll_interval_ms {
            self.watch.poll_interval_ms = interval;
        }
        if let Some(enabled) = layer.semantic.enabled {
            self.semantic.enabled = enabled;
        }
        if let Some(provider) = layer.semantic.provider {
            self.semantic.provider = provider;
        }
        if let Some(dimensions) = layer.semantic.dimensions {
            self.semantic.dimensions = dimensions;
        }
        if let Some(threshold) = layer.semantic.redundancy_threshold {
            self.semantic.redundancy_threshold = threshold;
        }
        if let Some(diversity) = layer.semantic.diversity {
            self.semantic.diversity = diversity;
        }
        if let Some(enabled) = layer.metrics.enabled {
            self.metrics.enabled = enabled;
        }
        if let Some(days) = layer.metrics.raw_retention_days {
            self.metrics.raw_retention_days = days;
        }
        if let Some(days) = layer.metrics.hourly_retention_days {
            self.metrics.hourly_retention_days = days;
        }
        if let Some(model) = layer.metrics.cost_model {
            self.metrics.cost_model = model;
        }
        if let Some(rate) = layer.metrics.cost_per_million_input_tokens {
            self.metrics.cost_per_million_input_tokens = rate;
        }
        if let Some(currency) = layer.metrics.cost_currency {
            self.metrics.cost_currency = currency;
        }
        if let Some(enabled) = layer.dashboard.enabled {
            self.dashboard.enabled = enabled;
        }
        if let Some(port) = layer.dashboard.port {
            self.dashboard.port = port;
        }
        if let Some(enabled) = layer.telemetry.enabled {
            self.telemetry.enabled = enabled;
        }
    }

    /// Reject combinations that would fail later, at a point where the message
    /// can still name the offending key.
    pub fn validate(&self) -> Result<()> {
        if self.core.mode != "local" {
            return Err(Error::ConfigValue {
                key: "core.mode".into(),
                reason: format!("unknown mode `{}`; expected `local`", self.core.mode),
            });
        }
        if !(0.0..=1.0).contains(&self.optimization.target_reduction) {
            return Err(Error::ConfigValue {
                key: "optimization.target_reduction".into(),
                reason: "must be between 0.0 and 1.0".into(),
            });
        }
        if self.budget.default == 0 {
            return Err(Error::ConfigValue {
                key: "budget.default".into(),
                reason: "must be greater than zero".into(),
            });
        }
        if crate::token::named(&self.budget.tokenizer).is_none() {
            return Err(Error::ConfigValue {
                key: "budget.tokenizer".into(),
                reason: format!(
                    "unknown tokenizer {:?}; this build offers {}",
                    self.budget.tokenizer,
                    crate::token::available().join(", ")
                ),
            });
        }
        for (key, value) in [
            ("ranking.keyword", self.ranking.keyword),
            ("ranking.semantic", self.ranking.semantic),
            ("ranking.symbol", self.ranking.symbol),
            ("ranking.graph", self.ranking.graph),
            ("ranking.recency", self.ranking.recency),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(Error::ConfigValue {
                    key: key.into(),
                    reason: "must be zero or more".into(),
                });
            }
        }
        if !(0.0..=1.0).contains(&self.ranking.hop_decay) {
            return Err(Error::ConfigValue {
                key: "ranking.hop_decay".into(),
                reason: "must be between 0.0 and 1.0".into(),
            });
        }
        if self.ranking.recency_half_life_days <= 0.0 {
            return Err(Error::ConfigValue {
                key: "ranking.recency_half_life_days".into(),
                reason: "must be greater than zero".into(),
            });
        }
        if self.semantic.dimensions == 0 {
            return Err(Error::ConfigValue {
                key: "semantic.dimensions".into(),
                reason: "must be greater than zero".into(),
            });
        }
        for (key, value) in [
            (
                "semantic.redundancy_threshold",
                self.semantic.redundancy_threshold,
            ),
            ("semantic.diversity", self.semantic.diversity),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(Error::ConfigValue {
                    key: key.into(),
                    reason: "must be between 0.0 and 1.0".into(),
                });
            }
        }

        let rate = self.metrics.cost_per_million_input_tokens;
        if !rate.is_finite() || rate < 0.0 {
            return Err(Error::ConfigValue {
                key: "metrics.cost_per_million_input_tokens".into(),
                reason: "must be zero or more; zero means no cost estimate".into(),
            });
        }
        if self.dashboard.port == 0 {
            return Err(Error::ConfigValue {
                key: "dashboard.port".into(),
                reason: "must be between 1 and 65535".into(),
            });
        }
        // Port 0 is meaningful for the daemon: it asks the operating system for
        // a free port, which is what an ephemeral or sandboxed instance wants.
        // The port it actually got is recorded in the lockfile.
        if self.daemon.port != 0 && self.daemon.port == self.dashboard.port {
            return Err(Error::ConfigValue {
                key: "dashboard.port".into(),
                reason: format!("must differ from daemon.port ({})", self.daemon.port),
            });
        }
        Ok(())
    }

    /// The database path, resolving `storage.path = "auto"` against the
    /// platform data directory.
    pub fn database_path(&self, paths: &Paths) -> PathBuf {
        if self.storage.path == "auto" {
            paths.database_file()
        } else {
            PathBuf::from(&self.storage.path)
        }
    }

    /// The tokenizer this configuration selects.
    ///
    /// Validation already refused an unknown name, so a configuration that
    /// loaded has a tokenizer that exists in this build.
    pub fn tokenizer(&self) -> std::sync::Arc<dyn crate::Tokenizer> {
        crate::token::named(&self.budget.tokenizer)
            .unwrap_or_else(|| std::sync::Arc::new(crate::HeuristicTokenizer::new()))
    }

    /// The default token budget as a typed value.
    pub fn default_budget(&self) -> TokenBudget {
        TokenBudget::new(self.budget.default)
    }

    /// Render as TOML, for `ctxc config show`.
    pub fn to_toml(&self) -> String {
        // The Config type is a plain tree of primitives, so serialization
        // cannot fail; treating it as fatal would be worse than unwrapping.
        toml::to_string_pretty(self).expect("configuration is always serializable")
    }
}

fn env_key(suffix: &str) -> String {
    format!("{ENV_PREFIX}_{suffix}")
}

fn string_var(env: &dyn Environment, suffix: &str) -> Option<String> {
    env.var(&env_key(suffix))
}

fn bool_var(env: &dyn Environment, suffix: &str) -> Result<Option<bool>> {
    let key = env_key(suffix);
    let Some(raw) = env.var(&key) else {
        return Ok(None);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        other => Err(Error::ConfigValue {
            key,
            reason: format!("`{other}` is not a boolean (use true or false)"),
        }),
    }
}

fn parse_var<T>(env: &dyn Environment, suffix: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let key = env_key(suffix);
    let Some(raw) = env.var(&key) else {
        return Ok(None);
    };
    raw.trim()
        .parse::<T>()
        .map(Some)
        .map_err(|err| Error::ConfigValue {
            key,
            reason: format!("`{raw}` is invalid: {err}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::MapEnvironment;

    fn empty_env() -> MapEnvironment {
        MapEnvironment::default()
    }

    #[test]
    fn defaults_match_the_documented_configuration() {
        let config = Config::default();
        assert_eq!(config.daemon.port, 7717);
        assert_eq!(config.dashboard.port, 7718);
        assert_eq!(config.watch.debounce_ms, 300);
        assert_eq!(config.metrics.raw_retention_days, 30);
        assert!(!config.telemetry.enabled, "telemetry must default to off");
        config.validate().unwrap();
    }

    #[test]
    fn merging_a_layer_only_touches_the_keys_it_sets() {
        let mut config = Config::default();
        let layer =
            PartialConfig::from_toml("[daemon]\nport = 9000\n", Path::new("test.toml")).unwrap();
        config.merge(layer);

        assert_eq!(config.daemon.port, 9000);
        assert!(
            config.daemon.enabled,
            "unset keys keep their previous value"
        );
        assert_eq!(config.daemon.bind, "127.0.0.1");
    }

    #[test]
    fn later_layers_win() {
        let mut config = Config::default();
        for port in ["7000", "8000"] {
            let text = format!("[daemon]\nport = {port}\n");
            config.merge(PartialConfig::from_toml(&text, Path::new("test.toml")).unwrap());
        }
        assert_eq!(config.daemon.port, 8000);
    }

    #[test]
    fn environment_overrides_files() {
        let dir = std::env::temp_dir().join(format!("ctxc-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.toml");
        std::fs::write(&file, "[daemon]\nport = 7000\n[watch]\nenabled = false\n").unwrap();

        let env = MapEnvironment::new([("CTXC_DAEMON_PORT", "7100")]);
        let loaded = Config::load(LoadOptions::new(&env).with_file(&file)).unwrap();

        assert_eq!(loaded.config.daemon.port, 7100, "env beats file");
        assert!(!loaded.config.watch.enabled, "file value survives");
        assert_eq!(loaded.layers.len(), 4);
        assert!(loaded.layers[1].applied);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn overrides_beat_everything() {
        let env = MapEnvironment::new([("CTXC_DAEMON_PORT", "7100")]);
        let mut options = LoadOptions::new(&env);
        options.overrides.daemon.port = Some(7200);
        let loaded = Config::load(options).unwrap();
        assert_eq!(loaded.config.daemon.port, 7200);
    }

    #[test]
    fn a_missing_file_is_recorded_but_not_an_error() {
        let env = empty_env();
        let missing = PathBuf::from("does-not-exist-ctxc.toml");
        let loaded = Config::load(LoadOptions::new(&env).with_file(&missing)).unwrap();

        assert_eq!(loaded.config, Config::default());
        let file_layer = &loaded.layers[1];
        assert_eq!(file_layer.kind, LayerKind::File);
        assert!(!file_layer.applied);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let error =
            PartialConfig::from_toml("[daemon]\nprot = 1\n", Path::new("config.toml")).unwrap_err();
        let text = crate::error::report(&error);
        assert!(text.contains("invalid configuration file"), "{text}");
        assert!(text.contains("prot"), "{text}");
    }

    #[test]
    fn env_values_are_validated() {
        let env = MapEnvironment::new([("CTXC_WATCH_ENABLED", "maybe")]);
        let error = Config::load(LoadOptions::new(&env)).unwrap_err();
        assert!(matches!(error, Error::ConfigValue { ref key, .. } if key == "CTXC_WATCH_ENABLED"));

        let env = MapEnvironment::new([("CTXC_DAEMON_PORT", "70000")]);
        assert!(Config::load(LoadOptions::new(&env)).is_err());
    }

    #[test]
    fn boolean_env_spellings() {
        for (raw, expected) in [
            ("true", true),
            ("TRUE", true),
            ("1", true),
            ("yes", true),
            ("on", true),
            ("false", false),
            ("0", false),
            ("off", false),
        ] {
            let env = MapEnvironment::new([("CTXC_TELEMETRY_ENABLED", raw)]);
            let loaded = Config::load(LoadOptions::new(&env)).unwrap();
            assert_eq!(loaded.config.telemetry.enabled, expected, "for {raw}");
        }
    }

    #[test]
    fn validation_catches_conflicting_ports() {
        let env = MapEnvironment::new([("CTXC_DASHBOARD_PORT", "7717")]);
        let error = Config::load(LoadOptions::new(&env)).unwrap_err();
        assert!(matches!(error, Error::ConfigValue { ref key, .. } if key == "dashboard.port"));
    }

    #[test]
    fn an_operating_system_assigned_daemon_port_is_allowed() {
        let env = MapEnvironment::new([("CTXC_DAEMON_PORT", "0")]);
        let loaded = Config::load(LoadOptions::new(&env)).unwrap();

        assert_eq!(
            loaded.config.daemon.port, 0,
            "zero means: pick a free port, and record which one in the lockfile"
        );
    }

    #[test]
    fn validation_catches_out_of_range_reduction() {
        let env = MapEnvironment::new([("CTXC_OPTIMIZATION_TARGET_REDUCTION", "1.5")]);
        assert!(Config::load(LoadOptions::new(&env)).is_err());
    }

    #[test]
    fn database_path_resolves_auto() {
        let paths = Paths::new(
            PathBuf::from("/cfg"),
            PathBuf::from("/data"),
            PathBuf::from("/cache"),
        );
        let mut config = Config::default();
        assert_eq!(config.database_path(&paths), paths.database_file());

        config.storage.path = "/tmp/custom.db".into();
        assert_eq!(
            config.database_path(&paths),
            PathBuf::from("/tmp/custom.db")
        );
    }

    #[test]
    fn rendered_toml_reparses_to_the_same_configuration() {
        let mut config = Config::default();
        config.daemon.port = 9100;
        config.core.mode = "local".into();

        let rendered = config.to_toml();
        let mut reparsed = Config::default();
        reparsed.merge(PartialConfig::from_toml(&rendered, Path::new("config.toml")).unwrap());

        assert_eq!(reparsed, config);
    }
}
