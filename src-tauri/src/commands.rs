//! Tauri commands: the frontend's only way into the backend.
//!
//! These carry structure, never pixels. Bitmaps go through the `izul://`
//! protocol (SPEC 4's hard rule against piping bitmaps through `invoke`).

use std::sync::Arc;

use izul_ipc::message::{DocId, Generation, RenderQuality, Request, Response};
use izul_model::geom::{PdfRectF, RotationQuarter};
use serde::Serialize;
use tokio::sync::RwLock;

use crate::supervisor::Pool;
use crate::version::{self, VersionInfo};

/// Everything the UI process keeps for the life of the run.
pub struct AppState {
    pub pool: Arc<RwLock<Pool>>,
    pub data_dir: std::path::PathBuf,
    pub next_doc_id: std::sync::atomic::AtomicU64,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize)]
pub struct OpenedDoc {
    pub doc: u64,
    pub page_count: u32,
    /// Width and height in PDF points, per page. The viewport needs all of them
    /// before it can size the scroll bar, and the Phase 0 spike showed this
    /// costs about 6 ms for 500 pages when read from the page tree.
    pub page_sizes: Vec<(f32, f32)>,
    pub permissions: u32,
    pub encrypted: bool,
}

#[derive(Debug, Serialize)]
pub struct TileHandle {
    pub doc: u64,
    pub page: u32,
    pub slot: u32,
    pub epoch: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// What the frontend should fetch. Built here so the URI shape lives in one
    /// place rather than being assembled by string concatenation in TypeScript.
    pub uri: String,
}

#[derive(Debug, Serialize)]
pub struct HealthReport {
    pub pool_size: usize,
    pub live_workers: usize,
    pub quarantined_documents: usize,
    pub sandbox_is_security_boundary: bool,
}

type CmdResult<T> = Result<T, String>;

#[tauri::command]
pub fn app_version() -> CmdResult<VersionInfo> {
    version::info().map_err(|e| format!("version.json rusak: {e}"))
}

#[tauri::command]
pub fn log_folder(state: tauri::State<'_, AppState>) -> CmdResult<String> {
    Ok(crate::logging::log_dir(&state.data_dir)
        .display()
        .to_string())
}

#[tauri::command]
pub async fn pool_health(state: tauri::State<'_, AppState>) -> CmdResult<HealthReport> {
    let pool = state.pool.read().await;
    Ok(HealthReport {
        pool_size: pool.size(),
        live_workers: pool.live_workers(),
        quarantined_documents: pool.quarantined_count(),
        sandbox_is_security_boundary: cfg!(windows),
    })
}

#[tauri::command]
pub async fn open_document(
    state: tauri::State<'_, AppState>,
    path: String,
) -> CmdResult<OpenedDoc> {
    let bytes = std::fs::read(&path).map_err(|e| format!("tidak dapat membaca {path}: {e}"))?;
    let key = izul_store::content_hash(&bytes);
    drop(bytes);

    let doc = DocId(
        state
            .next_doc_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    );
    let mut pool = state.pool.write().await;
    match pool.open(doc, &path, key, None).await {
        Ok(Response::Opened {
            page_count,
            page_sizes,
            permissions,
            encrypted,
            ..
        }) => Ok(OpenedDoc {
            doc: doc.0,
            page_count,
            page_sizes,
            permissions,
            encrypted,
        }),
        Ok(Response::Error {
            message_id, detail, ..
        }) => Err(format!("{message_id}: {detail}")),
        Ok(other) => Err(format!("balasan tak terduga: {other:?}")),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn close_document(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<()> {
    let pool = state.pool.read().await;
    let _ = pool
        .request(DocId(doc), Request::Close { doc: DocId(doc) })
        .await;
    Ok(())
}

/// Renders one tile and returns where to fetch its pixels.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn render_tile(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: u32,
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
    dest_w: u32,
    dest_h: u32,
    generation: u64,
    sharp: bool,
) -> CmdResult<TileHandle> {
    let doc_id = DocId(doc);
    let source = PdfRectF::new(left, bottom, right, top);
    if !source.is_valid() {
        return Err("kotak sumber tidak valid".to_string());
    }
    let req = Request::RenderTile {
        doc: doc_id,
        page,
        source,
        dest_w,
        dest_h,
        rotation: RotationQuarter::None,
        quality: if sharp {
            RenderQuality::Sharp
        } else {
            RenderQuality::Fast
        },
        generation: Generation(generation),
    };
    let pool = state.pool.read().await;
    match pool.request(doc_id, req).await {
        Ok(Response::TileReady { slot, page, .. }) => Ok(TileHandle {
            doc,
            page,
            slot: slot.slot,
            epoch: slot.epoch,
            width: slot.width,
            height: slot.height,
            stride: slot.stride,
            uri: format!("izul://tile/{doc}/{}/{}", slot.slot, slot.epoch),
        }),
        Ok(Response::Superseded { .. }) => Err("generasi kedaluwarsa".to_string()),
        Ok(Response::Error {
            message_id, detail, ..
        }) => Err(format!("{message_id}: {detail}")),
        Ok(other) => Err(format!("balasan tak terduga: {other:?}")),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn recent_files(state: tauri::State<'_, AppState>) -> CmdResult<Vec<String>> {
    let conn = izul_store::open(&state.data_dir, izul_store::Which::App)
        .map_err(|e| format!("basis data: {e}"))?;
    let rows = izul_store::files::recent(&conn, 20).map_err(|e| format!("basis data: {e}"))?;
    Ok(rows.into_iter().map(|r| r.path).collect())
}
