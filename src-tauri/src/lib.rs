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

pub mod annots;
pub mod background;
pub mod commands;
pub mod compare;
pub mod folders;
pub mod forms;
pub mod indexing;
pub mod logging;
pub mod ocr;
pub mod pagemap;
pub mod printing;
pub mod protocol;
pub mod render;
pub mod save_commands;
pub mod saving;
pub mod supervisor;
pub mod textedit;
pub mod textsearch;
pub mod thumbs;
pub mod version;
pub mod workspace;

use std::sync::atomic::{AtomicU64, Ordering};
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

    let swept = pagemap::sweep_sources(&data_dir);
    if swept > 0 {
        tracing::info!(swept, "salinan sumber halaman lama dihapus");
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

    // One session row per run. It is created before the window so that the
    // first document opened has somewhere to be recorded, and `latest()` skips
    // empty sessions so this one cannot shadow the arrangement it is about to
    // restore.
    let session = match izul_store::open(&data_dir, izul_store::Which::App)
        .map_err(|e| format!("{e}"))
        .and_then(|conn| {
            izul_store::sessions::prune(&conn, KEEP_SESSIONS).ok();
            izul_store::sessions::begin(&conn).map_err(|e| format!("{e}"))
        }) {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::warn!(error = %e, "sesi tidak dapat dibuka; susunan tab tidak akan tersimpan");
            None
        }
    };

    let state = AppState {
        pool: Arc::clone(&pool),
        render: Arc::clone(&renderer),
        data_dir,
        next_doc_id: AtomicU64::new(1),
        workspace: parking_lot::Mutex::new(workspace::Workspace::new(session)),
        indexer: Arc::new(indexing::Indexer::new()),
        startup_files: pdf_arguments(std::env::args().skip(1)),
        annots: Arc::new(annots::AnnotState::new()),
        stamps: parking_lot::Mutex::new(std::collections::HashMap::new()),
        hidden: pagemap::HiddenSources::default(),
        ocr: ocr::OcrRuns::default(),
        printing: printing::PrintJobs::default(),
    };

    let title = version::title_bar_text(&v);
    let handle = rt.handle().clone();
    let tile_renderer = Arc::clone(&renderer);
    let result = tauri::Builder::default()
        // First, before every other plugin: this one decides whether this
        // process is the application at all. A second launch — which is what a
        // double-clicked PDF produces once the file association is installed —
        // hands its arguments to the instance already running and exits, so the
        // file opens as a tab in the window the user is looking at instead of
        // in a second window with its own worker pool (SPEC 11.3).
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let files = pdf_arguments(argv.into_iter().skip(1));
            if let Some(w) = app.get_webview_window("main") {
                // Focus first: the user double-clicked something and expects a
                // window, whether or not the file turns out to be openable.
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
            if !files.is_empty() {
                let _ = app.emit("izul://open-files", files);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .register_asynchronous_uri_scheme_protocol("izul", move |ctx, request, responder| {
            // Asynchronous on purpose: a tile that is not cached has to wait for
            // a worker, and the synchronous form would block a webview thread
            // for the whole render. The handler returns immediately; the
            // responder is answered from the runtime.
            let service = Arc::clone(&tile_renderer);
            let uri = request.uri().to_string();
            // Images are answered from memory, so there is nothing to wait for
            // and no reason to go through the runtime.
            if protocol::parse_image_uri(&uri).is_ok() {
                let app = ctx.app_handle().clone();
                responder.respond(serve_image(&app, &uri));
                return;
            }
            if protocol::parse_print_uri(&uri).is_ok() {
                let app = ctx.app_handle().clone();
                // A file read: off the webview's thread.
                handle.spawn_blocking(move || responder.respond(serve_print(&app, &uri)));
                return;
            }
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
            commands::pin_recent,
            commands::known_folders,
            commands::browse_folder,
            commands::list_tabs,
            commands::activate_document,
            commands::reorder_tabs,
            commands::set_tab_pinned,
            commands::restore_session,
            commands::startup_files,
            commands::index_document,
            commands::index_progress,
            commands::search_page,
            commands::search_document,
            commands::search_library,
            commands::search_regex_page,
            commands::annot_list,
            commands::annot_add,
            commands::annot_replace,
            commands::annot_add_many,
            commands::annot_delete,
            commands::annot_undo,
            commands::annot_redo,
            commands::annot_history,
            commands::annot_display_lists,
            commands::annot_add_image,
            save_commands::save_document,
            save_commands::redact_apply,
            save_commands::redact_preview,
            ocr::ocr_available,
            ocr::ocr_apply,
            ocr::ocr_progress,
            ocr::ocr_cancel,
            background::background_available,
            background::annot_remove_background,
            forms::form_fields,
            forms::form_set,
            textedit::text_replace,
            printing::print_prepare,
            printing::print_done,
            save_commands::export_document,
            save_commands::export_history,
            save_commands::autosave_drafts,
            save_commands::draft_save_now,
            save_commands::draft_status,
            save_commands::draft_restore,
            save_commands::draft_discard,
            save_commands::file_status,
            save_commands::file_acknowledge,
            compare::compare_visual,
            commands::pref_get,
            commands::pref_set,
            pagemap::pages_state,
            pagemap::pages_apply,
            pagemap::pages_insert_file,
            pagemap::pages_copy_from,
        ])
        .run(tauri::generate_context!());

    let _ = stop_tx.send(());
    result.map_err(|e| format!("tauri: {e}"))
}

/// How many past runs of the application keep their tab arrangement.
///
/// Only the newest is ever restored; the rest are kept because a user who
/// restarts twice by accident has not lost the arrangement they wanted, and
/// pruning stops the table growing forever on a machine that is never shut
/// down.
const KEEP_SESSIONS: u32 = 10;

/// The PDFs named on the command line.
///
/// Windows hands a double-clicked file to the application as an argument, so
/// this is the whole of the file-association path on our side once the
/// installer has registered the type (SPEC 11.3). Everything that is not an
/// existing `.pdf` is ignored rather than reported: the argument list also
/// carries switches, and a typo in one must not open an error dialog before the
/// window is even up.
fn pdf_arguments<I: IntoIterator<Item = String>>(args: I) -> Vec<String> {
    args.into_iter()
        .filter(|a| !a.starts_with('-'))
        .filter(|a| {
            std::path::Path::new(a)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
        })
        .filter(|a| std::path::Path::new(a).is_file())
        .collect()
}

/// What became of one tile request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Served,
    BadUri,
    Superseded,
    SlotGone,
    Missing,
    WorkerDown,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Outcome::Served => "terkirim",
            Outcome::BadUri => "uri ditolak",
            Outcome::Superseded => "generasi lama",
            Outcome::SlotGone => "slot didaur ulang",
            Outcome::Missing => "dokumen/halaman tidak ada",
            Outcome::WorkerDown => "pekerja gagal",
        }
    }
}

