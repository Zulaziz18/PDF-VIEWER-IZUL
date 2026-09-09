//! PDF Studio Izul — the UI process (SPEC 5).
//!
//! This process owns the window, the supervisor, and the file associations. It
//! never opens a PDF itself: every byte of untrusted input is parsed in a
//! sandboxed worker, so a malformed file can take a tab down but not the
//! application.
//!
//! ## Why this is a library with a thin binary on top
//!
//! The supervisor, the tile protocol and the render backend are where the
//! hardest bugs of Phase 1 lived — a channel read cancelled mid-frame, a
//! heartbeat that executed healthy workers, a protocol answer the browser
//! discarded. None of them could be reached from a test while this code was a
//! binary, because a binary crate cannot be imported. Every one of them was
//! therefore found by a user running the real application on Windows, one
//! round trip at a time.
//!
//! So the process lives here, where `tests/` can drive it, and `main.rs` is
//! reduced to starting it.

#![forbid(unsafe_op_in_unsafe_fn)]
// SPEC 0 bans `unwrap`, `expect` and `panic` in production code, and the
// workspace lints deny them. Test code is the exception on purpose: inside a
// test, `expect` *is* the failure report, and rewriting every assertion into
// error propagation would make the tests harder to read without making anything
// safer. The relaxation is scoped to `cfg(test)`, so it can never reach a
// shipped build.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod commands;
pub mod logging;
pub mod protocol;
pub mod render;
pub mod supervisor;
pub mod version;

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use commands::AppState;
use izul_render::RenderError;
use render::{PoolBackend, RenderService};
use supervisor::worker::WorkerPaths;
use supervisor::Pool;
use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

/// Starts the application: logging, version, then the window.
///
/// Returns only when the window closes; errors are already reported.
pub fn start() {
    let portable = std::env::var_os("IZUL_PORTABLE").is_some();
    let data_dir = izul_store::default_data_dir(portable);
    let _log_guard = match logging::init(&data_dir) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!(
                "peringatan: log tidak dapat ditulis ke {}: {e}",
                data_dir.display()
            );
            None
        }
    };

    let v = match version::info() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("version.json rusak: {e}");
            std::process::exit(2);
        }
    };
    tracing::info!(
        version = %v.version,
        codename = %v.codename,
        phase = %v.phase,
        pdfium = %v.pdfium_version,
        "PDF Studio Izul mulai"
    );

    if let Err(e) = run(data_dir, v) {
        tracing::error!(error = %e, "aplikasi berhenti");
        std::process::exit(1);
    }
}

