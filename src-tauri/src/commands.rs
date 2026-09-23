//! Tauri commands: the frontend's only way into the backend.
//!
//! These carry structure, never pixels. Bitmaps go through the `izul://`
//! protocol (SPEC 4's hard rule against piping bitmaps through `invoke`), which
//! is also why there is no `render_tile` command here: a tile is fetched by its
//! URI, and the backend decides whether that means a cache read or a render.

use std::path::PathBuf;
use std::sync::Arc;

use izul_ipc::message::{DocId, OutlineEntry, Request, Response, SearchOptions};
use izul_model::geom::RotationQuarter;
use izul_store::files::{self, FileId, ReadingState, ViewMode};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::annots::{AnnotState, EditResult};
use crate::folders;
use crate::indexing::{IndexProgress, Indexer, Job};
use crate::render::{DocInfo, RenderService, RenderStats, TileKey, TileKind};
use crate::supervisor::Pool;
use crate::textsearch::{self, HitOut, MAX_HITS_PER_PAGE};
use crate::thumbs;
use crate::version::{self, VersionInfo};
use crate::workspace::{OpenDoc, Workspace};

/// Everything the UI process keeps for the life of the run.
pub struct AppState {
    pub pool: Arc<RwLock<Pool>>,
    pub render: Arc<RenderService>,
    pub data_dir: PathBuf,
    pub next_doc_id: std::sync::atomic::AtomicU64,
    /// Which documents are open, in which order, and which one has focus.
    ///
    /// A `parking_lot::Mutex` rather than a `tokio` one: nothing held here is
    /// ever held across an await, and every reader is a command that wants the
    /// answer immediately. An async lock would add a scheduling hop to
    /// something that is a hash lookup.
    pub workspace: Mutex<Workspace>,
    pub indexer: Arc<Indexer>,
    /// Paths the process was started with — a double-clicked PDF, on Windows
    /// with the file association installed.
    pub startup_files: Vec<String>,
    /// Annotations and their undo history, per document (SPEC 8).
    pub annots: Arc<AnnotState>,
    /// Size and time of each open file as this application last saw it —
    /// at open, and after each save — so a change made by another program is
    /// noticed (SPEC 8: "deteksi berkas yang berubah di disk").
    pub stamps: Mutex<std::collections::HashMap<u64, izul_store::FileStamp>>,
    /// Worker documents that render pages brought in from other files
    /// (Phase 5, `pagemap.rs`).
    pub hidden: crate::pagemap::HiddenSources,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

impl AppState {
    pub(crate) fn db(&self) -> Result<rusqlite::Connection, String> {
        izul_store::open(&self.data_dir, izul_store::Which::App)
            .map_err(|e| format!("basis data: {e}"))
    }

    /// Writes the current tab arrangement over this run's session row.
    ///
    /// Called after every change rather than at shutdown, because the one case
    /// session restore exists for — the application not being closed politely —
    /// is precisely the case where a shutdown hook never runs. The write is a
    /// delete and a handful of inserts in one transaction, on an arrangement
    /// that changes when a human clicks something, so its cost is irrelevant.
    pub(crate) fn save_session(&self) {
        let (session, slots) = {
            let ws = self.workspace.lock();
            (ws.session(), ws.slots())
        };
        let Some(session) = session else { return };
        let result = self.db().and_then(|conn| {
            izul_store::sessions::save_tabs(&conn, session, &slots)
                .map_err(|e| format!("sesi: {e}"))
        });
        if let Err(e) = result {
            tracing::warn!(error = %e, "susunan tab tidak dapat disimpan");
        }
    }

