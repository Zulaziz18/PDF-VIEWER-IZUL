//! Phase 5 numbers: how long rearranging pages costs at save time, on the
//! 500-page fixtures (`bench/make_fixtures.py`).
//!
//! ```text
//! cargo run --release -p izul-bench --bin fase5-uji
//! ```
//!
//! Three rearrangements, each applied to a fresh working copy exactly as
//! `WorkArrange` does in the worker, then saved with PDFium: every page in
//! reverse order; every other page deleted; and a whole second 500-page file
//! merged onto the end. Writes `bench/results/phase5-<os>.txt`.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use izul_pdf::{ArrangeSource, Arranged, Document, Engine};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn pdfium() -> PathBuf {
    if cfg!(windows) {
        repo_root().join("vendor/pdfium/win-x64/bin/pdfium.dll")
    } else {
        repo_root().join("vendor/pdfium/linux-x64/lib/libpdfium.so")
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Builds the arrangement for a document of `n` pages; the second argument
/// is the foreign document to merge from.
type Plan = Box<dyn for<'a> Fn(u32, Option<&'a Document>) -> Vec<Arranged<'a>>>;

fn own(p: u32) -> Arranged<'static> {
    Arranged {
        source: ArrangeSource::Own(p),
        rotation: 0,
    }
}

/// Opens a working copy, arranges it, saves it; returns (arrange ms, save
/// ms, output MB, page count after reopening).
fn run(
    engine: &'static Engine,
    path: &PathBuf,
    plan: impl Fn(u32, Option<&Document>) -> Vec<Arranged<'_>>,
    foreign: Option<&Document>,
) -> (f64, f64, f64, u32) {
    let work = engine.open_copied(path, None).expect("salinan kerja");
    work.set_strip_izul(false);
    let pages = plan(work.page_count(), foreign);
    let t = Instant::now();
    work.arrange(&pages, None).expect("susun");
    let arrange = ms(t);
    let t = Instant::now();
    let bytes = work.save_to_vec().expect("simpan");
    let save = ms(t);
    let mb = bytes.len() as f64 / 1_048_576.0;
    let back = engine
        .open_bytes(bytes, None::<&str>, None)
        .expect("buka ulang");
    (arrange, save, mb, back.page_count())
}

fn main() {
    let engine = Engine::load_from(pdfium()).expect("PDFium (jalankan vendor/pdfium/fetch.sh)");
    let fixtures = repo_root().join("test-fixtures");
    let mut report = String::new();
    let _ = writeln!(
        report,
        "Fase 5 — susun ulang halaman saat simpan ({} {})",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(
        report,
        "{:<22} {:<26} {:>10} {:>10} {:>9} {:>7}",
        "berkas", "operasi", "susun ms", "simpan ms", "hasil MB", "halaman"
    );
    let extra = fixtures.join("text-500p.pdf");
    let foreign = engine.open_copied(&extra, None).ok();
    for name in ["text-500p.pdf", "mixed-500p.pdf", "scan-50mb-500p.pdf"] {
        let path = fixtures.join(name);
        if !path.is_file() {
            let _ = writeln!(
                report,
                "{name:<22} (tidak ada — jalankan bench/make_fixtures.py)"
            );
            continue;
        }
        let cases: [(&str, Plan); 3] = [
            (
                "balik urutan semua",
                Box::new(|n, _| (0..n).rev().map(own).collect()),
            ),
            (
                "hapus tiap halaman genap",
                Box::new(|n, _| (0..n).step_by(2).map(own).collect()),
            ),
            (
                "gabung 500 halaman lain",
                Box::new(|n, f| {
                    let mut v: Vec<Arranged<'_>> = (0..n).map(own).collect();
                    if let Some(f) = f {
                        v.extend((0..f.page_count()).map(|p| Arranged {
                            source: ArrangeSource::From(f, p),
                            rotation: 0,
                        }));
                    }
                    v
                }),
            ),
        ];
        for (label, plan) in cases {
            let (arrange, save, mb, pages) = run(engine, &path, plan, foreign.as_ref());
            let _ = writeln!(
                report,
                "{name:<22} {label:<26} {arrange:>10.1} {save:>10.1} {mb:>9.1} {pages:>7}"
            );
        }
    }
    print!("{report}");
    let out = repo_root()
        .join("bench/results")
        .join(format!("phase5-{}.txt", std::env::consts::OS));
    std::fs::write(&out, report).expect("tulis hasil");
    println!("ditulis ke {}", out.display());
}
