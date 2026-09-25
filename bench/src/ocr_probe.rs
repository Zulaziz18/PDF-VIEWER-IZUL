//! Runs the application's OCR (`izul_ocr::ocr_page`, the routine the worker
//! runs) over every page of a PDF and writes the result, so that tools which
//! are not PDFium can check it (`tools/ocr-proof`).
//!
//!     ocr-probe <in.pdf> <out.pdf>
//!
//! One JSON line per page: {"page", "ms", "words" | "had_text": true}.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::Instant;

use izul_ocr::{ocr_page, Ocr, PageOutcome};
use izul_pdf::engine::Engine;
use serde_json::json;

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("ocr-probe <in.pdf> <out.pdf>");
    let output = args.next().expect("ocr-probe <in.pdf> <out.pdf>");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let lib = if cfg!(windows) {
        root.join("vendor/pdfium/win-x64/bin/pdfium.dll")
    } else {
        root.join("vendor/pdfium/linux-x64/lib/libpdfium.so")
    };
    let engine = Engine::load_from(&lib).expect("PDFium");
    let t = Instant::now();
    let ocr = Ocr::load(
        &root.join("vendor/ocrs/text-detection.rten"),
        &root.join("vendor/ocrs/text-recognition.rten"),
    )
    .expect("model OCR (vendor/ocrs/fetch.sh)");
    eprintln!(
        "model dimuat dalam {:.0} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    let doc = engine.open_copied(&input, None).expect("buka");
    for page in 0..doc.page_count() {
        let t = Instant::now();
        let outcome = ocr_page(&doc, page, &ocr, false).expect("ocr");
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let line = match outcome {
            PageOutcome::HadText => json!({"page": page, "ms": ms, "had_text": true}),
            PageOutcome::Read { words } => json!({"page": page, "ms": ms, "words": words}),
        };
        println!("{line}");
    }
    std::fs::write(&output, doc.save_to_vec().expect("simpan")).expect("tulis");
}
