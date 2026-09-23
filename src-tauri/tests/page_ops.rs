//! Page operations end to end (Phase 5): the page map, applied at save by a
//! real worker, read back by a real worker.
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

use izul_app::annots::{AnnotState, SourceFile};
use izul_app::saving;
use izul_app::supervisor::worker::WorkerPaths;
use izul_app::supervisor::Pool;
use izul_ipc::message::{DocId, Request, Response};
use izul_model::annot::{AnnotId, AnnotKind, AnnotObject, AnnotPayload, ShapeStyle};
use izul_model::geom::{PdfRectF, RotationQuarter};
use izul_model::pages::{PageCommand, PageEntry};
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

/// `n` pages saying "`label` 0", "`label` 1", …, with a bookmark on page 1.
fn numbered(label: &str, n: usize) -> Vec<u8> {
    let first = 5usize;
    let font = first + 2 * n;
    let mut objects: Vec<String> = vec![
        "<</Type/Catalog/Pages 2 0 R/Outlines 3 0 R>>".into(),
        String::new(),
        "<</Type/Outlines/First 4 0 R/Last 4 0 R/Count 1>>".into(),
        format!("<</Title(Bab)/Parent 3 0 R/Dest[{} 0 R/Fit]>>", first + 2),
    ];
    let mut kids = Vec::new();
    for i in 0..n {
        let page = first + 2 * i;
        kids.push(format!("{page} 0 R"));
        objects.push(format!(
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 400]/Resources<</Font<</F1 {font} 0 R>>>>/Contents {} 0 R>>",
            page + 1
        ));
        let text = format!("BT /F1 20 Tf 30 300 Td ({label} {i}) Tj ET");
        objects.push(format!(
            "<</Length {}>>stream\n{text}\nendstream",
            text.len()
        ));
    }
    objects.push("<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".into());
    objects[1] = format!("<</Type/Pages/Kids[{}]/Count {n}>>", kids.join(" "));
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
        let dir = std::env::temp_dir().join(format!("izul-pages-{name}-{}", std::process::id()));
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

    async fn ask(&self, doc: u64, req: Request) -> Response {
        let worker = self.pool.read().await.worker_for(DocId(doc)).unwrap();
        Pool::ask(&worker, req).await.unwrap()
    }

    async fn texts(&self, doc: u64, pages: u32) -> Vec<String> {
        let mut out = Vec::new();
        for page in 0..pages {
            match self
                .ask(
                    doc,
                    Request::ExtractText {
                        doc: DocId(doc),
                        page,
                        with_boxes: false,
                        rotation: RotationQuarter::None,
                    },
                )
                .await
            {
                Response::TextReady { text, .. } => out.push(text.trim().to_string()),
                other => panic!("{other:?}"),
            }
        }
        out
    }

    async fn outline(&self, doc: u64) -> Vec<(String, Option<u32>)> {
        match self.ask(doc, Request::Outline { doc: DocId(doc) }).await {
            Response::OutlineReady { nodes, .. } => {
                nodes.into_iter().map(|e| (e.title, e.page)).collect()
            }
            other => panic!("{other:?}"),
        }
    }

    async fn stop(self) {
        self.pool.write().await.shutdown().await;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn rect(page: u32, note: &str) -> AnnotObject {
    let mut obj = AnnotObject::new(
        AnnotId(0),
        page,
        AnnotKind::Rect,
        PdfRectF::new(20.0, 20.0, 120.0, 80.0),
        AnnotPayload::Shape {
            style: ShapeStyle::default(),
        },
    );
    obj.author_note = note.into();
    obj
}

/// Reads every page of `doc` into a fresh editor, as the page commands do
/// before their first operation.
async fn import_all(live: &Live, state: &AnnotState, doc: u64, sizes: Vec<(f32, f32)>) {
    let n = sizes.len() as u32;
    state.set_own_pages(doc, sizes);
    for p in 0..n {
        saving::import_page(&live.pool, state, doc, p)
            .await
            .unwrap();
    }
    state.mark_all_imported(doc);
}

#[tokio::test]
async fn rearranged_pages_are_saved_in_order_with_their_annotations() {
    let Some(live) = Live::start("order").await else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let path = live.dir.join("dokumen.pdf");
    std::fs::write(&path, numbered("Halaman", 4)).unwrap();
    let other = live.dir.join("lain.pdf");
    std::fs::write(&other, numbered("Lain", 2)).unwrap();

    let sizes = live.open(1, &path).await;
    let state = AnnotState::new();
    import_all(&live, &state, 1, sizes).await;
    // A box on page 2 and one on page 3.
    state.add(1, rect(2, "di halaman 2")).unwrap();
    state.add(1, rect(3, "di halaman 3")).unwrap();

    // Page 2 to the front; delete (then) page 1 — "Halaman 0"'s neighbour —
    // i.e. the page that says "Halaman 1"; duplicate the last; a blank page
    // after the first; the other file's page 1, turned once, at the end.
    let run = |cmd| state.apply_pages(1, cmd).unwrap();
    run(PageCommand::Move {
        pages: vec![2],
        before: 0,
    }); // 2 0 1 3
    run(PageCommand::Delete { pages: vec![2] }); //           2 0 3
    run(PageCommand::Duplicate { pages: vec![2] }); //        2 0 3 3
    run(PageCommand::InsertBlank {
        at: 1,
        count: 1,
        width: 300.0,
        height: 200.0,
    }); // 2 _ 0 3 3
    let source = state.add_source(
        1,
        SourceFile {
            path: other.to_string_lossy().to_string(),
            origin: "lain.pdf".into(),
            sizes: vec![(300.0, 400.0); 2],
        },
    );
    run(PageCommand::Insert {
        at: 5,
        entries: vec![PageEntry::Page {
            source,
            page: 1,
            rotation: 1,
        }],
        objects: vec![rect(0, "dari berkas lain")],
    });
    assert_eq!(state.resolved_pages(1).len(), 6);

    let report = saving::save(&live.pool, &state, 1, &path.to_string_lossy(), 4, None)
        .await
        .unwrap();
    let sizes = report.restructured.clone().expect("a page map was applied");
    assert_eq!(sizes.len(), 6);
    // The foreign page was turned a quarter: its sizes come back swapped.
    assert_eq!(sizes[5], (400.0, 300.0));
    assert_eq!(sizes[1], (300.0, 200.0), "the blank page");
    assert!(state.page_map(1).is_none(), "the map is spent once written");
    assert!(!state.is_dirty(1));

    // What a reader of the file sees, from a fresh open.
    live.open(2, &path).await;
    assert_eq!(
        live.texts(2, 6).await,
        vec![
            "Halaman 2",
            "",
            "Halaman 0",
            "Halaman 3",
            "Halaman 3",
            "Lain 1"
        ]
    );
    // The bookmark pointed at "Halaman 1", which was deleted: PDFium keeps
    // the entry but it no longer resolves to a page. The outline itself
    // survives the rearrangement, which rebuilding the document would lose.
    let outline = live.outline(2).await;
    assert_eq!(outline.len(), 1);
    assert_eq!(outline[0].0, "Bab");

    // Every annotation is on the page it was drawn on, copies included.
    let back = AnnotState::new();
    import_all(&live, &back, 2, vec![(0.0, 0.0); 6]).await;
    let mut found: Vec<(u32, String)> = back
        .objects(2, None)
        .into_iter()
        .map(|o| (o.page, o.author_note))
        .collect();
    found.sort();
    assert_eq!(
        found,
        vec![
            (0, "di halaman 2".to_string()),
            (3, "di halaman 3".to_string()),
            (4, "di halaman 3".to_string()),
            (5, "dari berkas lain".to_string()),
        ]
    );
    live.stop().await;
}

/// Saving twice must not rearrange twice: after the first save the file
/// *is* the arrangement, and the second save writes annotations only.
#[tokio::test]
async fn a_second_save_after_a_rearrangement_changes_nothing_further() {
    let Some(live) = Live::start("twice").await else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let path = live.dir.join("dokumen.pdf");
    std::fs::write(&path, numbered("Halaman", 3)).unwrap();
    let sizes = live.open(1, &path).await;
    let state = AnnotState::new();
    import_all(&live, &state, 1, sizes).await;
    state
        .apply_pages(
            1,
            PageCommand::Move {
                pages: vec![2],
                before: 0,
            },
        )
        .unwrap();
    let first = saving::save(&live.pool, &state, 1, &path.to_string_lossy(), 3, None)
        .await
        .unwrap();
    assert!(first.restructured.is_some());
    state.add(1, rect(0, "sesudah")).unwrap();
    let second = saving::save(&live.pool, &state, 1, &path.to_string_lossy(), 3, None)
        .await
        .unwrap();
    assert!(second.restructured.is_none());
    live.open(2, &path).await;
    assert_eq!(
        live.texts(2, 3).await,
        vec!["Halaman 2", "Halaman 0", "Halaman 1"]
    );
    live.stop().await;
}
