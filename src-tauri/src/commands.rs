//! Tauri commands: the frontend's only way into the backend.
//!
//! These carry structure, never pixels. Bitmaps go through the `izul://`
//! protocol (SPEC 4's hard rule against piping bitmaps through `invoke`), which
//! is also why there is no `render_tile` command here: a tile is fetched by its
//! URI, and the backend decides whether that means a cache read or a render.

use std::path::PathBuf;
use std::sync::Arc;

use izul_ipc::message::{DocId, OutlineEntry, Request, Response};
use izul_model::geom::RotationQuarter;
use izul_store::files::{self, ReadingState, ViewMode};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::render::{DocInfo, RenderService, RenderStats};
use crate::supervisor::Pool;
use crate::version::{self, VersionInfo};

/// Everything the UI process keeps for the life of the run.
pub struct AppState {
    pub pool: Arc<RwLock<Pool>>,
    pub render: Arc<RenderService>,
    pub data_dir: PathBuf,
    pub next_doc_id: std::sync::atomic::AtomicU64,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

impl AppState {
    fn db(&self) -> Result<rusqlite::Connection, String> {
        izul_store::open(&self.data_dir, izul_store::Which::App)
            .map_err(|e| format!("basis data: {e}"))
    }
}

/// Where the user was in a document the last time they closed it (SPEC 11.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedView {
    pub page: u32,
    pub scroll_y: f64,
    pub zoom: f64,
    pub rotation: u8,
    pub view_mode: String,
}

