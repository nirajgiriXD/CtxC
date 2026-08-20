-- Notes an agent asked CtxC to remember.
--
-- Separate from `contexts`: a context is a captured input CtxC can hand back
-- verbatim, and a note is something an agent decided was worth keeping. They
-- have different lifetimes and different owners, and merging them would make
-- pruning one prune the other.
--
-- Scoped by project rather than global. A note about one repository's build
-- quirks is noise in another, and an agent that reads every note ever written
-- has the context problem CtxC exists to solve.

CREATE TABLE memories (
    -- Project id, or '' for notes that belong to no project. Empty rather than
    -- NULL because it is half of the primary key.
    project_id TEXT    NOT NULL DEFAULT '',
    -- The agent's own name for this note; writing the same key twice replaces.
    key        TEXT    NOT NULL,
    value      TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (project_id, key)
) STRICT;

CREATE INDEX memories_recent_idx ON memories (project_id, updated_at DESC);
