//! The export history (SPEC 11.3: "riwayat ekspor yang bisa diklik").

use rusqlite::{params, Connection};

use crate::error::Result;
use crate::files::{now, FileId};

/// One export, newest first when listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRow {
    pub id: i64,
    pub source: String,
    pub out_path: String,
    /// `flat`, `pages`, `png`, `jpg`, `copy`.
    pub kind: String,
    pub created_at: i64,
}

pub fn record(conn: &Connection, file: FileId, out_path: &str, kind: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO export_history (file_id, out_path, kind, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![file.0, out_path, kind, now()],
    )?;
    Ok(())
}

pub fn recent(conn: &Connection, limit: u32) -> Result<Vec<ExportRow>> {
    let mut stmt = conn.prepare(
        "SELECT e.id, f.path, e.out_path, e.kind, e.created_at
         FROM export_history e JOIN files f ON f.id = e.file_id
         ORDER BY e.created_at DESC, e.id DESC
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(ExportRow {
                id: r.get(0)?,
                source: r.get(1)?,
                out_path: r.get(2)?,
                kind: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_memory, Which};
    use crate::files::touch;
    use crate::identity::FileStamp;
    use std::path::PathBuf;

    #[test]
    fn exports_are_listed_newest_first_with_their_source() {
        let c = open_memory(Which::App).unwrap();
        let f = touch(
            &c,
            &PathBuf::from("C:/a.pdf"),
            FileStamp { size: 1, mtime: 1 },
        )
        .unwrap();
        record(&c, f, "C:/a-rata.pdf", "flat").unwrap();
        record(&c, f, "C:/a-1.png", "png").unwrap();
        let rows = recent(&c, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].out_path, "C:/a-1.png");
        assert_eq!(rows[0].source, "C:/a.pdf");
        assert_eq!(rows[1].kind, "flat");
        assert_eq!(recent(&c, 1).unwrap().len(), 1);
    }
}