    /// The document a command names, or an error the frontend can show.
    fn open_doc(&self, doc: u64) -> Result<OpenDoc, String> {
        self.workspace
            .lock()
            .get(doc)
            .cloned()
            .ok_or_else(|| "dokumen tidak terbuka".to_string())
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
    /// A `data:` PNG of the first page, when one was captured while the
    /// document was open. Absent for a file this application has never
    /// rendered — making one would mean opening the PDF, which is exactly what
    /// a list of recent files must not do.
    pub cover: Option<String>,
    /// Bytes, from the file as it is now — or as it was when last opened, when
    /// it is no longer there.
    pub size: u64,
    /// Seconds since the Unix epoch; the home screen's "Terakhir Diubah".
    pub modified: i64,
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
    state.annots.set_own_pages(doc.0, page_sizes.clone());

    // Recording the open and reading back the last position are one step: a
    // document the user has seen before must come back where they left it
    // (SPEC 11.1), and the row that stores that is the same one that makes it
    // recent.
    let mut file_id: Option<FileId> = None;
    let restored = match state.db() {
        Ok(conn) => match files::touch(&conn, &file, stamp) {
            Ok(id) => {
                file_id = Some(id);
                files::reading_state(&conn, id)
                    .ok()
                    .flatten()
                    .map(SavedView::from)
            }
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

    let file_id = file_id.map(|f| f.0).unwrap_or(0);
    state.stamps.lock().insert(doc.0, stamp);
    state.workspace.lock().insert(OpenDoc {
        doc: doc.0,
        path: path.clone(),
        file_id,
        page_count,
        panel: 0,
        pinned: false,
    });
    state.save_session();

    if file_id > 0 {
        // Both of these are background work on a document that is now open and
        // will stay open: neither is allowed to delay the first page appearing,
        // so neither is awaited here.
        state.indexer.start(Job {
            doc: doc.0,
            file_id,
            page_count,
            data_dir: state.data_dir.clone(),
            pool: Arc::clone(&state.pool),
        });
    }
    capture_cover(
        Arc::clone(&state.render),
        state.data_dir.clone(),
        doc.0,
        file.clone(),
    );

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

/// Renders a cover for the recent-files list, if there is not one already.
///
/// Fire and forget, at prefetch priority: the user is waiting for page one, not
/// for a 200-pixel thumbnail of it, and the preview tier this uses is the same
/// bitmap the sidebar is about to ask for anyway — so in practice this is a
/// cache hit that costs an encode.
fn capture_cover(render: Arc<RenderService>, data_dir: PathBuf, doc: u64, file: PathBuf) {
    if thumbs::file_for(&data_dir, &file).is_file() {
        return;
    }
    tokio::spawn(async move {
        let key = TileKey {
            doc,
            page: 0,
            rotation: 0,
            ppp_milli: thumbs::COVER_MAX_EDGE,
            col: 0,
            row: 0,
            kind: TileKind::Preview,
        };
        let tile = match render
            .fetch(key, 0, crate::render::Priority::Prefetch)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                tracing::debug!(doc, error = %e, "sampul tidak dapat dirender");
                return;
            }
        };
        match thumbs::encode_png(&tile.bytes, tile.width, tile.height, tile.stride) {
            Ok(png) => {
                if let Err(e) = thumbs::store(&data_dir, &file, &png) {
                    tracing::debug!(doc, error = %e, "sampul tidak dapat disimpan");
                }
            }
            Err(e) => tracing::debug!(doc, error = %e, "sampul tidak dapat dikodekan"),
        }
    });
}

#[tauri::command]
pub async fn close_document(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<()> {
    // The indexer first: it holds a worker handle for this document and would
    // otherwise keep asking a closed document for pages until it ran out.
    state.indexer.forget(doc);
    let worker = { state.pool.read().await.worker_for(DocId(doc)) };
    if let Some(worker) = worker {
        let _ = Pool::ask(&worker, Request::Close { doc: DocId(doc) }).await;
    }
    state.render.forget(doc);
    for hidden in state.hidden.take(doc) {
        crate::pagemap::close_hidden(&state, hidden).await;
    }
    state.annots.forget(doc);
    state.stamps.lock().remove(&doc);
    state.workspace.lock().remove(doc);
    state.save_session();
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
    // Fifty, not twenty: since the 2026-09-23 home screen the list is a table
    // grouped by recency rather than a shelf of covers, and a table of twenty
    // rows looks like a list that forgot things.
    let rows = files::recent(&conn, 50).map_err(|e| format!("basis data: {e}"))?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let path = PathBuf::from(&r.path);
            // One `stat`, which answers "is it still there" and "how big is it
            // now" together.
            let meta = std::fs::metadata(&path).ok().filter(|m| m.is_file());
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(r.stamp.mtime, |d| d.as_secs() as i64);
            RecentFile {
                name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| r.path.clone()),
                available: meta.is_some(),
                size: meta.as_ref().map_or(r.stamp.size, std::fs::Metadata::len),
                modified,
                cover: thumbs::load_data_url(&state.data_dir, &path),
                path: r.path,
                last_opened: r.last_opened,
                pinned: r.pinned,
            }
        })
        .collect())
}

