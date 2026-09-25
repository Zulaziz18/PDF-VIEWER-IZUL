//! "Hapus Latar" (Phase 7) through a real worker: the picture goes up in
//! pieces, ONNX Runtime runs the model, the annotation gets the new picture in
//! one undo step, and undo brings the old one back. Scoring the model itself
//! is `tools/background-proof`'s job.
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
use std::sync::Arc;

use izul_app::annots::AnnotState;
use izul_app::saving;
use izul_app::supervisor::worker::WorkerPaths;
use izul_app::supervisor::Pool;
use izul_ipc::message::BackgroundKind;
use izul_ipc::message::{DocId, Response};
use izul_model::annot::{AnnotId, AnnotKind, AnnotObject, AnnotPayload};
use izul_model::display::ImageRef;
use izul_model::geom::PdfRectF;
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

fn runtime() -> Option<(PathBuf, PathBuf)> {
    let dir = repo_root().join("vendor/onnx");
    let lib = dir.join("linux-x64/libonnxruntime.so");
    let model = dir.join("u2netp.onnx");
    (lib.exists() && model.exists()).then_some((lib, model))
}

fn skip(reason: &str) {
    if std::env::var_os("IZUL_REQUIRE_FIXTURES").is_some() {
        panic!("prasyarat hilang dan IZUL_REQUIRE_FIXTURES diset: {reason}");
    }
    eprintln!("LEWATI: {reason}");
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

    async fn stop(self) {
        self.pool.write().await.shutdown().await;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A red stamp — two rings and a bar — on slightly shaded paper, as PNG.
fn stamp() -> Vec<u8> {
    let (w, h) = (240u32, 240u32);
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let (dx, dy) = (x as f32 - 120.0, y as f32 - 120.0);
        let r = (dx * dx + dy * dy).sqrt();
        let ink = (80.0..92.0).contains(&r)
            || (50.0..56.0).contains(&r)
            || (dy.abs() < 10.0 && dx.abs() < 45.0);
        if ink {
            image::Rgba([170, 30, 40, 255])
        } else {
            let shade = 1.0 - 0.15 * (x as f32 / w as f32);
            image::Rgba([
                (246.0 * shade) as u8,
                (243.0 * shade) as u8,
                (236.0 * shade) as u8,
                255,
            ])
        }
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn one_page() -> Vec<u8> {
    let objs = [
        "<</Type/Catalog/Pages 2 0 R>>".to_string(),
        "<</Type/Pages/Kids[3 0 R]/Count 1>>".to_string(),
        "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]>>".to_string(),
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

#[tokio::test]
async fn the_paper_goes_the_ink_stays_and_undo_brings_it_back() {
    let Some(live) = Live::start("latar").await else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let Some(paths) = runtime() else {
        return skip("ONNX Runtime atau model belum diambil (vendor/onnx/fetch.sh)");
    };
    let path = live.dir.join("surat.pdf");
    std::fs::write(&path, one_page()).unwrap();
    live.open(1, &path).await;
    let state = AnnotState::new();
    saving::import_page(&live.pool, &state, 1, 0).await.unwrap();
    let original = state.add_image(1, stamp()).unwrap();
    let (made, _) = state
        .add(
            1,
            AnnotObject::new(
                AnnotId(0),
                0,
                AnnotKind::Image,
                PdfRectF::new(100.0, 500.0, 220.0, 620.0),
                AnnotPayload::Image {
                    image: ImageRef(original),
                    crop: PdfRectF::new(0.0, 0.0, 1.0, 1.0),
                    opacity: 1.0,
                },
            ),
        )
        .unwrap();

    let result = izul_app::background::remove_background(
        &live.pool,
        &state,
        1,
        made.id.0,
        BackgroundKind::OnPaper,
        paths,
    )
    .await
    .unwrap();
    assert!(result.paper, "kertas polos dikenali");
    assert!(result.device.starts_with("CPU"), "{}", result.device);
    let obj = result
        .edit
        .objects
        .iter()
        .find(|o| o.id == made.id)
        .unwrap();
    let AnnotPayload::Image { image, .. } = obj.payload else {
        panic!("{obj:?}")
    };
    assert_ne!(image.0, original, "gambar baru");
    let png = image::load_from_memory(&state.image(1, image.0).unwrap().bytes)
        .unwrap()
        .to_rgba8();
    assert_eq!(png.dimensions(), (240, 240));
    // Ink (on the outer ring), paper inside the ring, paper outside it.
    assert!(
        png.get_pixel(206, 120).0[3] > 200,
        "tinta tetap: {:?}",
        png.get_pixel(206, 120)
    );
    assert!(
        png.get_pixel(120, 80).0[3] < 30,
        "kertas di dalam cincin: {:?}",
        png.get_pixel(120, 80)
    );
    assert!(
        png.get_pixel(10, 10).0[3] < 30,
        "kertas di luar: {:?}",
        png.get_pixel(10, 10)
    );

    // One undo step, and the original picture is back.
    let undone = state.undo(1, None).unwrap();
    let back = undone.objects.iter().find(|o| o.id == made.id).unwrap();
    assert!(matches!(back.payload, AnnotPayload::Image { image, .. } if image.0 == original));
    live.stop().await;
}
