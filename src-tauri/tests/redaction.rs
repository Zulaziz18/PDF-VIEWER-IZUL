//! Redaction end to end (Phase 6): marks in the editor, applied by a real
//! worker at save, the result read back by a real worker and by this test's
//! own reading of every stream in the file.
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
use izul_model::annot::{AnnotId, AnnotKind, AnnotObject, AnnotPayload, NoteIcon, ShapeStyle};
use izul_model::display::Rgba;
use izul_model::geom::{PdfRectF, RotationQuarter};
use tokio::sync::RwLock;

const SECRET: &str = "3201234567890001";
const PUBLIC: &str = "Jalan Merdeka";

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

/// Three pages, each with the secret on one line and public text on another:
/// an ordinary page; a page turned by its own `/Rotate 90` whose media box
/// does not start at the origin; and a page where the secret is also in an
/// invisible text layer, as an OCR'd scan has it. Helvetica with no
/// `/Widths`, so the standard-14 metrics are what the redactor measures with.
fn document() -> Vec<u8> {
    let line = |y: u32, text: &str| {
        format!("BT /F1 14 Tf 60 {y} Td (NIK: {SECRET}) Tj 0 -40 Td ({text}) Tj ET")
    };
    let pages = [
        ("/MediaBox[0 0 400 500]", line(400, PUBLIC)),
        ("/MediaBox[50 50 450 550]/Rotate 90", {
            // Drawn in the shifted box: 60 + 50, 400 + 50.
            format!("BT /F1 14 Tf 110 450 Td (NIK: {SECRET}) Tj 0 -40 Td ({PUBLIC}) Tj ET")
        }),
        (
            "/MediaBox[0 0 400 500]",
            format!(
                "{} BT 3 Tr /F1 14 Tf 60 300 Td ({SECRET}) Tj ET",
                line(400, PUBLIC)
            ),
        ),
    ];
    let first = 3usize;
    let font = first + 2 * pages.len();
    let mut objects: Vec<String> = vec!["<</Type/Catalog/Pages 2 0 R>>".into(), String::new()];
    let mut kids = Vec::new();
    for (i, (boxes, content)) in pages.iter().enumerate() {
        let page = first + 2 * i;
        kids.push(format!("{page} 0 R"));
        objects.push(format!(
            "<</Type/Page/Parent 2 0 R{boxes}/Resources<</Font<</F1 {font} 0 R>>>>/Contents {} 0 R>>",
            page + 1
        ));
        objects.push(format!(
            "<</Length {}>>stream\n{content}\nendstream",
            content.len()
        ));
    }
    objects.push("<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>".into());
    objects[1] = format!(
        "<</Type/Pages/Kids[{}]/Count {}>>",
        kids.join(" "),
        pages.len()
    );
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offs.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let x = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for o in offs {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{x}\n%%EOF\n",
            objects.len() + 1
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

/// The display-space box around the first run of `needle` on a page, as a
/// selection of that text would produce.
fn boxes_of(text: &str, chars: &[izul_ipc::message::CharBoxWire], needle: &str) -> Vec<PdfRectF> {
    let flat: Vec<char> = chars.iter().map(|c| c.unicode).collect();
    let n: Vec<char> = needle.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + n.len() <= flat.len() {
        if flat[i..i + n.len()] == n[..] {
            let r = chars[i..i + n.len()]
                .iter()
                .map(|c| c.rect)
                .reduce(|a, b| a.union(&b))
                .unwrap();
            out.push(r);
            i += n.len();
        } else {
            i += 1;
        }
    }
    assert!(!out.is_empty(), "'{needle}' tidak ditemukan di: {text}");
    out
}

fn mark(page: u32, quads: Vec<PdfRectF>) -> AnnotObject {
    let mut obj = AnnotObject::new(
        AnnotId(0),
        page,
        AnnotKind::Redact,
        PdfRectF::new(0.0, 0.0, 0.0, 0.0),
        AnnotPayload::Markup {
            quads,
            color: Rgba::BLACK,
        },
    );
    obj.recompute_rect();
    obj
}

/// Every stream in a file, decoded, as one string: what a tool that dumps a
/// PDF's insides would see.
fn every_stream(bytes: &[u8]) -> String {
    let pdf = izul_redact::file::Pdf::parse(bytes).expect("berkas terbaca");
    let mut out = String::new();
    for num in pdf.reachable().unwrap() {
        if let Ok(izul_redact::file::Stored::Stream(s)) = pdf.get(num) {
            if let Ok(d) = izul_redact::filters::decode_plain(Some(&pdf), &s.dict, &s.data) {
                out.push_str(&String::from_utf8_lossy(&d));
            }
        }
    }
    out.push_str(&String::from_utf8_lossy(bytes));
    out
}

#[tokio::test]
async fn marked_text_is_gone_from_the_file_and_nothing_else_is() {
    let Some(live) = Live::start("apply").await else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let path = live.dir.join("data-pribadi.pdf");
    std::fs::write(&path, document()).unwrap();
    assert!(every_stream(&std::fs::read(&path).unwrap()).contains(SECRET));
    let sizes = live.open(1, &path).await;
    assert_eq!(sizes[1], (500.0, 400.0), "halaman 2 berputar sendiri");

    let state = AnnotState::new();
    state.set_own_pages(1, sizes.clone());
    for p in 0..3 {
        saving::import_page(&live.pool, &state, 1, p).await.unwrap();
    }
    // Marks over every occurrence of the secret, from the text as selected.
    for page in 0..3 {
        let (text, chars) = live.text(1, page).await;
        state
            .add(1, mark(page, boxes_of(&text, &chars, SECRET)))
            .unwrap();
    }
    // A note of ours on the secret line goes with it; a box elsewhere stays.
    let (text, chars) = live.text(1, 0).await;
    let line = boxes_of(&text, &chars, SECRET)[0];
    let mut note = AnnotObject::new(
        AnnotId(0),
        0,
        AnnotKind::Note,
        PdfRectF::new(line.left, line.bottom, line.left + 20.0, line.top),
        AnnotPayload::Note {
            icon: NoteIcon::Comment,
            color: Rgba::BLACK,
            text: format!("nomor lengkap {SECRET}"),
        },
    );
    note.author_note = "catatan di atas NIK".into();
    state.add(1, note).unwrap();
    let mut kept = AnnotObject::new(
        AnnotId(0),
        0,
        AnnotKind::Rect,
        PdfRectF::new(20.0, 20.0, 80.0, 60.0),
        AnnotPayload::Shape {
            style: ShapeStyle::default(),
        },
    );
    kept.author_note = "kotak biasa".into();
    state.add(1, kept).unwrap();

    let redaction = state.redaction(1).expect("ada tanda");
    assert_eq!(redaction.pages.len(), 3);
    assert_eq!(
        redaction.doomed.len(),
        4,
        "tiga tanda dan catatan di atas NIK"
    );

    let out = live.dir.join("data-pribadi (diredaksi).pdf");
    let report = saving::save(
        &live.pool,
        &state,
        1,
        &path.to_string_lossy(),
        3,
        Some(&out.to_string_lossy()),
        Some(&saving::Rewrite {
            redaction: Some(redaction.clone()),
            ocr: None,
        }),
    )
    .await
    .unwrap();
    let summary = report.redaction.expect("ringkasan redaksi");
    assert_eq!(summary.pages, 3);
    // 16 digits on each of three pages, plus the invisible copy on page 3.
    assert!(summary.glyphs >= 16 * 4, "{summary:?}");

    // What a reader extracts.
    live.open(2, &out).await;
    for page in 0..3 {
        let (text, _) = live.text(2, page).await;
        assert!(!text.contains(SECRET), "halaman {}: {text}", page + 1);
        assert!(!text.contains("3201"), "halaman {}: {text}", page + 1);
        assert!(text.contains(PUBLIC), "halaman {}: {text}", page + 1);
        assert!(
            text.contains("NIK:"),
            "halaman {}: label tetap ada: {text}",
            page + 1
        );
    }
    // What anyone reading the file's insides finds.
    let bytes = std::fs::read(&out).unwrap();
    let inside = every_stream(&bytes);
    assert!(!inside.contains(SECRET), "rahasia masih di dalam berkas");
    assert!(
        !inside.contains("nomor lengkap"),
        "isi catatan masih di dalam berkas"
    );
    assert!(inside.contains(PUBLIC));

    // The editor: marks and the note are gone, the box stays, no undo back
    // into a state the file no longer has.
    let left: Vec<String> = state
        .objects(1, None)
        .into_iter()
        .map(|o| o.author_note)
        .collect();
    assert_eq!(left, vec!["kotak biasa".to_string()]);
    assert_eq!(state.history(1), (false, false));
    assert!(!state.is_dirty(1));

    // The original is untouched: redaction was saved as a new file.
    assert!(every_stream(&std::fs::read(&path).unwrap()).contains(SECRET));
    live.stop().await;
}

/// A redaction the worker cannot verify is refused, and nothing is written.
#[tokio::test]
async fn a_redaction_that_cannot_be_done_writes_nothing() {
    let Some(live) = Live::start("refuse").await else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    // A simple font with neither /Widths nor a standard name: the redactor
    // cannot know how wide its glyphs are, so it must refuse rather than
    // guess and move the rest of the line.
    let src = String::from_utf8(document())
        .unwrap()
        .replace("/BaseFont/Helvetica", "/BaseFont/Unknown");
    let path = live.dir.join("tanpa-lebar.pdf");
    std::fs::write(&path, src.as_bytes()).unwrap();
    let sizes = live.open(1, &path).await;
    let state = AnnotState::new();
    state.set_own_pages(1, sizes);
    saving::import_page(&live.pool, &state, 1, 0).await.unwrap();
    let (text, chars) = live.text(1, 0).await;
    state
        .add(1, mark(0, boxes_of(&text, &chars, SECRET)))
        .unwrap();
    let redaction = state.redaction(1).unwrap();
    let out = live.dir.join("hasil.pdf");
    let err = saving::save(
        &live.pool,
        &state,
        1,
        &path.to_string_lossy(),
        3,
        Some(&out.to_string_lossy()),
        Some(&saving::Rewrite {
            redaction: Some(redaction.clone()),
            ocr: None,
        }),
    )
    .await
    .expect_err("harus ditolak");
    assert!(err.contains("Redaksi dibatalkan"), "{err}");
    assert!(err.contains("Unknown"), "pesan menyebut fontnya: {err}");
    assert!(!out.exists(), "tidak ada berkas yang ditulis");
    assert_eq!(
        state.objects(1, None).len(),
        1,
        "tanda tetap ada untuk dicoba lagi"
    );
    live.stop().await;
}
