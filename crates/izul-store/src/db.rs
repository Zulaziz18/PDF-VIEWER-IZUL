//! Database open, pragmas, and migrations.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::{Result, StoreError};

/// Schema version this build understands. Bumped whenever a migration is added.
pub const APP_SCHEMA_VERSION: i64 = 2;
pub const CACHE_SCHEMA_VERSION: i64 = 1;

const APP_MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/app_001_initial.sql")),
    (2, include_str!("../migrations/app_002_index_state.sql")),
];
const CACHE_MIGRATIONS: &[(i64, &str)] =
    &[(1, include_str!("../migrations/cache_001_initial.sql"))];

/// Which of the two databases a connection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    /// `app.db`: losing it hurts.
    App,
    /// `cache.db`: safe to delete at any time.
    Cache,
}

impl Which {
    pub fn file_name(self) -> &'static str {
        match self {
            Which::App => "app.db",
            Which::Cache => "cache.db",
        }
    }

    fn migrations(self) -> &'static [(i64, &'static str)] {
        match self {
            Which::App => APP_MIGRATIONS,
            Which::Cache => CACHE_MIGRATIONS,
        }
    }

    fn target_version(self) -> i64 {
        match self {
            Which::App => APP_SCHEMA_VERSION,
            Which::Cache => CACHE_SCHEMA_VERSION,
        }
    }
}

/// Opens one of the databases, applying pragmas and migrations.
pub fn open(dir: &Path, which: Which) -> Result<Connection> {
    std::fs::create_dir_all(dir).map_err(|source| StoreError::DataDir {
        path: dir.display().to_string(),
        source,
    })?;
    let conn = Connection::open(dir.join(which.file_name()))?;
    apply_pragmas(&conn, which)?;
    migrate(&conn, which)?;
    Ok(conn)
}

/// Opens an in-memory database. Used by tests and by the first run of a portable
/// build before its data directory exists.
pub fn open_memory(which: Which) -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    apply_pragmas(&conn, which)?;
    migrate(&conn, which)?;
    Ok(conn)
}

fn apply_pragmas(conn: &Connection, which: Which) -> Result<()> {
    // WAL: readers never block the writer, which matters because the UI thread
    // reads recent files and reading positions while the indexer is writing
    // extracted text (SPEC 7).
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // NORMAL is the right durability point under WAL: a crash can lose the last
    // transaction but never corrupts the file. For app.db that costs at most one
    // autosave interval, which the draft system already tolerates.
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "busy_timeout", 5_000)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    match which {
        // Thumbnails are large blobs; a bigger page cache keeps the sidebar from
        // hitting the disk while scrolling.
        Which::Cache => conn.pragma_update(None, "cache_size", -32_000)?,
        Which::App => conn.pragma_update(None, "cache_size", -8_000)?,
    }
    Ok(())
}

fn migrate(conn: &Connection, which: Which) -> Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let target = which.target_version();
    if current > target {
        // A newer build wrote this file. Refusing is the only safe move: we
        // cannot know what its schema means, and guessing risks the user's
        // annotations.
        return Err(StoreError::SchemaFromTheFuture {
            found: current,
            expected: target,
        });
    }
    for (version, sql) in which.migrations() {
        if *version > current {
            conn.execute_batch(sql)?;
            conn.pragma_update(None, "user_version", *version)?;
        }
    }
    Ok(())
}

/// The file whose presence beside the executable makes a copy portable
/// (Phase 8): the portable ZIP ships with it, and an environment variable is
/// not something a user unpacking a ZIP can be asked to set.
pub const PORTABLE_MARKER: &str = "portable.txt";

/// Whether the copy in `exe_dir` runs portable: the marker is there, or
/// `IZUL_PORTABLE` is set (for development).
pub fn is_portable(exe_dir: Option<&Path>) -> bool {
    std::env::var_os("IZUL_PORTABLE").is_some()
        || exe_dir.is_some_and(|d| d.join(PORTABLE_MARKER).is_file())
}

/// Where the databases live.
///
/// A portable build keeps them beside the executable so the whole application
/// travels on a USB stick with its state intact; an installed build uses the
/// user's roaming profile.
pub fn default_data_dir(portable: bool) -> PathBuf {
    if portable {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("data")
    } else {
        dirs_app_data().join("PDF Studio Izul")
    }
}