pub fn run(data_dir: std::path::PathBuf, v: version::VersionInfo) -> Result<(), String> {
    // Databases are created before the window so a first run never shows an
    // empty recent-files list because the schema was not ready yet.
    let mut budget_pref = None;
    for which in [izul_store::Which::App, izul_store::Which::Cache] {
        let conn = izul_store::open(&data_dir, which)
            .map_err(|e| format!("{}: {e}", which.file_name()))?;
        if which == izul_store::Which::App {
            budget_pref = izul_store::prefs::get_u64(&conn, izul_store::prefs::CACHE_BUDGET_MB)
                .unwrap_or(None);
        }
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio: {e}"))?;

    let paths = WorkerPaths::beside_current_exe().map_err(|e| format!("jalur pekerja: {e}"))?;
    let pool = rt
        .block_on(Pool::start(paths))
        .map_err(|e| format!("kolam pekerja tidak dapat dimulai: {e}"))?;
    let pool_size = pool.size();
    let pool = Arc::new(RwLock::new(pool));

    // One outstanding render per worker: a worker renders on one thread, so a
    // second request would only queue inside it, where the scheduler can no
    // longer reprioritise it (SPEC 9's cancellation).
    let budget = render::cache_budget_bytes(budget_pref);
    let renderer = RenderService::new(PoolBackend::new(Arc::clone(&pool)), budget, pool_size);
    tracing::info!(
        budget_mb = budget / (1024 * 1024),
        concurrency = pool_size,
        "pipeline render siap"
    );

    // The supervision loop outlives every window; it is what keeps a killed
    // worker from becoming a permanently dead tab.
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let (recovered_tx, recovered_rx) = std::sync::mpsc::channel::<Vec<supervisor::RecoveredDoc>>();
    let sink: supervisor::RecoverySink = Box::new(move |docs| {
        let _ = recovered_tx.send(docs);
    });
    rt.spawn(supervisor::supervise(
        Arc::clone(&pool),
        Some(sink),
        stop_rx,
    ));
    {
        let _guard = rt.enter();
        renderer.spawn_dispatcher();
    }

    let state = AppState {
        pool: Arc::clone(&pool),
        render: Arc::clone(&renderer),
        data_dir,
        next_doc_id: AtomicU64::new(1),
    };

    let title = version::title_bar_text(&v);
    let handle = rt.handle().clone();
    let tile_renderer = Arc::clone(&renderer);
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .register_asynchronous_uri_scheme_protocol("izul", move |_ctx, request, responder| {
            // Asynchronous on purpose: a tile that is not cached has to wait for
            // a worker, and the synchronous form would block a webview thread
            // for the whole render. The handler returns immediately; the
            // responder is answered from the runtime.
            let service = Arc::clone(&tile_renderer);
            let uri = request.uri().to_string();
            handle.spawn(async move {
                responder.respond(serve_tile(&service, &uri).await);
            });
        })
        .setup(move |app| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title(&title);
            }
            // A crashed worker must reach the user as a tab that reloads itself,
            // not as a viewport that quietly stops updating (SPEC 3.4).
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                while let Ok(docs) = recovered_rx.recv() {
                    for doc in docs {
                        let _ = handle.emit(
                            "izul://document-recovered",
                            serde_json::json!({
                                "doc": doc.doc.0,
                                "path": doc.path,
                                "quarantined": doc.quarantined,
                            }),
                        );
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_version,
            commands::log_folder,
            commands::pool_health,
            commands::open_document,
            commands::close_document,
            commands::trim_document,
            commands::set_generation,
            commands::document_outline,
            commands::page_text,
            commands::page_size,
            commands::save_view_state,
            commands::render_stats,
            commands::recent_files,
        ])
        .run(tauri::generate_context!());

    let _ = stop_tx.send(());
    result.map_err(|e| format!("tauri: {e}"))
}

/// Serves `izul://tile/...`: cache hit, or render and then cache.
///
/// The status codes are part of the contract with the viewport:
///
/// * **409** the request belongs to a layout epoch the user has already left.
///   Normal during a fast zoom, and not an error the user should hear about.
/// * **410** the tile was rendered but its shared-memory slot was recycled
///   before we could copy it. The viewport re-requests.
/// * **404** the document is closed or the page does not exist.
/// * **503** the worker holding the document is gone; the supervisor is
///   already restarting it.
pub async fn serve_tile(service: &Arc<RenderService>, uri: &str) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{Response, StatusCode};

    // Every answer, including the refusals: a response the browser discards
    // for want of a CORS header cannot even report its own status code, so
    // the viewport's careful handling of 409 and 410 would never run.
    let deny = |code: StatusCode| -> Response<Vec<u8>> {
        let mut builder = Response::builder().status(code);
        for (k, val) in protocol::cors_headers() {
            builder = builder.header(k, val);
        }
        builder
            .body(Vec::new())
            .unwrap_or_else(|_| Response::new(Vec::new()))
    };

    let parsed = match protocol::parse_tile_uri(uri) {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!(error = %e, uri, "permintaan ubin ditolak");
            return deny(StatusCode::BAD_REQUEST);
        }
    };

    let tile = match service
        .fetch(parsed.key, parsed.generation, parsed.priority)
        .await
    {
        Ok(tile) => tile,
        Err(RenderError::Superseded) | Err(RenderError::Busy) | Err(RenderError::Cancelled) => {
            return deny(StatusCode::CONFLICT)
        }
        Err(RenderError::SlotGone) => return deny(StatusCode::GONE),
        Err(RenderError::UnknownDocument(_)) | Err(RenderError::PageOutOfRange { .. }) => {
            return deny(StatusCode::NOT_FOUND)
        }
        Err(RenderError::Worker(e)) => {
            tracing::warn!(error = %e, uri, "render gagal");
            return deny(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/octet-stream")
        // The URI names the tile's content exactly, so a tile that is served
        // once can be served from the webview's own memory afterwards. The
        // backend cache still holds it; this saves the second copy.
        .header("Cache-Control", "private, max-age=60");
    for (k, val) in protocol::cors_headers() {
        builder = builder.header(k, val);
    }
    for (k, val) in protocol::tile_headers(tile.width, tile.height, tile.stride) {
        builder = builder.header(k, val);
    }
    builder
        .body(tile.bytes.to_vec())
        .unwrap_or_else(|_| deny(StatusCode::INTERNAL_SERVER_ERROR))
}

/// Priority is part of the URI, so it is worth one assertion that the mapping
/// the viewport writes is the one the scheduler reads.
#[cfg(test)]
mod tests {
    use crate::render::Priority;

    #[test]
    fn the_viewports_priority_numbers_mean_what_the_scheduler_thinks() {
        assert_eq!(Priority::from_u8(2), Priority::Visible);
        assert_eq!(Priority::from_u8(1), Priority::Preview);
        assert_eq!(Priority::from_u8(0), Priority::Prefetch);
    }
}
