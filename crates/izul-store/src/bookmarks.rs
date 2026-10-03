//! The `bookmarks` table: bookmarks the user makes, kept beside the file
//! rather than in it (SPEC 11.1: "Bookmark buatan pengguna, terpisah dari
//! outline bawaan PDF"). The table has existed since Phase 0; nothing wrote to
//! it until the Phase 8 audit found the feature missing.
//!
//! Keyed by file identity, so a renamed or moved file keeps its bookmarks,
//! and a file's bookmarks go with it when its row is deleted.

use rusqlite::{params, Connection};

use crate::error::Result;
use crate::files::{now, FileId};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Bookmark {
    pub id: i64,
    pub page: u32,
    pub label: String,
    pub created_at: i64,
}

/// A file's bookmarks, in page order (then in the order they were made).
pub fn list(conn: &Connection, file: FileId) -> Result<Vec<Bookmark>> {
    let mut stmt = conn.prepare(
        "SELECT id, page, label, created_at FROM bookmarks WHERE file_id = ?1
         ORDER BY page, created_at, id",
    )?;
    let rows = stmt.query_map(params![file.0], |r| {
        Ok(Bookmark {
            id: r.get(0)?,
            page: r.get(1)?,
            label: r.get(2)?,
            created_at: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn add(conn: &Connection, file: FileId, page: u32, label: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO bookmarks (file_id, page, label, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![file.0, page, label, now()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Renames bookmark `id` of `file`. A bookmark of another file is not touched,
/// so a stale id from a closed tab cannot rename someone else's.
pub fn rename(conn: &Connection, file: FileId, id: i64, label: &str) -> Result<bool> {
    let n = conn.execute(
        "UPDATE bookmarks SET label = ?3 WHERE id = ?1 AND file_id = ?2",
        params![id, file.0, label],
    )?;
    Ok(n > 0)
}

pub fn remove(conn: &Connection, file: FileId, id: i64) -> Result<bool> {
    let n = conn.execute(
        "DELETE FROM bookmarks WHERE id = ?1 AND file_id = ?2",
        params![id, file.0],
    )?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_memory, Which};

    fn file(conn: &Connection, path: &str) -> FileId {
        conn.execute(
            "INSERT INTO files (path, path_hash, size, mtime) VALUES (?1, ?2, 1, 1)",
            params![path, path.as_bytes()],
        )
        .expect("file");
        FileId(conn.last_insert_rowid())
    }

    #[test]
    fn bookmarks_are_kept_per_file_in_page_order() {
        let conn = open_memory(Which::App).expect("db");
        let a = file(&conn, "a.pdf");
        let b = file(&conn, "bb.pdf");
        add(&conn, a, 9, "Bab 3").expect("add");
        add(&conn, a, 2, "Pendahuluan").expect("add");
        add(&conn, b, 0, "lain").expect("add");
        let got: Vec<(u32, String)> = list(&conn, a)
            .expect("list")
            .into_iter()
            .map(|m| (m.page, m.label))
            .collect();
        assert_eq!(got, vec![(2, "Pendahuluan".into()), (9, "Bab 3".into())]);
    }

    #[test]
    fn another_files_bookmark_cannot_be_renamed_or_removed() {
        let conn = open_memory(Which::App).expect("db");
        let a = file(&conn, "a.pdf");
        let b = file(&conn, "bb.pdf");
        let id = add(&conn, a, 1, "milik a").expect("add");
        assert!(!rename(&conn, b, id, "diambil").expect("rename"));
        assert!(!remove(&conn, b, id).expect("remove"));
        assert!(rename(&conn, a, id, "baru").expect("rename"));
        assert_eq!(list(&conn, a).expect("list")[0].label, "baru");
        assert!(remove(&conn, a, id).expect("remove"));
        assert!(list(&conn, a).expect("list").is_empty());
    }
}
