-- Initial schema.
--
-- Contexts are content addressed, so writing the same context twice replaces
-- the same row rather than creating a duplicate. Source provenance is stored as
-- a (kind, reference) pair so that new source variants need no schema change.

CREATE TABLE contexts (
    id           TEXT    NOT NULL PRIMARY KEY,
    content      TEXT    NOT NULL,
    source_kind  TEXT    NOT NULL,
    source_ref   TEXT,
    content_type TEXT    NOT NULL,
    token_count  INTEGER,
    byte_len     INTEGER NOT NULL,
    created_at   INTEGER NOT NULL
) STRICT;

CREATE INDEX contexts_created_at_idx ON contexts (created_at DESC);
CREATE INDEX contexts_source_idx ON contexts (source_kind, source_ref);