impl From<ReadingState> for SavedView {
    fn from(s: ReadingState) -> Self {
        SavedView {
            page: s.page,
            scroll_y: s.scroll_y,
            zoom: s.zoom,
            rotation: s.rotation,
            view_mode: s.view_mode.as_str().to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct OpenedDoc {
    pub doc: u64,
    pub path: String,
    pub page_count: u32,
    /// Width and height in PDF points, per page, with the page's own `/Rotate`
    /// applied. The viewport needs all of them before it can size the scroll
    /// bar, and the Phase 0 spike showed this costs about 6 ms for 500 pages
    /// when read from the page tree.
    pub page_sizes: Vec<(f32, f32)>,
    pub permissions: u32,
    pub encrypted: bool,
    /// Absent for a document that has never been opened here before.
    pub restored: Option<SavedView>,
}

#[derive(Debug, Serialize)]
pub struct RecentFile {
    pub path: String,
    pub name: String,
    pub last_opened: Option<i64>,
    pub pinned: bool,
    /// False when the file is no longer where it was (SPEC 11.3's "detects a
    /// file that moved"), so the list can show it as unavailable instead of
    /// failing when it is clicked.
    pub available: bool,
}

#[derive(Debug, Serialize)]
pub struct HealthReport {
    pub pool_size: usize,
    pub live_workers: usize,
    pub quarantined_documents: usize,
    pub sandbox_is_security_boundary: bool,
    pub render: RenderStats,
}

#[derive(Debug, Serialize)]
pub struct CharBoxOut {
    pub c: String,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

#[derive(Debug, Serialize)]
pub struct PageTextOut {
    pub page: u32,
    pub text: String,
    pub chars: Vec<CharBoxOut>,
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
    let (pool_size, live_workers, quarantined_documents) = {
        let pool = state.pool.read().await;
        (pool.size(), pool.live_workers(), pool.quarantined_count())
    };
    Ok(HealthReport {
        pool_size,
        live_workers,
        quarantined_documents,
        sandbox_is_security_boundary: cfg!(windows),
        render: state.render.stats(),
    })
}

#[tauri::command]
pub async fn open_document(
    state: tauri::State<'_, AppState>,
    path: String,
) -> CmdResult<OpenedDoc> {
    let file = PathBuf::from(&path);
    let stamp =
        izul_store::FileStamp::of(&file).map_err(|e| format!("tidak dapat membaca {path}: {e}"))?;
    // Hashing the bytes identifies the *content*, which is what the poison list
    // keys on: the same broken file under two names must not get two chances to
    // take a worker down.
    let bytes = std::fs::read(&file).map_err(|e| format!("tidak dapat membaca {path}: {e}"))?;
    let key = izul_store::content_hash(&bytes);
    drop(bytes);

    let doc = DocId(
        state
            .next_doc_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    );
    let opened = {
        let mut pool = state.pool.write().await;
        pool.open(doc, &path, key, None).await
    };
    let (page_count, page_sizes, permissions, encrypted) = match opened {
        Ok(Response::Opened {
            page_count,
            page_sizes,
            permissions,
            encrypted,
            ..
        }) => (page_count, page_sizes, permissions, encrypted),
        Ok(Response::Error {
            message_id, detail, ..
        }) => return Err(format!("{message_id}: {detail}")),
        Ok(other) => return Err(format!("balasan tak terduga: {other:?}")),
        Err(e) => return Err(e.to_string()),
    };

    state.render.register(
        doc.0,
        DocInfo {
            path: path.clone(),
            page_sizes: page_sizes.clone(),
            generation: 0,
        },
    );

    // Recording the open and reading back the last position are one step: a
    // document the user has seen before must come back where they left it
    // (SPEC 11.1), and the row that stores that is the same one that makes it
    // recent.
    let restored = match state.db() {
        Ok(conn) => match files::touch(&conn, &file, stamp) {
            Ok(id) => files::reading_state(&conn, id)
                .ok()
                .flatten()
                .map(SavedView::from),
            Err(e) => {
                tracing::warn!(error = %e, "riwayat berkas tidak dapat ditulis");
                None
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "basis data tidak tersedia");
            None
        }
    };

    Ok(OpenedDoc {
        doc: doc.0,
        path,
        page_count,
        page_sizes,
        permissions,
        encrypted,
        restored,
    })
}

#[tauri::command]
pub async fn close_document(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<()> {
    let worker = { state.pool.read().await.worker_for(DocId(doc)) };
    if let Some(worker) = worker {
        let _ = Pool::ask(&worker, Request::Close { doc: DocId(doc) }).await;
    }
    state.render.forget(doc);
    Ok(())
}

/// Releases a document's full-resolution bitmaps without closing it (SPEC 10).
///
/// What an inactive tab costs. Previews, position and undo stack survive; the
/// megabytes do not.
#[tauri::command]
pub async fn trim_document(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<usize> {
    let dropped = state.render.trim(doc);
    let worker = { state.pool.read().await.worker_for(DocId(doc)) };
    if let Some(worker) = worker {
        let _ = Pool::ask(&worker, Request::Trim { doc: DocId(doc) }).await;
    }
    Ok(dropped)
}

/// Records a new layout epoch, cancelling everything older (SPEC 6).
#[tauri::command]
pub async fn set_generation(
    state: tauri::State<'_, AppState>,
    doc: u64,
    generation: u64,
) -> CmdResult<()> {
    state.render.bump_generation(doc, generation);
    Ok(())
}

#[tauri::command]
pub async fn document_outline(
    state: tauri::State<'_, AppState>,
    doc: u64,
) -> CmdResult<Vec<OutlineEntry>> {
    let worker = { state.pool.read().await.worker_for(DocId(doc)) };
    let worker = worker.ok_or_else(|| "dokumen tidak terbuka".to_string())?;
    match Pool::ask(&worker, Request::Outline { doc: DocId(doc) }).await {
        Ok(Response::OutlineReady { nodes, .. }) => Ok(nodes),
        Ok(Response::Error {
            message_id, detail, ..
        }) => Err(format!("{message_id}: {detail}")),
        Ok(other) => Err(format!("balasan tak terduga: {other:?}")),
        Err(e) => Err(e.to_string()),
    }
}

/// Text of one page, with per-character boxes for the selection layer.
#[tauri::command]
pub async fn page_text(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: u32,
    with_boxes: bool,
    rotation: u8,
) -> CmdResult<PageTextOut> {
    let worker = { state.pool.read().await.worker_for(DocId(doc)) };
    let worker = worker.ok_or_else(|| "dokumen tidak terbuka".to_string())?;
    let req = Request::ExtractText {
        doc: DocId(doc),
        page,
        with_boxes,
        rotation: RotationQuarter::from_degrees(i32::from(rotation) * 90),
    };
    match Pool::ask(&worker, req).await {
        Ok(Response::TextReady {
            page, text, chars, ..
        }) => Ok(PageTextOut {
            page,
            text,
            chars: chars
                .into_iter()
                .map(|c| CharBoxOut {
                    c: c.unicode.to_string(),
                    left: c.rect.left,
                    bottom: c.rect.bottom,
                    right: c.rect.right,
                    top: c.rect.top,
                })
                .collect(),
        }),
        Ok(Response::Error {
            message_id, detail, ..
        }) => Err(format!("{message_id}: {detail}")),
        Ok(other) => Err(format!("balasan tak terduga: {other:?}")),
        Err(e) => Err(e.to_string()),
    }
}

/// Stores where the user is, so the document reopens there (SPEC 11.1).
#[tauri::command]
pub async fn save_view_state(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: u32,
    scroll_y: f64,
    zoom: f64,
    rotation: u8,
    view_mode: String,
) -> CmdResult<()> {
    let Some(path) = state.render.path_of(doc) else {
        return Ok(());
    };
    let file = PathBuf::from(&path);
    let conn = state.db()?;
    let stamp = izul_store::FileStamp::of(&file).map_err(|e| format!("{path}: {e}"))?;
    let id = files::touch(&conn, &file, stamp).map_err(|e| format!("basis data: {e}"))?;
    files::save_reading_state(
        &conn,
        id,
        ReadingState {
            page,
            scroll_y,
            zoom,
            rotation: rotation.min(3),
            view_mode: ViewMode::parse(&view_mode),
        },
    )
    .map_err(|e| format!("basis data: {e}"))
}

/// Display size of a page in points, after rotation.
///
/// The viewport keeps its own copy from `open_document`; this exists for the
/// benchmark harness and for anything that needs one page rather than all of
/// them.
#[tauri::command]
pub async fn page_size(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: u32,
    rotation: u8,
) -> CmdResult<(f32, f32)> {
    state
        .render
        .display_size(
            doc,
            page,
            RotationQuarter::from_degrees(i32::from(rotation) * 90),
        )
        .ok_or_else(|| "halaman tidak dikenal".to_string())
}

#[tauri::command]
pub async fn render_stats(state: tauri::State<'_, AppState>) -> CmdResult<RenderStats> {
    Ok(state.render.stats())
}

#[tauri::command]
pub async fn recent_files(state: tauri::State<'_, AppState>) -> CmdResult<Vec<RecentFile>> {
    let conn = state.db()?;
    let rows = files::recent(&conn, 20).map_err(|e| format!("basis data: {e}"))?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let path = PathBuf::from(&r.path);
            RecentFile {
                name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| r.path.clone()),
                available: path.is_file(),
                path: r.path,
                last_opened: r.last_opened,
                pinned: r.pinned,
            }
        })
        .collect())
}