fn dirs_app_data() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_beside_the_executable_makes_it_portable() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Checked only when the variable is not set by whoever runs the tests.
        if std::env::var_os("IZUL_PORTABLE").is_none() {
            assert!(!is_portable(Some(dir.path())), "tanpa penanda: terpasang");
            assert!(!is_portable(None));
        }
        std::fs::write(dir.path().join(PORTABLE_MARKER), "").expect("marker");
        assert!(is_portable(Some(dir.path())), "dengan penanda: portabel");
    }

    #[test]
    fn app_schema_creates_every_table_the_spec_names() {
        let conn = open_memory(Which::App).expect("open");
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view') ORDER BY name")
            .expect("prepare");
        let names: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .expect("query")
            .filter_map(|r| r.ok())
            .collect();
        for expected in [
            "files",
            "reading_state",
            "sessions",
            "session_tabs",
            "drafts",
            "bookmarks",
            "doc_text",
            "doc_fts",
            "export_history",
            "prefs",
            "shortcuts",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "missing table {expected} in {names:?}"
            );
        }
    }

    #[test]
    fn cache_schema_creates_its_tables() {
        let conn = open_memory(Which::Cache).expect("open");
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'thumbs'",
                [],
                |r| r.get(0),
            )
            .expect("query");
        assert_eq!(n, 1);
    }

    #[test]
    fn migrations_are_idempotent_across_reopens() {
        let dir = tempfile::tempdir().expect("tempdir");
        for _ in 0..3 {
            let conn = open(dir.path(), Which::App).expect("open");
            let v: i64 = conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .expect("v");
            assert_eq!(v, APP_SCHEMA_VERSION);
        }
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_guessed_at() {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let conn = open(dir.path(), Which::App).expect("open");
            conn.pragma_update(None, "user_version", APP_SCHEMA_VERSION + 5)
                .expect("bump");
        }
        match open(dir.path(), Which::App) {
            Err(StoreError::SchemaFromTheFuture { found, expected }) => {
                assert_eq!(found, APP_SCHEMA_VERSION + 5);
                assert_eq!(expected, APP_SCHEMA_VERSION);
            }
            other => panic!("expected SchemaFromTheFuture, got {other:?}"),
        }
    }

    #[test]
    fn wal_and_foreign_keys_are_on() {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = open(dir.path(), Which::App).expect("open");
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .expect("mode");
        assert_eq!(mode.to_lowercase(), "wal");
        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .expect("fk");
        assert_eq!(fk, 1);
    }

    #[test]
    fn fts_index_follows_its_content_table() {
        let conn = open_memory(Which::App).expect("open");
        conn.execute(
            "INSERT INTO files (path, path_hash, size, mtime) VALUES ('/a.pdf', X'00', 1, 1)",
            [],
        )
        .expect("file");
        let fid: i64 = conn
            .query_row("SELECT id FROM files", [], |r| r.get(0))
            .expect("id");
        conn.execute(
            "INSERT INTO doc_text (file_id, page, text) VALUES (?1, 0, 'anotasi sorotan halaman')",
            [fid],
        )
        .expect("text");

        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM doc_fts WHERE doc_fts MATCH 'sorotan'",
                [],
                |r| r.get(0),
            )
            .expect("search");
        assert_eq!(hits, 1, "insert must reach the index");

        conn.execute("DELETE FROM doc_text WHERE file_id = ?1", [fid])
            .expect("delete");
        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM doc_fts WHERE doc_fts MATCH 'sorotan'",
                [],
                |r| r.get(0),
            )
            .expect("search");
        assert_eq!(hits, 0, "delete must reach the index");
    }

    #[test]
    fn deleting_a_file_row_cascades_to_its_children() {
        let conn = open_memory(Which::App).expect("open");
        conn.execute(
            "INSERT INTO files (path, path_hash, size, mtime) VALUES ('/a.pdf', X'01', 1, 1)",
            [],
        )
        .expect("file");
        let fid: i64 = conn
            .query_row("SELECT id FROM files", [], |r| r.get(0))
            .expect("id");
        conn.execute(
            "INSERT INTO reading_state (file_id, updated_at) VALUES (?1, 0)",
            [fid],
        )
        .expect("state");
        conn.execute(
            "INSERT INTO bookmarks (file_id, page, label, created_at) VALUES (?1, 3, 'x', 0)",
            [fid],
        )
        .expect("bookmark");

        conn.execute("DELETE FROM files WHERE id = ?1", [fid])
            .expect("delete");
        for table in ["reading_state", "bookmarks"] {
            let n: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .expect("count");
            assert_eq!(n, 0, "{table} should have cascaded");
        }
    }

    #[test]
    fn rotation_outside_the_quarter_turns_is_refused() {
        let conn = open_memory(Which::App).expect("open");
        conn.execute(
            "INSERT INTO files (path, path_hash, size, mtime) VALUES ('/a.pdf', X'02', 1, 1)",
            [],
        )
        .expect("file");
        let fid: i64 = conn
            .query_row("SELECT id FROM files", [], |r| r.get(0))
            .expect("id");
        assert!(conn
            .execute(
                "INSERT INTO reading_state (file_id, rotation, updated_at) VALUES (?1, 7, 0)",
                [fid]
            )
            .is_err());
    }
}
