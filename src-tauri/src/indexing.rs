//! Background text extraction into the FTS5 index (SPEC 7, SPEC 11.1).
//!
//! Search is answered by two different things, and this is what feeds the first
//! of them: the index says *which pages* contain the words, and a worker then
//! says *where on the page* they sit. Extracting the text is the expensive half
//! and it only has to happen once per file version, so it happens here, in the
//! background, at a pace that leaves the viewport alone.
//!
//! Three properties are load-bearing:
//!
//! * **Page at a time.** The worker is the same process that renders tiles and
//!   answers heartbeats. A request that went away to extract five hundred pages
//!   would be killed at six seconds by the supervisor (SPEC 3.4) — the same
//!   reason [`izul_ipc::message::Request::Search`] is page-scoped.
//! * **Resumable.** `doc_index_state` records how far the last run got, so a
//!   document closed halfway through does not start over, and one whose bytes
//!   changed is thrown away rather than half-trusted.
//! * **Cancellable.** Closing a tab stops its indexing. Without that, closing
//!   fifty documents would leave fifty tasks competing for the pool they no
//!   longer have any reason to touch.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use izul_ipc::message::{DocId, Request, Response};
use izul_model::geom::RotationQuarter;
use izul_store::files::FileId;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::supervisor::Pool;

/// Pages held back before a commit.
///
/// One transaction per page would spend most of its time in fsync — the FTS
/// triggers turn each insert into several writes — and one transaction for the
/// whole document would lose everything if the run were interrupted. Sixteen is
/// small enough that a cancelled run loses at most a few pages of work.
const BATCH_PAGES: usize = 16;

/// Pause between pages.
///
/// Indexing is never what the user is waiting for; a tile always is. This is a
/// deliberately blunt way of keeping the worker free most of the time, and it
/// is honest about being blunt: a 500-page document takes about eight seconds
/// of wall clock to index, which is fine for something nobody is watching.
const PACE: Duration = Duration::from_millis(15);

/// What the search panel needs to know about one document's index.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexProgress {
    pub pages_done: u32,
    pub page_count: u32,
    pub running: bool,
    /// Set when the run stopped early. The panel shows results from what was
    /// indexed and says so, rather than pretending the document was searched
    /// in full.
    pub error: Option<String>,
}

impl IndexProgress {
    pub fn complete(&self) -> bool {
        !self.running && self.error.is_none() && self.pages_done >= self.page_count
    }
}

struct Entry {
    progress: IndexProgress,
    cancel: Arc<AtomicBool>,
}

/// Every indexing run in flight, keyed by the document that started it.
#[derive(Default)]
pub struct Indexer {
    runs: Mutex<HashMap<u64, Entry>>,
}

impl std::fmt::Debug for Indexer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Indexer")
            .field("runs", &self.runs.lock().len())
            .finish()
    }
}

/// Everything one run needs, gathered so the task can own it.
#[derive(Debug)]
pub struct Job {
    pub doc: u64,
    pub file_id: i64,
    pub page_count: u32,
    pub data_dir: PathBuf,
    pub pool: Arc<RwLock<Pool>>,
}

