-- Optimizer output, keyed by everything that decides what it is.
--
-- Agents re-run the same commands constantly, so the same bytes are optimized
-- over and over for an answer that cannot have changed. Four things decide the
-- output: the content, which optimizer handled it, the budget it had to fit,
-- and the settings the engine was built with. All four are in the key, so a
-- hit is a guarantee rather than a guess — a configuration change produces a
-- different `settings` and therefore a miss, not a stale answer.
--
-- This is a cache: nothing here is the only copy of anything, and deleting
-- every row costs time and no information.

CREATE TABLE optimizations (
    content_hash TEXT    NOT NULL,
    optimizer    TEXT    NOT NULL,
    budget       INTEGER NOT NULL,
    settings     TEXT    NOT NULL,
    content      TEXT    NOT NULL,
    -- The measurements, as the JSON the CLI already reports.
    result       TEXT    NOT NULL,
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (content_hash, optimizer, budget, settings)
) STRICT;

CREATE INDEX optimizations_created_idx ON optimizations (created_at);
