//! Content optimizers.
//!
//! One optimizer per kind of material, behind a single trait, so the router can
//! choose without knowing what any of them do. There is deliberately no
//! "general compression algorithm": JSON, logs and source code are shortened
//! for different reasons, and mixing those reasons is how meaning gets lost.
//!
//! Every optimizer must be *safe* on anything it claims to support. An
//! optimizer that would rather do nothing than risk removing something load
//! bearing is behaving correctly.

pub mod error;
pub mod json;
pub mod log;
pub mod passthrough;
pub(crate) mod stages;
pub mod text;
pub mod tool;

pub use error::{OptimizerError, Result};
pub use json::JsonOptimizer;
pub use log::LogOptimizer;
pub use passthrough::PassthroughOptimizer;
pub use text::{TextOptimizer, TextPolicy};
pub use tool::{ToolOutputOptimizer, ToolProfile};

use ctxc_core::{Context, OptimizedContext, TokenBudget};

/// Shortens one kind of context.
pub trait ContextOptimizer: Send + Sync {
    /// Stable name, reported in results so a number can be traced to the code
    /// that produced it.
    fn name(&self) -> &'static str;

    /// Whether this optimizer handles `context`.
    fn supports(&self, context: &Context) -> bool;

    /// Optimize `context`, staying within `budget` when one is given.
    fn optimize(&self, context: &Context, budget: Option<TokenBudget>) -> Result<OptimizedContext>;
}
