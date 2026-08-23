//! Foundational types for CtxC.
//!
//! This crate is the bottom of the dependency graph: everything else may depend
//! on it, and it depends on nothing inside the workspace. It deliberately holds
//! data types, error types and the platform/configuration boundary only —
//! optimization logic belongs in `ctxc-engine` and `ctxc-optimizer`.

pub mod config;
pub mod config_file;
pub mod context;
pub mod error;
pub mod id;
pub mod optimization;
pub mod platform;
pub mod processes;
pub mod time;
pub mod token;

pub use config::{Config, PartialConfig};
pub use context::{ContentType, Context, ContextMetadata, ContextSource};
pub use error::{report, report_with, Error, Result};
pub use id::ContextId;
pub use optimization::{OptimizationResult, OptimizedContext, SavingsByStage, Stage};
pub use platform::{Os, Paths, Platform};
pub use time::Timestamp;
pub use token::{HeuristicTokenizer, TokenBudget, Tokenizer};

/// The name used for configuration and data directories on every platform.
pub const APP_NAME: &str = "ctxc";

/// Version of the `ctxc` crate family, taken from the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
