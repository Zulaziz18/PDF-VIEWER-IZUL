//! The `sessions` and `session_tabs` tables: which documents were open, in
//! which panel, and in what order (SPEC 7, SPEC 10).
//!
//! One session row per application run. A run's tabs are rewritten in place as
//! the user opens, closes and reorders them rather than appended, because a
//! reorder is not history anyone will ever ask to see — only the last
//! arrangement of a run is ever restored. Older runs are pruned, so the table
//! cannot grow without bound on a machine that is never shut down.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::files::{now, FileId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionId(pub i64);

/// Where one tab sat in the window.
///
/// This is the write shape: everything here is something the UI decides. The
/// path is deliberately absent — it lives in `files` and is looked up on
/// restore, so a document that was renamed between runs comes back under the
/// name it has now rather than the one it had then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabSlot {
    pub file_id: FileId,
    /// Which of the up-to-four split panels held it (SPEC 10).
    pub panel: u8,
    pub tab_order: u32,
    pub pinned: bool,
    pub is_active: bool,
}

/// A restored tab: its slot plus the path to reopen.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionTab {
    pub slot: TabSlot,
    pub path: String,
}

/// Opens a new session for this run.
///
/// The row is created empty and stays empty until the first document is opened.
/// [`latest`] ignores empty sessions for exactly this reason: starting the
/// application must never be able to overwrite the arrangement it is about to
/// restore.
pub fn begin(conn: &Connection) -> Result<SessionId> {
    conn.execute(
        "INSERT INTO sessions (created_at) VALUES (?1)",
        params![now()],
    )?;
    Ok(SessionId(conn.last_insert_rowid()))
}

