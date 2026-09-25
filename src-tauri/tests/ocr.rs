//! OCR on save (Phase 7) through a real worker: the invisible layer lands
//! where the text is seen — on a page turned by its own `/Rotate` too — a page
//! that already has text is left alone, and a cancelled run writes nothing.
//!
//! The pages here have real (vector) text, read again with `force`: that keeps
//! the test free of image fixtures, and gives a second, independent copy of
//! every word's position — PDFium's own layout of the original — to check the
//! OCR layer against. The scanned-page accuracy is measured separately, by
//! `tools/ocr-proof` with four extractors that are not PDFium.
//!
//! Unix-only like the other worker tests (see CLAUDE.md).

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use izul_app::annots::AnnotState;
use izul_app::saving::{self, OcrJob, Rewrite};
use izul_app::supervisor::worker::WorkerPaths;
use izul_app::supervisor::Pool;
use izul_ipc::message::{DocId, Request, Response};
use izul_model::geom::{PdfRectF, RotationQuarter};
use tokio::sync::RwLock;

const LINE: &str = "Laporan kegiatan bulanan";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn worker_paths() -> Option<WorkerPaths> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?.to_path_buf();
    let worker = dir.join("izul-worker");
    let pdfium = repo_root().join("vendor/pdfium/linux-x64/lib/libpdfium.so");
    (worker.exists() && pdfium.exists()).then_some(WorkerPaths {
        executable: worker,
        pdfium,
    })
}

fn models() -> Option<PathBuf> {
    let dir = repo_root().join("vendor/ocrs");
    dir.join("text-recognition.rten").exists().then_some(dir)
}

fn skip(reason: &str) {
    if std::env::var_os("IZUL_REQUIRE_FIXTURES").is_some() {
        panic!("prasyarat hilang dan IZUL_REQUIRE_FIXTURES diset: {reason}");
    }
    eprintln!("LEWATI: {reason}");
}

