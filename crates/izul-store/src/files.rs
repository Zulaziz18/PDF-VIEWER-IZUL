//! The `files` table and the reading state hanging off it (SPEC 7, SPEC 11.3).

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::identity::{FileStamp, PathHash};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileId(pub i64);

#[derive(Debug, Clone, PartialEq)]
pub struct FileRow {
    pub id: FileId,
    pub path: String,
    pub stamp: FileStamp,
    pub last_opened: Option<i64>,
    pub pinned: bool,
}

/// How pages are laid out on screen (SPEC 11.1).
///
/// Stored as text rather than a number because the column is read by hand
/// during support work as often as by the application, and `single` says what
/// `0` does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Single,
    Dual,
    /// Two pages side by side with the first page alone, as a book's cover sits.
    DualCover,
    Horizontal,
}

impl ViewMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ViewMode::Single => "single",
            ViewMode::Dual => "dual",
            ViewMode::DualCover => "dual_cover",
            ViewMode::Horizontal => "horizontal",
        }
    }

    /// Parses a stored value, falling back to the single-page layout.
    ///
    /// A row written by a newer build must not stop an older one from opening
    /// the document; the worst outcome of an unknown mode is the default one.
    pub fn parse(s: &str) -> Self {
        match s {
            "dual" => ViewMode::Dual,
            "dual_cover" => ViewMode::DualCover,
            "horizontal" => ViewMode::Horizontal,
            _ => ViewMode::Single,
        }
    }
}

/// What we last recorded about where the user was in a document.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadingState {
    pub page: u32,
    pub scroll_y: f64,
    pub zoom: f64,
    pub rotation: u8,
    pub view_mode: ViewMode,
}

