//! Autosaved drafts (SPEC 8: "autosave draf ke SQLite tiap 20 detik,
//! dipulihkan setelah crash atau tutup paksa").
//!
//! One row per file: the unsaved annotations of that file as an opaque blob,
//! and the identity of the file they were made against. `base` is what keeps a
//! draft from ever being applied to a file that has changed underneath it —
//! the caller compares it with the file as it is now and offers the draft only
//! when they match.
//!
//! What the blob contains is the UI process's business (it is the annotation
//! state, JSON); this module only keeps it.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::files::{now, FileId};

/// A stored draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub base: Vec<u8>,
    pub blob: Vec<u8>,
    /// Seconds since the Unix epoch.
    pub updated_at: i64,
}

/// Writes (or replaces) the draft for a file.
pub fn put(conn: &Connection, file: FileId, base: &[u8], blob: &[u8]) -> Result<()> {
    conn.execute(
        "INSERT INTO drafts (file_id, base_hash, ops_blob, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (file_id) DO UPDATE SET
           base_hash = excluded.base_hash,
           ops_blob = excluded.ops_blob,
           updated_at = excluded.updated_at",
        params![file.0, base, blob, now()],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, file: FileId) -> Result<Option<Draft>> {
    Ok(conn
        .query_row(
            "SELECT base_hash, ops_blob, updated_at FROM drafts WHERE file_id = ?1",
            params![file.0],
            |r| {
                Ok(Draft {
                    base: r.get(0)?,
                    blob: r.get(1)?,
                    updated_at: r.get(2)?,
                })
            },
        )
        .optional()?)
}

/// Forgets a draft — after a save, or when the user discards the changes.
pub fn delete(conn: &Connection, file: FileId) -> Result<()> {
    conn.execute("DELETE FROM drafts WHERE file_id = ?1", params![file.0])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_memory, Which};
    use crate::files::touch;
    use crate::identity::FileStamp;
    use std::path::PathBuf;

    #[test]
    fn a_draft_is_written_replaced_read_and_forgotten() {
        let c = open_memory(Which::App).unwrap();
        let f = touch(
            &c,
            &PathBuf::from("C:/a.pdf"),
            FileStamp { size: 1, mtime: 1 },
        )
        .unwrap();
        assert_eq!(get(&c, f).unwrap(), None);
        put(&c, f, b"1:1", b"satu").unwrap();
        put(&c, f, b"1:1", b"dua").unwrap();
        let d = get(&c, f).unwrap().unwrap();
        assert_eq!(
            (d.base.as_slice(), d.blob.as_slice()),
            (b"1:1".as_slice(), b"dua".as_slice())
        );
        assert!(d.updated_at > 1_600_000_000);
        delete(&c, f).unwrap();
        assert_eq!(get(&c, f).unwrap(), None);
    }

    #[test]
    fn drafts_of_two_files_are_independent() {
        let c = open_memory(Which::App).unwrap();
        let a = touch(
            &c,
            &PathBuf::from("C:/a.pdf"),
            FileStamp { size: 1, mtime: 1 },
        )
        .unwrap();
        let b = touch(
            &c,
            &PathBuf::from("C:/b.pdf"),
            FileStamp { size: 1, mtime: 1 },
        )
        .unwrap();
        put(&c, a, b"x", b"a").unwrap();
        put(&c, b, b"x", b"b").unwrap();
        delete(&c, a).unwrap();
        assert_eq!(get(&c, b).unwrap().unwrap().blob, b"b");
    }
}
