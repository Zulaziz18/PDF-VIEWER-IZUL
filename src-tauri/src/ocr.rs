//! "Kenali Teks (OCR)" (Phase 7): scanned pages made searchable.
//!
//! Recognition runs in a worker, one page per request, as part of a save
//! (`saving::OcrJob`): the text layer is page content, so it is written the
//! way redaction is — into the working copy, checked, then renamed over the
//! target. While it runs, `ocr_progress` answers how far it is and
//! `ocr_cancel` stops it between pages; a cancelled run writes nothing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;

use crate::commands::AppState;
use crate::saving::{OcrJob, Rewrite, SaveReport};

type CmdResult<T> = Result<T, String>;

/// The folder the models ship in: `ocrs` beside the executable, where
/// `src-tauri/build.rs` copies them for a dev build and the installer puts
/// them for a release.
pub fn models_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("ocrs");
    let present = ["text-detection.rten", "text-recognition.rten"]
        .iter()
        .all(|n| dir.join(n).is_file());
    present.then_some(dir)
}

/// One run in flight: how far it is, and the flag that stops it.
#[derive(Debug, Default)]
struct Run {
    done: AtomicU32,
    total: AtomicU32,
    cancel: Arc<AtomicBool>,
}

/// Runs in flight, per document.
#[derive(Debug, Default)]
pub struct OcrRuns {
    runs: Mutex<HashMap<u64, Arc<Run>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OcrProgress {
    pub running: bool,
    pub done: u32,
    pub total: u32,
}

/// Whether OCR can run here at all (the models are installed).
#[tauri::command]
pub fn ocr_available() -> bool {
    models_dir().is_some()
}

/// Reads `pages` (all pages when `None`) and saves the result to `target`, or
/// over the tab's own file when `None`. Pages that already have text are left
/// alone unless `force`.
#[tauri::command]
pub async fn ocr_apply(
    state: tauri::State<'_, AppState>,
    doc: u64,
    target: Option<String>,
    pages: Option<Vec<u32>>,
    force: bool,
) -> CmdResult<SaveReport> {
    let models = models_dir().ok_or_else(|| {
        "Model OCR belum terpasang. Jalankan vendor/ocrs/fetch.sh lalu bangun ulang aplikasi."
            .to_string()
    })?;
    let page_count = {
        let mapped = state.annots.page_map(doc).map(|m| m.len() as u32);
        let ws = state.workspace.lock();
        let open = ws
            .get(doc)
            .ok_or_else(|| "dokumen tidak terbuka".to_string())?;
        mapped.unwrap_or(open.page_count)
    };
    let pages: Vec<u32> = match pages {
        Some(p) => p.into_iter().filter(|&n| n < page_count).collect(),
        None => (0..page_count).collect(),
    };
    if pages.is_empty() {
        return Err("Tidak ada halaman untuk dikenali.".into());
    }
    let run = Arc::new(Run::default());
    run.total.store(pages.len() as u32, Ordering::Relaxed);
    {
        let mut runs = state.ocr.runs.lock();
        if runs.contains_key(&doc) {
            return Err("OCR sedang berjalan untuk dokumen ini.".into());
        }
        runs.insert(doc, Arc::clone(&run));
    }
    let seen = Arc::clone(&run);
    let job = OcrJob {
        pages,
        force,
        models,
        progress: Some(Arc::new(move |done, total| {
            seen.done.store(done, Ordering::Relaxed);
            seen.total.store(total, Ordering::Relaxed);
        })),
        cancel: Arc::clone(&run.cancel),
    };
    let rewrite = Rewrite {
        redaction: None,
        ocr: Some(job),
        text: None,
    };
    let result = crate::save_commands::save_with(&state, doc, target, Some(&rewrite)).await;
    state.ocr.runs.lock().remove(&doc);
    let report = result?;
    if let Some(o) = &report.ocr {
        tracing::info!(
            doc,
            pages = o.pages_read,
            skipped = o.pages_had_text,
            words = o.words,
            "OCR selesai"
        );
    }
    Ok(report)
}

#[tauri::command]
pub fn ocr_progress(state: tauri::State<'_, AppState>, doc: u64) -> OcrProgress {
    match state.ocr.runs.lock().get(&doc) {
        Some(run) => OcrProgress {
            running: true,
            done: run.done.load(Ordering::Relaxed),
            total: run.total.load(Ordering::Relaxed),
        },
        None => OcrProgress {
            running: false,
            done: 0,
            total: 0,
        },
    }
}

/// Stops the run on `doc` before its next page. Nothing is written.
#[tauri::command]
pub fn ocr_cancel(state: tauri::State<'_, AppState>, doc: u64) {
    if let Some(run) = state.ocr.runs.lock().get(&doc) {
        run.cancel.store(true, Ordering::Relaxed);
    }
}