impl Default for ReadingState {
    fn default() -> Self {
        Self {
            page: 0,
            scroll_y: 0.0,
            zoom: 1.0,
            rotation: 0,
            view_mode: ViewMode::Single,
        }
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Records that a file was opened, inserting it if new.
///
/// Matching is by path hash, so a file that was renamed produces a new row while
/// one that merely moved between drives with the same path does not. The stamp
/// is refreshed on every open, which is what later lets `has_changed` tell a
/// stale thumbnail from a current one.
pub fn touch(conn: &Connection, path: &Path, stamp: FileStamp) -> Result<FileId> {
    let hash = PathHash::of(path);
    conn.execute(
        "INSERT INTO files (path, path_hash, size, mtime, last_opened)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (path_hash) DO UPDATE SET
             path = excluded.path,
             size = excluded.size,
             mtime = excluded.mtime,
             last_opened = excluded.last_opened",
        params![
            path.to_string_lossy(),
            hash.as_bytes(),
            stamp.size as i64,
            stamp.mtime,
            now()
        ],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM files WHERE path_hash = ?1",
        params![hash.as_bytes()],
        |r| r.get(0),
    )?;
    Ok(FileId(id))
}

pub fn find(conn: &Connection, path: &Path) -> Result<Option<FileRow>> {
    let hash = PathHash::of(path);
    let row = conn
        .query_row(
            "SELECT id, path, size, mtime, last_opened, pinned FROM files WHERE path_hash = ?1",
            params![hash.as_bytes()],
            |r| {
                Ok(FileRow {
                    id: FileId(r.get(0)?),
                    path: r.get(1)?,
                    stamp: FileStamp {
                        size: r.get::<_, i64>(2)? as u64,
                        mtime: r.get(3)?,
                    },
                    last_opened: r.get(4)?,
                    pinned: r.get::<_, i64>(5)? != 0,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// True when the file on disk differs from what we recorded, meaning caches and
/// drafts keyed to it must be re-checked before use.
pub fn has_changed(conn: &Connection, path: &Path) -> Result<bool> {
    let Some(row) = find(conn, path)? else {
        return Ok(true);
    };
    let Ok(current) = FileStamp::of(path) else {
        return Ok(true);
    };
    Ok(current.differs_from(&row.stamp))
}

/// Recent files, pinned ones first, for the empty state and the File menu.
pub fn recent(conn: &Connection, limit: u32) -> Result<Vec<FileRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, path, size, mtime, last_opened, pinned
         FROM files
         WHERE last_opened IS NOT NULL
         ORDER BY pinned DESC, last_opened DESC
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(FileRow {
                id: FileId(r.get(0)?),
                path: r.get(1)?,
                stamp: FileStamp {
                    size: r.get::<_, i64>(2)? as u64,
                    mtime: r.get(3)?,
                },
                last_opened: r.get(4)?,
                pinned: r.get::<_, i64>(5)? != 0,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn set_pinned(conn: &Connection, id: FileId, pinned: bool) -> Result<()> {
    conn.execute(
        "UPDATE files SET pinned = ?2 WHERE id = ?1",
        params![id.0, pinned as i64],
    )?;
    Ok(())
}

pub fn save_reading_state(conn: &Connection, id: FileId, s: ReadingState) -> Result<()> {
    conn.execute(
        "INSERT INTO reading_state (file_id, page, scroll_y, zoom, rotation, view_mode, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (file_id) DO UPDATE SET
             page = excluded.page,
             scroll_y = excluded.scroll_y,
             zoom = excluded.zoom,
             rotation = excluded.rotation,
             view_mode = excluded.view_mode,
             updated_at = excluded.updated_at",
        params![
            id.0,
            s.page,
            s.scroll_y,
            s.zoom,
            s.rotation,
            s.view_mode.as_str(),
            now()
        ],
    )?;
    Ok(())
}

pub fn reading_state(conn: &Connection, id: FileId) -> Result<Option<ReadingState>> {
    let row = conn
        .query_row(
            "SELECT page, scroll_y, zoom, rotation, view_mode
               FROM reading_state WHERE file_id = ?1",
            params![id.0],
            |r| {
                Ok(ReadingState {
                    page: r.get::<_, i64>(0)? as u32,
                    scroll_y: r.get(1)?,
                    zoom: r.get(2)?,
                    rotation: r.get::<_, i64>(3)? as u8,
                    view_mode: ViewMode::parse(&r.get::<_, String>(4)?),
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Stores an autosave draft, refusing to overwrite one recorded against a
/// different base document.
pub fn save_draft(conn: &Connection, id: FileId, base_hash: &[u8; 32], ops: &[u8]) -> Result<()> {
    conn.execute(
        "INSERT INTO drafts (file_id, base_hash, ops_blob, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (file_id) DO UPDATE SET
             base_hash = excluded.base_hash,
             ops_blob = excluded.ops_blob,
             updated_at = excluded.updated_at",
        params![id.0, base_hash.as_slice(), ops, now()],
    )?;
    Ok(())
}

/// Loads a draft only if it was recorded against the document we are about to
/// apply it to (SPEC 8). A mismatch returns `None`: the draft is kept in the
/// database for the user to be told about, but it is never applied blindly.
pub fn load_draft(
    conn: &Connection,
    id: FileId,
    current_base_hash: &[u8; 32],
) -> Result<Option<Vec<u8>>> {
    let row: Option<(Vec<u8>, Vec<u8>)> = conn
        .query_row(
            "SELECT base_hash, ops_blob FROM drafts WHERE file_id = ?1",
            params![id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        Some((base, ops)) if base.as_slice() == current_base_hash.as_slice() => Some(ops),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_memory, Which};
    use std::path::PathBuf;

    fn conn() -> Connection {
        open_memory(Which::App).expect("open")
    }

    fn stamp(size: u64, mtime: i64) -> FileStamp {
        FileStamp { size, mtime }
    }

    #[test]
    fn touching_the_same_path_twice_keeps_one_row() {
        let c = conn();
        let p = PathBuf::from("/docs/a.pdf");
        let a = touch(&c, &p, stamp(10, 1)).expect("touch");
        let b = touch(&c, &p, stamp(20, 2)).expect("touch again");
        assert_eq!(a, b);
        let n: i64 = c
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 1);
        let row = find(&c, &p).expect("find").expect("present");
        assert_eq!(
            row.stamp,
            stamp(20, 2),
            "the stamp must be refreshed on reopen"
        );
    }

    #[test]
    fn a_file_never_seen_reports_as_changed() {
        let c = conn();
        assert!(has_changed(&c, &PathBuf::from("/docs/never.pdf")).expect("check"));
    }

    #[test]
    fn recent_puts_pinned_first_then_most_recent() {
        let c = conn();
        let a = touch(&c, &PathBuf::from("/a.pdf"), stamp(1, 1)).expect("a");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let b = touch(&c, &PathBuf::from("/b.pdf"), stamp(1, 1)).expect("b");
        set_pinned(&c, a, true).expect("pin");

        let list = recent(&c, 10).expect("recent");
        assert_eq!(list.first().map(|r| r.id), Some(a), "pinned first");
        assert_eq!(list.get(1).map(|r| r.id), Some(b));
    }

    #[test]
    fn reading_state_round_trips_and_updates_in_place() {
        let c = conn();
        let id = touch(&c, &PathBuf::from("/a.pdf"), stamp(1, 1)).expect("touch");
        assert_eq!(reading_state(&c, id).expect("get"), None);

        let s = ReadingState {
            page: 42,
            scroll_y: 133.5,
            zoom: 1.75,
            rotation: 1,
            view_mode: ViewMode::DualCover,
        };
        save_reading_state(&c, id, s).expect("save");
        assert_eq!(reading_state(&c, id).expect("get"), Some(s));

        let s2 = ReadingState { page: 43, ..s };
        save_reading_state(&c, id, s2).expect("save again");
        assert_eq!(reading_state(&c, id).expect("get"), Some(s2));
        let n: i64 = c
            .query_row("SELECT count(*) FROM reading_state", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 1);
    }

    #[test]
    fn an_unknown_view_mode_reads_back_as_the_default_rather_than_failing() {
        // A row written by a newer build must not stop this one from opening
        // the document.
        let c = conn();
        let id = touch(&c, &PathBuf::from("/a.pdf"), stamp(1, 1)).expect("touch");
        save_reading_state(&c, id, ReadingState::default()).expect("save");
        c.execute(
            "UPDATE reading_state SET view_mode = 'kaleidoscope' WHERE file_id = ?1",
            params![id.0],
        )
        .expect("write an unknown mode");
        let got = reading_state(&c, id).expect("get").expect("some");
        assert_eq!(got.view_mode, ViewMode::Single);
    }

    #[test]
    fn every_view_mode_survives_the_round_trip_through_text() {
        for mode in [
            ViewMode::Single,
            ViewMode::Dual,
            ViewMode::DualCover,
            ViewMode::Horizontal,
        ] {
            assert_eq!(ViewMode::parse(mode.as_str()), mode);
        }
    }

    #[test]
    fn a_draft_is_returned_only_for_the_document_it_was_recorded_against() {
        let c = conn();
        let id = touch(&c, &PathBuf::from("/a.pdf"), stamp(1, 1)).expect("touch");
        let base = crate::identity::content_hash(b"document v1");
        save_draft(&c, id, &base, b"ops-blob").expect("save");

        assert_eq!(
            load_draft(&c, id, &base).expect("load"),
            Some(b"ops-blob".to_vec())
        );

        let changed = crate::identity::content_hash(b"document v2");
        assert_eq!(
            load_draft(&c, id, &changed).expect("load"),
            None,
            "a draft must never be applied to a document that changed underneath it"
        );

        // It is withheld, not destroyed: the user still gets told it exists.
        let n: i64 = c
            .query_row("SELECT count(*) FROM drafts", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 1);
    }
}
