//! Replacing a document's own text end to end (Phase 7): a real worker
//! rewrites the working copy at save, checks it with PDFium, and the file is
//! read back by another.
//!
//! Gated on Unix like the other integration suites — the harness speaks to
//! workers over Unix sockets; see `render_end_to_end.rs`.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use izul_app::annots::AnnotState;
use izul_app::saving;
use izul_app::supervisor::worker::WorkerPaths;
use izul_app::supervisor::Pool;
use izul_ipc::message::{DocId, Request, Response};
use izul_model::geom::{PdfRectF, RotationQuarter};
use tokio::sync::RwLock;

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

fn skip(reason: &str) {
    if std::env::var_os("IZUL_REQUIRE_FIXTURES").is_some() {
        panic!("prasyarat hilang dan IZUL_REQUIRE_FIXTURES diset: {reason}");
    }
    eprintln!("LEWATI: {reason}");
}

async fn open(pool: &Arc<RwLock<Pool>>, doc: u64, path: &Path) -> Vec<(f32, f32)> {
    let bytes = std::fs::read(path).unwrap();
    let key = izul_store::content_hash(&bytes);
    match pool
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

async fn text(
    pool: &Arc<RwLock<Pool>>,
    doc: u64,
    page: u32,
) -> (String, Vec<izul_ipc::message::CharBoxWire>) {
    let worker = pool.read().await.worker_for(DocId(doc)).unwrap();
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

/// The display-space box of the first `needle`, as selecting it gives.
fn box_of(chars: &[izul_ipc::message::CharBoxWire], needle: &str) -> PdfRectF {
    let flat: Vec<char> = chars.iter().map(|c| c.unicode).collect();
    let n: Vec<char> = needle.chars().collect();
    let at = (0..=flat.len() - n.len())
        .find(|&i| flat[i..i + n.len()] == n[..])
        .expect("ada di halaman");
    chars[at..at + n.len()]
        .iter()
        .map(|c| c.rect)
        .reduce(|a, b| a.union(&b))
        .unwrap()
}

fn job(page: u32, rect: PdfRectF, text: &str) -> saving::Rewrite {
    saving::Rewrite {
        redaction: None,
        ocr: None,
        text: Some(saving::TextJob {
            page,
            rect,
            text: text.to_string(),
        }),
    }
}

#[tokio::test]
async fn a_word_is_replaced_in_the_saved_file_and_a_missing_letter_is_refused() {
    let fixture = repo_root().join("test-fixtures/viewer-10p.pdf");
    if !fixture.exists() {
        return skip("test-fixtures/viewer-10p.pdf belum dibuat (bench/make_fixtures.py)");
    }
    let Some(paths) = worker_paths() else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let pool = Arc::new(RwLock::new(Pool::start(paths).await.unwrap()));
    let dir = std::env::temp_dir().join(format!("izul-textedit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bab.pdf");
    std::fs::copy(&fixture, &path).unwrap();
    let sizes = open(&pool, 1, &path).await;
    let state = AnnotState::new();
    state.set_own_pages(1, sizes.clone());
    let (before, chars) = text(&pool, 1, 0).await;
    let area = box_of(&chars, "halaman");

    // A letter the embedded subset does not have: refused, nothing written.
    let refused = dir.join("tidak.pdf");
    let err = saving::save(
        &pool,
        &state,
        1,
        &path.to_string_lossy(),
        sizes.len() as u32,
        Some(&refused.to_string_lossy()),
        Some(&job(0, area, "halamän")),
    )
    .await
    .unwrap_err();
    assert!(err.starts_with("Teks tidak diganti"), "{err}");
    assert!(err.contains("\"ä\""), "{err}");
    assert!(!refused.exists());

    let out = dir.join("bab (disunting).pdf");
    let report = saving::save(
        &pool,
        &state,
        1,
        &path.to_string_lossy(),
        sizes.len() as u32,
        Some(&out.to_string_lossy()),
        Some(&job(0, area, "lembar")),
    )
    .await
    .unwrap();
    let t = report.text.expect("ringkasan penggantian");
    assert_eq!((t.before.as_str(), t.glyphs), ("halaman", 7));

    open(&pool, 2, &out).await;
    let (after, _) = text(&pool, 2, 0).await;
    assert_eq!(after, before.replacen("halaman", "lembar", 1));
    // The other pages are as they were.
    for p in 1..sizes.len() as u32 {
        assert_eq!(
            text(&pool, 1, p).await.0,
            text(&pool, 2, p).await.0,
            "halaman {p}"
        );
    }
    pool.write().await.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}
