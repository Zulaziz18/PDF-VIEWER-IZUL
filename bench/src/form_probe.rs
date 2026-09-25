//! Fills a form with `izul_pdf::forms` (the routine the worker runs) and
//! writes the result, for `tools/form-proof`.
//!
//!     form-probe <in.pdf> <out.pdf> '<json: [[name, value], ...]>'
//!
//! Values: {"Text": "..."} | {"Checked": true} | {"Radio": "export"} |
//! {"Choice": [index, ...]}. Prints the widgets before and after as JSON.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use izul_pdf::engine::Engine;
use izul_pdf::forms::FieldValue;
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
    let values: Vec<(String, FieldValue)> = serde_json::from_str(&args[2]).expect("nilai");
    let before = doc.form_widgets().expect("baca");
    let (changed, _) = doc.fill_form(&values).expect("isi");
    let after = doc.form_widgets().expect("baca lagi");
    std::fs::write(&args[1], doc.save_to_vec().expect("simpan")).expect("tulis");
    println!(
        "{}",
        json!({"changed": changed, "before": before, "after": after})
    );
}
