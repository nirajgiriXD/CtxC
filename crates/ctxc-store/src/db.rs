//! Database handle: opening, pragmas, and schema upkeep.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

use ctxc_core::Paths;

use crate::error::{Result, StoreError};
use crate::migrations;

/// How long a writer waits for a lock before giving up. The daemon and a CLI
/// invocation can legitimately overlap; waiting briefly is better than failing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// An open CtxC database, migrated to the current schema version.
pub struct Database {
    conn: Connection,
    /// `None` for in-memory databases.
    path: Option<PathBuf>,
}

impl Database {
    /// Open (creating if needed) the database at `path` and migrate it.
    ///
    /// Parent directories are created, because on a fresh install the platform
    /// data directory does not exist yet.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                Paths::ensure_dir(parent)?;
            }
        }

        let conn = Connection::open(path).map_err(|source| StoreError::Open {
            path: path.to_path_buf(),
            source,
        })?;

        let mut database = Database {
            conn,
            path: Some(path.to_path_buf()),
        };
        database.configure()?;
        database.migrate()?;
        Ok(database)
    }

    /// A private database that never touches the filesystem. Used by tests and
    /// by one-shot commands that must not disturb persistent state.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|source| StoreError::Open {
            path: PathBuf::from(":memory:"),
            source,
        })?;
        let mut database = Database { conn, path: None };
        database.configure()?;
        database.migrate()?;
        Ok(database)
    }

    /// Borrow the underlying connection. Only repository implementations in
    /// this crate should need it.
    pub(crate) fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Where this database lives, if it is on disk.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Schema version currently recorded in the database.
    pub fn schema_version(&self) -> Result<u32> {
        migrations::current_version(&self.conn)
    }

    /// Run `work` inside a transaction, committing on success.
    ///
    /// Indexing writes thousands of small rows; without a surrounding
    /// transaction each one would be its own durable commit, which is roughly
    /// two orders of magnitude slower. It also makes an interrupted index run
    /// leave the previous index intact rather than half replaced.
    pub fn transaction<T, E>(
        &self,
        work: impl FnOnce() -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<StoreError>,
    {
        let transaction = self
            .conn
            .unchecked_transaction()
            .map_err(|source| E::from(StoreError::from(source)))?;

        let value = work()?;

        transaction
            .commit()
            .map_err(|source| E::from(StoreError::from(source)))?;
        Ok(value)
    }

    /// Bytes on disk, including the write-ahead log. `None` for in-memory
    /// databases or when the files cannot be stat'ed.
    pub fn size_on_disk(&self) -> Option<u64> {
        let path = self.path.as_ref()?;
        let main = std::fs::metadata(path).ok()?.len();
        // SQLite appends "-wal" to the full filename; it is not an extension.
        let mut wal = path.as_os_str().to_owned();
        wal.push("-wal");
        let wal_len = std::fs::metadata(PathBuf::from(wal))
            .map(|meta| meta.len())
            .unwrap_or(0);
        Some(main + wal_len)
    }

    /// Durability and concurrency settings.
    ///
    /// WAL keeps a reader (a CLI invocation) from blocking the daemon's writes;
    /// `synchronous = NORMAL` is the standard companion, trading a fsync per
    /// commit for one per checkpoint. Neither can corrupt the database on an
    /// unclean shutdown.
    fn configure(&self) -> Result<()> {
        self.conn.busy_timeout(BUSY_TIMEOUT)?;
        self.conn
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;

        if self.path.is_some() {
            let mode: String = self
                .conn
                .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
            if !mode.eq_ignore_ascii_case("wal") {
                // Network filesystems refuse WAL. Journalled mode still works.
                tracing::warn!(
                    mode,
                    "write-ahead logging unavailable; using fallback journal"
                );
            }
        }

        Ok(())
    }

    fn migrate(&mut self) -> Result<()> {
        let applied = migrations::migrate(&mut self.conn)?;
        if !applied.is_empty() {
            tracing::info!(?applied, "database schema updated");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts5_is_available_in_the_bundled_sqlite() {
        let database = Database::open_in_memory().unwrap();
        database
            .connection()
            .execute_batch(
                "CREATE VIRTUAL TABLE probe USING fts5(body);
                 INSERT INTO probe(body) VALUES ('authentication timeout error');",
            )
            .expect("FTS5 must be compiled into the bundled SQLite");

        let rank: f64 = database
            .connection()
            .query_row(
                "SELECT bm25(probe) FROM probe WHERE probe MATCH 'timeout'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            rank < 0.0,
            "bm25 returns negative scores, better is smaller"
        );
    }
    #[test]
    fn in_memory_database_is_migrated() {
        let database = Database::open_in_memory().unwrap();
        assert_eq!(
            database.schema_version().unwrap(),
            migrations::latest_version()
        );
        assert!(database.path().is_none());
        assert!(database.size_on_disk().is_none());
    }

    #[test]
    fn opening_creates_missing_directories() {
        let dir = std::env::temp_dir()
            .join("ctxc-db-tests")
            .join(format!("{}-nested", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("deeper").join("ctxc.db");

        let database = Database::open(&path).unwrap();
        assert!(path.exists());
        assert_eq!(database.path(), Some(path.as_path()));
        assert!(database.size_on_disk().is_some());

        drop(database);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reopening_keeps_the_schema_version() {
        let dir = std::env::temp_dir()
            .join("ctxc-db-tests")
            .join(format!("{}-reopen", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("ctxc.db");

        drop(Database::open(&path).unwrap());
        let database = Database::open(&path).unwrap();
        assert_eq!(
            database.schema_version().unwrap(),
            migrations::latest_version()
        );

        drop(database);
        std::fs::remove_dir_all(&dir).ok();
    }
}
