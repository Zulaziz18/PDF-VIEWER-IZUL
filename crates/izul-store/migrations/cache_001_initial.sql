-- cache.db — reconstructible data only (SPEC 7).
--
-- Safe to delete at any time. Nothing in here is the only copy of anything.

CREATE TABLE thumbs (
    path_hash  BLOB    NOT NULL,
    page       INTEGER NOT NULL,
    -- Included in the key because a thumbnail rendered for a 100% display is
    -- not the one a 175% display needs (SPEC 9).
    scale_bp   INTEGER NOT NULL,
    width      INTEGER NOT NULL,
    height     INTEGER NOT NULL,
    -- File identity at render time. A mismatch means the file changed and the
    -- thumbnail is wrong, so it is discarded rather than shown.
    size       INTEGER NOT NULL,
    mtime      INTEGER NOT NULL,
    png        BLOB    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (path_hash, page, scale_bp)
);
CREATE INDEX thumbs_age ON thumbs (created_at);

CREATE TABLE cache_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
