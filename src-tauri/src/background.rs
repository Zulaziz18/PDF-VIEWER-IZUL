//! "Hapus Latar" for a picture annotation (Phase 7).
//!
//! The picture's bytes go to a worker in pieces (a frame holds 8 MB), the
//! worker runs the segmentation model (`izul_ocr::background`, ONNX Runtime
//! with DirectML on Windows) and hands back a PNG with alpha, which becomes a
//! new image of the document. The annotation then points at it — one undo
//! step, so a result the user does not like is one Ctrl+Z away, and the
//! original picture is still in the registry for that step to return to.

use std::path::PathBuf;
use std::sync::Arc;

use izul_ipc::message::{BackgroundKind, Request, Response};
use izul_model::annot::AnnotPayload;
use izul_model::display::ImageRef;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::annots::{AnnotState, EditResult};
use crate::commands::AppState;
use crate::saving::{ask, fetch_blob, worker_of};
use crate::supervisor::Pool;

type Out<T> = Result<T, String>;

/// Bytes per upload frame: well under the 8 MB frame limit.
const CHUNK: usize = 4 * 1024 * 1024;

/// ONNX Runtime and the model, where `src-tauri/build.rs` puts them beside
/// the executable (and the installer will).
pub fn runtime_paths() -> Option<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let runtime = dir.join(if cfg!(windows) {
        "onnxruntime.dll"
    } else {
        "libonnxruntime.so"
    });
    let model = dir.join("onnx").join("u2netp.onnx");
    (runtime.is_file() && model.is_file()).then_some((runtime, model))
}

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundResult {
    pub edit: EditResult,
    /// Where the model ran: "DirectML", or "CPU" with the reason.
    pub device: String,
    /// The paper went too (a signature or stamp on a plain background).
    pub paper: bool,
}

/// Replaces the picture of annotation `id` with its background removed.
pub async fn remove_background(
    pool: &Arc<RwLock<Pool>>,
    annots: &AnnotState,
    doc: u64,
    id: u64,
    kind: BackgroundKind,
    (runtime, model): (PathBuf, PathBuf),
) -> Out<BackgroundResult> {
    let mut obj = annots
        .objects(doc, None)
        .into_iter()
        .find(|o| o.id.0 == id)
        .ok_or_else(|| "objek tidak ditemukan".to_string())?;
    let AnnotPayload::Image { image, .. } = obj.payload else {
        return Err("Hapus latar hanya untuk gambar.".into());
    };
    let stored = annots
        .image(doc, image.0)
        .ok_or_else(|| "gambar tidak terdaftar".to_string())?;
    let worker = worker_of(pool, doc).await?;
    let mut blob = 0;
    for piece in stored.bytes.chunks(CHUNK) {
        match ask(
            &worker,
            Request::BlobAppend {
                blob,
                data: piece.to_vec(),
            },
        )
        .await?
        {
            Response::BlobAppended { blob: b, .. } => blob = b,
            other => return Err(format!("balasan tak terduga: {other:?}")),
        }
    }
    let reply = ask(
        &worker,
        Request::RemoveBackground {
            blob,
            runtime: runtime.to_string_lossy().to_string(),
            model: model.to_string_lossy().to_string(),
            kind,
        },
    )
    .await
    .map_err(|e| {
        let detail = e
            .strip_prefix("background.failed: ")
            .unwrap_or(&e)
            .to_string();
        format!("Latar tidak bisa dihapus: {detail}")
    })?;
    let Response::BackgroundRemoved {
        blob,
        len,
        device,
        paper,
    } = reply
    else {
        return Err(format!("balasan tak terduga: {reply:?}"));
    };
    let png = fetch_blob(&worker, blob, len).await?;
    let handle = annots.add_image(doc, png)?;
    if let AnnotPayload::Image { image, .. } = &mut obj.payload {
        *image = ImageRef(handle);
    }
    let edit = annots.replace(doc, vec![obj]).map_err(|e| e.to_string())?;
    tracing::info!(doc, id, %device, paper, "latar dihapus");
    Ok(BackgroundResult {
        edit,
        device,
        paper,
    })
}

/// Whether the runtime and model are installed.
#[tauri::command]
pub fn background_available() -> bool {
    runtime_paths().is_some()
}

#[tauri::command]
pub async fn annot_remove_background(
    state: tauri::State<'_, AppState>,
    doc: u64,
    id: u64,
    kind: BackgroundKind,
) -> Out<BackgroundResult> {
    let paths = runtime_paths().ok_or_else(|| {
        "ONNX Runtime atau model hapus latar belum terpasang. Jalankan vendor/onnx/fetch.sh lalu \
         bangun ulang aplikasi."
            .to_string()
    })?;
    remove_background(&state.pool, &state.annots, doc, id, kind, paths).await
}
