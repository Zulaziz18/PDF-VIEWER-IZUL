//! Printing (SPEC 11.1: "cetak dengan pratinjau, rentang, skala, opsi cetak
//! anotasi"), found missing in Phase 8.
//!
//! The pages are rendered by a worker from the document as the editor has it
//! — with our annotations burnt in, or without them — through the same path
//! as "PDF ke Gambar" (`saving::export`), as JPEG files in a scratch folder.
//! The webview loads them from `izul://print/{job}/{index}` and prints them
//! through WebView2's own print dialog, which is where the printer, the
//! preview and the paper are chosen. When it closes, the folder goes.
//!
//! Images rather than the PDF itself: the print dialog of the webview prints
//! what the page shows, and handing a PDF to another program to print would
//! make printing depend on which PDF reader the user has.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use serde::Serialize;

use crate::commands::AppState;
use crate::saving::{self, Export};

type CmdResult<T> = Result<T, String>;

/// The most pages one print job renders: a printout of a 2 000-page book is
/// a job for the printer's own software.
const MAX_PAGES: usize = 1000;

#[derive(Debug, Default)]
pub struct PrintJobs {
    next: AtomicU64,
    jobs: Mutex<HashMap<u64, (PathBuf, Vec<PathBuf>)>>,
}

impl PrintJobs {
    /// The file behind `index` of `job`.
    pub fn page(&self, job: u64, index: u32) -> Option<PathBuf> {
        self.jobs
            .lock()
            .get(&job)
            .and_then(|(_, pages)| pages.get(index as usize).cloned())
    }

    fn forget(&self, job: u64) {
        if let Some((dir, _)) = self.jobs.lock().remove(&job) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PrintJob {
    pub job: u64,
    pub count: u32,
}

/// Renders `pages` (display pages) at `dpi` for printing.
#[tauri::command]
pub async fn print_prepare(
    state: tauri::State<'_, AppState>,
    doc: u64,
    pages: Vec<u32>,
    dpi: u32,
    annotations: bool,
) -> CmdResult<PrintJob> {
    if pages.is_empty() {
        return Err("Tidak ada halaman untuk dicetak.".into());
    }
    if pages.len() > MAX_PAGES {
        return Err(format!(
            "Paling banyak {MAX_PAGES} halaman sekali cetak; pilih rentang yang lebih kecil."
        ));
    }
    let path = state
        .workspace
        .lock()
        .get(doc)
        .map(|d| d.path.clone())
        .ok_or_else(|| format!("dokumen {doc} tidak terbuka"))?;
    crate::save_commands::prepare_fonts_for(&state, doc).await?;
    let job = state.printing.next.fetch_add(1, Ordering::Relaxed) + 1;
    let dir = state.data_dir.join("tmp").join(format!("cetak-{job}"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let scratch = state.data_dir.join("tmp");
    let written = saving::export(
        &state.pool,
        &state.annots,
        doc,
        &path,
        &scratch,
        Export::Images {
            folder: dir.to_string_lossy().to_string(),
            stem: "hal".into(),
            pages,
            dpi: dpi.clamp(72, 600),
            jpeg_quality: Some(92),
        },
        !annotations,
    )
    .await;
    let written = match written {
        Ok(w) => w,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
    };
    let count = written.len() as u32;
    state
        .printing
        .jobs
        .lock()
        .insert(job, (dir, written.into_iter().map(PathBuf::from).collect()));
    tracing::info!(
        doc,
        job,
        count,
        dpi,
        annotations,
        "halaman disiapkan untuk dicetak"
    );
    Ok(PrintJob { job, count })
}

/// The print dialog closed: its pages are no longer needed.
#[tauri::command]
pub fn print_done(state: tauri::State<'_, AppState>, job: u64) {
    state.printing.forget(job);
}
