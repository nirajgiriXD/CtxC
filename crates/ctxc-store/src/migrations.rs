//! Forward-only schema migrations.
//!
//! The applied version lives in SQLite's own `user_version` pragma, so there is
//! no bootstrapping table to create and no chance of the tracking table and the
//! schema disagreeing. Each migration runs inside a transaction together with
//! the version bump, so an interrupted upgrade leaves the database on the
//! previous version rather than half-migrated.

use rusqlite::Connection;

use crate::error::{Result, StoreError};

/// A single, immutable schema step.
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// All migrations, in application order. Never edit a released entry; add a new
/// one instead.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "index",
        sql: include_str!("../migrations/0002_index.sql"),
    },
    Migration {
        version: 3,
        name: "search",
        sql: include_str!("../migrations/0003_search.sql"),
    },
    Migration {
        version: 4,
        name: "projects",
        sql: include_str!("../migrations/0004_projects.sql"),
    },
    Migration {
        version: 5,
        name: "metrics",
        sql: include_str!("../migrations/0005_metrics.sql"),
    },
    Migration {
        version: 6,
        name: "memory",
        sql: include_str!("../migrations/0006_memory.sql"),
    },
    Migration {
        version: 7,
        name: "embeddings",
        sql: include_str!("../migrations/0007_embeddings.sql"),
    },
    Migration {
        version: 8,
        name: "optimizations",
        sql: include_str!("../migrations/0008_optimizations.sql"),
    },
];

/// The schema version a fully migrated database has.
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map(|m| m.version).unwrap_or(0)
}

/// Read the version recorded in the database.
pub fn current_version(conn: &Connection) -> Result<u32> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(version.max(0) as u32)
}

/// Apply every migration the database has not seen yet.
///
/// Returns the versions that were applied, which is empty when the database was
/// already current.
pub fn migrate(conn: &mut Connection) -> Result<Vec<u32>> {
    let current = current_version(conn)?;
    let latest = latest_version();
    if current > latest {
        return Err(StoreError::SchemaTooNew {
            found: current,
            supported: latest,
        });
    }

    let mut applied = Vec::new();
    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
        let transaction = conn.transaction()?;
        transaction
            .execute_batch(migration.sql)
            .and_then(|()| {
                // PRAGMA user_version does not accept bound parameters.
                transaction.execute_batch(&format!("PRAGMA user_version = {}", migration.version))
            })
            .map_err(|source| StoreError::Migration {
                version: migration.version,
                name: migration.name,
                source,
            })?;
        transaction.commit()?;

        tracing::debug!(
            version = migration.version,
            name = migration.name,
            "applied migration"
        );
        applied.push(migration.version);
    }

    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_sequential_and_unique() {
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                migration.version as usize,
                index + 1,
                "migrations must be numbered 1..n in order"
            );
        }
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(migrate(&mut conn).unwrap(), vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(current_version(&conn).unwrap(), latest_version());
        assert!(migrate(&mut conn).unwrap().is_empty());
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 9999").unwrap();
        let error = migrate(&mut conn).unwrap_err();
        assert!(matches!(
            error,
            StoreError::SchemaTooNew { found: 9999, .. }
        ));
        assert!(error.hint().unwrap().contains("upgrade CtxC"));
    }
}
