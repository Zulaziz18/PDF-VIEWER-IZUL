-- What the full-text index in `doc_text` / `doc_fts` was built from (SPEC 11.1).
--
-- Two problems this solves, neither of which the text itself can answer:
--
--   1. Staleness. Extracted text is only trustworthy while the file on disk is
--      still the file it was extracted from. Holding the stamp the index was
--      built against lets a changed document be re-indexed instead of answering
--      searches from text it no longer contains.
--   2. Resumption. Indexing a 500-page document is not instant, and the
--      application will be closed in the middle of one. `pages_done` is the
--      resume point, so the next run continues rather than starting over.
CREATE TABLE doc_index_state (
    file_id    INTEGER PRIMARY KEY REFERENCES files (id) ON DELETE CASCADE,
    -- The stamp of the file the text was read from, mirroring `files`.
    size       INTEGER NOT NULL,
    mtime      INTEGER NOT NULL,
    page_count INTEGER NOT NULL,
    -- Pages indexed so far, counted from page 0. Indexing runs in page order,
    -- so this is both a progress figure and the page to continue from.
    pages_done INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);
