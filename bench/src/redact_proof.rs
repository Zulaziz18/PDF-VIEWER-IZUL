//! Runs the application's redaction on the cases of `tools/redaction-proof`
//! (Phase 6), so that tools which are not PDFium can check the result.
//!
//! The routine is `izul_pdf::redaction::redact_document` — the one the worker
//! runs — followed by PDFium's full save, as the worker's `WorkSave` does. The
//! marks are placed the way a user places them: over the text as PDFium lays
//! it out, in display space, or as a dragged rectangle.
//!
//!     redact-proof <spec.json>
//!
//! `spec.json`: `{"cases":[{"name","in","out","marks":[{"page","find"}|{"page","rect":[l,b,r,t]}]}]}`.
//! One JSON line per case on stdout.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use izul_model::geom::{PdfRectF, RotationQuarter};
use izul_pdf::engine::{Document, Engine};
use izul_pdf::redaction::{redact_document, AreaRequest, PageRequest, RedactFailure};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Spec {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    #[serde(rename = "in")]
    input: String,
    out: String,
    marks: Vec<Mark>,
}

#[derive(Deserialize)]
struct Mark {
    page: u32,
    #[serde(default)]
    find: Option<String>,
    #[serde(default)]
    rect: Option<[f32; 4]>,
}

/// Every occurrence of `needle` on a page, one box per line it spans, in
/// display space — a selection of that text.
fn find(doc: &Document, page: u32, needle: &str) -> Result<Vec<PdfRectF>, String> {
    let text = doc
        .page_text_boxed(page, RotationQuarter::None)
        .map_err(|e| e.to_string())?;
    let chars: Vec<char> = text.chars.iter().map(|c| c.unicode).collect();
    let want: Vec<char> = needle.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + want.len() <= chars.len() {
        if chars.get(i..i + want.len()) == Some(&want[..]) {
            let mut line: Option<PdfRectF> = None;
            for c in text.chars.get(i..i + want.len()).unwrap_or_default() {
                line = Some(match line {
                    // A new line when the box does not overlap the run so far
                    // vertically (text rotated in the page counts as one run).
                    Some(r) if c.rect.top < r.bottom || c.rect.bottom > r.top => {
                        out.push(r);
                        c.rect
                    }
                    Some(r) => r.union(&c.rect),
                    None => c.rect,
                });
            }
            out.extend(line);
            i += want.len();
        } else {
            i += 1;
        }
    }
    if out.is_empty() {
        return Err(format!("'{needle}' tidak ada di halaman {}", page + 1));
    }
    Ok(out)
}

fn run(engine: &'static Engine, case: &Case) -> Result<serde_json::Value, String> {
    let doc = engine
        .open_copied(&case.input, None)
        .map_err(|e| e.to_string())?;
    let before: Vec<String> = (0..doc.page_count())
        .map(|p| doc.page_text(p).unwrap_or_default())
        .collect();
    let mut requests: Vec<PageRequest> = Vec::new();
    for m in &case.marks {
        let rects = match (&m.find, m.rect) {
            (Some(needle), _) => find(&doc, m.page, needle)?,
            (None, Some([l, b, r, t])) => vec![PdfRectF::new(l, b, r, t)],
            (None, None) => return Err("tanda tanpa find/rect".into()),
        };
        let areas = rects.into_iter().map(|rect| AreaRequest {
            rect,
            fill: Some([0.0, 0.0, 0.0]),
        });
        match requests.iter_mut().find(|r| r.page == m.page) {
            Some(r) => r.areas.extend(areas),
            None => requests.push(PageRequest {
                page: m.page,
                areas: areas.collect(),
            }),
        }
    }
    let (redacted, results) = redact_document(&doc, &requests).map_err(|e| match e {
        RedactFailure::Pdf(e) => e.to_string(),
        RedactFailure::Refused(s) => s,
    })?;
    let bytes = redacted.save_to_vec().map_err(|e| e.to_string())?;
    std::fs::write(&case.out, &bytes).map_err(|e| e.to_string())?;
    let texts: Vec<String> = (0..redacted.page_count())
        .map(|p| redacted.page_text(p).unwrap_or_default())
        .collect();
    let pages: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            json!({
                "page": r.page,
                "glyphs": r.counts.glyphs,
                "images_removed": r.counts.images_removed,
                "images_cleared": r.counts.images_cleared,
                "images_unsupported": r.counts.images_unsupported,
                "paths": r.counts.paths,
                "forms": r.counts.forms,
                "marked_content": r.counts.marked_content,
                "annotations": r.annotations,
                "areas": r.areas.iter().map(|a| [a.left, a.bottom, a.right, a.top]).collect::<Vec<_>>(),
                "beyond": r.beyond,
            })
        })
        .collect();
    Ok(
        json!({ "pages": pages, "pdfium_text": texts, "pdfium_before": before, "bytes": bytes.len() }),
    )
}

fn main() {
    let spec_path = std::env::args().nth(1).expect("redact-proof <spec.json>");
    let spec: Spec =
        serde_json::from_slice(&std::fs::read(&spec_path).expect("spec")).expect("spec JSON");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let lib = if cfg!(windows) {
        root.join("vendor/pdfium/win-x64/bin/pdfium.dll")
    } else {
        root.join("vendor/pdfium/linux-x64/lib/libpdfium.so")
    };
    let engine = Engine::load_from(&lib).expect("PDFium (vendor/pdfium/fetch.sh)");
    for case in &spec.cases {
        let line = match run(engine, case) {
            Ok(v) => json!({ "name": case.name, "ok": true, "result": v }),
            Err(e) => json!({ "name": case.name, "ok": false, "error": e }),
        };
        println!("{line}");
    }
}
