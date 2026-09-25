//! Form filling (Phase 7) through a real worker: the field list, a value set
//! as one undo step and shown in the open document, undo and redo, and a save
//! that puts the values into the file where a reopened document reads them.
//! That other readers draw them is `tools/form-proof`'s job.
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
use izul_app::forms::{field_list, set_field};
use izul_app::saving;
use izul_app::supervisor::worker::WorkerPaths;
use izul_app::supervisor::Pool;
use izul_ipc::message::{DocId, Response};
use izul_model::FormValue;
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

/// A text field and a checkbox whose "on" state is /Ya.
fn form_pdf() -> Vec<u8> {
    let on = "0 g 3 3 10 10 re f";
    let objs = [
        "<</Type/Catalog/Pages 2 0 R/AcroForm 6 0 R>>".to_string(),
        "<</Type/Pages/Kids[3 0 R]/Count 1>>".to_string(),
        "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Annots[4 0 R 5 0 R]>>".to_string(),
        "<</Type/Annot/Subtype/Widget/FT/Tx/T(nama)/V()/Rect[100 700 300 720]/DA(/Helv 11 Tf 0 g)/F 4/P 3 0 R>>".to_string(),
        "<</Type/Annot/Subtype/Widget/FT/Btn/T(setuju)/V/Off/AS/Off/Rect[100 650 116 666]/AP<</N<</Ya 8 0 R/Off 9 0 R>>>>/F 4/P 3 0 R>>".to_string(),
        "<</Fields[4 0 R 5 0 R]/DA(/Helv 0 Tf 0 g)/DR<</Font<</Helv 7 0 R>>>>>>".to_string(),
        "<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>".to_string(),
        format!("<</Type/XObject/Subtype/Form/BBox[0 0 16 16]/Length {}>>\nstream\n{on}\nendstream", on.len() + 1),
        "<</Type/XObject/Subtype/Form/BBox[0 0 16 16]/Length 1>>\nstream\n\nendstream".to_string(),
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

    #[allow(dead_code)]
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

#[tokio::test]
async fn fields_are_filled_undone_saved_and_read_back() {
    let Some(live) = Live::start("form").await else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let path = live.dir.join("formulir.pdf");
    std::fs::write(&path, form_pdf()).unwrap();
    let pages = live.open(1, &path).await;
    let state = AnnotState::new();
    saving::import_page(&live.pool, &state, 1, 0).await.unwrap();

    let fields = field_list(&live.pool, &state, 1).await.unwrap();
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["nama", "setuju"]);
    assert_eq!(fields[0].value, FormValue::Text(String::new()));
    assert_eq!(fields[1].value, FormValue::Checked(false));

    let r = set_field(
        &live.pool,
        &state,
        None,
        1,
        "nama".into(),
        FormValue::Text("Siti Rahayu".into()),
    )
    .await
    .unwrap();
    assert_eq!(r.repaint, [0], "halaman isian digambar ulang");
    assert!(r.dirty);
    set_field(
        &live.pool,
        &state,
        None,
        1,
        "setuju".into(),
        FormValue::Checked(true),
    )
    .await
    .unwrap();
    // The open document itself shows the values already.
    let now = field_list(&live.pool, &state, 1).await.unwrap();
    assert_eq!(now[0].value, FormValue::Text("Siti Rahayu".into()));
    assert_eq!(now[1].value, FormValue::Checked(true));
    // A kind of value the field does not take is refused.
    assert!(set_field(
        &live.pool,
        &state,
        None,
        1,
        "setuju".into(),
        FormValue::Text("x".into())
    )
    .await
    .is_err());

    // Undo the tick, then redo it: one step each.
    let undone = state.undo(1, None).unwrap();
    assert_eq!(
        undone.form_changes,
        [("setuju".to_string(), FormValue::Checked(false))]
    );
    let redone = state.redo(1, None).unwrap();
    assert_eq!(
        redone.form_changes,
        [("setuju".to_string(), FormValue::Checked(true))]
    );

    saving::save(
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
    let saved = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_string();
    assert!(
        saved.contains("/AS/Ya") || saved.contains("/AS /Ya"),
        "keadaan 'on' aslinya dipakai"
    );

    // A fresh session reads the values from the file.
    live.pool.write().await.release(DocId(1)).await.unwrap();
    live.open(2, &path).await;
    let fresh = AnnotState::new();
    let back = field_list(&live.pool, &fresh, 2).await.unwrap();
    assert_eq!(back[0].value, FormValue::Text("Siti Rahayu".into()));
    assert_eq!(back[1].value, FormValue::Checked(true));
    live.stop().await;
}
