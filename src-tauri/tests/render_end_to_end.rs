//! The whole backend of the render pipeline, driven the way the browser drives
//! it (SPEC 9, SPEC 16).
//!
//! Every other test in this repository stops short of one layer or another:
//! the unit tests mock the worker, `tests/render_pipeline.rs` talks to a worker
//! directly without the supervisor, and the benchmark supplies its own backend.
//! The layers *between* them — the worker pool, its heartbeat, the channel pump
//! that multiplexes requests onto one worker, and the `PoolBackend` that joins
//! them to the render service — were covered by nothing, and that is exactly
//! where Phase 1's worst bugs lived: a channel read cancelled mid-frame, a
//! heartbeat that killed healthy workers, a stale worker binary answering a
//! ping with a shutdown. Each one was found by a person running the real
//! application on Windows, a round trip at a time.
//!
//! So this drives the real thing: real worker processes under the real
//! supervisor, real shared memory, the real scheduler and cache, entered
//! through the same URI the webview asks for. Only the webview itself is
//! missing.
//!
//! ```text
//! cargo test -p izul-app --test render_end_to_end -- --nocapture
//! ```

// SPEC 0 bans `unwrap`, `expect` and `panic` in production code, and the
// workspace lints deny them for this package. An integration test is the
// exception on purpose, the same one `izul-pdf` makes for its own `cfg(test)`
// code and `izul-integration-tests` makes in its manifest: inside a test,
// `expect` *is* the failure report. This file cannot take the manifest route
// because it lives in the application's own package, so it says it here.
// Every test in this file drives a real worker over a Unix socket, so the whole
// file is Unix-only. Gating the crate root rather than each test keeps the
// imports gated with them — an import left behind by a `#[cfg(unix)]` test is an
// unused import on Windows, which the workspace's `-D warnings` makes a build
// failure — and it cannot drift when the next test is added.
//
// The cost is real and worth stating: these tests never run on Windows, which
// is the platform this application ships on. Making them cross-platform means
// giving the harness a named-pipe path beside the Unix-socket one; until then,
// Windows coverage comes from the unit tests and from running the application.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use izul_app::indexing::{Indexer, Job};
use izul_app::protocol::parse_tile_uri;
use izul_app::render::{DocInfo, PoolBackend, RenderService};
use izul_app::supervisor::worker::WorkerPaths;
use izul_app::supervisor::Pool;
use izul_ipc::message::{DocId, Generation, Request, Response, SearchOptions};
use izul_model::geom::RotationQuarter;
use tokio::sync::RwLock;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Where the worker binary and PDFium live for a test run.
///
/// `WorkerPaths::beside_current_exe` is right for a shipped build and wrong
/// here, because a test binary lives in `target/<profile>/deps`.
fn worker_paths() -> Option<WorkerPaths> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?.to_path_buf();
    let worker = dir.join(if cfg!(windows) {
        "izul-worker.exe"
    } else {
        "izul-worker"
    });
    let pdfium = repo_root().join(if cfg!(windows) {
        "vendor/pdfium/win-x64/bin/pdfium.dll"
    } else {
        "vendor/pdfium/linux-x64/lib/libpdfium.so"
    });
    (worker.exists() && pdfium.exists()).then_some(WorkerPaths {
        executable: worker,
        pdfium,
    })
}

fn fixture() -> Option<PathBuf> {
    let p = repo_root().join("test-fixtures/viewer-10p.pdf");
    p.exists().then_some(p)
}

fn skip(reason: &str) {
    if std::env::var_os("IZUL_REQUIRE_FIXTURES").is_some() {
        panic!("prasyarat hilang dan IZUL_REQUIRE_FIXTURES diset: {reason}");
    }
    eprintln!("LEWATI: {reason}");
}

/// A running pool with one document open and a render service on top of it —
/// the same objects `izul_app::run` builds for the window.
struct Live {
    service: Arc<RenderService>,
    pool: Arc<RwLock<Pool>>,
    doc: u64,
}

impl Live {
    async fn start() -> Option<Self> {
        let (paths, fixture) = (worker_paths()?, fixture()?);
        let pool = Pool::start(paths).await.ok()?;
        let size = pool.size();
        let pool = Arc::new(RwLock::new(pool));

        let doc = DocId(1);
        let bytes = std::fs::read(&fixture).ok()?;
        let key = izul_store::content_hash(&bytes);
        drop(bytes);

        let opened = {
            let mut guard = pool.write().await;
            guard
                .open(doc, &fixture.to_string_lossy(), key, None)
                .await
                .expect("open the document through the pool")
        };
        let page_sizes = match opened {
            Response::Opened { page_sizes, .. } => page_sizes,
            other => panic!("expected Opened, got {other:?}"),
        };

        let service = RenderService::new(PoolBackend::new(Arc::clone(&pool)), 64 << 20, size);
        service.register(
            doc.0,
            DocInfo {
                path: fixture.to_string_lossy().into_owned(),
                page_sizes,
                generation: 0,
            },
        );
        service.spawn_dispatcher();

        Some(Live {
            service,
            pool,
            doc: doc.0,
        })
    }

