//! Representing and acquiring context.
//!
//! `ctxc-core` defines what a context *is*; this crate turns real material into
//! one and cuts it into the fragments optimizers work on. It knows nothing
//! about optimization strategy, and nothing about storage.

pub mod detect;
pub mod error;
pub mod fragment;
pub mod ingest;
pub mod walk;

pub use error::{ContextError, Result};
pub use fragment::{join, split, strategy_for, Fragment, SplitStrategy};
pub use walk::{walk, IgnoreRules, Walk, WalkEntry, WalkOptions};
