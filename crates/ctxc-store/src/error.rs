//! Storage errors.
//!
//! rusqlite errors are wrapped rather than surfaced, so that nothing above this
//! crate has to know which database is underneath.

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("failed to open context database: {}", path.display())]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error("failed to apply database migration {version} ({name})")]
    Migration {
        version: u32,
        name: &'static str,
        #[source]
        source: rusqlite::Error,
    },

    #[error(
        "database schema is version {found}, but this build of CtxC supports up to {supported}"
    )]
    SchemaTooNew { found: u32, supported: u32 },

    #[error("database query failed")]
    Query(#[from] rusqlite::Error),

    #[error("stored context {id} could not be read back: {reason}")]
    CorruptRow { id: String, reason: String },

    #[error(transparent)]
    Core(#[from] ctxc_core::Error),
}

impl StoreError {
    /// A concrete next step, shown under `Try:` when the error is reported.
    pub fn hint(&self) -> Option<String> {
        match self {
            StoreError::Open { source, .. } | StoreError::Query(source) if is_busy(source) => {
                Some("another CtxC process is using the database; stop it and retry".into())
            }
            StoreError::Open { path, .. } => Some(format!(
                "check that {} is writable, or set storage.path in your configuration",
                path.display()
            )),
            StoreError::SchemaTooNew { .. } => {
                Some("upgrade CtxC, or point storage.path at a different database".into())
            }
            StoreError::Migration { .. } | StoreError::CorruptRow { .. } => {
                Some("run `ctxc status` to see which database is in use".into())
            }
            StoreError::Query(_) => None,
            StoreError::Core(err) => err.hint(),
        }
    }
}

fn is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked,
                ..
            },
            _
        )
    )
}
