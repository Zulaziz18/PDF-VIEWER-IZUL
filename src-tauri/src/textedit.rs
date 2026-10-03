//! Replacing a document's own text, within one line (Phase 7, SPEC 17).
//!
//! Like redaction and OCR, a replacement is made while saving, on the
//! working copy, and checked by a worker before the file is written
//! (`saving::build_current`, `izul_pdf::textedit`): the rewrite has to be
//! exact — the rest of the line in place, the new text in the file's own
//! embedded font — and PDFium's in-memory editing was measured writing wrong
//! character codes, so there is no preview that could be trusted to match.
//! The dialog says so, and offers to write a copy instead.

use izul_model::geom::PdfRectF;
use izul_model::pages::PageEntry;

use crate::commands::AppState;
use crate::saving::{Rewrite, SaveReport, TextJob};

type CmdResult<T> = Result<T, String>;

/// The file's own page shown as display page `page`, or why its text cannot
/// be edited now.
pub fn own_page(map: Option<&[PageEntry]>, page: u32) -> CmdResult<u32> {
    let Some(map) = map else { return Ok(page) };
    match map.get(page as usize) {
        Some(PageEntry::Page {
            source: 0,
            page: own,
            rotation: 0,
        }) => Ok(*own),
        Some(_) => Err(
            "Halaman ini diputar, kosong, atau berasal dari berkas lain. Simpan dulu susunan \
             halamannya, lalu sunting teksnya."
                .into(),
        ),
        None => Err(format!("halaman {} tidak ada", page + 1)),
    }
}

/// Replaces the text in `rect` (display space) on display page `page` with
/// `text`, and saves — over the file, or to `target`.
#[tauri::command]
pub async fn text_replace(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: u32,
    rect: PdfRectF,
    text: String,
    target: Option<String>,
) -> CmdResult<SaveReport> {
    if text.contains(['\n', '\r']) {
        return Err("Teks pengganti harus satu baris.".into());
    }
    let own = own_page(state.annots.page_map(doc).as_deref(), page)?;
    let rewrite = Rewrite {
        redaction: None,
        ocr: None,
        text: Some(TextJob {
            page: own,
            rect,
            text,
        }),
        bare: false,
    };
    let report = crate::save_commands::save_with(&state, doc, target, Some(&rewrite)).await?;
    // The search index holds the page's old text; it is read again.
    state.indexer.forget(doc);
    let file_id = state.workspace.lock().get(doc).map_or(0, |d| d.file_id);
    if file_id > 0 {
        if let Ok(conn) = state.db() {
            if let Err(e) = izul_store::search::clear(&conn, izul_store::files::FileId(file_id)) {
                tracing::warn!(error = %e, "indeks teks lama tidak dapat dihapus");
            }
        }
    }
    if let Some(t) = &report.text {
        tracing::info!(doc, page, glyphs = t.glyphs, "teks diganti");
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_files_own_unturned_pages_can_be_edited() {
        assert_eq!(own_page(None, 3), Ok(3));
        let map = vec![
            PageEntry::own(2),
            PageEntry::Page {
                source: 0,
                page: 0,
                rotation: 1,
            },
            PageEntry::Page {
                source: 1,
                page: 0,
                rotation: 0,
            },
            PageEntry::Blank {
                width: 1.0,
                height: 1.0,
                rotation: 0,
            },
        ];
        assert_eq!(own_page(Some(&map), 0), Ok(2));
        for p in 1..4 {
            assert!(own_page(Some(&map), p).is_err(), "halaman {p}");
        }
        assert!(own_page(Some(&map), 9).is_err());
    }
}
