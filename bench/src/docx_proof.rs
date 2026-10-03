//! Converts PDFs to Word with the application's own conversion (7.1.0), so
//! that a reader which is not this application — LibreOffice in
//! `tools/docx-proof` — can check the result.
//!
//! The routine is the one the export runs: `Document::page_layout` for every
//! page (what the worker answers `WorkLayout` with), then
//! `izul_docx::analyse` and `izul_docx::build` (what the UI process does with
//! the pages).
//!
//!     docx-proof <in.pdf> <out.docx> [--dump]
//!
//! One JSON line on stdout with what came across; `--dump` also prints each
//! paragraph's style and text.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use izul_docx::layout::Block;
use izul_pdf::engine::Engine;
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input = args
        .get(1)
        .expect("docx-proof <in.pdf> <out.docx> [--dump]");
    let output = args
        .get(2)
        .expect("docx-proof <in.pdf> <out.docx> [--dump]");
    let dump = args.iter().any(|a| a == "--dump");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let lib = if cfg!(windows) {
        root.join("vendor/pdfium/win-x64/bin/pdfium.dll")
    } else {
        root.join("vendor/pdfium/linux-x64/lib/libpdfium.so")
    };
    let engine = Engine::load_from(&lib).expect("PDFium (vendor/pdfium/fetch.sh)");
    let doc = engine.open(input, None).expect("PDF");
    let started = std::time::Instant::now();
    let pages: Vec<_> = (0..doc.page_count())
        .map(|p| doc.page_layout(p).expect("halaman"))
        .collect();
    let read_ms = started.elapsed().as_millis();
    let analysed = izul_docx::analyse(pages);
    if dump {
        for block in &analysed.blocks {
            match block {
                Block::Paragraph(p) => {
                    let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                    let sizes: Vec<String> = p
                        .runs
                        .iter()
                        .map(|r| {
                            format!(
                                "{}{}{}",
                                r.size,
                                if r.bold { "b" } else { "" },
                                if r.italic { "i" } else { "" }
                            )
                        })
                        .collect();
                    println!(
                        "P h={:?} {:?} ind={:.0}/{:.0} sb={:.0} ls={:.2} pb={} [{}] {}",
                        p.heading,
                        p.align,
                        p.indent_left,
                        p.first_line,
                        p.space_before,
                        p.line_spacing,
                        p.page_break_before,
                        sizes.join(","),
                        text
                    );
                }
                Block::Picture(p) => println!("IMG {}x{} {:?}", p.width, p.height, p.align),
            }
        }
    }
    let title = std::path::Path::new(input)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let built = izul_docx::build(&analysed, &title).expect("docx");
    std::fs::write(output, &built.bytes).expect("tulis");
    let s = built.stats;
    println!(
        "{}",
        json!({
            "in": input, "out": output, "bytes": built.bytes.len(),
            "pages": s.pages, "paragraphs": s.paragraphs, "pictures": s.pictures,
            "skipped_turned": s.skipped_turned, "skipped_pictures": s.skipped_pictures,
            "pages_without_text": s.pages_without_text,
            "body_size": analysed.body_size, "body_font": analysed.body_font,
            "ms": started.elapsed().as_millis(), "read_ms": read_ms,
        })
    );
}
