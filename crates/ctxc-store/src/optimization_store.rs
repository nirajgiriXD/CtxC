//! Remembering what optimizing something produced.
//!
//! Optimization is deterministic: the same bytes, through the same optimizer,
//! under the same budget and the same settings, produce the same document. So
//! the second time is work that has already been done — and agents re-run the
//! same commands constantly, which makes that the common case rather than an
//! edge one.
//!
//! The key carries every input that decides the answer. A cache whose key is
//! narrower than what it caches returns yesterday's answer to today's question,
//! and that is worse than being slow.

use rusqlite::{Connection, OptionalExtension};

use ctxc_core::optimization::OptimizationResult;
use ctxc_core::Timestamp;

use crate::db::Database;
use crate::error::{Result, StoreError};

/// Everything that decides what optimizing produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OptimizationKey<'a> {
    /// Fingerprint of the content that went in.
    pub content_hash: &'a str,
    /// The optimizer that handled it.
    pub optimizer: &'a str,
    /// The budget it had to fit, in tokens.
    pub budget: u32,
    /// Fingerprint of the engine settings in force.
    pub settings: &'a str,
}

/// What an earlier run produced.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedOptimization {
    pub content: String,
    pub result: OptimizationResult,
}

/// Reading and writing remembered optimizations.
pub trait OptimizationStore {
    /// What this exact work produced before, if it has been done.
    fn cached(&self, key: OptimizationKey<'_>) -> Result<Option<CachedOptimization>>;

    /// Remember what this work produced.
    fn remember(&self, key: OptimizationKey<'_>, cached: &CachedOptimization) -> Result<()>;

    /// Forget everything, returning how many rows went.
    fn clear_optimizations(&self) -> Result<u64>;
}

/// SQLite-backed [`OptimizationStore`].
pub struct SqliteOptimizationStore<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteOptimizationStore<'a> {
    pub fn new(database: &'a Database) -> Self {
        SqliteOptimizationStore {
            conn: database.connection(),
        }
    }
}

impl OptimizationStore for SqliteOptimizationStore<'_> {
    fn cached(&self, key: OptimizationKey<'_>) -> Result<Option<CachedOptimization>> {
        let row: Option<(String, String)> = self
            .conn
            .prepare_cached(
                "SELECT content, result FROM optimizations
                 WHERE content_hash = ?1 AND optimizer = ?2 AND budget = ?3 AND settings = ?4",
            )?
            .query_row(
                rusqlite::params![
                    key.content_hash,
                    key.optimizer,
                    key.budget as i64,
                    key.settings
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let Some((content, result)) = row else {
            return Ok(None);
        };

        // A row that will not parse is a row from a build that measured
        // different things. Treating it as a miss re-does the work and
        // replaces it, which is what a cache should do about its own past.
        match serde_json::from_str(&result) {
            Ok(result) => Ok(Some(CachedOptimization { content, result })),
            Err(err) => {
                tracing::debug!(error = %err, "ignoring an unreadable cached optimization");
                Ok(None)
            }
        }
    }

    fn remember(&self, key: OptimizationKey<'_>, cached: &CachedOptimization) -> Result<()> {
        let result =
            serde_json::to_string(&cached.result).map_err(|err| StoreError::CorruptRow {
                id: key.content_hash.to_owned(),
                reason: format!("the optimization result could not be encoded: {err}"),
            })?;

        self.conn
            .prepare_cached(
                "INSERT INTO optimizations
                     (content_hash, optimizer, budget, settings, content, result, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(content_hash, optimizer, budget, settings) DO UPDATE SET
                     content    = excluded.content,
                     result     = excluded.result,
                     created_at = excluded.created_at",
            )?
            .execute(rusqlite::params![
                key.content_hash,
                key.optimizer,
                key.budget as i64,
                key.settings,
                cached.content,
                result,
                Timestamp::now().as_millis(),
            ])?;
        Ok(())
    }

    fn clear_optimizations(&self) -> Result<u64> {
        let removed = self
            .conn
            .prepare_cached("DELETE FROM optimizations")?
            .execute([])?;
        Ok(removed as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::optimization::SavingsByStage;

    fn result(optimized: u32) -> OptimizationResult {
        OptimizationResult {
            optimizer: "log".into(),
            tokenizer: "heuristic".into(),
            estimated: true,
            original_tokens: 100,
            optimized_tokens: optimized,
            reduction_ratio: 0.5,
            compression_ratio: 2.0,
            preserved_fragments: 4,
            removed_fragments: 2,
            savings_by_stage: SavingsByStage::default(),
        }
    }

    fn key<'a>(hash: &'a str, budget: u32, settings: &'a str) -> OptimizationKey<'a> {
        OptimizationKey {
            content_hash: hash,
            optimizer: "log",
            budget,
            settings,
        }
    }

    #[test]
    fn an_optimization_comes_back_exactly_as_it_went_in() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteOptimizationStore::new(&database);
        let cached = CachedOptimization {
            content: "shortened".into(),
            result: result(50),
        };

        store.remember(key("h1", 8000, "s1"), &cached).unwrap();
        assert_eq!(store.cached(key("h1", 8000, "s1")).unwrap(), Some(cached));
    }

    /// Each part of the key decides the answer, so changing any of them must
    /// miss rather than return the answer to a different question.
    #[test]
    fn every_part_of_the_key_is_part_of_the_key() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteOptimizationStore::new(&database);
        let cached = CachedOptimization {
            content: "shortened".into(),
            result: result(50),
        };
        store.remember(key("h1", 8000, "s1"), &cached).unwrap();

        assert!(store.cached(key("h2", 8000, "s1")).unwrap().is_none());
        assert!(store.cached(key("h1", 4000, "s1")).unwrap().is_none());
        assert!(store.cached(key("h1", 8000, "s2")).unwrap().is_none());
        assert!(store
            .cached(OptimizationKey {
                optimizer: "text",
                ..key("h1", 8000, "s1")
            })
            .unwrap()
            .is_none());
    }

    #[test]
    fn remembering_the_same_work_twice_replaces_rather_than_duplicates() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteOptimizationStore::new(&database);

        store
            .remember(
                key("h1", 8000, "s1"),
                &CachedOptimization {
                    content: "first".into(),
                    result: result(50),
                },
            )
            .unwrap();
        store
            .remember(
                key("h1", 8000, "s1"),
                &CachedOptimization {
                    content: "second".into(),
                    result: result(40),
                },
            )
            .unwrap();

        let found = store.cached(key("h1", 8000, "s1")).unwrap().unwrap();
        assert_eq!(found.content, "second");
        assert_eq!(store.clear_optimizations().unwrap(), 1);
        assert!(store.cached(key("h1", 8000, "s1")).unwrap().is_none());
    }
}
