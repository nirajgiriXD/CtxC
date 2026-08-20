-- Metrics: raw per-operation events, and the aggregates that outlive them.
--
-- Neither table references `projects`. A project can be removed, and its
-- history is exactly what someone looking at metrics afterwards wants; a
-- foreign key would cascade it away.

CREATE TABLE metric_events (
    id                  INTEGER PRIMARY KEY,
    -- NULL for operations with no project: a piped `git status` has none.
    project_id          TEXT,
    operation           TEXT    NOT NULL,
    source              TEXT    NOT NULL,
    optimizer           TEXT,
    recorded_at         INTEGER NOT NULL,
    input_tokens        INTEGER NOT NULL,
    output_tokens       INTEGER NOT NULL,
    duration_ms         INTEGER NOT NULL,
    -- Signed: a stage can legitimately cost tokens.
    saved_filtering     INTEGER NOT NULL DEFAULT 0,
    saved_deduplication INTEGER NOT NULL DEFAULT 0,
    saved_compression   INTEGER NOT NULL DEFAULT 0,
    saved_selection     INTEGER NOT NULL DEFAULT 0,
    -- Cache lookups this operation answered. Zero lookups means it consulted
    -- no cache, which is not the same as consulting one and missing.
    cache_hits          INTEGER NOT NULL DEFAULT 0,
    cache_lookups       INTEGER NOT NULL DEFAULT 0,
    outcome             TEXT    NOT NULL,
    detail              TEXT,
    estimated           INTEGER NOT NULL DEFAULT 1
) STRICT;

-- Rolling up and pruning both scan by time; the activity feed reads the tail.
CREATE INDEX metric_events_time_idx ON metric_events (recorded_at);
CREATE INDEX metric_events_project_idx ON metric_events (project_id, recorded_at);

-- Aggregates, one row per bucket per project per operation.
--
-- `project_id` is '' rather than NULL for operations with no project, because
-- it is part of the primary key and SQLite treats NULLs as distinct from each
-- other — which would let the same bucket be inserted twice.
CREATE TABLE metric_rollups (
    granularity         TEXT    NOT NULL,
    bucket_start        INTEGER NOT NULL,
    project_id          TEXT    NOT NULL DEFAULT '',
    operation           TEXT    NOT NULL,
    operations          INTEGER NOT NULL,
    input_tokens        INTEGER NOT NULL,
    output_tokens       INTEGER NOT NULL,
    saved_filtering     INTEGER NOT NULL,
    saved_deduplication INTEGER NOT NULL,
    saved_compression   INTEGER NOT NULL,
    saved_selection     INTEGER NOT NULL,
    duration_ms_total   INTEGER NOT NULL,
    duration_ms_max     INTEGER NOT NULL,
    errors              INTEGER NOT NULL,
    degradations        INTEGER NOT NULL,
    cache_hits          INTEGER NOT NULL,
    cache_lookups       INTEGER NOT NULL,
    estimated           INTEGER NOT NULL,
    PRIMARY KEY (granularity, bucket_start, project_id, operation)
) STRICT;

CREATE INDEX metric_rollups_bucket_idx ON metric_rollups (granularity, bucket_start);