impl Indexer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn progress(&self, doc: u64) -> Option<IndexProgress> {
        self.runs.lock().get(&doc).map(|e| e.progress.clone())
    }

    /// Stops the run for `doc`, if any. Idempotent: closing a tab twice, or
    /// closing one that was never indexed, is not an error.
    pub fn cancel(&self, doc: u64) {
        if let Some(entry) = self.runs.lock().get(&doc) {
            entry.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn forget(&self, doc: u64) {
        self.cancel(doc);
        self.runs.lock().remove(&doc);
    }

    /// Starts indexing `job`, unless a run for that document is already going.
    ///
    /// Returns whether a new run was started, which is what makes calling this
    /// on every tab activation harmless.
    pub fn start(self: &Arc<Self>, job: Job) -> bool {
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut runs = self.runs.lock();
            if runs.get(&job.doc).is_some_and(|e| e.progress.running) {
                return false;
            }
            runs.insert(
                job.doc,
                Entry {
                    progress: IndexProgress {
                        pages_done: 0,
                        page_count: job.page_count,
                        running: true,
                        error: None,
                    },
                    cancel: Arc::clone(&cancel),
                },
            );
        }
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let doc = job.doc;
            let result = this.run(&job, &cancel).await;
            let mut runs = this.runs.lock();
            if let Some(entry) = runs.get_mut(&doc) {
                entry.progress.running = false;
                if let Err(e) = result {
                    tracing::warn!(doc, error = %e, "pengindeksan berhenti");
                    entry.progress.error = Some(e);
                }
            }
        });
        true
    }

    async fn run(&self, job: &Job, cancel: &AtomicBool) -> Result<(), String> {
        let conn = izul_store::open(&job.data_dir, izul_store::Which::App)
            .map_err(|e| format!("basis data: {e}"))?;
        let file_id = FileId(job.file_id);
        // The stamp `files` recorded when the document was opened, not a fresh
        // `stat`: re-reading it here could pick up an edit made after the pages
        // were read, and the index would then look current while describing
        // bytes that are gone.
        let stamp = stamp_of(&conn, file_id)?;
        let start = izul_store::search::begin(&conn, file_id, stamp, job.page_count)
            .map_err(|e| format!("indeks: {e}"))?;
        if start >= job.page_count {
            self.set_done(job.doc, job.page_count);
            return Ok(());
        }
        tracing::info!(
            doc = job.doc,
            dari = start,
            sampai = job.page_count,
            "mulai mengindeks teks"
        );

        let mut batch: Vec<(u32, String)> = Vec::with_capacity(BATCH_PAGES);
        for page in start..job.page_count {
            if cancel.load(Ordering::Relaxed) {
                flush(&conn, file_id, &mut batch)?;
                self.set_done(job.doc, page);
                return Ok(());
            }
            let text = self.page_text(job, page).await?;
            batch.push((page, text));
            if batch.len() >= BATCH_PAGES {
                flush(&conn, file_id, &mut batch)?;
                self.set_done(job.doc, page + 1);
            }
            tokio::time::sleep(PACE).await;
        }
        flush(&conn, file_id, &mut batch)?;
        self.set_done(job.doc, job.page_count);
        tracing::info!(
            doc = job.doc,
            halaman = job.page_count,
            "indeks teks selesai"
        );
        Ok(())
    }

    async fn page_text(&self, job: &Job, page: u32) -> Result<String, String> {
        let worker = { job.pool.read().await.worker_for(DocId(job.doc)) };
        let worker = worker.ok_or_else(|| "dokumen tidak lagi terbuka".to_string())?;
        let req = Request::ExtractText {
            doc: DocId(job.doc),
            page,
            // No boxes: the index stores words, not geometry. Asking for the
            // per-character rectangles would multiply the wire cost of every
            // page by an order of magnitude for something nothing reads.
            with_boxes: false,
            rotation: RotationQuarter::from_degrees(0),
        };
        match Pool::ask(&worker, req).await {
            Ok(Response::TextReady { text, .. }) => Ok(text),
            // A page PDFium refuses is indexed as empty rather than failing the
            // run: one broken page in a thousand must not cost the other 999.
            Ok(Response::Error { detail, .. }) => {
                tracing::debug!(doc = job.doc, page, detail, "halaman tidak dapat dibaca");
                Ok(String::new())
            }
            Ok(_) => Ok(String::new()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set_done(&self, doc: u64, pages: u32) {
        if let Some(entry) = self.runs.lock().get_mut(&doc) {
            entry.progress.pages_done = pages.max(entry.progress.pages_done);
        }
    }
}

fn flush(
    conn: &rusqlite::Connection,
    file_id: FileId,
    batch: &mut Vec<(u32, String)>,
) -> Result<(), String> {
    if batch.is_empty() {
        return Ok(());
    }
    izul_store::search::put_pages(conn, file_id, batch).map_err(|e| format!("indeks: {e}"))?;
    batch.clear();
    Ok(())
}

/// The stamp `files` recorded for this document when it was opened.
fn stamp_of(conn: &rusqlite::Connection, file_id: FileId) -> Result<izul_store::FileStamp, String> {
    conn.query_row(
        "SELECT size, mtime FROM files WHERE id = ?1",
        rusqlite::params![file_id.0],
        |r| {
            Ok(izul_store::FileStamp {
                size: r.get::<_, i64>(0)? as u64,
                mtime: r.get(1)?,
            })
        },
    )
    .map_err(|e| format!("berkas tidak dikenal: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_only_complete_when_every_page_is_in() {
        let mut p = IndexProgress {
            pages_done: 10,
            page_count: 20,
            running: true,
            error: None,
        };
        assert!(!p.complete(), "masih berjalan");
        p.running = false;
        assert!(!p.complete(), "baru separuh");
        p.pages_done = 20;
        assert!(p.complete());
        p.error = Some("berhenti".into());
        assert!(!p.complete(), "yang berhenti dengan galat bukan lengkap");
    }

    /// Two tabs of the same document, or one tab activated twice, must not put
    /// two runs on the same rows: the second would race the first for
    /// `pages_done` and could walk it backwards.
    #[test]
    fn a_run_already_going_is_reported_and_can_be_stopped() {
        let indexer = Arc::new(Indexer::new());
        {
            let mut runs = indexer.runs.lock();
            runs.insert(
                7,
                Entry {
                    progress: IndexProgress {
                        pages_done: 0,
                        page_count: 5,
                        running: true,
                        error: None,
                    },
                    cancel: Arc::new(AtomicBool::new(false)),
                },
            );
        }
        assert_eq!(indexer.progress(7).map(|p| p.running), Some(true));
        indexer.cancel(7);
        assert!(
            indexer.runs.lock()[&7].cancel.load(Ordering::Relaxed),
            "pembatalan harus sampai ke tugasnya"
        );
        indexer.forget(7);
        assert!(indexer.progress(7).is_none());
    }

    /// `pages_done` is written from the task and read by the panel; a batch
    /// that lands out of order must never rewind it past text already stored.
    #[test]
    fn progress_only_moves_forward() {
        let indexer = Indexer::new();
        indexer.runs.lock().insert(
            1,
            Entry {
                progress: IndexProgress {
                    pages_done: 0,
                    page_count: 100,
                    running: true,
                    error: None,
                },
                cancel: Arc::new(AtomicBool::new(false)),
            },
        );
        indexer.set_done(1, 32);
        indexer.set_done(1, 16);
        assert_eq!(indexer.progress(1).map(|p| p.pages_done), Some(32));
    }
}
