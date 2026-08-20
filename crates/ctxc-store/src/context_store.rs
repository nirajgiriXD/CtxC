//! Context persistence, behind a repository trait.
//!
//! Callers depend on [`ContextStore`], never on rusqlite, so the backing store
//! stays replaceable.

use rusqlite::{Connection, OptionalExtension};

use ctxc_core::context::{ContentType, Context, ContextMetadata, ContextSource};
use ctxc_core::{ContextId, Timestamp};

use crate::db::Database;
use crate::error::{Result, StoreError};

/// Read and write access to stored contexts.
pub trait ContextStore {
    /// Persist a context. Contexts are content addressed, so saving the same
    /// context twice updates the existing row instead of duplicating it.
    fn save_context(&self, context: &Context) -> Result<()>;

    /// Fetch a context by id, or `None` when it is not stored.
    fn get_context(&self, id: &ContextId) -> Result<Option<Context>>;

    /// Delete a context, reporting whether a row was removed.
    fn delete_context(&self, id: &ContextId) -> Result<bool>;

    /// Number of stored contexts.
    fn count_contexts(&self) -> Result<u64>;
}

/// SQLite-backed [`ContextStore`].
pub struct SqliteContextStore<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteContextStore<'a> {
    pub fn new(database: &'a Database) -> Self {
        SqliteContextStore {
            conn: database.connection(),
        }
    }
}

impl ContextStore for SqliteContextStore<'_> {
    fn save_context(&self, context: &Context) -> Result<()> {
        let metadata = &context.metadata;
        self.conn.execute(
            "INSERT INTO contexts (
                 id, content, source_kind, source_ref, content_type,
                 token_count, byte_len, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                 content      = excluded.content,
                 source_kind  = excluded.source_kind,
                 source_ref   = excluded.source_ref,
                 content_type = excluded.content_type,
                 token_count  = excluded.token_count,
                 byte_len     = excluded.byte_len",
            rusqlite::params![
                context.id.as_str(),
                context.content,
                metadata.source.kind(),
                metadata.source.reference(),
                metadata.content_type.as_str(),
                metadata.token_count,
                metadata.byte_len as i64,
                metadata.created_at.as_millis(),
            ],
        )?;
        Ok(())
    }

    fn get_context(&self, id: &ContextId) -> Result<Option<Context>> {
        let row = self
            .conn
            .query_row(
                "SELECT content, source_kind, source_ref, content_type,
                        token_count, byte_len, created_at
                 FROM contexts WHERE id = ?1",
                [id.as_str()],
                |row| {
                    Ok(StoredRow {
                        content: row.get(0)?,
                        source_kind: row.get(1)?,
                        source_ref: row.get(2)?,
                        content_type: row.get(3)?,
                        token_count: row.get(4)?,
                        byte_len: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                },
            )
            .optional()?;

        row.map(|row| row.into_context(id)).transpose()
    }

    fn delete_context(&self, id: &ContextId) -> Result<bool> {
        let removed = self
            .conn
            .execute("DELETE FROM contexts WHERE id = ?1", [id.as_str()])?;
        Ok(removed > 0)
    }

    fn count_contexts(&self) -> Result<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM contexts", [], |row| row.get(0))?;
        Ok(count as u64)
    }
}

/// A row as SQLite hands it back, before it is validated into a [`Context`].
struct StoredRow {
    content: String,
    source_kind: String,
    source_ref: Option<String>,
    content_type: String,
    token_count: Option<u32>,
    byte_len: i64,
    created_at: i64,
}

impl StoredRow {
    fn into_context(self, id: &ContextId) -> Result<Context> {
        let byte_len = u64::try_from(self.byte_len).map_err(|_| StoreError::CorruptRow {
            id: id.to_string(),
            reason: format!("negative byte length {}", self.byte_len),
        })?;

        let metadata = ContextMetadata {
            source: ContextSource::from_parts(&self.source_kind, self.source_ref),
            content_type: ContentType::from_str_lossy(&self.content_type),
            token_count: self.token_count,
            byte_len,
            created_at: Timestamp::from_millis(self.created_at),
        };

        Ok(Context::from_parts(id.clone(), self.content, metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample() -> Context {
        Context::new(
            ContextSource::File {
                path: PathBuf::from("src/auth.rs"),
            },
            ContentType::Code,
            "fn authenticate() {}",
        )
        .with_token_count(7)
    }

    #[test]
    fn saves_and_reads_back() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteContextStore::new(&database);
        let context = sample();

        store.save_context(&context).unwrap();
        let loaded = store.get_context(&context.id).unwrap().unwrap();

        assert_eq!(loaded, context);
        assert_eq!(store.count_contexts().unwrap(), 1);
    }

    #[test]
    fn saving_twice_is_idempotent() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteContextStore::new(&database);
        let context = sample();

        store.save_context(&context).unwrap();
        store.save_context(&context).unwrap();

        assert_eq!(store.count_contexts().unwrap(), 1);
    }

    #[test]
    fn missing_contexts_read_as_none() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteContextStore::new(&database);
        let id = ContextId::from_content("stdin", b"never stored");

        assert!(store.get_context(&id).unwrap().is_none());
        assert!(!store.delete_context(&id).unwrap());
    }

    #[test]
    fn deleting_removes_the_row() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteContextStore::new(&database);
        let context = sample();

        store.save_context(&context).unwrap();
        assert!(store.delete_context(&context.id).unwrap());
        assert_eq!(store.count_contexts().unwrap(), 0);
    }

    #[test]
    fn sources_without_a_reference_roundtrip() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteContextStore::new(&database);
        let context = Context::new(ContextSource::Stdin, ContentType::PlainText, "hello");

        store.save_context(&context).unwrap();
        let loaded = store.get_context(&context.id).unwrap().unwrap();

        assert_eq!(loaded.metadata.source, ContextSource::Stdin);
        assert_eq!(loaded.metadata.token_count, None);
    }

    #[test]
    fn unicode_content_survives_the_roundtrip() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteContextStore::new(&database);
        let context = Context::new(
            ContextSource::Command {
                command: "git log".into(),
            },
            ContentType::Terminal,
            "commit \u{2713}\r\nauthor: \u{4e2d}\u{6587}\n",
        );

        store.save_context(&context).unwrap();
        assert_eq!(store.get_context(&context.id).unwrap().unwrap(), context);
    }
}