/// Desktop, Documents, Downloads and the drives, for the home screen's
/// navigation. A folder the platform does not report is simply absent.
#[tauri::command]
pub fn known_folders(app: tauri::AppHandle) -> Vec<folders::KnownFolder> {
    use tauri::Manager;
    let paths = app.path();
    let mut out = Vec::new();
    for (kind, found) in [
        ("desktop", paths.desktop_dir()),
        ("documents", paths.document_dir()),
        ("downloads", paths.download_dir()),
    ] {
        if let Ok(dir) = found {
            out.push(folders::KnownFolder {
                kind,
                path: dir.to_string_lossy().to_string(),
            });
        }
    }
    for drive in folders::drives() {
        out.push(folders::KnownFolder {
            kind: "drive",
            path: drive.to_string_lossy().to_string(),
        });
    }
    out
}

/// Sub-folders and PDFs in one folder.
#[tauri::command]
pub async fn browse_folder(path: String) -> CmdResult<Vec<folders::FolderEntry>> {
    // Off the async runtime's worker: a slow network share must not stall the
    // commands queued behind it.
    tokio::task::spawn_blocking(move || folders::list(std::path::Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("folder tidak dapat dibaca: {e}"))
}

// ---------------------------------------------------------------------------
// Tabs and the session (SPEC 10, SPEC 11.3)
// ---------------------------------------------------------------------------

/// One open tab, as the tab bar draws it.
#[derive(Debug, Clone, Serialize)]
pub struct TabOut {
    pub doc: u64,
    pub path: String,
    pub name: String,
    pub page_count: u32,
    pub pinned: bool,
    pub active: bool,
}

/// A tab from the last run, waiting to be reopened.
#[derive(Debug, Clone, Serialize)]
pub struct RestorableTab {
    pub path: String,
    pub name: String,
    pub pinned: bool,
    pub active: bool,
    /// False when the file has moved or been deleted since. Restoring is a
    /// convenience, so a missing file is dropped quietly by the frontend rather
    /// than opening with an error dialog the user never asked for.
    pub available: bool,
}

fn base_name(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

#[tauri::command]
pub fn list_tabs(state: tauri::State<'_, AppState>) -> CmdResult<Vec<TabOut>> {
    let ws = state.workspace.lock();
    let active = ws.active();
    Ok(ws
        .tabs()
        .into_iter()
        .map(|d| TabOut {
            doc: d.doc,
            name: base_name(&d.path),
            path: d.path.clone(),
            page_count: d.page_count,
            pinned: d.pinned,
            active: active == Some(d.doc),
        })
        .collect())
}

/// Gives a tab focus.
///
/// Memory for the tabs that lost it is the frontend's call, not this command's:
/// it knows which tabs the user has been flipping between, and trimming a tab
/// the user is about to come back to is worse than holding its bitmaps a little
/// longer. `trim_document` is the lever it pulls.
#[tauri::command]
pub fn activate_document(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<()> {
    if !state.workspace.lock().set_active(doc) {
        return Err("dokumen tidak terbuka".into());
    }
    state.save_session();
    Ok(())
}

#[tauri::command]
pub fn reorder_tabs(state: tauri::State<'_, AppState>, order: Vec<u64>) -> CmdResult<()> {
    state.workspace.lock().reorder(&order);
    state.save_session();
    Ok(())
}

#[tauri::command]
pub fn set_tab_pinned(state: tauri::State<'_, AppState>, doc: u64, pinned: bool) -> CmdResult<()> {
    if !state.workspace.lock().set_pinned(doc, pinned) {
        return Err("dokumen tidak terbuka".into());
    }
    state.save_session();
    Ok(())
}

/// The tabs that were open when the application last held any (SPEC 11.3).
///
/// This does not reopen anything. The frontend decides — it has to, because
/// reopening is what creates the tabs it draws, and doing it here would mean
/// the window came up with documents the store knew nothing about.
#[tauri::command]
pub fn restore_session(state: tauri::State<'_, AppState>) -> CmdResult<Vec<RestorableTab>> {
    let conn = state.db()?;
    let Some((_, tabs)) = izul_store::sessions::latest(&conn).map_err(|e| format!("sesi: {e}"))?
    else {
        return Ok(Vec::new());
    };
    Ok(tabs
        .into_iter()
        .map(|t| {
            let path = PathBuf::from(&t.path);
            RestorableTab {
                name: base_name(&t.path),
                available: path.is_file(),
                pinned: t.slot.pinned,
                active: t.slot.is_active,
                path: t.path,
            }
        })
        .collect())
}

/// PDFs named on the command line — a double-clicked file, on Windows with the
/// association installed (SPEC 11.3).
#[tauri::command]
pub fn startup_files(state: tauri::State<'_, AppState>) -> CmdResult<Vec<String>> {
    Ok(state.startup_files.clone())
}

#[tauri::command]
pub fn pin_recent(state: tauri::State<'_, AppState>, path: String, pinned: bool) -> CmdResult<()> {
    let conn = state.db()?;
    let file = PathBuf::from(&path);
    let row = files::find(&conn, &file)
        .map_err(|e| format!("basis data: {e}"))?
        .ok_or_else(|| "berkas tidak ada dalam riwayat".to_string())?;
    files::set_pinned(&conn, row.id, pinned).map_err(|e| format!("basis data: {e}"))
}

// ---------------------------------------------------------------------------
// Indexing (SPEC 7)
// ---------------------------------------------------------------------------

/// Starts, or resumes, building the text index for an open document.
///
/// Opening a document already does this; the command exists so the search panel
/// can nudge an index that stopped — a tab closed halfway through, a worker
/// that died — without the user having to close and reopen the file.
#[tauri::command]
pub fn index_document(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<bool> {
    let open = state.open_doc(doc)?;
    if open.file_id <= 0 {
        return Err("berkas ini tidak tercatat, indeks tidak dapat dibuat".into());
    }
    Ok(state.indexer.start(Job {
        doc,
        file_id: open.file_id,
        page_count: open.page_count,
        data_dir: state.data_dir.clone(),
        pool: Arc::clone(&state.pool),
    }))
}

/// How far the index for a document has got.
///
/// `None` means nothing has been started in this run. That is not the same as
/// "not indexed": an index built during an earlier run is still on disk and
/// still answers searches, which is what [`izul_store::search::begin`]'s resume
/// point is for.
#[tauri::command]
pub fn index_progress(
    state: tauri::State<'_, AppState>,
    doc: u64,
) -> CmdResult<Option<IndexProgress>> {
    Ok(state.indexer.progress(doc))
}

// ---------------------------------------------------------------------------
// Search, in three tiers (SPEC 11.1)
// ---------------------------------------------------------------------------

/// A hit from the index: which page, and what it looks like.
#[derive(Debug, Clone, Serialize)]
pub struct IndexHit {
    pub file_id: i64,
    pub path: String,
    pub name: String,
    pub page: u32,
    /// The matching text with the match bracketed, straight from FTS5's own
    /// `snippet()`.
    pub snippet: String,
    /// The open document holding this file, when there is one — so clicking a
    /// library hit jumps in an existing tab instead of opening a second copy.
    pub doc: Option<u64>,
}

/// **Tier one** — the page in front of the user.
///
/// Answered by the worker holding the document, because this is the only tier
/// that needs geometry: where on the page each match sits, in the same display
/// space the tiles are drawn in. `generation` makes it cancellable — typing
/// raises it per keystroke, and a query the user has already typed past is
/// dropped rather than finished.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn search_page(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: u32,
    query: String,
    case_sensitive: bool,
    whole_word: bool,
    rotation: u8,
    generation: u64,
) -> CmdResult<Vec<HitOut>> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let worker = { state.pool.read().await.worker_for(DocId(doc)) };
    let worker = worker.ok_or_else(|| "dokumen tidak terbuka".to_string())?;
    let req = Request::Search {
        doc: DocId(doc),
        page,
        query,
        opts: SearchOptions {
            case_sensitive,
            whole_word,
            max_hits: MAX_HITS_PER_PAGE as u32,
        },
        rotation: RotationQuarter::from_degrees(i32::from(rotation) * 90),
        generation: izul_ipc::message::Generation(generation),
    };
    match Pool::ask(&worker, req).await {
        Ok(Response::SearchReady { hits, .. }) => Ok(hits
            .into_iter()
            .map(|h| HitOut {
                char_index: h.char_index,
                char_count: h.char_count,
                rects: h.rects.into_iter().map(Into::into).collect(),
            })
            .collect()),
        // A superseded search is the normal outcome of typing, not a failure:
        // the frontend already has a newer query in flight.
        Ok(Response::Superseded { .. }) => Ok(Vec::new()),
        Ok(Response::Error {
            message_id, detail, ..
        }) => Err(format!("{message_id}: {detail}")),
        Ok(other) => Err(format!("balasan tak terduga: {other:?}")),
        Err(e) => Err(e.to_string()),
    }
}

/// **Tier two** — the whole of one document, from its index.
///
/// The index is what makes this answerable without walking five hundred pages
/// through a worker. It returns pages; tier one then fills in where on the page
/// the match is, once the user goes there.
#[tauri::command]
pub fn search_document(
    state: tauri::State<'_, AppState>,
    doc: u64,
    query: String,
    limit: u32,
) -> CmdResult<Vec<IndexHit>> {
    let open = state.open_doc(doc)?;
    if open.file_id <= 0 {
        return Ok(Vec::new());
    }
    let conn = state.db()?;
    let hits = izul_store::search::search_file(&conn, FileId(open.file_id), &query, limit)
        .map_err(|e| format!("pencarian: {e}"))?;
    Ok(decorate(&state, hits))
}

/// **Tier three** — every document that has ever been indexed (SPEC 11.1).
#[tauri::command]
pub fn search_library(
    state: tauri::State<'_, AppState>,
    query: String,
    limit: u32,
) -> CmdResult<Vec<IndexHit>> {
    let conn = state.db()?;
    let hits = izul_store::search::search_library(&conn, &query, limit)
        .map_err(|e| format!("pencarian: {e}"))?;
    Ok(decorate(&state, hits))
}

/// Attaches the file name and, where there is one, the open tab holding it.
fn decorate(state: &AppState, hits: Vec<izul_store::Hit>) -> Vec<IndexHit> {
    let by_path: std::collections::HashMap<String, u64> = state
        .workspace
        .lock()
        .tabs()
        .into_iter()
        .map(|d| (d.path.clone(), d.doc))
        .collect();
    hits.into_iter()
        .map(|h| IndexHit {
            file_id: h.file_id.0,
            name: base_name(&h.path),
            doc: by_path.get(&h.path).copied(),
            page: h.page,
            snippet: h.snippet,
            path: h.path,
        })
        .collect()
}

/// Regular-expression search over **one page of an open document**.
///
/// Deliberately its own command rather than a flag on the others. SPEC 11.1
/// allows regex but forbids presenting it as the equal of the indexed tiers,
/// and the reason is in the shape of the data: FTS5 indexes tokens, so there is
/// no ordered text there for a pattern to run against. A regex needs extracted
/// characters, which means a document that is open and one page at a time.
///
/// See `textsearch` for the longer version of that argument.
#[tauri::command]
pub async fn search_regex_page(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: u32,
    pattern: String,
    case_sensitive: bool,
    rotation: u8,
) -> CmdResult<Vec<HitOut>> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let re = textsearch::compile(&pattern, case_sensitive)?;
    let worker = { state.pool.read().await.worker_for(DocId(doc)) };
    let worker =
        worker.ok_or_else(|| "regex hanya berlaku untuk dokumen yang terbuka".to_string())?;
    let req = Request::ExtractText {
        doc: DocId(doc),
        page,
        // With boxes: a regex match is a range of character indices, and the
        // highlight rectangles have to be built from the characters themselves.
        with_boxes: true,
        rotation: RotationQuarter::from_degrees(i32::from(rotation) * 90),
    };
    let (text, chars) = match Pool::ask(&worker, req).await {
        Ok(Response::TextReady { text, chars, .. }) => (text, chars),
        Ok(Response::Error {
            message_id, detail, ..
        }) => return Err(format!("{message_id}: {detail}")),
        Ok(other) => return Err(format!("balasan tak terduga: {other:?}")),
        Err(e) => return Err(e.to_string()),
    };
    Ok(textsearch::regex_hits(&text, &re, MAX_HITS_PER_PAGE)
        .into_iter()
        .map(|(start, len)| HitOut {
            char_index: start,
            char_count: len,
            rects: textsearch::rects_for_range(&chars, start as usize, len as usize),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Annotations (SPEC 8, SPEC 11.2)
// ---------------------------------------------------------------------------

use izul_model::annot::{AnnotObject, AnnotPayload, FontSpec};

/// One object's display list, as the canvas backend consumes it.
///
/// The list is built in Rust, by the same pure function the appearance-stream
/// backend reads, and handed over as data. The frontend draws it; it never
/// works out geometry of its own. That is the whole parity mechanism (SPEC 3.2)
/// and it is why this command exists at all rather than the frontend being told
/// "here is a rectangle, draw it".
#[derive(Debug, Serialize)]
pub struct DisplayListOut {
    pub id: u64,
    pub ops: izul_model::DisplayList,
}

/// The text an object will draw, for the metric pre-fetch.
fn text_of(obj: &AnnotObject) -> Option<(&FontSpec, &str)> {
    match &obj.payload {
        AnnotPayload::FreeText { text, font, .. } => Some((font, text.as_str())),
        AnnotPayload::Stamp { label, font, .. } => Some((font, label.as_str())),
        _ => None,
    }
}

/// Makes sure the metrics for whatever text these objects draw are cached.
///
/// Returns an error naming the characters the face does not have, which is what
/// SPEC 11.2 asks for instead of drawing empty boxes.
pub(crate) async fn prepare_fonts(
    state: &AppState,
    doc: u64,
    objects: &[AnnotObject],
) -> CmdResult<()> {
    for obj in objects {
        let Some((spec, text)) = text_of(obj) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let missing = state
            .annots
            .ensure_metrics(&state.pool, doc, spec, text)
            .await?;
        if !missing.is_empty() {
            let chars: String = missing.into_iter().take(8).collect();
            return Err(format!(
                "font {} tidak memuat karakter: {chars}",
                spec.family
            ));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn annot_list(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: Option<u32>,
) -> CmdResult<Vec<AnnotObject>> {
    // The first look at a page brings the annotations the file itself carries
    // into the editor (Phase 4), so they can be edited like any other.
    if let Some(p) = page {
        crate::saving::import_page(&state.pool, &state.annots, doc, p).await?;
    }
    Ok(state.annots.objects(doc, page))
}

#[tauri::command]
pub async fn annot_add(
    state: tauri::State<'_, AppState>,
    doc: u64,
    object: AnnotObject,
) -> CmdResult<(AnnotObject, EditResult)> {
    prepare_fonts(&state, doc, std::slice::from_ref(&object)).await?;
    state.annots.add(doc, object).map_err(|e| e.to_string())
}

/// Replaces objects as one undo step — every finished gesture, and every
/// change from the properties panel.
#[tauri::command]
pub async fn annot_replace(
    state: tauri::State<'_, AppState>,
    doc: u64,
    objects: Vec<AnnotObject>,
) -> CmdResult<EditResult> {
    prepare_fonts(&state, doc, &objects).await?;
    state
        .annots
        .replace(doc, objects)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn annot_delete(
    state: tauri::State<'_, AppState>,
    doc: u64,
    ids: Vec<u64>,
) -> CmdResult<EditResult> {
    state.annots.delete(doc, &ids).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn annot_undo(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: Option<u32>,
) -> CmdResult<EditResult> {
    state.annots.undo(doc, page).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn annot_redo(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: Option<u32>,
) -> CmdResult<EditResult> {
    state.annots.redo(doc, page).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn annot_history(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<(bool, bool)> {
    Ok(state.annots.history(doc))
}

/// The display lists for one page.
///
/// Async because it may have to ask a worker for font metrics first — the one
/// thing the UI process cannot answer for itself, because PDFium lives in the
/// sandbox (SPEC 5).
#[tauri::command]
pub async fn annot_display_lists(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: u32,
) -> CmdResult<Vec<DisplayListOut>> {
    crate::saving::import_page(&state.pool, &state.annots, doc, page).await?;
    let objects = state.annots.objects(doc, Some(page));
    prepare_fonts(&state, doc, &objects).await.ok();
    Ok(state
        .annots
        .display_lists(doc, page)
        .into_iter()
        .map(|(id, ops)| DisplayListOut { id, ops })
        .collect())
}

/// Stores an image file and returns the handle an `Image` annotation refers to.
///
/// The bytes stay in memory for this run. Writing them into the PDF is Phase 4;
/// until then an image annotation is as non-destructive as every other edit
/// (SPEC 8), and the canvas fetches the pixels back through
/// `izul://image/{doc}/{ref}`.
#[tauri::command]
pub fn annot_add_image(
    state: tauri::State<'_, AppState>,
    doc: u64,
    path: String,
) -> CmdResult<u32> {
    let bytes = std::fs::read(&path).map_err(|e| format!("tidak dapat membaca {path}: {e}"))?;
    // A generous cap, and a cap all the same: this is held in memory per
    // document, and a 400 MB TIFF pasted into a tab should be refused with a
    // message rather than by the process dying.
    const MAX_BYTES: usize = 64 * 1024 * 1024;
    if bytes.len() > MAX_BYTES {
        return Err(format!(
            "gambar terlalu besar ({} MB, batas {} MB)",
            bytes.len() / (1024 * 1024),
            MAX_BYTES / (1024 * 1024)
        ));
    }
    state.annots.add_image(doc, bytes)
}
