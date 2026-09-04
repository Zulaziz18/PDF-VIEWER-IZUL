-- app.db — state whose loss hurts (SPEC 7).
--
-- Kept separate from cache.db on purpose: cache.db can be deleted at any time
-- without losing anything, and this file cannot.

CREATE TABLE files (
    id           INTEGER PRIMARY KEY,
    path         TEXT    NOT NULL,
    -- BLAKE3 of the normalised absolute path. Indexed instead of `path` so
    -- lookups do not compare long strings, and so a case-insensitive Windows
    -- path maps to one row.
    path_hash    BLOB    NOT NULL UNIQUE,
    size         INTEGER NOT NULL,
    mtime        INTEGER NOT NULL,
    last_opened  INTEGER,
    pinned       INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1))
);
CREATE INDEX files_last_opened ON files (last_opened DESC);
CREATE INDEX files_pinned ON files (pinned, last_opened DESC);

CREATE TABLE reading_state (
    file_id    INTEGER PRIMARY KEY REFERENCES files (id) ON DELETE CASCADE,
    page       INTEGER NOT NULL DEFAULT 0,
    scroll_y   REAL    NOT NULL DEFAULT 0,
    zoom       REAL    NOT NULL DEFAULT 1.0,
    view_mode  TEXT    NOT NULL DEFAULT 'single',
    rotation   INTEGER NOT NULL DEFAULT 0 CHECK (rotation IN (0, 1, 2, 3)),
    updated_at INTEGER NOT NULL
);

CREATE TABLE sessions (
    id         INTEGER PRIMARY KEY,
    created_at INTEGER NOT NULL
);

CREATE TABLE session_tabs (
    session_id INTEGER NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    file_id    INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    -- Which of the up-to-four split panels holds this tab (SPEC 10).
    panel      INTEGER NOT NULL DEFAULT 0 CHECK (panel BETWEEN 0 AND 3),
    tab_order  INTEGER NOT NULL,
    pinned     INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    is_active  INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0, 1)),
    PRIMARY KEY (session_id, file_id, panel)
);
CREATE INDEX session_tabs_order ON session_tabs (session_id, panel, tab_order);

CREATE TABLE drafts (
    file_id    INTEGER PRIMARY KEY REFERENCES files (id) ON DELETE CASCADE,
    -- Hash of the document the operations were recorded against. A draft is
    -- never replayed onto a file that has changed underneath it (SPEC 8).
    base_hash  BLOB    NOT NULL,
    ops_blob   BLOB    NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE bookmarks (
    id         INTEGER PRIMARY KEY,
    file_id    INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    page       INTEGER NOT NULL,
    label      TEXT    NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX bookmarks_file ON bookmarks (file_id, page);

-- Extracted page text. `doc_fts` is an external-content FTS5 index over this
-- table, so the text is stored once and the index holds only postings.
CREATE TABLE doc_text (
    file_id INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    page    INTEGER NOT NULL,
    text    TEXT    NOT NULL,
    PRIMARY KEY (file_id, page)
);

CREATE VIRTUAL TABLE doc_fts USING fts5 (
    text,
    content = 'doc_text',
    content_rowid = 'rowid',
    tokenize = 'unicode61 remove_diacritics 2'
);

-- External-content FTS5 does not track its content table by itself.
CREATE TRIGGER doc_text_ai AFTER INSERT ON doc_text BEGIN
    INSERT INTO doc_fts (rowid, text) VALUES (new.rowid, new.text);
END;
CREATE TRIGGER doc_text_ad AFTER DELETE ON doc_text BEGIN
    INSERT INTO doc_fts (doc_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
END;
CREATE TRIGGER doc_text_au AFTER UPDATE ON doc_text BEGIN
    INSERT INTO doc_fts (doc_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
    INSERT INTO doc_fts (rowid, text) VALUES (new.rowid, new.text);
END;

CREATE TABLE export_history (
    id         INTEGER PRIMARY KEY,
    file_id    INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
    out_path   TEXT    NOT NULL,
    kind       TEXT    NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX export_history_recent ON export_history (created_at DESC);

CREATE TABLE prefs (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE shortcuts (
    command TEXT PRIMARY KEY,
    keys    TEXT NOT NULL
);
