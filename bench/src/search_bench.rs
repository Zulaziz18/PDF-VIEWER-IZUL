//! SPEC 13's two search targets, measured (Phase 8): "Pencarian dalam dokumen
//! 500 halaman < 200 ms" and "Pencarian pustaka (FTS5) < 100 ms". No earlier
//! phase measured either; this harness is what the final report quotes.
//!
//! The same pieces the application runs, without the webview:
//! - the page text PDFium extracts (`izul_pdf`), indexed into the real app
//!   database schema through `izul_store::search`, 16 pages per commit as the
//!   background indexer does;
//! - "in the document" = the FTS5 query restricted to the file (which pages
//!   match, with snippets — what the results panel lists), then PDFium's own
//!   find on the pages of the first screen of results (the boxes to
//!   highlight). Both halves are timed, and their sum is held to 200 ms;
//! - "library" = the FTS5 query over every indexed document, against a
//!   library of 20 x 500 pages;
//! - for reference, PDFium's find over all 500 pages with no index at all —
//!   the cost the index exists to avoid.
//!
//!   cargo run --release -p izul-bench --bin search-bench [-- --out file.txt]

use std::path::PathBuf;
use std::time::Instant;

use izul_pdf::engine::Engine;
use izul_pdf::find::FindOptions;
use izul_pdf::geom::RotationQuarter;
use izul_store::files::FileId;
use izul_store::identity::FileStamp;
use izul_store::{search, Which};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn p50_p95(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    (at(0.5), at(0.95))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let lib = if cfg!(windows) {
        root.join("vendor/pdfium/win-x64/bin/pdfium.dll")
    } else {
        root.join("vendor/pdfium/linux-x64/lib/libpdfium.so")
    };
    let fixture = root.join("test-fixtures/text-500p.pdf");
    let engine = Engine::load_from(&lib).expect("PDFium");
    let doc = engine
        .open_copied(&fixture, None)
        .expect("buka text-500p.pdf");
    let pages = doc.page_count();

    let mut report = Vec::new();
    let mut line = |s: String| {
        println!("{s}");
        report.push(s);
    };
    line(format!(
        "Fase 8 — pencarian (SPEC 13), {} {}, build {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    ));
    line(format!("Dokumen: text-500p.pdf, {pages} halaman."));
    line(String::new());

    // Extract once, as the background indexer does page by page.
    let t = Instant::now();
    let texts: Vec<(u32, String)> = (0..pages)
        .map(|p| (p, doc.page_text(p).expect("teks")))
        .collect();
    let extract_ms = ms(t);

    let dir = std::env::temp_dir().join(format!("izul-search-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let conn = izul_store::open(&dir, Which::App).expect("basis data");
    let stamp = FileStamp { size: 1, mtime: 1 };
    // Twenty documents in the library: the one being read and nineteen more.
    let mut files = Vec::new();
    let t = Instant::now();
    for i in 0..20 {
        conn.execute(
            "INSERT INTO files (path, path_hash, size, mtime) VALUES (?1, ?2, 1, 1)",
            rusqlite::params![
                format!("C:\\\\pustaka\\\\dokumen-{i:02}.pdf"),
                format!("h{i}").into_bytes()
            ],
        )
        .expect("berkas");
        let file = FileId(conn.last_insert_rowid());
        search::begin(&conn, file, stamp, pages).expect("mulai");
        for chunk in texts.chunks(16) {
            search::put_pages(&conn, file, chunk).expect("halaman");
        }
        files.push(file);
    }
    let index_ms = ms(t);
    line(format!(
        "Indeks: ekstraksi teks {extract_ms:.0} ms untuk {pages} halaman; menulis 20 x {pages} = {} halaman ke FTS5 {index_ms:.0} ms.",
        20 * pages
    ));
    line(String::new());

    let opts = FindOptions {
        case_sensitive: false,
        whole_word: false,
        max_hits: 200,
    };
    let queries = [
        ("kata umum", "kesimpulan"),
        ("kata jarang (satu halaman)", "Halaman 347"),
        ("dua kata", "structured document"),
    ];
    line(format!(
        "{:<28} {:>7} {:>10} {:>10} {:>12} {:>10}  {}",
        "kueri", "hal.", "FTS p50", "FTS p95", "sorot p50", "total p95", "target 200 ms"
    ));
    let mut all_pass = true;
    for (label, q) in queries {
        let mut fts = Vec::new();
        let mut boxes = Vec::new();
        let mut total = Vec::new();
        let mut hit_pages = 0;
        for _ in 0..20 {
            let t = Instant::now();
            let hits = search::search_file(&conn, files[0], q, 500).expect("cari");
            let a = ms(t);
            hit_pages = hits.len();
            // The first screen of results: the ten best pages get their boxes.
            let t = Instant::now();
            for h in hits.iter().take(10) {
                let _ = doc
                    .find_on_page(h.page, q, opts, RotationQuarter::None)
                    .expect("find");
            }
            let b = ms(t);
            fts.push(a);
            boxes.push(b);
            total.push(a + b);
        }
        let (f50, f95) = p50_p95(fts);
        let (b50, _) = p50_p95(boxes);
        let (_, t95) = p50_p95(total);
        let pass = t95 < 200.0;
        all_pass &= pass;
        line(format!(
            "{label:<28} {hit_pages:>7} {f50:>8.2}ms {f95:>8.2}ms {b50:>10.2}ms {t95:>8.2}ms  {}",
            if pass { "LULUS" } else { "GAGAL" }
        ));
    }
    line(String::new());

    line(format!(
        "{:<28} {:>7} {:>10} {:>10}  {}",
        "pustaka (20 dokumen)", "hasil", "p50", "p95", "target 100 ms"
    ));
    for (label, q) in queries {
        let mut v = Vec::new();
        let mut n = 0;
        for _ in 0..20 {
            let t = Instant::now();
            n = search::search_library(&conn, q, 200)
                .expect("pustaka")
                .len();
            v.push(ms(t));
        }
        let (p50, p95) = p50_p95(v);
        let pass = p95 < 100.0;
        all_pass &= pass;
        line(format!(
            "{label:<28} {n:>7} {p50:>8.2}ms {p95:>8.2}ms  {}",
            if pass { "LULUS" } else { "GAGAL" }
        ));
    }
    line(String::new());

    // What the index saves: PDFium's find on every page, no index.
    let t = Instant::now();
    let mut n = 0;
    for p in 0..pages {
        n += doc
            .find_on_page(p, "kesimpulan", opts, RotationQuarter::None)
            .expect("find")
            .len();
    }
    line(format!(
        "Rujukan tanpa indeks: find PDFium di {pages} halaman, \"kesimpulan\": {:.0} ms, {n} kecocokan.",
        ms(t)
    ));
    line(format!(
        "Semua target: {}",
        if all_pass { "LULUS" } else { "ADA YANG GAGAL" }
    ));

    let _ = std::fs::remove_dir_all(&dir);
    if let Some(out) = out {
        std::fs::write(out, report.join("\n") + "\n").expect("tulis hasil");
    }
}
