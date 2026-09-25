//! Replaces text with `izul_pdf::textedit` (the routine the worker runs) and
//! writes the result, for `tools/textedit-proof`.
//!
//!     textedit-probe <in.pdf> <out.pdf> <page> <text on the page> <replacement>
//!
//! The area is the display-space box of the first occurrence of the text, as
//! selecting it in the viewer gives. Prints `{"ok": true, "before", "glyphs"}`
//! or `{"ok": false, "refused": "..."}`; exits 0 either way.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use izul_pdf::engine::Engine;
use izul_pdf::geom::{PdfRectF, RotationQuarter};
use izul_pdf::redaction::RedactFailure;
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let lib = if cfg!(windows) {
        root.join("vendor/pdfium/win-x64/bin/pdfium.dll")
    } else {
        root.join("vendor/pdfium/linux-x64/lib/libpdfium.so")
    };
    let engine = Engine::load_from(&lib).expect("PDFium");
    let doc = engine.open_copied(&args[0], None).expect("buka");
    let page: u32 = args[2].parse().expect("halaman");
    let text = doc
        .page_text_boxed(page, RotationQuarter::None)
        .expect("teks");
    let start = text.text.find(&args[3]).expect("teks tidak ada di halaman");
    let first = text.text[..start].chars().count();
    let n = args[3].chars().count();
    let area = text.chars[first..first + n]
        .iter()
        .map(|c| c.rect)
        .reduce(|a, b| {
            PdfRectF::new(
                a.left.min(b.left),
                a.bottom.min(b.bottom),
                a.right.max(b.right),
                a.top.max(b.top),
            )
        })
        .expect("kotak");
    match izul_pdf::textedit::replace_text_document(&doc, page, area, &args[4]) {
        Ok((copy, done)) => {
            std::fs::write(&args[1], copy.save_to_vec().expect("simpan")).expect("tulis");
            println!(
                "{}",
                json!({"ok": true, "before": done.before, "glyphs": done.glyphs,
                       "area": [area.left, area.bottom, area.right, area.top]})
            );
        }
        Err(RedactFailure::Refused(why)) => println!("{}", json!({"ok": false, "refused": why})),
        Err(RedactFailure::Pdf(e)) => panic!("PDFium: {e}"),
    }
}