/// Running tally of what happens to tile requests.
///
/// This exists because three rounds of a "the pages are blank" report were
/// inconclusive: a refused tile was logged at `debug!`, which the default
/// `info` filter drops, and a served one was not logged at all. The log
/// therefore looked exactly the same whether the viewport never asked for a
/// tile or the backend refused every single one — the two cases that need
/// telling apart first, and the only ones a user can report without opening
/// DevTools.
///
/// Logging every tile is not an option: one screenful is dozens of requests
/// and a scroll is thousands. So the first few of each outcome are logged in
/// full, which answers "did anything arrive, and what happened to it", and
/// after that only a periodic summary keeps the log honest without flooding it.
struct TileTraffic {
    served: AtomicU64,
    bad_uri: AtomicU64,
    superseded: AtomicU64,
    slot_gone: AtomicU64,
    missing: AtomicU64,
    worker_down: AtomicU64,
    total: AtomicU64,
}

static TRAFFIC: TileTraffic = TileTraffic {
    served: AtomicU64::new(0),
    bad_uri: AtomicU64::new(0),
    superseded: AtomicU64::new(0),
    slot_gone: AtomicU64::new(0),
    missing: AtomicU64::new(0),
    worker_down: AtomicU64::new(0),
    total: AtomicU64::new(0),
};

/// How many of each outcome are reported in full before summaries take over.
const VERBOSE_FIRST: u64 = 3;
/// How many requests between summary lines once past that.
const SUMMARY_EVERY: u64 = 200;

/// Whether the `n`th occurrence of an outcome is worth a line of its own.
fn is_verbose(n: u64) -> bool {
    n <= VERBOSE_FIRST
}

/// Whether the `total`th request should carry a summary of the tally.
fn is_summary(total: u64) -> bool {
    total > 0 && total.is_multiple_of(SUMMARY_EVERY)
}

