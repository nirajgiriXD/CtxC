-- Embeddings, one per indexed file.
--
-- The provider and dimension count are stored with every row because vectors
-- from different providers are not comparable. Reading them back without
-- checking would produce plausible-looking similarity numbers that mean
-- nothing, which is worse than having none.
--
-- `content_hash` is the fingerprint of what was embedded, so re-indexing an
-- unchanged file skips the work.

CREATE TABLE embeddings (
    root         TEXT    NOT NULL,
    path         TEXT    NOT NULL,
    provider     TEXT    NOT NULL,
    dimensions   INTEGER NOT NULL,
    vector       BLOB    NOT NULL,
    content_hash TEXT    NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (root, path)
) STRICT;

CREATE INDEX embeddings_provider_idx ON embeddings (root, provider, dimensions);
