-- Code index: files, what they define, and how they relate.
--
-- Files are keyed by (root, path) rather than by an absolute path, so the same
-- repository indexed from a different mount point stays one index, and the
-- project registry can adopt these rows later without a rewrite.

CREATE TABLE files (
    id           INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    root         TEXT    NOT NULL,
    path         TEXT    NOT NULL,
    language     TEXT,
    size         INTEGER NOT NULL,
    mtime_ms     INTEGER NOT NULL,
    content_hash TEXT    NOT NULL,
    indexed_at   INTEGER NOT NULL,
    UNIQUE (root, path)
) STRICT;

CREATE INDEX files_root_idx ON files (root);

-- Symbols and relationships belong to their file: when a file is re-indexed or
-- deleted, everything derived from it goes with it.
CREATE TABLE symbols (
    id         INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    file_id    INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    kind       TEXT    NOT NULL,
    start_line INTEGER NOT NULL,
    end_line   INTEGER NOT NULL
) STRICT;

CREATE INDEX symbols_file_idx ON symbols (file_id);
CREATE INDEX symbols_name_idx ON symbols (name);

CREATE TABLE relationships (
    id          INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    file_id     INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL,
    from_symbol TEXT,
    target      TEXT    NOT NULL,
    -- Set when the target resolved to a file in the same root.
    target_path TEXT,
    line        INTEGER NOT NULL
) STRICT;

CREATE INDEX relationships_file_idx ON relationships (file_id);
CREATE INDEX relationships_target_idx ON relationships (target_path);

CREATE TABLE index_state (
    root            TEXT    NOT NULL PRIMARY KEY,
    last_indexed_at INTEGER NOT NULL,
    file_count      INTEGER NOT NULL,
    symbol_count    INTEGER NOT NULL
) STRICT;
