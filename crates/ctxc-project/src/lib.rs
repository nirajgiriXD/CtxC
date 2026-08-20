//! Projects: the registry, what a project says about itself, and what CtxC can
//! work out for itself.
//!
//! The registry is the shared source of truth for the CLI, the daemon, the
//! watcher and the dashboard. Persistence sits behind [`ProjectStore`], so the
//! rules about identity, detection and status live here rather than in SQL.

pub mod config;
pub mod detect;
pub mod error;
pub mod model;
pub mod registry;

pub use config::ProjectFile;
pub use detect::detect;
pub use error::{ProjectError, Result};
pub use model::{Detection, Project, ProjectId, ProjectStatus};
pub use registry::{canonical_root, ProjectStore, Registration, Registry};
