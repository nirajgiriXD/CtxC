-- Full-text search over indexed file content.
--
-- The FTS table's rowid is the file's id, which is what ties a match back to
-- everything else the index knows about that file. FTS5 has no foreign keys, so
-- the rows are maintained explicitly alongside the file they belong to.
--
-- The default tokenizer splits on underscores and case boundaries are left
-- alone, so `get_user` indexes as `get` and `user`: someone searching for
-- "user" should find it.

CREATE VIRTUAL TABLE file_search USING fts5(
    path UNINDEXED,
    content,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- The index is derived data, so the cheapest way to make existing installs
-- searchable is to have them rebuild it: without this, files already recorded
-- would keep passing the "unchanged" check and never have their text indexed.
DELETE FROM files;
DELETE FROM index_state;
