//! Extracted page text and the full-text index over it (SPEC 7, SPEC 11.1).
//!
//! `doc_text` holds the text; `doc_fts` is an external-content FTS5 index over
//! it, kept in step by triggers, so the text is stored once and the index holds
//! only postings. `doc_index_state` records what the index was built from, which
//! is what lets a stale index be rebuilt and an interrupted one be resumed.
//!
//! This module never opens a PDF. It is handed text that somebody else
//! extracted, which is what keeps it testable without a rendering engine.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::files::{now, FileId};
use crate::identity::FileStamp;

/// What the index for one document was built from, and how far it got.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndexState {
    pub stamp: FileStamp,
    pub page_count: u32,
    /// Pages indexed so far, counted from page 0. Indexing runs in page order,
    /// so this is also the page to resume at.
    pub pages_done: u32,
}

impl IndexState {
    pub fn is_complete(&self) -> bool {
        self.pages_done >= self.page_count
    }
}

/// One search result.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub file_id: FileId,
    pub path: String,
    pub page: u32,
    /// The matching text with the match marked, for the results panel.
    pub snippet: String,
}

/// Turns whatever the user typed into an FTS5 `MATCH` expression that cannot be
/// a syntax error.
///
/// FTS5 has a query language of its own — `AND`, `OR`, `NOT`, `NEAR`, `*`, `^`,
/// `-`, parentheses, double quotes. Someone searching a datasheet for `size 10"
/// x 8"` means those characters literally. Passing them through raw is a syntax
/// error at best and, worse, occasionally a *different valid query* that quietly
/// returns the wrong rows.
///
/// So every whitespace-separated token is wrapped in double quotes, making it a
/// literal string, with any quote inside it doubled — FTS5's own escape. Tokens
/// are joined with spaces, which FTS5 reads as AND, and which is what a
/// multi-word search is taken to mean.
///
/// Tokens with no alphanumeric character in them are dropped: the tokenizer
/// would reduce them to nothing, and an empty phrase *is* a syntax error.
/// `None` means nothing searchable was left, so the caller skips the query
/// rather than asking the database to match nothing.
fn match_expression(query: &str) -> Option<String> {
    let mut out = String::new();
    for token in query.split_whitespace() {
        if !token.chars().any(char::is_alphanumeric) {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push('"');
        for ch in token.chars() {
            if ch == '"' {
                out.push('"');
            }
            out.push(ch);
        }
        out.push('"');
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub fn index_state(conn: &Connection, file_id: FileId) -> Result<Option<IndexState>> {
    let row = conn
        .query_row(
            "SELECT size, mtime, page_count, pages_done
               FROM doc_index_state WHERE file_id = ?1",
            params![file_id.0],
            |r| {
                Ok(IndexState {
                    stamp: FileStamp {
                        size: r.get::<_, i64>(0)? as u64,
                        mtime: r.get(1)?,
                    },
                    page_count: r.get::<_, i64>(2)? as u32,
                    pages_done: r.get::<_, i64>(3)? as u32,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Prepares to index `file_id`, returning the page to start from.
///
/// A document whose stamp and page count still match what the existing index was
/// built from resumes where it left off. Anything else — a file edited outside
/// the application, a different file that took the same path, a first sighting —
/// throws the old text away and starts at page 0. Answering a search from text
/// a document no longer contains is worse than not answering it, so the
/// invalidation is unconditional rather than a heuristic.
pub fn begin(conn: &Connection, file_id: FileId, stamp: FileStamp, page_count: u32) -> Result<u32> {
    if let Some(state) = index_state(conn, file_id)? {
        if !state.stamp.differs_from(&stamp) && state.page_count == page_count {
            return Ok(state.pages_done.min(page_count));
        }
    }
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM doc_text WHERE file_id = ?1",
        params![file_id.0],
    )?;
    tx.execute(
        "INSERT INTO doc_index_state (file_id, size, mtime, page_count, pages_done, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?5)
         ON CONFLICT (file_id) DO UPDATE SET
             size = excluded.size,
             mtime = excluded.mtime,
             page_count = excluded.page_count,
             pages_done = 0,
             updated_at = excluded.updated_at",
        params![file_id.0, stamp.size as i64, stamp.mtime, page_count, now()],
    )?;
    tx.commit()?;
    Ok(0)
}

/// Stores a run of pages and advances the resume point.
///
/// One transaction per batch rather than per page: the FTS triggers make each
/// insert several writes, and a 500-page document committed page by page spends
/// most of its time in fsync. `pages_done` only ever moves forward, so a batch
/// that arrives out of order cannot rewind the resume point past text that is
/// already stored.
pub fn put_pages(conn: &Connection, file_id: FileId, pages: &[(u32, String)]) -> Result<()> {
    if pages.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO doc_text (file_id, page, text) VALUES (?1, ?2, ?3)
             ON CONFLICT (file_id, page) DO UPDATE SET text = excluded.text",
        )?;
        for (page, text) in pages {
            stmt.execute(params![file_id.0, page, text])?;
        }
    }
    let highest = pages.iter().map(|(p, _)| *p).max().unwrap_or(0);
    tx.execute(
        "UPDATE doc_index_state
            SET pages_done = MAX(pages_done, ?2), updated_at = ?3
          WHERE file_id = ?1",
        params![file_id.0, highest.saturating_add(1), now()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Drops a document's text and index state.
pub fn clear(conn: &Connection, file_id: FileId) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM doc_text WHERE file_id = ?1",
        params![file_id.0],
    )?;
    tx.execute(
        "DELETE FROM doc_index_state WHERE file_id = ?1",
        params![file_id.0],
    )?;
    tx.commit()?;
    Ok(())
}

/// The stored text of one page, if it has been indexed.
pub fn page_text(conn: &Connection, file_id: FileId, page: u32) -> Result<Option<String>> {
    let row = conn
        .query_row(
            "SELECT text FROM doc_text WHERE file_id = ?1 AND page = ?2",
            params![file_id.0, page],
            |r| r.get(0),
        )
        .optional()?;
    Ok(row)
}

/// Searches every indexed document (SPEC 11.1's third tier).
pub fn search_library(conn: &Connection, query: &str, limit: u32) -> Result<Vec<Hit>> {
    run(conn, query, None, limit)
}

/// Searches one document's stored text, for a document that is indexed but not
/// currently open in a worker.
pub fn search_file(
    conn: &Connection,
    file_id: FileId,
    query: &str,
    limit: u32,
) -> Result<Vec<Hit>> {
    run(conn, query, Some(file_id), limit)
}

fn run(conn: &Connection, query: &str, only: Option<FileId>, limit: u32) -> Result<Vec<Hit>> {
    let Some(expr) = match_expression(query) else {
        return Ok(Vec::new());
    };
    // bm25 returns more-negative scores for better matches, so ascending order
    // puts the best hit first.
    let sql = "SELECT t.file_id, t.page, f.path, snippet(doc_fts, 0, '[', ']', '…', 12)
                 FROM doc_fts
                 JOIN doc_text t ON t.rowid = doc_fts.rowid
                 JOIN files f ON f.id = t.file_id
                WHERE doc_fts MATCH ?1
                  AND (?2 IS NULL OR t.file_id = ?2)
                ORDER BY bm25(doc_fts), t.file_id, t.page
                LIMIT ?3";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params![expr, only.map(|f| f.0), limit], |r| {
            Ok(Hit {
                file_id: FileId(r.get(0)?),
                page: r.get::<_, i64>(1)? as u32,
                path: r.get(2)?,
                snippet: r.get(3)?,
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
    use std::path::PathBuf;

    fn conn() -> Connection {
        open_memory(Which::App).expect("open")
    }

    fn stamp(size: u64, mtime: i64) -> FileStamp {
        FileStamp { size, mtime }
    }

    fn file(c: &Connection, path: &str, s: FileStamp) -> FileId {
        touch(c, &PathBuf::from(path), s).expect("touch")
    }

    #[test]
    fn text_is_found_across_the_library() {
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(1, 1));
        let b = file(&c, "/b.pdf", stamp(1, 1));
        begin(&c, a, stamp(1, 1), 1).expect("begin a");
        begin(&c, b, stamp(1, 1), 1).expect("begin b");
        put_pages(&c, a, &[(0, "the quick brown fox".into())]).expect("put a");
        put_pages(&c, b, &[(0, "a lazy dog sleeps".into())]).expect("put b");

        let hits = search_library(&c, "fox", 10).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits.first().map(|h| h.file_id), Some(a));
        assert_eq!(hits.first().map(|h| h.path.as_str()), Some("/a.pdf"));
        assert!(
            hits.first()
                .map(|h| h.snippet.contains("[fox]"))
                .unwrap_or(false),
            "the snippet must mark the match: {hits:?}"
        );
    }

    #[test]
    fn a_search_can_be_confined_to_one_document() {
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(1, 1));
        let b = file(&c, "/b.pdf", stamp(1, 1));
        for id in [a, b] {
            begin(&c, id, stamp(1, 1), 1).expect("begin");
            put_pages(&c, id, &[(0, "shared word".into())]).expect("put");
        }
        assert_eq!(search_library(&c, "shared", 10).expect("all").len(), 2);
        let one = search_file(&c, b, "shared", 10).expect("one");
        assert_eq!(one.len(), 1);
        assert_eq!(one.first().map(|h| h.file_id), Some(b));
    }

    #[test]
    fn multiple_words_must_all_appear() {
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(1, 1));
        begin(&c, a, stamp(1, 1), 2).expect("begin");
        put_pages(
            &c,
            a,
            &[(0, "alpha beta".into()), (1, "alpha gamma".into())],
        )
        .expect("put");

        let hits = search_library(&c, "alpha gamma", 10).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits.first().map(|h| h.page), Some(1));
    }

    #[test]
    fn punctuation_a_reader_would_type_is_never_a_syntax_error() {
        // Every one of these is either FTS5 syntax or an unbalanced fragment of
        // it. None may reach the query planner as an operator, and none may
        // make the search fail — a search box that errors on a quote is broken.
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(1, 1));
        begin(&c, a, stamp(1, 1), 1).expect("begin");
        put_pages(&c, a, &[(0, r#"width 10" and height 8" nominal"#.into())]).expect("put");

        for query in [
            r#"10""#,
            r#""unbalanced"#,
            "width AND height",
            "width OR height",
            "NEAR(width height)",
            "width*",
            "-width",
            "^width",
            "(width",
            "width)",
            "\"\"",
            "***",
            "",
            "   ",
        ] {
            let result = search_library(&c, query, 10);
            assert!(result.is_ok(), "query {query:?} must not fail: {result:?}");
        }
    }

    #[test]
    fn a_quoted_token_is_escaped_rather_than_reinterpreted() {
        assert_eq!(match_expression("fox"), Some("\"fox\"".into()));
        assert_eq!(match_expression("a b"), Some("\"a\" \"b\"".into()));
        // One embedded quote becomes two, which is how FTS5 escapes it.
        assert_eq!(match_expression(r#"10""#), Some(r#""10""""#.into()));
        // Tokens the tokenizer would reduce to nothing are dropped, because an
        // empty phrase is itself a syntax error.
        assert_eq!(match_expression("-"), None);
        assert_eq!(match_expression("  "), None);
        assert_eq!(match_expression("- fox -"), Some("\"fox\"".into()));
    }

    #[test]
    fn an_operator_word_is_searched_for_literally() {
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(1, 1));
        begin(&c, a, stamp(1, 1), 1).expect("begin");
        put_pages(&c, a, &[(0, "terms and conditions".into())]).expect("put");
        // "and" is a word in the document, not a boolean operator joining two
        // empty sides.
        let hits = search_library(&c, "and", 10).expect("search");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn indexing_resumes_where_it_stopped() {
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(500, 7));
        assert_eq!(begin(&c, a, stamp(500, 7), 10).expect("begin"), 0);
        put_pages(&c, a, &[(0, "one".into()), (1, "two".into())]).expect("put");

        // A later run of the same unchanged file picks up at page 2.
        assert_eq!(begin(&c, a, stamp(500, 7), 10).expect("resume"), 2);
        assert_eq!(
            page_text(&c, a, 0).expect("text"),
            Some("one".into()),
            "resuming must not throw away what was already indexed"
        );
    }

    #[test]
    fn a_file_changed_on_disk_is_indexed_again_from_scratch() {
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(500, 7));
        begin(&c, a, stamp(500, 7), 10).expect("begin");
        put_pages(&c, a, &[(0, "original text".into())]).expect("put");
        assert_eq!(search_library(&c, "original", 10).expect("search").len(), 1);

        // Same path, different bytes.
        assert_eq!(begin(&c, a, stamp(900, 9), 10).expect("rebegin"), 0);
        assert_eq!(page_text(&c, a, 0).expect("text"), None);
        assert!(
            search_library(&c, "original", 10)
                .expect("search")
                .is_empty(),
            "a stale index must never keep answering searches"
        );
    }

    #[test]
    fn a_page_count_that_changed_also_invalidates() {
        // Same size and mtime but a different page count means the stamp lied —
        // some filesystems have one-second mtime resolution, so an edit inside
        // the same second is invisible to it.
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(500, 7));
        begin(&c, a, stamp(500, 7), 10).expect("begin");
        put_pages(&c, a, &[(0, "text".into())]).expect("put");
        assert_eq!(begin(&c, a, stamp(500, 7), 11).expect("rebegin"), 0);
        assert_eq!(page_text(&c, a, 0).expect("text"), None);
    }

    #[test]
    fn state_reports_completion_only_when_every_page_is_in() {
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(1, 1));
        begin(&c, a, stamp(1, 1), 3).expect("begin");
        put_pages(&c, a, &[(0, "a".into()), (1, "b".into())]).expect("put");
        let state = index_state(&c, a).expect("state").expect("some");
        assert_eq!(state.pages_done, 2);
        assert!(!state.is_complete());

        put_pages(&c, a, &[(2, "c".into())]).expect("put last");
        let state = index_state(&c, a).expect("state").expect("some");
        assert!(state.is_complete());
    }

    #[test]
    fn re_indexing_a_page_replaces_its_text_in_the_index_too() {
        // The external-content index is only correct if the update trigger fires;
        // a stale posting would keep matching text that is no longer there.
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(1, 1));
        begin(&c, a, stamp(1, 1), 1).expect("begin");
        put_pages(&c, a, &[(0, "before".into())]).expect("put");
        put_pages(&c, a, &[(0, "after".into())]).expect("overwrite");

        assert!(search_library(&c, "before", 10).expect("search").is_empty());
        assert_eq!(search_library(&c, "after", 10).expect("search").len(), 1);
    }

    #[test]
    fn clearing_a_document_removes_it_from_results() {
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(1, 1));
        begin(&c, a, stamp(1, 1), 1).expect("begin");
        put_pages(&c, a, &[(0, "findme".into())]).expect("put");
        clear(&c, a).expect("clear");
        assert!(search_library(&c, "findme", 10).expect("search").is_empty());
        assert_eq!(index_state(&c, a).expect("state"), None);
    }

    #[test]
    fn deleting_the_file_row_takes_its_text_and_index_state_with_it() {
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(1, 1));
        begin(&c, a, stamp(1, 1), 1).expect("begin");
        put_pages(&c, a, &[(0, "findme".into())]).expect("put");
        c.execute("DELETE FROM files WHERE id = ?1", params![a.0])
            .expect("delete");

        assert_eq!(page_text(&c, a, 0).expect("text"), None);
        assert_eq!(index_state(&c, a).expect("state"), None);
        assert!(search_library(&c, "findme", 10).expect("search").is_empty());
    }

    #[test]
    fn diacritics_are_folded_so_indonesian_typing_finds_the_word() {
        // The index is declared with `remove_diacritics 2`; this proves the
        // declaration is actually in force.
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(1, 1));
        begin(&c, a, stamp(1, 1), 1).expect("begin");
        put_pages(&c, a, &[(0, "kolésom dan café".into())]).expect("put");
        assert_eq!(search_library(&c, "kolesom", 10).expect("search").len(), 1);
        assert_eq!(search_library(&c, "cafe", 10).expect("search").len(), 1);
    }

    #[test]
    fn the_limit_is_honoured() {
        let c = conn();
        let a = file(&c, "/a.pdf", stamp(1, 1));
        begin(&c, a, stamp(1, 1), 5).expect("begin");
        let pages: Vec<(u32, String)> = (0..5).map(|p| (p, "repeated".to_string())).collect();
        put_pages(&c, a, &pages).expect("put");
        assert_eq!(search_library(&c, "repeated", 2).expect("search").len(), 2);
    }
}