/// Two pages with one line of 24 pt text each: an ordinary page, and one
/// turned by its own `/Rotate 90` with a media box away from the origin.
fn document() -> Vec<u8> {
    let plain = format!("BT /F1 24 Tf 60 700 Td ({LINE}) Tj ET");
    let turned = format!("BT /F1 24 Tf 160 300 Td 0 1 -1 0 160 300 Tm ({LINE}) Tj ET");
    let objs = [
        "<</Type/Catalog/Pages 2 0 R>>".to_string(),
        "<</Type/Pages/Kids[3 0 R 4 0 R]/Count 2>>".to_string(),
        "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Contents 5 0 R/Resources<</Font<</F1 7 0 R>>>>>>".to_string(),
        "<</Type/Page/Parent 2 0 R/MediaBox[100 200 695 1042]/Rotate 90/Contents 6 0 R/Resources<</Font<</F1 7 0 R>>>>>>".to_string(),
        format!("<</Length {}>>\nstream\n{plain}\nendstream", plain.len() + 1),
        format!("<</Length {}>>\nstream\n{turned}\nendstream", turned.len() + 1),
        "<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_string(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offs.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let x = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{x}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

struct Live {
    pool: Arc<RwLock<Pool>>,
    dir: PathBuf,
}

impl Live {
    async fn start(name: &str) -> Option<Self> {
        let pool = Pool::start(worker_paths()?).await.ok()?;
        let dir = std::env::temp_dir().join(format!("izul-redact-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Live {
            pool: Arc::new(RwLock::new(pool)),
            dir,
        })
    }

    async fn open(&self, doc: u64, path: &Path) -> Vec<(f32, f32)> {
        let bytes = std::fs::read(path).unwrap();
        let key = izul_store::content_hash(&bytes);
        match self
            .pool
            .write()
            .await
            .open(DocId(doc), &path.to_string_lossy(), key, None)
            .await
            .unwrap()
        {
            Response::Opened { page_sizes, .. } => page_sizes,
            other => panic!("{other:?}"),
        }
    }

    async fn text(&self, doc: u64, page: u32) -> (String, Vec<izul_ipc::message::CharBoxWire>) {
        let worker = self.pool.read().await.worker_for(DocId(doc)).unwrap();
        let req = Request::ExtractText {
            doc: DocId(doc),
            page,
            with_boxes: true,
            rotation: RotationQuarter::None,
        };
        match Pool::ask(&worker, req).await.unwrap() {
            Response::TextReady { text, chars, .. } => (text, chars),
            other => panic!("{other:?}"),
        }
    }

    async fn stop(self) {
        self.pool.write().await.shutdown().await;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Display-space boxes of every run of `needle` on the page.
fn runs_of(chars: &[izul_ipc::message::CharBoxWire], needle: &str) -> Vec<PdfRectF> {
    let flat: Vec<char> = chars.iter().map(|c| c.unicode).collect();
    let n: Vec<char> = needle.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + n.len() <= flat.len() {
        if flat[i..i + n.len()] == n[..] {
            out.push(
                chars[i..i + n.len()]
                    .iter()
                    .map(|c| c.rect)
                    .reduce(|a, b| a.union(&b))
                    .unwrap(),
            );
            i += n.len();
        } else {
            i += 1;
        }
    }
    out
}

fn job(pages: Vec<u32>, force: bool, models: PathBuf, cancel: bool) -> Rewrite {
    Rewrite {
        redaction: None,
        ocr: Some(OcrJob {
            pages,
            force,
            models,
            progress: None,
            cancel: Arc::new(AtomicBool::new(cancel)),
        }),
    }
}

#[tokio::test]
async fn the_invisible_layer_lies_over_the_words_it_read() {
    let Some(live) = Live::start("ocr-letak").await else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let Some(models) = models() else {
        return skip("model OCR belum diambil (vendor/ocrs/fetch.sh)");
    };
    let path = live.dir.join("laporan.pdf");
    std::fs::write(&path, document()).unwrap();
    live.open(1, &path).await;
    let state = AnnotState::new();
    for p in 0..2 {
        saving::import_page(&live.pool, &state, 1, p).await.unwrap();
    }
    let out = live.dir.join("laporan (OCR).pdf");
    let report = saving::save(
        &live.pool,
        &state,
        1,
        &path.to_string_lossy(),
        2,
        Some(&out.to_string_lossy()),
        Some(&job(vec![0, 1], true, models, false)),
    )
    .await
    .unwrap();
    let ocr = report.ocr.expect("ringkasan OCR");
    assert_eq!(ocr.pages_read, 2, "{ocr:?}");
    assert!(ocr.words >= 6, "{ocr:?}");

    live.open(2, &out).await;
    for page in 0..2 {
        let (text, chars) = live.text(2, page).await;
        let runs = runs_of(&chars, "Laporan");
        assert_eq!(
            runs.len(),
            2,
            "halaman {}: asli + lapisan OCR: {text}",
            page + 1
        );
        // The layer's word lies over the original, as shown: centres within
        // a few points (a 24 pt word is ~90 pt wide).
        let (a, b) = (runs[0], runs[1]);
        let (ax, ay) = ((a.left + a.right) / 2.0, (a.top + a.bottom) / 2.0);
        let (bx, by) = ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0);
        assert!(
            (ax - bx).abs() < 6.0 && (ay - by).abs() < 6.0,
            "halaman {}: asli {a:?}, OCR {b:?}",
            page + 1
        );
        assert!(text.contains("kegiatan bulanan"), "{text}");
    }
    live.stop().await;
}

#[tokio::test]
async fn pages_with_text_are_left_alone_and_a_cancelled_run_writes_nothing() {
    let Some(live) = Live::start("ocr-lewati").await else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let Some(models) = models() else {
        return skip("model OCR belum diambil (vendor/ocrs/fetch.sh)");
    };
    let path = live.dir.join("laporan.pdf");
    std::fs::write(&path, document()).unwrap();
    let original = std::fs::read(&path).unwrap();
    live.open(1, &path).await;
    let state = AnnotState::new();
    for p in 0..2 {
        saving::import_page(&live.pool, &state, 1, p).await.unwrap();
    }
    let out = live.dir.join("laporan (OCR).pdf");
    let report = saving::save(
        &live.pool,
        &state,
        1,
        &path.to_string_lossy(),
        2,
        Some(&out.to_string_lossy()),
        Some(&job(vec![0, 1], false, models.clone(), false)),
    )
    .await
    .unwrap();
    let ocr = report.ocr.expect("ringkasan OCR");
    assert_eq!((ocr.pages_read, ocr.pages_had_text, ocr.words), (0, 2, 0));

    let again = live.dir.join("batal.pdf");
    let err = saving::save(
        &live.pool,
        &state,
        1,
        &path.to_string_lossy(),
        2,
        Some(&again.to_string_lossy()),
        Some(&job(vec![0, 1], true, models, true)),
    )
    .await
    .unwrap_err();
    assert_eq!(err, saving::OCR_CANCELLED);
    assert!(!again.exists(), "dibatalkan berarti tidak ada berkas");
    assert_eq!(std::fs::read(&path).unwrap(), original);
    live.stop().await;
}
