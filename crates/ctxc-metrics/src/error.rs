//! Metrics errors.
//!
//! Recording never fails: [`Collector::record`](crate::Collector::record)
//! returns nothing, because a metrics problem must never break the operation
//! being measured. These errors belong to the parts a person asked for —
//! flushing, rolling up, and reading reports back.

use thiserror::Error;

pub type Result<T, E = MetricsError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum MetricsError {
    /// The storage layer failed. Kept as text so this crate does not depend on
    /// the database it happens to be backed by.
    #[error("metrics storage: {0}")]
    Storage(String),

    #[error("a stored metric row could not be read back: {0}")]
    CorruptRow(String),

    #[error("{start} is after {end}, which is not a time window")]
    BadWindow { start: String, end: String },
}

impl MetricsError {
    /// A concrete next step, shown under `Try:` when the error is reported.
    pub fn hint(&self) -> Option<String> {
        match self {
            MetricsError::Storage(_) => {
                Some("run `ctxc status` to see which database is in use".into())
            }
            MetricsError::CorruptRow(_) => {
                Some("upgrade CtxC, or start a fresh database with storage.path".into())
            }
            MetricsError::BadWindow { .. } => None,
        }
    }
}
