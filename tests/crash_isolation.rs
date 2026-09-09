//! Phase 0 exit criterion: force-killing a worker must not take the UI down.
//!
//! This drives the real pieces — a real worker process, a real command channel,
//! a real shared-memory ring, and a real PDF — because the failure this guards
//! against is precisely the one a mock would paper over.
//!
//! Run with the fixtures present:
//! ```text
//! cargo test --test crash_isolation -- --nocapture
//! ```

use std::path::{Path, PathBuf};

// Everything below drives a live worker over a Unix socket, which is what the
// five tests in this file are for. Windows has no `UnixStream`, so those tests
// — and everything only they use — are configured out there, leaving the two
// platform-neutral checks at the bottom. Without gating the imports too, an
// unused one is an error on Windows under the workspace's `-D warnings`.
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use izul_ipc::codec::{read_frame, write_frame};
#[cfg(unix)]
use izul_ipc::message::{DocId, Envelope, Generation, RenderQuality, Request, RequestId, Response};
#[cfg(unix)]
use izul_ipc::ring::TileRing;
#[cfg(unix)]
use izul_ipc::{region_bytes, ChannelName, Listener, SharedRegion};
#[cfg(unix)]
use izul_model::geom::{PdfRectF, RotationQuarter};

#[cfg(unix)]
const SLOTS: u32 = 16;

/// The workspace root.
///
/// `CARGO_MANIFEST_DIR` names the host crate `tests-integration/`, one level
/// below the fixtures and the vendored library, so it is walked up.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(unix)]
fn worker_binary() -> Option<PathBuf> {
    // The integration test runs from `target/<profile>/deps`, so the worker sits
    // two levels up.
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.parent()?;
    let name = if cfg!(windows) {
        "izul-worker.exe"
    } else {
        "izul-worker"
    };
    let path = dir.join(name);
    path.exists().then_some(path)
}

#[cfg(unix)]
fn pdfium() -> Option<PathBuf> {
    let p = if cfg!(windows) {
        repo_root().join("vendor/pdfium/win-x64/bin/pdfium.dll")
    } else {
        repo_root().join("vendor/pdfium/linux-x64/lib/libpdfium.so")
    };
    p.exists().then_some(p)
}

