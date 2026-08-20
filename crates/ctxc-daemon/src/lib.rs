//! The CtxC background runtime.
//!
//! The daemon is not an accelerator bolted onto the CLI: it is where continuous
//! work belongs. It keeps registered projects indexed, serves the local HTTP
//! API, and stays out of the way — everything it does is also possible without
//! it, only slower.

pub mod error;
pub mod lifecycle;
pub mod lock;
pub mod runtime;
pub mod supervisor;

pub use error::{DaemonError, Result};
pub use lifecycle::{status, stop, DaemonState};
pub use lock::{Lock, Occupancy};
pub use runtime::{run, DaemonOptions};
pub use supervisor::{Supervisor, SupervisorOptions};