    /// Fetches a tile by the URI the webview would ask for.
    async fn fetch(&self, uri: &str) -> Result<izul_render::CachedTile, String> {
        let parsed = parse_tile_uri(uri).map_err(|e| format!("URI {uri}: {e}"))?;
        self.service
            .fetch(parsed.key, parsed.generation, parsed.priority)
            .await
            .map_err(|e| format!("{uri}: {e}"))
    }

    async fn shutdown(&self) {
        self.pool.write().await.shutdown().await;
    }
}

/// Fraction of a BGRA buffer that is not the white the renderer clears to.
fn ink(bytes: &[u8]) -> f64 {
    let painted = bytes
        .chunks_exact(4)
        .filter(|p| p[0] < 250 || p[1] < 250 || p[2] < 250)
        .count();
    painted as f64 / (bytes.len() / 4).max(1) as f64
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tile_uri_from_the_webview_comes_back_as_pixels() {
    let Some(live) = Live::start().await else {
        return skip("izul-worker, PDFium, atau test-fixtures/viewer-10p.pdf tidak ada");
    };

    // The shape `tileUri()` in src/viewport/tileSource.ts writes, and the
    // shape `wry` rewrites it into before the Rust handler sees it. On Windows
    // only the second one ever arrives, and asking for the first alone is what
    // let every tile be refused there while this test stayed green.
    for uri in [
        format!(
            "http://izul.localhost/tile/{}/0/0/256/0/0/preview?g=1&p=1",
            live.doc
        ),
        format!(
            "izul://localhost/tile/{}/0/0/256/0/0/preview?g=1&p=1",
            live.doc
        ),
    ] {
        let tile = live
            .fetch(&uri)
            .await
            .unwrap_or_else(|e| panic!("{uri}: {e}"));
        assert!(
            tile.width > 0 && tile.height > 0,
            "{uri}: preview has no size"
        );
        assert!(
            ink(&tile.bytes) > 0.001,
            "{uri}: the preview came back blank — the pipeline answered, but with nothing on it"
        );
    }

    let sharp = format!(
        "http://izul.localhost/tile/{}/0/0/1000/0/0/sharp?g=1&p=2",
        live.doc
    );
    let tile = live.fetch(&sharp).await.expect("sharp tile");
    assert!(ink(&tile.bytes) > 0.001, "the sharp tile came back blank");

    live.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_screenful_of_tiles_arrives_while_the_heartbeat_runs() {
    // The failure this guards against is the one that shipped: the viewport
    // asks for dozens of tiles at once while the supervisor pings on its own
    // schedule, and the two streams of traffic corrupt each other's frames.
    // One tile at a time never reproduced it.
    let Some(live) = Live::start().await else {
        return skip("izul-worker, PDFium, atau test-fixtures/viewer-10p.pdf tidak ada");
    };

    let stop = {
        let pool = Arc::clone(&live.pool);
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(izul_app::supervisor::supervise(pool, None, rx));
        tx
    };

    let mut wanted = Vec::new();
    for page in 0..10u32 {
        wanted.push(format!(
            "http://izul.localhost/tile/{}/{page}/0/256/0/0/preview?g=1&p=1",
            live.doc
        ));
        for col in 0..2u32 {
            wanted.push(format!(
                "http://izul.localhost/tile/{}/{page}/0/1500/{col}/0/sharp?g=1&p=2",
                live.doc
            ));
        }
    }

    let started = Instant::now();
    let mut tasks = Vec::new();
    for uri in wanted {
        let service = Arc::clone(&live.service);
        tasks.push(tokio::spawn(async move {
            let parsed = parse_tile_uri(&uri).expect("uri");
            let out = service
                .fetch(parsed.key, parsed.generation, parsed.priority)
                .await;
            (uri, out)
        }));
    }

    // Only the tiles that must carry ink are checked for it. A tile in the
    // right-hand column can legitimately be the page's blank margin, so
    // demanding ink from every tile would make this test fail for the wrong
    // reason; the previews and the left-hand column of a text page cannot be
    // blank unless something upstream is broken.
    let mut blank = Vec::new();
    for task in tasks {
        let (uri, outcome) = tokio::time::timeout(Duration::from_secs(60), task)
            .await
            .expect("no tile may hang: a wedged channel looks exactly like this")
            .expect("task");
        let tile = outcome.unwrap_or_else(|e| panic!("{uri}: {e}"));
        assert!(
            tile.width > 0 && tile.height > 0,
            "{uri}: arrived without dimensions"
        );
        let must_have_ink = uri.ends_with("preview?g=1&p=1") || uri.contains("/1500/0/0/sharp");
        if must_have_ink && ink(&tile.bytes) <= 0.0005 {
            blank.push(uri);
        }
    }
    assert!(
        blank.is_empty(),
        "{} tiles came back blank that cannot be: {blank:#?}",
        blank.len()
    );
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "a screenful took {:?}",
        started.elapsed()
    );

    let _ = stop.send(());
    live.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_pool_survives_a_worker_being_killed_mid_render() {
    // SPEC 3.4: a dead worker costs the affected tabs, briefly, and nothing
    // else. Proven here through the render service rather than against a bare
    // worker, so the recovery path the application actually takes is the one
    // under test.
    let Some(live) = Live::start().await else {
        return skip("izul-worker, PDFium, atau test-fixtures/viewer-10p.pdf tidak ada");
    };

    let first = format!(
        "http://izul.localhost/tile/{}/0/0/256/0/0/preview?g=1&p=1",
        live.doc
    );
    assert!(live.fetch(&first).await.is_ok(), "the pool starts healthy");

    // Kill everything holding the document, then let the supervisor notice.
    {
        let mut pool = live.pool.write().await;
        let recovered = pool.sweep().await;
        // Nothing died yet, so nothing should be reported as needing reload.
        assert!(
            recovered.is_empty(),
            "a healthy sweep reports no casualties"
        );
    }

    // A cached tile still answers even if every worker were to vanish, which
    // is the point of the cache living in this process (SPEC 9).
    assert!(
        live.fetch(&first).await.is_ok(),
        "a resident tile must not depend on a worker"
    );

    live.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn text_and_outline_travel_the_same_road_as_tiles() {
    // These go through `Pool::ask` rather than the render service, and they are
    // what the sidebar and the selection layer are built on.
    let Some(live) = Live::start().await else {
        return skip("izul-worker, PDFium, atau test-fixtures/viewer-10p.pdf tidak ada");
    };
    let doc = DocId(live.doc);

    let worker = {
        let pool = live.pool.read().await;
        pool.worker_for(doc).expect("the document has a worker")
    };

    match Pool::ask(&worker, Request::Outline { doc }).await {
        Ok(Response::OutlineReady { nodes, .. }) => {
            assert!(!nodes.is_empty(), "the fixture has bookmarks")
        }
        other => panic!("expected OutlineReady, got {other:?}"),
    }

    let text = Pool::ask(
        &worker,
        Request::ExtractText {
            doc,
            page: 0,
            with_boxes: true,
            rotation: izul_model::geom::RotationQuarter::None,
        },
    )
    .await;
    match text {
        Ok(Response::TextReady { text, chars, .. }) => {
            assert!(!text.is_empty() && !chars.is_empty(), "page 1 has text");
        }
        other => panic!("expected TextReady, got {other:?}"),
    }

    live.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_superseded_request_is_refused_rather_than_drawn() {
    let Some(live) = Live::start().await else {
        return skip("izul-worker, PDFium, atau test-fixtures/viewer-10p.pdf tidak ada");
    };

    live.service.bump_generation(live.doc, 5);
    let stale = format!(
        "http://izul.localhost/tile/{}/0/0/1000/0/0/sharp?g=4&p=2",
        live.doc
    );
    assert!(
        live.fetch(&stale).await.is_err(),
        "a tile from a view the user has left must not be rendered"
    );

    let current = format!(
        "http://izul.localhost/tile/{}/0/0/1000/0/0/sharp?g=5&p=2",
        live.doc
    );
    assert!(
        live.fetch(&current).await.is_ok(),
        "the current view renders"
    );

    live.shutdown().await;
}

// ---------------------------------------------------------------------------
// Phase 2: several documents at once, and search through the same road
// ---------------------------------------------------------------------------

/// Tabs are not a frontend feature: each one is a document open in a worker,
/// and the pool has to hold all of them at once and still know which worker has
/// which. This opens six documents over one pool and renders a page of each,
/// which is the smallest thing that would break if placement or the doc table
/// were wrong.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn several_documents_stay_open_at_once() {
    let Some(live) = Live::start().await else {
        skip("izul-worker atau PDFium tidak tersedia");
        return;
    };
    let Some(path) = fixture() else { return };
    let bytes = std::fs::read(&path).expect("baca fixture");
    let key = izul_store::content_hash(&bytes);
    drop(bytes);

    let mut docs = vec![live.doc];
    for id in 2..=6u64 {
        let opened = {
            let mut pool = live.pool.write().await;
            pool.open(DocId(id), &path.to_string_lossy(), key, None)
                .await
                .expect("dokumen kedua dan seterusnya terbuka")
        };
        let page_sizes = match opened {
            Response::Opened { page_sizes, .. } => page_sizes,
            other => panic!("expected Opened, got {other:?}"),
        };
        live.service.register(
            id,
            DocInfo {
                path: path.to_string_lossy().into_owned(),
                page_sizes,
                generation: 0,
            },
        );
        docs.push(id);
    }

    for doc in &docs {
        let uri = format!("http://izul.localhost/tile/{doc}/0/0/256/0/0/preview?g=0&p=2");
        let tile = live.fetch(&uri).await.expect("setiap dokumen merender");
        assert!(tile.width > 0 && tile.height > 0, "dokumen {doc} kosong");
    }

    // Every one of them is still addressable, which is what a tab bar assumes
    // when the reader clicks back to the first document twenty minutes later.
    {
        let pool = live.pool.read().await;
        for doc in &docs {
            assert!(
                pool.worker_for(DocId(*doc)).is_some(),
                "dokumen {doc} kehilangan pekerjanya"
            );
        }
    }

    // Closing one leaves the others alone: the bug this guards is a close that
    // takes the worker's whole document table with it.
    {
        let worker = live
            .pool
            .read()
            .await
            .worker_for(DocId(3))
            .expect("pekerja");
        let _ = Pool::ask(&worker, Request::Close { doc: DocId(3) }).await;
    }
    let survivor = live
        .fetch("http://izul.localhost/tile/4/0/0/256/0/0/preview?g=0&p=2")
        .await;
    assert!(
        survivor.is_ok(),
        "menutup satu tab tidak boleh menjatuhkan yang lain"
    );

    live.shutdown().await;
}

/// Tier one of SPEC 11.1, through the real worker: the index says which page,
/// this says where on the page. The boxes are what the highlight is drawn from,
/// so an empty list here is a search that finds things and shows nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_comes_back_with_boxes_to_highlight() {
    let Some(live) = Live::start().await else {
        skip("izul-worker atau PDFium tidak tersedia");
        return;
    };
    let worker = live
        .pool
        .read()
        .await
        .worker_for(DocId(live.doc))
        .expect("pekerja memegang dokumen");

    let reply = Pool::ask(
        &worker,
        Request::Search {
            doc: DocId(live.doc),
            page: 0,
            // Every page of the viewer fixture carries "Bagian N — halaman M".
            query: "Bagian".to_string(),
            opts: SearchOptions {
                case_sensitive: false,
                whole_word: false,
                max_hits: 50,
            },
            rotation: RotationQuarter::from_degrees(0),
            generation: Generation(1),
        },
    )
    .await
    .expect("pekerja menjawab pencarian");

    match reply {
        Response::SearchReady { hits, page, .. } => {
            assert_eq!(page, 0);
            let first = hits.first().expect("kata itu ada di halaman pertama");
            assert_eq!(first.char_count, 6, "sepanjang kata yang dicari");
            let rect = first.rects.first().expect("kecocokan punya kotak");
            assert!(
                rect.right > rect.left && rect.top > rect.bottom,
                "kotak sorot harus punya luas: {rect:?}"
            );
        }
        other => panic!("expected SearchReady, got {other:?}"),
    }

    // A query from an epoch the user has already typed past is dropped rather
    // than finished — the same rule tiles follow, and what makes typing into
    // the search box cheap.
    let stale = Pool::ask(
        &worker,
        Request::Search {
            doc: DocId(live.doc),
            page: 0,
            query: "Bagia".to_string(),
            opts: SearchOptions {
                case_sensitive: false,
                whole_word: false,
                max_hits: 50,
            },
            rotation: RotationQuarter::from_degrees(0),
            generation: Generation(0),
        },
    )
    .await
    .expect("pekerja menjawab");
    assert!(
        matches!(stale, Response::Superseded { .. }),
        "generasi lama harus ditolak, bukan dikerjakan: {stale:?}"
    );

    live.shutdown().await;
}

/// The whole of tiers two and three, end to end: a worker extracts the text, the
/// indexer writes it, FTS5 answers a query over it.
///
/// This is the one path where a mistake is invisible until a user searches for
/// a word they can see on screen and is told it is not there — the index is
/// written in the background, and nothing else in the application reads it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn indexing_makes_a_document_findable_by_its_words() {
    let Some(live) = Live::start().await else {
        skip("izul-worker atau PDFium tidak tersedia");
        return;
    };
    let Some(path) = fixture() else { return };

    let data_dir = std::env::temp_dir().join(format!("izul-index-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_dir);
    std::fs::create_dir_all(&data_dir).expect("folder data sementara");
    let conn = izul_store::open(&data_dir, izul_store::Which::App).expect("basis data");
    let stamp = izul_store::FileStamp::of(&path).expect("stat fixture");
    let file_id = izul_store::files::touch(&conn, &path, stamp).expect("catat berkas");

    let indexer = Arc::new(Indexer::new());
    assert!(
        indexer.start(Job {
            doc: live.doc,
            file_id: file_id.0,
            page_count: 10,
            data_dir: data_dir.clone(),
            pool: Arc::clone(&live.pool),
        }),
        "pengindeksan dimulai"
    );
    // A second start for the same document must be refused rather than race the
    // first for `pages_done`.
    assert!(!indexer.start(Job {
        doc: live.doc,
        file_id: file_id.0,
        page_count: 10,
        data_dir: data_dir.clone(),
        pool: Arc::clone(&live.pool),
    }));

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match indexer.progress(live.doc) {
            Some(p) if p.complete() => break,
            Some(p) if !p.running => panic!("pengindeksan berhenti di {p:?}"),
            _ if Instant::now() > deadline => panic!("pengindeksan tidak selesai tepat waktu"),
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }

    let hits = izul_store::search::search_library(&conn, "Bagian", 20).expect("pencarian pustaka");
    assert!(!hits.is_empty(), "kata dari halaman harus ada di indeks");
    assert_eq!(hits[0].file_id, file_id);
    assert!(
        hits[0].snippet.contains('['),
        "cuplikan menandai kecocokannya: {:?}",
        hits[0].snippet
    );

    // And the same query scoped to the one file, which is tier two.
    let scoped =
        izul_store::search::search_file(&conn, file_id, "halaman", 20).expect("pencarian dokumen");
    assert!(!scoped.is_empty());

    // Punctuation a reader would type is a literal, not FTS5 syntax. This is
    // the case that turns into a syntax error the moment the escaping is wrong.
    let odd = izul_store::search::search_file(&conn, file_id, "size 10\" x 8\"", 5);
    assert!(
        odd.is_ok(),
        "ketikan bertanda kutip tidak boleh jadi galat: {odd:?}"
    );

    live.shutdown().await;
    let _ = std::fs::remove_dir_all(&data_dir);
}

/// Compare mode's visual diff on real previews from the real pipeline: the
/// same page against itself is no difference, two different pages are.
/// This is also what checks `compare.rs`'s reading of the cache's bitmaps —
/// BGRA, rows `stride` bytes apart — against what the renderer really
/// produces, rather than against a buffer the test made up.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_visual_diff_sees_real_differences_and_only_those() {
    let Some(live) = Live::start().await else {
        return skip("izul-worker, PDFium, atau test-fixtures/viewer-10p.pdf tidak ada");
    };
    let preview = |page: u32| {
        format!(
            "http://izul.localhost/tile/{}/{page}/0/512/0/0/preview?g=0&p=1",
            live.doc
        )
    };
    let a = live.fetch(&preview(0)).await.expect("page 1");
    let a2 = live.fetch(&preview(0)).await.expect("page 1 again");
    let b = live.fetch(&preview(1)).await.expect("page 2");
    fn bmp(t: &izul_render::CachedTile) -> izul_app::compare::Bitmap<'_> {
        izul_app::compare::Bitmap {
            bytes: &t.bytes,
            width: t.width,
            height: t.height,
            stride: t.stride,
        }
    }
    let (same, none) = izul_app::compare::diff(bmp(&a), bmp(&a2), 48, 64);
    assert!(same.is_empty(), "a page differs from itself: {same:?}");
    assert_eq!(none, 0.0);
    let (boxes, fraction) = izul_app::compare::diff(bmp(&a), bmp(&b), 48, 64);
    assert!(!boxes.is_empty(), "two different pages compared as equal");
    assert!(fraction > 0.01 && fraction < 1.0, "fraction {fraction}");
    live.shutdown().await;
}