/// Replaces this session's tabs with `tabs`.
///
/// Delete-then-insert inside one transaction, so a crash mid-write leaves the
/// previous arrangement intact rather than half of the new one. Tabs are
/// addressed by `(file_id, panel)`, which is the table's own primary key, so
/// the same document open in two panels is two rows and not a conflict.
pub fn save_tabs(conn: &Connection, session: SessionId, tabs: &[TabSlot]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM session_tabs WHERE session_id = ?1",
        params![session.0],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO session_tabs
                 (session_id, file_id, panel, tab_order, pinned, is_active)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for tab in tabs {
            stmt.execute(params![
                session.0,
                tab.file_id.0,
                tab.panel,
                tab.tab_order,
                tab.pinned as i64,
                tab.is_active as i64,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// The tabs of one session, ordered as they sat on screen.
pub fn tabs(conn: &Connection, session: SessionId) -> Result<Vec<SessionTab>> {
    let mut stmt = conn.prepare(
        "SELECT t.file_id, t.panel, t.tab_order, t.pinned, t.is_active, f.path
           FROM session_tabs t
           JOIN files f ON f.id = t.file_id
          WHERE t.session_id = ?1
          ORDER BY t.panel, t.tab_order",
    )?;
    let rows = stmt
        .query_map(params![session.0], |r| {
            Ok(SessionTab {
                slot: TabSlot {
                    file_id: FileId(r.get(0)?),
                    panel: r.get::<_, i64>(1)? as u8,
                    tab_order: r.get::<_, i64>(2)? as u32,
                    pinned: r.get::<_, i64>(3)? != 0,
                    is_active: r.get::<_, i64>(4)? != 0,
                },
                path: r.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The most recent session that actually held documents, for restore at start.
///
/// Empty sessions are skipped rather than returned as an empty arrangement. The
/// current run has just created one, and a run that ended without opening
/// anything should not be what the user comes back to.
pub fn latest(conn: &Connection) -> Result<Option<(SessionId, Vec<SessionTab>)>> {
    let id: Option<i64> = conn
        .query_row(
            "SELECT s.id
               FROM sessions s
              WHERE EXISTS (SELECT 1 FROM session_tabs t WHERE t.session_id = s.id)
              ORDER BY s.created_at DESC, s.id DESC
              LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Ok(None);
    };
    let session = SessionId(id);
    Ok(Some((session, tabs(conn, session)?)))
}

/// Drops all but the `keep` newest sessions.
///
/// `session_tabs` rows go with them through `ON DELETE CASCADE`. Returns how
/// many sessions were removed.
pub fn prune(conn: &Connection, keep: u32) -> Result<usize> {
    let removed = conn.execute(
        "DELETE FROM sessions
          WHERE id NOT IN (
              SELECT id FROM sessions ORDER BY created_at DESC, id DESC LIMIT ?1
          )",
        params![keep],
    )?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_memory, Which};
    use crate::files::touch;
    use crate::identity::FileStamp;
    use std::path::PathBuf;

    fn conn() -> Connection {
        open_memory(Which::App).expect("open")
    }

    fn file(c: &Connection, path: &str) -> FileId {
        touch(
            c,
            &PathBuf::from(path),
            FileStamp {
                size: 1,
                mtime: 1,
            },
        )
        .expect("touch")
    }

    fn slot(file_id: FileId, order: u32) -> TabSlot {
        TabSlot {
            file_id,
            panel: 0,
            tab_order: order,
            pinned: false,
            is_active: false,
        }
    }

    #[test]
    fn tabs_round_trip_in_panel_then_order() {
        let c = conn();
        let s = begin(&c).expect("begin");
        let a = file(&c, "/a.pdf");
        let b = file(&c, "/b.pdf");

        save_tabs(
            &c,
            s,
            &[
                TabSlot {
                    panel: 1,
                    ..slot(a, 0)
                },
                TabSlot {
                    tab_order: 5,
                    pinned: true,
                    is_active: true,
                    ..slot(b, 5)
                },
            ],
        )
        .expect("save");

        let got = tabs(&c, s).expect("tabs");
        assert_eq!(got.len(), 2);
        // Panel 0 sorts before panel 1 regardless of insertion order.
        assert_eq!(got.first().map(|t| t.slot.file_id), Some(b));
        assert_eq!(got.first().map(|t| t.path.as_str()), Some("/b.pdf"));
        assert!(got.first().map(|t| t.slot.pinned).unwrap_or(false));
        assert!(got.first().map(|t| t.slot.is_active).unwrap_or(false));
        assert_eq!(got.get(1).map(|t| t.slot.panel), Some(1));
    }

    #[test]
    fn saving_again_replaces_the_arrangement_rather_than_adding_to_it() {
        let c = conn();
        let s = begin(&c).expect("begin");
        let a = file(&c, "/a.pdf");
        let b = file(&c, "/b.pdf");

        save_tabs(&c, s, &[slot(a, 0), slot(b, 1)]).expect("save");
        save_tabs(&c, s, &[slot(b, 0)]).expect("save again");

        let got = tabs(&c, s).expect("tabs");
        assert_eq!(got.len(), 1, "the closed tab must be gone, not duplicated");
        assert_eq!(got.first().map(|t| t.slot.file_id), Some(b));
    }

    #[test]
    fn the_same_document_can_sit_in_two_panels() {
        // SPEC 10 allows one document open in more than one split panel, and the
        // table's primary key is (session, file, panel) for that reason.
        let c = conn();
        let s = begin(&c).expect("begin");
        let a = file(&c, "/a.pdf");
        save_tabs(
            &c,
            s,
            &[
                slot(a, 0),
                TabSlot {
                    panel: 2,
                    ..slot(a, 0)
                },
            ],
        )
        .expect("save");
        assert_eq!(tabs(&c, s).expect("tabs").len(), 2);
    }

    #[test]
    fn latest_skips_the_empty_session_this_run_just_opened() {
        // This is the whole reason `latest` filters on having tabs: startup
        // creates a session before it restores one, and without the filter the
        // new empty row would be "most recent" and wipe the arrangement.
        let c = conn();
        let previous = begin(&c).expect("begin previous");
        let a = file(&c, "/a.pdf");
        save_tabs(&c, previous, &[slot(a, 0)]).expect("save");

        let _current = begin(&c).expect("begin current");

        let (found, restored) = latest(&c).expect("latest").expect("some");
        assert_eq!(found, previous);
        assert_eq!(restored.len(), 1);
    }

    #[test]
    fn latest_is_none_when_nothing_was_ever_opened() {
        let c = conn();
        let _ = begin(&c).expect("begin");
        assert_eq!(latest(&c).expect("latest"), None);
    }

    #[test]
    fn prune_keeps_the_newest_and_takes_their_tabs_with_the_rest() {
        let c = conn();
        let a = file(&c, "/a.pdf");
        let mut ids = Vec::new();
        for _ in 0..4 {
            let s = begin(&c).expect("begin");
            save_tabs(&c, s, &[slot(a, 0)]).expect("save");
            ids.push(s);
        }

        let removed = prune(&c, 2).expect("prune");
        assert_eq!(removed, 2);

        let left: i64 = c
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .expect("count");
        assert_eq!(left, 2);
        let orphans: i64 = c
            .query_row("SELECT count(*) FROM session_tabs", [], |r| r.get(0))
            .expect("count");
        assert_eq!(orphans, 2, "cascade must clear the deleted sessions' tabs");
    }

    #[test]
    fn a_tab_disappears_when_its_file_row_does() {
        let c = conn();
        let s = begin(&c).expect("begin");
        let a = file(&c, "/a.pdf");
        save_tabs(&c, s, &[slot(a, 0)]).expect("save");

        c.execute("DELETE FROM files WHERE id = ?1", params![a.0])
            .expect("delete file");

        assert!(
            tabs(&c, s).expect("tabs").is_empty(),
            "a session must never point at a file row that is gone"
        );
    }
}