impl TileTraffic {
    fn counter(&self, outcome: Outcome) -> &AtomicU64 {
        match outcome {
            Outcome::Served => &self.served,
            Outcome::BadUri => &self.bad_uri,
            Outcome::Superseded => &self.superseded,
            Outcome::SlotGone => &self.slot_gone,
            Outcome::Missing => &self.missing,
            Outcome::WorkerDown => &self.worker_down,
        }
    }

    fn record(&self, outcome: Outcome, uri: &str, detail: &str) {
        let n = self.counter(outcome).fetch_add(1, Ordering::Relaxed) + 1;
        let total = self.total.fetch_add(1, Ordering::Relaxed) + 1;
        if is_verbose(n) {
            tracing::info!(hasil = outcome.label(), ke = n, uri, detail, "ubin");
        }
        if is_summary(total) {
            tracing::info!(
                total,
                terkirim = self.served.load(Ordering::Relaxed),
                uri_ditolak = self.bad_uri.load(Ordering::Relaxed),
                generasi_lama = self.superseded.load(Ordering::Relaxed),
                slot_hilang = self.slot_gone.load(Ordering::Relaxed),
                tidak_ada = self.missing.load(Ordering::Relaxed),
                pekerja_gagal = self.worker_down.load(Ordering::Relaxed),
                "ringkasan lalu lintas ubin"
            );
        }
    }
}

/// Serves `izul://image/{doc}/{ref}`: the bytes of an inserted image.
///
/// Separate from the tile route because it answers from memory and carries a
/// real media type — the webview has to decode it as an image, which it will
/// only do if told what it is. The CORS headers are the same ones every answer
/// from this protocol needs; the Phase 1 notes in CLAUDE.md explain what
/// happens without them, which is a `TypeError` with no status code at all.
/// A page prepared for printing (`printing.rs`), from its scratch file.
pub fn serve_print(app: &tauri::AppHandle, uri: &str) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{Response, StatusCode};
    use tauri::Manager as _;

    let respond = |code: StatusCode, body: Vec<u8>| -> Response<Vec<u8>> {
        let mut builder = Response::builder().status(code);
        if code == StatusCode::OK {
            builder = builder.header("Content-Type", "image/jpeg");
        }
        // Every answer, errors included: without these the browser drops
        // the response before our code sees it (Phase 1, bug 5).
        for (k, val) in protocol::cors_headers() {
            builder = builder.header(k, val);
        }
        builder
            .body(body)
            .unwrap_or_else(|_| Response::new(Vec::new()))
    };
    let Ok(parsed) = protocol::parse_print_uri(uri) else {
        return respond(StatusCode::BAD_REQUEST, Vec::new());
    };
    let Some(state) = app.try_state::<AppState>() else {
        return respond(StatusCode::SERVICE_UNAVAILABLE, Vec::new());
    };
    let Some(file) = state.printing.page(parsed.job, parsed.index) else {
        return respond(StatusCode::NOT_FOUND, Vec::new());
    };
    match std::fs::read(&file) {
        Ok(bytes) => respond(StatusCode::OK, bytes),
        Err(_) => respond(StatusCode::GONE, Vec::new()),
    }
}

