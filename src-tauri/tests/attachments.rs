//! Attachments (Phase 8) through a real worker: the list, and an attachment
//! larger than one blob chunk read back byte for byte — the path the
//! "Lampiran" panel's "Simpan" takes.
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

use izul_app::saving::fetch_blob;
use izul_app::supervisor::worker::WorkerPaths;
use izul_app::supervisor::Pool;
use izul_ipc::message::{DocId, Request, Response, BLOB_CHUNK};

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

/// One page and two embedded files, with a correct cross-reference table.
fn pdf_with(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut objs: Vec<Vec<u8>> = Vec::new();
    let names: String = (0..files.len())
        .map(|i| format!("({}) {} 0 R ", files[i].0, 4 + i * 2))
        .collect();
    objs.push(
        format!("<</Type/Catalog/Pages 2 0 R/Names<</EmbeddedFiles<</Names[{names}]>>>>>>")
            .into_bytes(),
    );
    objs.push(b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec());
    objs.push(b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>".to_vec());
    for (i, (name, data)) in files.iter().enumerate() {
        objs.push(
            format!(
                "<</Type/Filespec/F({name})/UF({name})/EF<</F {} 0 R>>>>",
                5 + i * 2
            )
            .into_bytes(),
        );
        let mut s = format!("<</Type/EmbeddedFile/Length {}>>\nstream\n", data.len()).into_bytes();
        s.extend_from_slice(data);
        s.extend_from_slice(b"\nendstream");
        objs.push(s);
    }
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offs.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
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
async fn attachments_are_listed_and_a_large_one_comes_back_whole() {
    let Some(paths) = worker_paths() else {
        return skip("izul-worker atau PDFium belum dibangun");
    };
    let mut pool = Pool::start(paths).await.unwrap();
    // More than two blob chunks, and not a pattern PDFium could shortcut.
    let big: Vec<u8> = (0..(BLOB_CHUNK as usize * 2 + 12_345))
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    let dir = std::env::temp_dir().join(format!("izul-attach-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lampiran.pdf");
    std::fs::write(
        &path,
        pdf_with(&[("kecil.txt", b"halo"), ("besar.bin", &big)]),
    )
    .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let doc = DocId(1);
    pool.open(
        doc,
        &path.to_string_lossy(),
        izul_store::content_hash(&bytes),
        None,
    )
    .await
    .unwrap();
    let worker = pool.worker_for(doc).unwrap();

    match Pool::ask(&worker, Request::Attachments { doc })
        .await
        .unwrap()
    {
        Response::AttachmentsReady { items, .. } => assert_eq!(
            items,
            vec![
                ("kecil.txt".to_string(), 4),
                ("besar.bin".to_string(), big.len() as u64)
            ]
        ),
        other => panic!("{other:?}"),
    }
    let back = match Pool::ask(&worker, Request::AttachmentData { doc, index: 1 })
        .await
        .unwrap()
    {
        Response::BlobReady { blob, len } => fetch_blob(&worker, blob, len).await.unwrap(),
        other => panic!("{other:?}"),
    };
    assert!(
        back == big,
        "isi lampiran berubah di jalan ({} vs {} byte)",
        back.len(),
        big.len()
    );
    match Pool::ask(&worker, Request::AttachmentData { doc, index: 9 })
        .await
        .unwrap()
    {
        Response::Error { .. } => {}
        other => panic!("indeks di luar daftar harus ditolak: {other:?}"),
    }

    pool.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}
