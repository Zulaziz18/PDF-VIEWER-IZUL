//! PDF Studio Izul — the UI process (SPEC 5).
//!
//! This process owns the window, the supervisor, and the file associations. It
//! never opens a PDF itself: every byte of untrusted input is parsed in a
//! sandboxed worker, so a malformed file can take a tab down but not the
//! application.

// A shipped Windows build must not open a console behind the window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
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

mod commands;
mod logging;
mod protocol;
mod supervisor;
mod version;

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use commands::AppState;
use supervisor::worker::WorkerPaths;
use supervisor::Pool;
use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

fn main() {
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

fn run(data_dir: std::path::PathBuf, v: version::VersionInfo) -> Result<(), String> {
    // Databases are created before the window so a first run never shows an
    // empty recent-files list because the schema was not ready yet.
    for which in [izul_store::Which::App, izul_store::Which::Cache] {
        izul_store::open(&data_dir, which).map_err(|e| format!("{}: {e}", which.file_name()))?;
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio: {e}"))?;

    let paths = WorkerPaths::beside_current_exe().map_err(|e| format!("jalur pekerja: {e}"))?;
    let pool = rt
        .block_on(Pool::start(paths))
        .map_err(|e| format!("kolam pekerja tidak dapat dimulai: {e}"))?;
    let pool = Arc::new(RwLock::new(pool));

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

    let state = AppState {
        pool: Arc::clone(&pool),
        data_dir,
        next_doc_id: AtomicU64::new(1),
    };

    let title = version::title_bar_text(&v);
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .register_uri_scheme_protocol("izul", move |ctx, request| {
            serve_tile(ctx.app_handle(), request)
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
            commands::render_tile,
            commands::recent_files,
        ])
        .run(tauri::generate_context!());

    let _ = stop_tx.send(());
    result.map_err(|e| format!("tauri: {e}"))
}

/// Serves `izul://tile/{doc}/{slot}/{epoch}` straight out of shared memory.
fn serve_tile(
    app: &tauri::AppHandle,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{Response, StatusCode};

    let deny = |code: StatusCode| -> Response<Vec<u8>> {
        Response::builder()
            .status(code)
            .body(Vec::new())
            .unwrap_or_else(|_| Response::new(Vec::new()))
    };

    let uri = match protocol::parse_tile_uri(&request.uri().to_string()) {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!(error = %e, uri = %request.uri(), "permintaan ubin ditolak");
            return deny(StatusCode::BAD_REQUEST);
        }
    };

    let Some(state) = app.try_state::<AppState>() else {
        return deny(StatusCode::SERVICE_UNAVAILABLE);
    };
    // `blocking_read` is correct here: the protocol handler already runs off the
    // UI thread, and the lock is only ever held for the length of one request.
    let ring = { state.pool.blocking_read().worker_ring(uri.doc) };
    let Some(ring) = ring else {
        return deny(StatusCode::NOT_FOUND);
    };

    // The stride is read back from the slot itself; the hint only has to be
    // non-zero for the reference to be well formed.
    let Some(tile) = protocol::read_tile(&ring, uri, 1) else {
        // A stale epoch is normal during fast scrolling: the tile was recycled
        // before the webview asked for it. The frontend re-requests.
        return deny(StatusCode::GONE);
    };

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/octet-stream")
        // Every URI is unique by epoch, so a cached response could only ever be
        // wrong.
        .header("Cache-Control", "no-store");
    for (k, val) in tile.headers() {
        builder = builder.header(k, val);
    }
    builder
        .body(tile.bytes)
        .unwrap_or_else(|_| deny(StatusCode::INTERNAL_SERVER_ERROR))
}