pub fn serve_image(app: &tauri::AppHandle, uri: &str) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{Response, StatusCode};
    use tauri::Manager as _;

    let deny = |code: StatusCode| -> Response<Vec<u8>> {
        let mut builder = Response::builder().status(code);
        for (k, val) in protocol::cors_headers() {
            builder = builder.header(k, val);
        }
        builder
            .body(Vec::new())
            .unwrap_or_else(|_| Response::new(Vec::new()))
    };

    let Ok(parsed) = protocol::parse_image_uri(uri) else {
        return deny(StatusCode::BAD_REQUEST);
    };
    let Some(state) = app.try_state::<AppState>() else {
        return deny(StatusCode::SERVICE_UNAVAILABLE);
    };
    let Some(image) = state.annots.image(parsed.doc, parsed.image) else {
        return deny(StatusCode::NOT_FOUND);
    };
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", image.media_type)
        // The bytes never change under a handle, so the webview may keep them.
        .header("Cache-Control", "private, max-age=3600");
    for (k, val) in protocol::cors_headers() {
        builder = builder.header(k, val);
    }
    builder
        .body(image.bytes)
        .unwrap_or_else(|_| deny(StatusCode::INTERNAL_SERVER_ERROR))
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
            TRAFFIC.record(Outcome::BadUri, uri, &e.to_string());
            return deny(StatusCode::BAD_REQUEST);
        }
    };

    let tile = match service
        .fetch(parsed.key, parsed.generation, parsed.priority)
        .await
    {
        Ok(tile) => tile,
        Err(e @ (RenderError::Superseded | RenderError::Busy | RenderError::Cancelled)) => {
            TRAFFIC.record(Outcome::Superseded, uri, &e.to_string());
            return deny(StatusCode::CONFLICT);
        }
        Err(e @ RenderError::SlotGone) => {
            TRAFFIC.record(Outcome::SlotGone, uri, &e.to_string());
            return deny(StatusCode::GONE);
        }
        Err(e @ (RenderError::UnknownDocument(_) | RenderError::PageOutOfRange { .. })) => {
            TRAFFIC.record(Outcome::Missing, uri, &e.to_string());
            return deny(StatusCode::NOT_FOUND);
        }
        Err(RenderError::Worker(e)) => {
            TRAFFIC.record(Outcome::WorkerDown, uri, &e.to_string());
            tracing::warn!(error = %e, uri, "render gagal");
            return deny(StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    TRAFFIC.record(
        Outcome::Served,
        uri,
        &format!("{}x{}", tile.width, tile.height),
    );

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
    use super::{is_summary, is_verbose, Outcome, SUMMARY_EVERY, VERBOSE_FIRST};
    use crate::render::Priority;

    /// The instrument has to answer "did anything arrive at all" on the very
    /// first request, or it is useless for the report it exists to serve.
    #[test]
    fn the_first_tile_of_each_outcome_is_always_reported() {
        assert!(is_verbose(1), "yang pertama harus selalu tercatat");
        for n in 1..=VERBOSE_FIRST {
            assert!(is_verbose(n), "ke-{n} masih dalam jatah rinci");
        }
    }

    /// And it must not drown the log once the viewport is scrolling: a
    /// screenful is dozens of tiles, a scroll is thousands.
    #[test]
    fn a_scroll_does_not_flood_the_log() {
        assert!(!is_verbose(VERBOSE_FIRST + 1));
        assert!(!is_verbose(10_000));
        let lines = (1..=10_000u64).filter(|n| is_verbose(*n)).count()
            + (1..=10_000u64).filter(|t| is_summary(*t)).count();
        assert!(
            lines < 100,
            "10.000 ubin tidak boleh jadi {lines} baris log"
        );
    }

    #[test]
    fn the_tally_is_summarised_at_a_steady_interval() {
        assert!(!is_summary(0), "belum ada apa-apa untuk diringkas");
        assert!(is_summary(SUMMARY_EVERY));
        assert!(is_summary(SUMMARY_EVERY * 2));
        assert!(!is_summary(SUMMARY_EVERY + 1));
    }

    /// Each outcome needs its own words: a log that calls every refusal
    /// "ditolak" is the log that made three rounds of this inconclusive.
    #[test]
    fn every_outcome_reads_differently_in_the_log() {
        let all = [
            Outcome::Served,
            Outcome::BadUri,
            Outcome::Superseded,
            Outcome::SlotGone,
            Outcome::Missing,
            Outcome::WorkerDown,
        ];
        let mut labels: Vec<&str> = all.iter().map(|o| o.label()).collect();
        labels.sort_unstable();
        let before = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), before, "dua hasil memakai kata yang sama");
    }

    /// The argument list is not a list of files: it carries switches, and on
    /// Windows it carries whatever the shell felt like passing. Opening a
    /// dialog about one before the window exists would be the first thing a
    /// user saw.
    #[test]
    fn only_real_pdf_arguments_are_opened_at_startup() {
        let dir = std::env::temp_dir().join("izul-args-test");
        std::fs::create_dir_all(&dir).expect("folder sementara");
        let real = dir.join("Laporan.PDF");
        std::fs::write(&real, b"%PDF-1.7\n").expect("tulis");
        let args = vec![
            "--flag".to_string(),
            "tidak-ada.pdf".to_string(),
            dir.join("bukan.txt").display().to_string(),
            real.display().to_string(),
        ];
        assert_eq!(
            super::pdf_arguments(args),
            vec![real.display().to_string()],
            "hanya berkas .pdf yang benar-benar ada"
        );
        let _ = std::fs::remove_file(&real);
    }

    #[test]
    fn the_viewports_priority_numbers_mean_what_the_scheduler_thinks() {
        assert_eq!(Priority::from_u8(2), Priority::Visible);
        assert_eq!(Priority::from_u8(1), Priority::Preview);
        assert_eq!(Priority::from_u8(0), Priority::Prefetch);
    }
}
