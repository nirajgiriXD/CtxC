-- The project registry.
--
-- A project's identity is a generated id rather than its path, so that moving
-- or renaming a directory keeps its history. The path is still unique: the same
-- directory must never be registered twice.

CREATE TABLE projects (
    id              TEXT    NOT NULL PRIMARY KEY,
    name            TEXT    NOT NULL,
    -- Canonical root key, the same form the index uses.
    path            TEXT    NOT NULL UNIQUE,
    status          TEXT    NOT NULL,
    added_at        INTEGER NOT NULL,
    last_indexed_at INTEGER,
    -- Detection results: hints, never requirements, and re-runnable.
    languages       TEXT    NOT NULL DEFAULT '',
    frameworks      TEXT    NOT NULL DEFAULT '',
    package_manager TEXT,
    has_git         INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE INDEX projects_status_idx ON projects (status);

-- Per-project overrides, kept as key/value so a new setting needs no migration.
CREATE TABLE project_settings (
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    value      TEXT NOT NULL,
    PRIMARY KEY (project_id, key)
) STRICT;
