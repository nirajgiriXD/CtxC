//! Notes an agent asked CtxC to remember.
//!
//! Deliberately the smallest thing that can be called memory: a key, a value,
//! and when it was written, scoped to a project. There is no ranking, no
//! embedding, and no expiry — an agent decides what is worth keeping and what
//! to call it, and CtxC stores exactly that.
//!
//! Keeping it dumb is the point. A memory system that decides what to surface
//! is a retrieval system, and CtxC already has one of those built on the index;
//! a second one with different rules would be a source of quiet disagreement.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use ctxc_core::Timestamp;

use crate::db::Database;
use crate::error::{Result, StoreError};

/// The longest note CtxC will store.
///
/// Memory is for things worth re-reading, not for a place to park a file. A
/// note that does not fit is a sign the agent wanted `ctxc optimize` instead.
pub const MAX_VALUE_BYTES: usize = 64 * 1024;

/// One remembered note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    /// The project it belongs to, or `None` when it belongs to none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub key: String,
    pub value: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Reading and writing notes.
pub trait MemoryStore {
    /// Write a note, replacing any note with the same key.
    ///
    /// Returns the stored note, whose `created_at` is from the first time the
    /// key was written — replacing a note is an edit, not a new note.
    fn remember(&self, project_id: Option<&str>, key: &str, value: &str) -> Result<Memory>;

    /// One note, if it is there.
    fn recall(&self, project_id: Option<&str>, key: &str) -> Result<Option<Memory>>;

    /// Every note for a project, most recently written first.
    fn memories(&self, project_id: Option<&str>, limit: usize) -> Result<Vec<Memory>>;

    /// Delete a note. Returns whether there was one.
    fn forget(&self, project_id: Option<&str>, key: &str) -> Result<bool>;
}

/// SQLite-backed [`MemoryStore`].
pub struct SqliteMemoryStore<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteMemoryStore<'a> {
    pub fn new(database: &'a Database) -> Self {
        SqliteMemoryStore {
            conn: database.connection(),
        }
    }
}

/// The stored form of "no project".
fn scope(project_id: Option<&str>) -> &str {
    project_id.unwrap_or("")
}

/// The reverse: an empty scope is no project.
fn unscope(stored: String) -> Option<String> {
    (!stored.is_empty()).then_some(stored)
}