#[cfg(unix)]
fn fixture() -> Option<PathBuf> {
    let dir = repo_root().join("test-fixtures");
    for name in [
        "text-500p.pdf",
        "mixed-raw-500p.pdf",
        "scan-realistic-50mb-500p.pdf",
    ] {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Everything needed to talk to one live worker.
#[cfg(unix)]
struct Harness {
    child: std::process::Child,
    stream: tokio::net::UnixStream,
    _region: SharedRegion,
    ring: TileRing,
    _listener: Listener,
    next_id: u64,
}

#[cfg(unix)]
async fn spawn_worker(worker_id: u32, session: u64) -> Option<Harness> {
    let (worker, lib) = (worker_binary()?, pdfium()?);

    let channel = ChannelName::for_worker(session, worker_id);
    let mut listener = Listener::bind(channel).expect("bind channel");

    let shm_name = format!("crashtest-{session:x}-w{worker_id}");
    let len = region_bytes(SLOTS);
    let region = SharedRegion::create(&shm_name, len).expect("create shm");
    // SAFETY: freshly created, zeroed, correctly sized, not yet shared.
    let ring = unsafe { TileRing::initialise(region.as_ptr(), len, SLOTS) }.expect("init ring");

    let child = Command::new(&worker)
        .env("IZUL_WORKER_ID", worker_id.to_string())
        .env("IZUL_SESSION_ID", session.to_string())
        .env("IZUL_SHM_NAME", &shm_name)
        .env("IZUL_SHM_SLOTS", SLOTS.to_string())
        .env("IZUL_PDFIUM_PATH", &lib)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn worker");

    let mut stream = tokio::time::timeout(Duration::from_secs(15), listener.accept())
        .await
        .expect("worker connected in time")
        .expect("accept");

    // The worker announces its protocol before anything is asked of it, and
    // the supervisor refuses one whose number does not match. Asserting it
    // here means a stale worker binary fails this suite loudly instead of
    // being discovered by a user whose pages never render.
    let hello: Envelope<Response> =
        tokio::time::timeout(Duration::from_secs(10), read_frame(&mut stream))
            .await
            .expect("worker sent its protocol version")
            .expect("read hello");
    match hello.payload {
        Response::Hello { protocol, .. } => assert_eq!(
            protocol,
            izul_ipc::PROTOCOL_VERSION,
            "izul-worker is built against another protocol version; run `cargo build --workspace`"
        ),
        other => panic!("expected Hello, got {other:?}"),
    }

    Some(Harness {
        child,
        stream,
        _region: region,
        ring,
        _listener: listener,
        next_id: 1,
    })
}

#[cfg(unix)]
impl Harness {
    async fn call(&mut self, req: Request) -> Response {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        write_frame(&mut self.stream, &Envelope { id, payload: req })
            .await
            .expect("write");
        let env: Envelope<Response> =
            tokio::time::timeout(Duration::from_secs(30), read_frame(&mut self.stream))
                .await
                .expect("worker answered in time")
                .expect("read");
        env.payload
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Skips a test that cannot run because a prerequisite is missing.
///
/// A silently skipped test is a test that has stopped guarding anything, so CI
/// sets `IZUL_REQUIRE_FIXTURES=1` and a skip becomes a failure there. Locally it
/// stays a skip, because a fresh clone has no fixtures until
/// `bench/make_fixtures.py` has been run.
#[cfg(unix)]
fn skip(reason: &str) {
    if std::env::var_os("IZUL_REQUIRE_FIXTURES").is_some() {
        panic!("prasyarat hilang dan IZUL_REQUIRE_FIXTURES diset: {reason}");
    }
    eprintln!("LEWATI: {reason}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_worker_opens_renders_and_reports_honestly() {
    let Some(fixture) = fixture() else {
        return skip("test-fixtures kosong; jalankan bench/make_fixtures.py");
    };
    let Some(mut h) = spawn_worker(1, 0xA1).await else {
        return skip(
            "izul-worker atau PDFium tidak ada; jalankan cargo build dan vendor/pdfium/fetch.sh",
        );
    };

    assert_eq!(
        h.call(Request::Ping { nonce: 5 }).await,
        Response::Pong { nonce: 5 }
    );

    let doc = DocId(1);
    let opened = h
        .call(Request::Open {
            doc,
            path: fixture.to_string_lossy().into_owned(),
            password: None,
        })
        .await;
    let (page_count, page_sizes) = match opened {
        Response::Opened {
            page_count,
            page_sizes,
            ..
        } => (page_count, page_sizes),
        other => panic!("expected Opened, got {other:?}"),
    };
    assert!(page_count > 0, "the fixture must have pages");
    assert_eq!(page_sizes.len(), page_count as usize, "one size per page");

    let (w, h_pt) = page_sizes.first().copied().expect("first page");
    let tile = h
        .call(Request::RenderTile {
            doc,
            page: 0,
            source: PdfRectF::new(0.0, 0.0, w, h_pt),
            dest_w: 256,
            dest_h: 256,
            rotation: RotationQuarter::None,
            quality: RenderQuality::Sharp,
            generation: Generation(1),
        })
        .await;
    let slot = match tile {
        Response::TileReady { slot, .. } => slot,
        other => panic!("expected TileReady, got {other:?}"),
    };

    // The pixels must actually be in shared memory, and must not be a blank
    // rectangle: a renderer that silently draws nothing would otherwise pass.
    let read = h.ring.acquire(&slot).expect("acquire published tile");
    let bytes = read.bytes();
    let used = slot.stride as usize * slot.height as usize;
    let painted = bytes.get(..used).expect("slot holds the tile");
    assert!(
        painted.iter().any(|b| *b != 0),
        "the tile must not be all zeroes"
    );
    assert!(
        painted.iter().any(|b| *b != 0xFF),
        "the tile must contain something other than white background"
    );
    drop(read);

    h.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn a_request_for_a_missing_page_is_refused_not_fatal() {
    let Some(fixture) = fixture() else {
        return skip("test-fixtures kosong");
    };
    let Some(mut h) = spawn_worker(2, 0xA2).await else {
        return skip("worker tidak tersedia");
    };

    let doc = DocId(1);
    let page_count = match h
        .call(Request::Open {
            doc,
            path: fixture.to_string_lossy().into_owned(),
            password: None,
        })
        .await
    {
        Response::Opened { page_count, .. } => page_count,
        other => panic!("expected Opened, got {other:?}"),
    };

    let bad = h
        .call(Request::RenderTile {
            doc,
            page: page_count + 100,
            source: PdfRectF::new(0.0, 0.0, 612.0, 792.0),
            dest_w: 64,
            dest_h: 64,
            rotation: RotationQuarter::None,
            quality: RenderQuality::Fast,
            generation: Generation(1),
        })
        .await;
    assert!(
        matches!(bad, Response::Error { .. }),
        "expected an error, got {bad:?}"
    );

    // The worker must still be alive and answering afterwards: a bad request is
    // refused, never fatal (SPEC 15).
    assert_eq!(
        h.call(Request::Ping { nonce: 9 }).await,
        Response::Pong { nonce: 9 }
    );
    h.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn killing_one_worker_leaves_the_others_serving() {
    let Some(fixture) = fixture() else {
        return skip("test-fixtures kosong");
    };
    let Some(mut a) = spawn_worker(3, 0xA3).await else {
        return skip("worker tidak tersedia");
    };
    let Some(mut b) = spawn_worker(4, 0xA3).await else {
        return skip("worker tidak tersedia");
    };

    let path = fixture.to_string_lossy().into_owned();
    for h in [&mut a, &mut b] {
        let r = h
            .call(Request::Open {
                doc: DocId(1),
                path: path.clone(),
                password: None,
            })
            .await;
        assert!(
            matches!(r, Response::Opened { .. }),
            "both workers must open the document"
        );
    }

    // SIGKILL: no unwinding, no destructors, no cooperation. This is the shape
    // of a real PDFium crash.
    a.kill();

    // The surviving worker keeps answering, and keeps its own document.
    assert_eq!(
        b.call(Request::Ping { nonce: 42 }).await,
        Response::Pong { nonce: 42 }
    );
    let (w, h_pt) = match b
        .call(Request::Open {
            doc: DocId(2),
            path,
            password: None,
        })
        .await
    {
        Response::Opened { page_sizes, .. } => page_sizes.first().copied().expect("a page"),
        other => panic!("expected Opened, got {other:?}"),
    };
    let tile = b
        .call(Request::RenderTile {
            doc: DocId(2),
            page: 0,
            source: PdfRectF::new(0.0, 0.0, w, h_pt),
            dest_w: 128,
            dest_h: 128,
            rotation: RotationQuarter::None,
            quality: RenderQuality::Fast,
            generation: Generation(1),
        })
        .await;
    assert!(
        matches!(tile, Response::TileReady { .. }),
        "survivor must still render"
    );

    b.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn a_dead_worker_is_replaceable_and_the_document_comes_back() {
    let Some(fixture) = fixture() else {
        return skip("test-fixtures kosong");
    };
    let path = fixture.to_string_lossy().into_owned();

    let Some(mut first) = spawn_worker(5, 0xA4).await else {
        return skip("worker tidak tersedia");
    };
    let before = match first
        .call(Request::Open {
            doc: DocId(1),
            path: path.clone(),
            password: None,
        })
        .await
    {
        Response::Opened { page_count, .. } => page_count,
        other => panic!("expected Opened, got {other:?}"),
    };
    first.kill();
    drop(first);

    // A replacement binds the same worker slot and reopens the document, which
    // is exactly what the supervisor does after a crash.
    let Some(mut second) = spawn_worker(5, 0xA4).await else {
        return skip("worker pengganti tidak dapat dijalankan");
    };
    let after = match second
        .call(Request::Open {
            doc: DocId(1),
            path,
            password: None,
        })
        .await
    {
        Response::Opened { page_count, .. } => page_count,
        other => panic!("expected Opened, got {other:?}"),
    };
    assert_eq!(
        before, after,
        "the document must come back identical after a restart"
    );
    second.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn a_superseded_generation_is_dropped_rather_than_rendered() {
    let Some(fixture) = fixture() else {
        return skip("test-fixtures kosong");
    };
    let Some(mut h) = spawn_worker(6, 0xA5).await else {
        return skip("worker tidak tersedia");
    };

    let doc = DocId(1);
    let (w, h_pt) = match h
        .call(Request::Open {
            doc,
            path: fixture.to_string_lossy().into_owned(),
            password: None,
        })
        .await
    {
        Response::Opened { page_sizes, .. } => page_sizes.first().copied().expect("a page"),
        other => panic!("expected Opened, got {other:?}"),
    };

    // The user zoomed: generation 10 is now current.
    h.call(Request::Cancel {
        doc,
        generation: Generation(10),
    })
    .await;

    // A tile from the layout they left must not be rendered.
    let stale = h
        .call(Request::RenderTile {
            doc,
            page: 0,
            source: PdfRectF::new(0.0, 0.0, w, h_pt),
            dest_w: 128,
            dest_h: 128,
            rotation: RotationQuarter::None,
            quality: RenderQuality::Fast,
            generation: Generation(3),
        })
        .await;
    assert!(
        matches!(stale, Response::Superseded { .. }),
        "an old generation must be dropped, got {stale:?}"
    );

    // The current one still renders.
    let fresh = h
        .call(Request::RenderTile {
            doc,
            page: 0,
            source: PdfRectF::new(0.0, 0.0, w, h_pt),
            dest_w: 128,
            dest_h: 128,
            rotation: RotationQuarter::None,
            quality: RenderQuality::Fast,
            generation: Generation(10),
        })
        .await;
    assert!(
        matches!(fresh, Response::TileReady { .. }),
        "current generation must render"
    );
    h.kill();
}

#[test]
fn the_vendored_pdfium_is_present_for_the_target_we_ship() {
    // A missing library is a packaging bug that would otherwise only surface on
    // a user's machine, as a window that opens and then cannot render anything.
    let win = repo_root().join("vendor/pdfium/win-x64/bin/pdfium.dll");
    assert!(
        win.exists(),
        "vendor/pdfium/win-x64/bin/pdfium.dll tidak ada; jalankan vendor/pdfium/fetch.sh"
    );
    let version = repo_root().join("vendor/pdfium/win-x64/VERSION");
    let text = std::fs::read_to_string(&version).expect("VERSION file");
    assert!(
        text.contains("BUILD=7881"),
        "vendored PDFium must match the pinned tag: {text}"
    );
}

#[test]
fn fixtures_are_documented_even_when_absent() {
    let dir: &Path = &repo_root().join("test-fixtures");
    assert!(dir.exists(), "test-fixtures/ harus ada (boleh kosong)");
}
