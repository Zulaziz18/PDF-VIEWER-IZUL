//! SPEC 16's integration test, as written there: *buka → anotasi → simpan →
//! buka ulang → verifikasi objek masih editable* — through a real worker
//! process, the real save path, and the real atomic replace.
//!
//! Also the ways a save goes wrong quietly: saving twice must not duplicate an
//! annotation, "save as" must leave the original alone, and an export must
//! not change the file the tab has open.
//!
//! Unix-only like the other worker tests: the harness talks to the worker over
//! a Unix socket (see CLAUDE.md, "Cakupan yang belum ada").

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
use izul_app::saving::{self, Export};
use izul_app::supervisor::worker::WorkerPaths;
use izul_app::supervisor::Pool;
use izul_ipc::message::{DocId, Response};
use izul_model::annot::{
    AnnotId, AnnotKind, AnnotObject, AnnotPayload, FontSpec, ShapeStyle, TextAlign,
};
use izul_model::display::{ImageRef, Rgba};
use izul_model::geom::{PdfPointF, PdfRectF};
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

/// A two-page document with a line of text on each page.
fn source_pdf() -> Vec<u8> {
    let content = "BT /F1 18 Tf 60 740 Td (Dokumen uji simpan) Tj ET";
    let objs = [
        "<</Type/Catalog/Pages 2 0 R>>".to_string(),
        "<</Type/Pages/Kids[3 0 R 4 0 R]/Count 2>>".to_string(),
        "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Contents 5 0 R/Resources<</Font<</F1 6 0 R>>>>>>".to_string(),
        "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Contents 5 0 R/Resources<</Font<</F1 6 0 R>>>>>>".to_string(),
        format!("<</Length {}>>\nstream\n{content}\nendstream", content.len() + 1),
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

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([
            (x * 40) as u8,
            (y * 60) as u8,
            90,
            if x == 0 { 128 } else { 255 },
        ])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn objects(image: u32) -> Vec<AnnotObject> {
    let mut rect = AnnotObject::new(
        AnnotId(0),
        0,
        AnnotKind::Rect,
        PdfRectF::new(50.0, 500.0, 250.0, 600.0),
        AnnotPayload::Shape {
            style: ShapeStyle {
                fill: Some(Rgba::new(0.2, 0.6, 1.0, 0.4)),
                stroke: Some(Rgba::new(0.0, 0.2, 0.6, 1.0)),
                stroke_width: 2.0,
                ..ShapeStyle::default()
            },
        },
    );
    rect.author_note = "Kotak catatan — é".into();
    let highlight = AnnotObject::new(
        AnnotId(0),
        0,
        AnnotKind::Highlight,
        PdfRectF::new(58.0, 735.0, 300.0, 760.0),
        AnnotPayload::Markup {
            quads: vec![PdfRectF::new(58.0, 735.0, 300.0, 760.0)],
            color: Rgba::new(1.0, 0.85, 0.0, 1.0),
        },
    );
    let ink = AnnotObject::new(
        AnnotId(0),
        1,
        AnnotKind::Ink,
        PdfRectF::new(0.0, 0.0, 1.0, 1.0),
        AnnotPayload::Ink {
            strokes: vec![vec![
                PdfPointF::new(100.0, 100.0),
                PdfPointF::new(150.0, 160.0),
                PdfPointF::new(220.0, 110.0),
            ]],
            color: Rgba::new(0.1, 0.6, 0.2, 1.0),
            width: 3.0,
            smooth: true,
        },
    );
    let text = AnnotObject::new(
        AnnotId(0),
        1,
        AnnotKind::FreeText,
        PdfRectF::new(300.0, 300.0, 520.0, 360.0),
        AnnotPayload::FreeText {
            text: "Teks disimpan".into(),
            font: FontSpec::default(),
            color: Rgba::BLACK,
            align: TextAlign::Left,
            line_spacing: 1.2,
            background: None,
            border: Some(Rgba::BLACK),
        },
    );
    let picture = AnnotObject::new(
        AnnotId(0),
        1,
        AnnotKind::Image,
        PdfRectF::new(60.0, 400.0, 180.0, 480.0),
        AnnotPayload::Image {
            image: ImageRef(image),
            crop: PdfRectF::new(0.0, 0.0, 1.0, 1.0),
            opacity: 1.0,
        },
    );
    vec![rect, highlight, ink, text, picture]
}

struct Live {
    pool: Arc<RwLock<Pool>>,
    dir: PathBuf,
}

impl Live {
    async fn start(name: &str) -> Option<Self> {
        let pool = Pool::start(worker_paths()?).await.ok()?;
        let dir = std::env::temp_dir().join(format!("izul-save-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Live {
            pool: Arc::new(RwLock::new(pool)),
            dir,
        })
    }

    async fn open(&self, doc: u64, path: &Path) -> u32 {
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
            Response::Opened { page_count, .. } => page_count,
            other => panic!("{other:?}"),
        }
    }

    /// A fresh editor state, as after restarting the application, with every
    /// page's saved annotations taken in.
    async fn reopen(&self, doc: u64, path: &Path) -> (AnnotState, Vec<AnnotObject>) {
        let pages = self.open(doc, path).await;
        let state = AnnotState::new();
        for page in 0..pages {
            saving::import_page(&self.pool, &state, doc, page)
                .await
                .unwrap();
        }
        let objs = state.objects(doc, None);
        (state, objs)
    }

    fn leftovers(&self) -> Vec<String> {
        std::fs::read_dir(&self.dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(izul_write::atomic::TEMP_PREFIX))
            .collect()
    }

    async fn stop(self) {
        self.pool.write().await.shutdown().await;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Lays text out: the metrics come from the worker, as in the application.
async fn edit(live: &Live, state: &AnnotState, doc: u64) -> Vec<AnnotObject> {
    let image = state.add_image(doc, png(5, 3)).unwrap();
    let mut out = Vec::new();
    for obj in objects(image) {
        if let AnnotPayload::FreeText { font, text, .. } = &obj.payload {
            let missing = state
                .ensure_metrics(&live.pool, doc, font, text)
                .await
                .unwrap();
            assert!(missing.is_empty());
        }
        let (created, _) = state.add(doc, obj).unwrap();
        out.push(created);
    }
    out
}

#[tokio::test]
async fn open_annotate_save_reopen_and_edit_again() {
    let Some(live) = Live::start("roundtrip").await else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let path = live.dir.join("dokumen.pdf");
    std::fs::write(&path, source_pdf()).unwrap();
    let original = std::fs::read(&path).unwrap();

    // Open, annotate.
    let pages = live.open(1, &path).await;
    let state = AnnotState::new();
    for p in 0..pages {
        saving::import_page(&live.pool, &state, 1, p).await.unwrap();
    }
    let made = edit(&live, &state, 1).await;
    assert!(state.is_dirty(1));

    // Save over the file itself.
    let report = saving::save(
        &live.pool,
        &state,
        1,
        &path.to_string_lossy(),
        pages,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(report.annotations, made.len() as u32);
    assert!(!state.is_dirty(1), "a save leaves nothing unsaved");
    let saved = std::fs::read(&path).unwrap();
    assert!(saved.len() > original.len());
    assert!(
        live.leftovers().is_empty(),
        "no temporary file left: {:?}",
        live.leftovers()
    );

    // The tab keeps working after its file was replaced under it.
    assert!(!state.write_set(1).unwrap().1.is_empty());

    // Reopen as a fresh application would: every object is back, live.
    let (state2, back) = live.reopen(2, &path).await;
    let mut want = made.clone();
    want.sort_by_key(|o| o.id);
    let mut got = back.clone();
    got.sort_by_key(|o| o.id);
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(want.iter()) {
        assert_eq!(g.kind, w.kind);
        assert_eq!(g.rect, w.rect, "{:?}", g.kind);
        assert_eq!(g.author_note, w.author_note);
        if !matches!(g.payload, AnnotPayload::Image { .. }) {
            assert_eq!(g.payload, w.payload, "{:?}", g.kind);
        }
    }
    // The picture came back as the same pixels, alpha included.
    let pic = got.iter().find(|o| o.kind == AnnotKind::Image).unwrap();
    let AnnotPayload::Image { image, .. } = pic.payload else {
        unreachable!()
    };
    let stored = state2.image(2, image.0).unwrap();
    let decoded = image::load_from_memory(&stored.bytes).unwrap().to_rgba8();
    assert_eq!(
        decoded.into_raw(),
        image::load_from_memory(&png(5, 3))
            .unwrap()
            .to_rgba8()
            .into_raw()
    );

    // Edit an object that came from the file: it is editable, and saving again
    // replaces it rather than adding a second copy.
    let mut moved = got
        .iter()
        .find(|o| o.kind == AnnotKind::Rect)
        .unwrap()
        .clone();
    moved.translate(10.0, -20.0);
    state2.replace(2, vec![moved.clone()]).unwrap();
    live.pool.write().await.release(DocId(1)).await.unwrap();
    saving::save(
        &live.pool,
        &state2,
        2,
        &path.to_string_lossy(),
        pages,
        None,
        None,
    )
    .await
    .unwrap();

    let (_, third) = live.reopen(3, &path).await;
    assert_eq!(
        third.len(),
        made.len(),
        "saving twice must not duplicate annotations"
    );
    let rect = third.iter().find(|o| o.kind == AnnotKind::Rect).unwrap();
    assert_eq!(rect.rect, moved.rect, "the edit survived");

    live.stop().await;
}

#[tokio::test]
async fn save_as_moves_the_tab_and_leaves_the_original() {
    let Some(live) = Live::start("saveas").await else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let path = live.dir.join("asli.pdf");
    std::fs::write(&path, source_pdf()).unwrap();
    let original = std::fs::read(&path).unwrap();
    let pages = live.open(1, &path).await;
    let state = AnnotState::new();
    saving::import_page(&live.pool, &state, 1, 0).await.unwrap();
    edit(&live, &state, 1).await;

    let copy = live.dir.join("salinan.pdf");
    let report = saving::save(
        &live.pool,
        &state,
        1,
        &path.to_string_lossy(),
        pages,
        Some(&copy.to_string_lossy()),
        None,
    )
    .await
    .unwrap();
    assert_eq!(PathBuf::from(&report.path), copy);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "the original is untouched"
    );
    let (_, back) = live.reopen(2, &copy).await;
    assert_eq!(back.len(), 5);
    live.stop().await;
}

#[tokio::test]
async fn exports_write_new_files_and_touch_nothing_else() {
    let Some(live) = Live::start("export").await else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let path = live.dir.join("sumber.pdf");
    std::fs::write(&path, source_pdf()).unwrap();
    let original = std::fs::read(&path).unwrap();
    let pages = live.open(1, &path).await;
    let state = AnnotState::new();
    for p in 0..pages {
        saving::import_page(&live.pool, &state, 1, p).await.unwrap();
    }
    edit(&live, &state, 1).await;
    let scratch = live.dir.join("tmp");
    let source = path.to_string_lossy().to_string();

    // Flattened: the annotations are page content now, none is left as an
    // annotation this application would pick up again.
    let flat = live.dir.join("rata.pdf");
    let out = saving::export(
        &live.pool,
        &state,
        1,
        &source,
        &scratch,
        Export::Flat {
            target: flat.to_string_lossy().into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(out.len(), 1);
    let (_, none) = live.reopen(2, &flat).await;
    assert!(none.is_empty(), "a flattened file has no live annotations");

    // One page, as a new document, annotations still live.
    let one = live.dir.join("halaman-2.pdf");
    saving::export(
        &live.pool,
        &state,
        1,
        &source,
        &scratch,
        Export::Pages {
            target: one.to_string_lossy().into(),
            pages: vec![1],
        },
    )
    .await
    .unwrap();
    let (_, second_page) = live.reopen(3, &one).await;
    assert_eq!(
        second_page.len(),
        3,
        "page 2's ink, text and picture came along"
    );

    // Both pages as PNG at 72 dpi: A4 is 595 x 842 points.
    let images = saving::export(
        &live.pool,
        &state,
        1,
        &source,
        &scratch,
        Export::Images {
            folder: live.dir.to_string_lossy().into(),
            stem: "hal".into(),
            pages: vec![0, 1],
            dpi: 72,
            jpeg_quality: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(images.len(), 2);
    let first = image::open(&images[0]).unwrap();
    assert_eq!((first.width(), first.height()), (595, 842));

    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "exporting never writes the source"
    );
    assert!(state.is_dirty(1), "an export is not a save");
    assert!(live.leftovers().is_empty());
    live.stop().await;
}
