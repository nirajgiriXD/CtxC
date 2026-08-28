//! Storing vectors next to the files they describe.
//!
//! The provider and dimension count travel with every row, and reads filter on
//! them. That is the whole trick: vectors from two providers are not comparable,
//! and comparing them anyway yields numbers that look like answers. Rows written
//! by a provider this build is not using are ignored rather than mixed in.

use rusqlite::{Connection, OptionalExtension};

use ctxc_core::Timestamp;
use ctxc_semantic::Embedding;

use crate::db::Database;
use crate::error::{Result, StoreError};

/// One stored vector, with what produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEmbedding {
    pub path: String,
    pub embedding: Embedding,
    /// Fingerprint of the text that was embedded.
    pub content_hash: String,
}

/// Which vectors a caller can use.
///
/// Both halves matter: the same provider at a different dimension count
/// produces vectors that are the wrong length, and the same length from a
/// different provider produces vectors that mean something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provider<'a> {
    pub name: &'a str,
    pub dimensions: usize,
}

/// Reading and writing embeddings.
pub trait EmbeddingStore {
    /// Write a file's vector, replacing any earlier one.
    fn put_embedding(
        &self,
        root: &str,
        path: &str,
        provider: Provider<'_>,
        embedding: &Embedding,
        content_hash: &str,
    ) -> Result<()>;

    /// One file's vector, if it was written by this provider.
    fn embedding(
        &self,
        root: &str,
        path: &str,
        provider: Provider<'_>,
    ) -> Result<Option<StoredEmbedding>>;

    /// Every vector under a root that this provider can use.
    fn embeddings(&self, root: &str, provider: Provider<'_>) -> Result<Vec<StoredEmbedding>>;

    /// Vectors for named files only, for re-ranking a candidate set.
    fn embeddings_for(
        &self,
        root: &str,
        paths: &[String],
        provider: Provider<'_>,
    ) -> Result<Vec<StoredEmbedding>>;

    /// Drop a file's vector, when the file is gone from the index.
    fn delete_embedding(&self, root: &str, path: &str) -> Result<()>;

    /// How many vectors this provider has under a root.
    fn embedding_count(&self, root: &str, provider: Provider<'_>) -> Result<u64>;
}

/// SQLite-backed [`EmbeddingStore`].
pub struct SqliteEmbeddingStore<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteEmbeddingStore<'a> {
    pub fn new(database: &'a Database) -> Self {
        SqliteEmbeddingStore {
            conn: database.connection(),
        }
    }
}

impl EmbeddingStore for SqliteEmbeddingStore<'_> {
    fn put_embedding(
        &self,
        root: &str,
        path: &str,
        provider: Provider<'_>,
        embedding: &Embedding,
        content_hash: &str,
    ) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO embeddings
                     (root, path, provider, dimensions, vector, content_hash, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(root, path) DO UPDATE SET
                 provider     = excluded.provider,
                 dimensions   = excluded.dimensions,
                 vector       = excluded.vector,
                 content_hash = excluded.content_hash,
                 updated_at   = excluded.updated_at",
            )?
            .execute(rusqlite::params![
                root,
                path,
                provider.name,
                provider.dimensions as i64,
                embedding.to_bytes(),
                content_hash,
                Timestamp::now().as_millis(),
            ])?;
        Ok(())
    }

    fn embedding(
        &self,
        root: &str,
        path: &str,
        provider: Provider<'_>,
    ) -> Result<Option<StoredEmbedding>> {
        let row = self
            .conn
            .prepare_cached(
                "SELECT path, vector, content_hash FROM embeddings
                 WHERE root = ?1 AND path = ?2 AND provider = ?3 AND dimensions = ?4",
            )?
            .query_row(
                rusqlite::params![root, path, provider.name, provider.dimensions as i64],
                read,
            )
            .optional()?;

        row.transpose()
    }

    fn embeddings(&self, root: &str, provider: Provider<'_>) -> Result<Vec<StoredEmbedding>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT path, vector, content_hash FROM embeddings
             WHERE root = ?1 AND provider = ?2 AND dimensions = ?3
             ORDER BY path",
        )?;

        let rows = statement.query_map(
            rusqlite::params![root, provider.name, provider.dimensions as i64],
            read,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect()
    }

    fn embeddings_for(
        &self,
        root: &str,
        paths: &[String],
        provider: Provider<'_>,
    ) -> Result<Vec<StoredEmbedding>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }

        // Built rather than bound as one parameter: SQLite has no array type,
        // and the count is the number of search results, which is small.
        let placeholders = (0..paths.len())
            .map(|index| format!("?{}", index + 4))
            .collect::<Vec<String>>()
            .join(", ");

        let mut statement = self.conn.prepare(&format!(
            "SELECT path, vector, content_hash FROM embeddings
             WHERE root = ?1 AND provider = ?2 AND dimensions = ?3
               AND path IN ({placeholders})"
        ))?;

        let mut parameters: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(root.to_owned()),
            Box::new(provider.name.to_owned()),
            Box::new(provider.dimensions as i64),
        ];
        for path in paths {
            parameters.push(Box::new(path.clone()));
        }

        let rows = statement.query_map(rusqlite::params_from_iter(parameters.iter()), read)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect()
    }

    fn delete_embedding(&self, root: &str, path: &str) -> Result<()> {
        self.conn
            .prepare_cached("DELETE FROM embeddings WHERE root = ?1 AND path = ?2")?
            .execute([root, path])?;
        Ok(())
    }

    fn embedding_count(&self, root: &str, provider: Provider<'_>) -> Result<u64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM embeddings
             WHERE root = ?1 AND provider = ?2 AND dimensions = ?3",
            rusqlite::params![root, provider.name, provider.dimensions as i64],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u64)
    }
}