impl MemoryStore for SqliteMemoryStore<'_> {
    fn remember(&self, project_id: Option<&str>, key: &str, value: &str) -> Result<Memory> {
        let key = key.trim();
        if key.is_empty() {
            return Err(StoreError::CorruptRow {
                id: "memory".into(),
                reason: "a note needs a key to be found by".into(),
            });
        }
        if value.len() > MAX_VALUE_BYTES {
            return Err(StoreError::CorruptRow {
                id: key.to_owned(),
                reason: format!(
                    "note is {} bytes, and the limit is {MAX_VALUE_BYTES}",
                    value.len()
                ),
            });
        }

        let now = Timestamp::now();
        // `created_at` is kept from the existing row, so a note that has been
        // edited still says when it was first written.
        self.conn.execute(
            "INSERT INTO memories (project_id, key, value, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(project_id, key) DO UPDATE SET
                 value      = excluded.value,
                 updated_at = excluded.updated_at",
            rusqlite::params![scope(project_id), key, value, now.as_millis()],
        )?;

        self.recall(project_id, key)?
            .ok_or_else(|| StoreError::CorruptRow {
                id: key.to_owned(),
                reason: "the note could not be read back after writing".into(),
            })
    }

    fn recall(&self, project_id: Option<&str>, key: &str) -> Result<Option<Memory>> {
        let memory = self
            .conn
            .query_row(
                "SELECT project_id, key, value, created_at, updated_at
                 FROM memories WHERE project_id = ?1 AND key = ?2",
                rusqlite::params![scope(project_id), key.trim()],
                read,
            )
            .optional()?;
        Ok(memory)
    }

    fn memories(&self, project_id: Option<&str>, limit: usize) -> Result<Vec<Memory>> {
        let mut statement = self.conn.prepare(
            "SELECT project_id, key, value, created_at, updated_at
             FROM memories WHERE project_id = ?1
             ORDER BY updated_at DESC, key LIMIT ?2",
        )?;

        let memories = statement
            .query_map(rusqlite::params![scope(project_id), limit as i64], read)?
            .collect::<rusqlite::Result<Vec<Memory>>>()?;
        Ok(memories)
    }

    fn forget(&self, project_id: Option<&str>, key: &str) -> Result<bool> {
        let removed = self.conn.execute(
            "DELETE FROM memories WHERE project_id = ?1 AND key = ?2",
            rusqlite::params![scope(project_id), key.trim()],
        )?;
        Ok(removed > 0)
    }
}

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    Ok(Memory {
        project_id: unscope(row.get(0)?),
        key: row.get(1)?,
        value: row.get(2)?,
        created_at: Timestamp::from_millis(row.get(3)?),
        updated_at: Timestamp::from_millis(row.get(4)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(database: &Database) -> SqliteMemoryStore<'_> {
        SqliteMemoryStore::new(database)
    }

    #[test]
    fn a_note_round_trips() {
        let database = Database::open_in_memory().unwrap();
        let store = store(&database);

        let written = store
            .remember(Some("acme"), "build", "Run `make release`, not cargo.")
            .unwrap();
        assert_eq!(written.key, "build");
        assert_eq!(written.project_id.as_deref(), Some("acme"));

        let read = store.recall(Some("acme"), "build").unwrap().unwrap();
        assert_eq!(read, written);
    }

    #[test]
    fn writing_the_same_key_edits_rather_than_duplicates() {
        let database = Database::open_in_memory().unwrap();
        let store = store(&database);

        let first = store.remember(None, "note", "one").unwrap();
        let second = store.remember(None, "note", "two").unwrap();

        assert_eq!(second.value, "two");
        assert_eq!(
            second.created_at, first.created_at,
            "an edited note still says when it was first written"
        );
        assert_eq!(store.memories(None, 10).unwrap().len(), 1);
    }

    #[test]
    fn notes_are_scoped_to_their_project() {
        let database = Database::open_in_memory().unwrap();
        let store = store(&database);

        store.remember(Some("acme"), "note", "acme's").unwrap();
        store.remember(Some("other"), "note", "other's").unwrap();
        store.remember(None, "note", "nobody's").unwrap();

        assert_eq!(
            store.recall(Some("acme"), "note").unwrap().unwrap().value,
            "acme's"
        );
        assert_eq!(
            store.recall(None, "note").unwrap().unwrap().value,
            "nobody's"
        );
        assert_eq!(store.memories(Some("acme"), 10).unwrap().len(), 1);
    }

    #[test]
    fn a_note_with_no_project_reads_back_with_none() {
        let database = Database::open_in_memory().unwrap();
        let store = store(&database);

        store.remember(None, "note", "value").unwrap();
        assert_eq!(
            store.recall(None, "note").unwrap().unwrap().project_id,
            None
        );
    }

    #[test]
    fn listing_puts_the_most_recent_first() {
        let database = Database::open_in_memory().unwrap();
        let store = store(&database);

        store.remember(None, "first", "1").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.remember(None, "second", "2").unwrap();

        let listed = store.memories(None, 10).unwrap();
        assert_eq!(listed[0].key, "second");
        assert_eq!(store.memories(None, 1).unwrap().len(), 1);
    }

    #[test]
    fn forgetting_says_whether_there_was_anything_to_forget() {
        let database = Database::open_in_memory().unwrap();
        let store = store(&database);

        store.remember(None, "note", "value").unwrap();
        assert!(store.forget(None, "note").unwrap());
        assert!(!store.forget(None, "note").unwrap());
        assert!(store.recall(None, "note").unwrap().is_none());
    }

    #[test]
    fn keys_are_trimmed_so_a_stray_space_is_not_a_second_note() {
        let database = Database::open_in_memory().unwrap();
        let store = store(&database);

        store.remember(None, "  note  ", "value").unwrap();
        assert!(store.recall(None, "note").unwrap().is_some());
        assert_eq!(store.memories(None, 10).unwrap().len(), 1);
    }

    #[test]
    fn a_note_without_a_key_is_refused() {
        let database = Database::open_in_memory().unwrap();
        let error = store(&database).remember(None, "   ", "value").unwrap_err();

        assert!(matches!(error, StoreError::CorruptRow { .. }));
    }

    #[test]
    fn a_note_too_large_to_be_a_note_is_refused() {
        let database = Database::open_in_memory().unwrap();
        let oversized = "x".repeat(MAX_VALUE_BYTES + 1);

        let error = store(&database)
            .remember(None, "big", &oversized)
            .unwrap_err();
        assert!(matches!(error, StoreError::CorruptRow { .. }));

        // And nothing was written.
        assert!(store(&database).recall(None, "big").unwrap().is_none());
    }
}
