//! Continuous observation of registered projects.
//!
//! ```text
//! filesystem events -> ignore rules -> debounce -> settled batch -> indexer
//! ```
//!
//! The ordering matters: rules are applied to raw events so that ignored trees
//! cost nothing, and debouncing sits between the platform's noise and any real
//! work. What comes out is a small, coalesced batch of changes that is safe to
//! apply more than once.

pub mod change;
pub mod debounce;
pub mod error;
pub mod watcher;

pub use change::{Change, ChangeKind};
pub use debounce::{Debouncer, DEFAULT_MAX_DELAY, DEFAULT_QUIET};
pub use error::{Result, WatchError};
pub use watcher::ProjectWatcher;