/// Read one row, refusing a vector that will not decode.
fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<StoredEmbedding>> {
    let path: String = row.get(0)?;
    let bytes: Vec<u8> = row.get(1)?;

    let Some(embedding) = Embedding::from_bytes(&bytes) else {
        return Ok(Err(StoreError::CorruptRow {
            id: path,
            reason: "the stored vector could not be decoded".into(),
        }));
    };

    Ok(Ok(StoredEmbedding {
        path,
        embedding,
        content_hash: row.get(2)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_semantic::{Embedder, HashedEmbedder};

    const ROOT: &str = "/work/app";

    fn provider() -> Provider<'static> {
        Provider {
            name: "hashed",
            dimensions: 256,
        }
    }

    fn embed(text: &str) -> Embedding {
        HashedEmbedder::default().embed(text)
    }

    #[test]
    fn a_vector_round_trips() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);
        let original = embed("fn authenticate(user)");

        store
            .put_embedding(ROOT, "src/auth.rs", provider(), &original, "abc123")
            .unwrap();

        let stored = store
            .embedding(ROOT, "src/auth.rs", provider())
            .unwrap()
            .unwrap();
        assert_eq!(stored.embedding, original);
        assert_eq!(stored.content_hash, "abc123");
    }

    #[test]
    fn writing_a_file_twice_replaces_rather_than_duplicates() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);

        store
            .put_embedding(ROOT, "a.rs", provider(), &embed("first"), "h1")
            .unwrap();
        store
            .put_embedding(ROOT, "a.rs", provider(), &embed("second"), "h2")
            .unwrap();

        assert_eq!(store.embedding_count(ROOT, provider()).unwrap(), 1);
        assert_eq!(
            store
                .embedding(ROOT, "a.rs", provider())
                .unwrap()
                .unwrap()
                .content_hash,
            "h2"
        );
    }

    #[test]
    fn vectors_from_another_provider_are_not_returned() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);

        let other = Provider {
            name: "some-model",
            dimensions: 384,
        };
        store
            .put_embedding(ROOT, "a.rs", other, &embed("text"), "h")
            .unwrap();

        assert!(
            store.embedding(ROOT, "a.rs", provider()).unwrap().is_none(),
            "comparing across providers produces numbers that mean nothing"
        );
        assert_eq!(store.embedding_count(ROOT, provider()).unwrap(), 0);
        assert!(store.embeddings(ROOT, provider()).unwrap().is_empty());
    }

    #[test]
    fn the_same_provider_at_a_different_size_is_also_refused() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);

        let smaller = Provider {
            name: "hashed",
            dimensions: 64,
        };
        store
            .put_embedding(ROOT, "a.rs", smaller, &embed("text"), "h")
            .unwrap();

        assert!(store.embedding(ROOT, "a.rs", provider()).unwrap().is_none());
    }

    #[test]
    fn roots_do_not_see_each_others_vectors() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);

        store
            .put_embedding(ROOT, "a.rs", provider(), &embed("one"), "h")
            .unwrap();
        store
            .put_embedding("/work/other", "a.rs", provider(), &embed("two"), "h")
            .unwrap();

        assert_eq!(store.embeddings(ROOT, provider()).unwrap().len(), 1);
    }

    #[test]
    fn a_candidate_set_reads_back_only_what_was_asked_for() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);

        for path in ["a.rs", "b.rs", "c.rs"] {
            store
                .put_embedding(ROOT, path, provider(), &embed(path), "h")
                .unwrap();
        }

        let wanted = vec!["a.rs".to_string(), "c.rs".to_string()];
        let found = store.embeddings_for(ROOT, &wanted, provider()).unwrap();

        let paths: Vec<&str> = found.iter().map(|stored| stored.path.as_str()).collect();
        assert_eq!(paths.len(), 2);
        assert!(
            paths.contains(&"a.rs") && paths.contains(&"c.rs"),
            "{paths:?}"
        );
    }

    #[test]
    fn asking_for_nothing_returns_nothing_without_a_query() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);

        assert!(store
            .embeddings_for(ROOT, &[], provider())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_deleted_file_takes_its_vector_with_it() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);

        store
            .put_embedding(ROOT, "gone.rs", provider(), &embed("text"), "h")
            .unwrap();
        store.delete_embedding(ROOT, "gone.rs").unwrap();

        assert!(store
            .embedding(ROOT, "gone.rs", provider())
            .unwrap()
            .is_none());
        // Deleting what is not there is not an error.
        store.delete_embedding(ROOT, "gone.rs").unwrap();
    }

    #[test]
    fn a_corrupt_vector_is_reported_rather_than_misread() {
        let database = Database::open_in_memory().unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO embeddings
                     (root, path, provider, dimensions, vector, content_hash, updated_at)
                 VALUES (?1, 'bad.rs', 'hashed', 256, X'00010203040506', 'h', 0)",
                [ROOT],
            )
            .unwrap();

        let store = SqliteEmbeddingStore::new(&database);
        let error = store.embedding(ROOT, "bad.rs", provider()).unwrap_err();
        assert!(matches!(error, StoreError::CorruptRow { .. }));
    }
}
