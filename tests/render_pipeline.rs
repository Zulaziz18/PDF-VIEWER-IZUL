//! Phase 1: the render pipeline, driven against a real worker process.
//!
//! Everything here goes through the same path the viewport uses — a real
//! worker, a real command channel, a real shared-memory ring and a real PDF —
//! because the things that break in a render pipeline are exactly the things a
//! mock cannot have: a rotation applied in the wrong space, a tile grid that
//! disagrees with the backend by a pixel, a cancellation that arrives too late.
//!
//! ```text
//! cargo test --test render_pipeline -- --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use izul_ipc::codec::{read_frame, write_frame};
use izul_ipc::message::{
    DocId, Envelope, Generation, RenderQuality, Request, RequestId, Response, SlotRef,
};
use izul_ipc::ring::TileRing;
use izul_ipc::{region_bytes, ChannelName, Listener, SharedRegion};
use izul_model::geom::{PdfRectF, RotationQuarter};

const SLOTS: u32 = 16;
const TILE_EDGE: u32 = izul_ipc::ring::TILE_EDGE;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn worker_binary() -> Option<PathBuf> {
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

fn pdfium() -> Option<PathBuf> {
    let p = if cfg!(windows) {
        repo_root().join("vendor/pdfium/win-x64/bin/pdfium.dll")
    } else {
        repo_root().join("vendor/pdfium/linux-x64/lib/libpdfium.so")
    };
    p.exists().then_some(p)
}

/// The viewer fixture: a bookmark tree, mixed page sizes, one rotated page.
fn viewer_fixture() -> Option<PathBuf> {
    let p = repo_root().join("test-fixtures/viewer-10p.pdf");
    p.exists().then_some(p)
}

fn skip(reason: &str) {
    if std::env::var_os("IZUL_REQUIRE_FIXTURES").is_some() {
        panic!("prasyarat hilang dan IZUL_REQUIRE_FIXTURES diset: {reason}");
    }
    eprintln!("LEWATI: {reason}");
}

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

    let shm_name = format!("pipetest-{session:x}-w{worker_id}");
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

    async fn open(&mut self, doc: DocId, path: &Path) -> (u32, Vec<(f32, f32)>) {
        match self
            .call(Request::Open {
                doc,
                path: path.to_string_lossy().into_owned(),
                password: None,
            })
            .await
        {
            Response::Opened {
                page_count,
                page_sizes,
                ..
            } => (page_count, page_sizes),
            other => panic!("expected Opened, got {other:?}"),
        }
    }

    /// Copies a published tile out of the ring, as the UI process does.
    fn take(&self, slot: &SlotRef) -> Vec<u8> {
        let held = self.ring.acquire(slot).expect("acquire published tile");
        let used = slot.stride as usize * slot.height as usize;
        held.bytes()
            .get(..used)
            .expect("slot holds the tile")
            .to_vec()
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

/// The tile grid, exactly as `render::scheduler::tile_source` computes it.
fn tile_source(page_w: f32, page_h: f32, ppp: f32, col: u32, row: u32) -> (PdfRectF, u32, u32) {
    let pw = (page_w * ppp).round().max(1.0) as f64;
    let ph = (page_h * ppp).round().max(1.0) as f64;
    let x = f64::from(col) * f64::from(TILE_EDGE);
    let y = f64::from(row) * f64::from(TILE_EDGE);
    let w = (pw - x).min(f64::from(TILE_EDGE));
    let h = (ph - y).min(f64::from(TILE_EDGE));
    let k = f64::from(ppp);
    (
        PdfRectF::new(
            (x / k) as f32,
            (page_h as f64 - (y + h) / k) as f32,
            ((x + w) / k) as f32,
            (page_h as f64 - y / k) as f32,
        ),
        w.round() as u32,
        h.round() as u32,
    )
}

#[cfg(unix)]
#[tokio::test]
async fn the_preview_tier_answers_with_a_whole_page_small_enough_to_be_instant() {
    let Some(fixture) = viewer_fixture() else {
        return skip("test-fixtures/viewer-10p.pdf; jalankan bench/make_fixtures.py . viewer");
    };
    let Some(mut h) = spawn_worker(1, 0xB1).await else {
        return skip("izul-worker atau PDFium tidak ada");
    };
    let doc = DocId(1);
    let (_, sizes) = h.open(doc, &fixture).await;

    let reply = h
        .call(Request::RenderPreview {
            doc,
            page: 0,
            max_edge_px: 256,
            rotation: RotationQuarter::None,
            generation: Generation(1),
        })
        .await;
    let slot = match reply {
        Response::TileReady { slot, page, .. } => {
            assert_eq!(page, 0);
            slot
        }
        other => panic!("expected TileReady, got {other:?}"),
    };

    assert!(
        slot.width <= 256 && slot.height <= 256,
        "preview must fit the cap"
    );
    let (w, ph) = sizes.first().copied().expect("a first page");
    let aspect = f64::from(w) / f64::from(ph);
    let drawn = f64::from(slot.width) / f64::from(slot.height);
    assert!(
        (aspect - drawn).abs() < 0.02,
        "preview aspect {drawn:.3} should match the page's {aspect:.3}"
    );
    assert!(
        ink(&h.take(&slot)) > 0.001,
        "the preview must have content on it"
    );
    h.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn a_rotated_page_renders_turned_rather_than_stretched() {
    let Some(fixture) = viewer_fixture() else {
        return skip("test-fixtures/viewer-10p.pdf");
    };
    let Some(mut h) = spawn_worker(2, 0xB2).await else {
        return skip("izul-worker atau PDFium tidak ada");
    };
    let doc = DocId(1);
    let (_, sizes) = h.open(doc, &fixture).await;
    let (w, ph) = sizes.first().copied().expect("a first page");

    // Upright and quarter-turned previews of the same page. The turned one must
    // have the page's aspect inverted — that is the whole observable difference,
    // and it is what a matrix that ignored rotation would get wrong.
    let upright = match h
        .call(Request::RenderPreview {
            doc,
            page: 0,
            max_edge_px: 256,
            rotation: RotationQuarter::None,
            generation: Generation(1),
        })
        .await
    {
        Response::TileReady { slot, .. } => slot,
        other => panic!("expected TileReady, got {other:?}"),
    };
    let turned = match h
        .call(Request::RenderPreview {
            doc,
            page: 0,
            max_edge_px: 256,
            rotation: RotationQuarter::Cw90,
            generation: Generation(1),
        })
        .await
    {
        Response::TileReady { slot, .. } => slot,
        other => panic!("expected TileReady, got {other:?}"),
    };

    assert!(w < ph, "the fixture's first page is portrait");
    assert!(upright.height > upright.width, "upright stays portrait");
    assert!(
        turned.width > turned.height,
        "a quarter turn makes it landscape"
    );
    assert!(
        ink(&h.take(&turned)) > 0.001,
        "the turned page must still have content"
    );
    h.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn a_page_with_its_own_rotate_is_laid_out_and_drawn_the_same_way() {
    let Some(fixture) = viewer_fixture() else {
        return skip("test-fixtures/viewer-10p.pdf");
    };
    let Some(mut h) = spawn_worker(3, 0xB3).await else {
        return skip("izul-worker atau PDFium tidak ada");
    };
    let doc = DocId(1);
    let (_, sizes) = h.open(doc, &fixture).await;

    // Page 3 of the fixture carries /Rotate 90 and a landscape MediaBox, so its
    // *displayed* size is portrait. The size reported at open time is what the
    // viewport lays out with, and the render must agree with it — if the two
    // disagreed, tiles would land in the wrong place on exactly this page.
    let (w, ph) = sizes.get(2).copied().expect("the rotated page");
    let reply = h
        .call(Request::RenderPreview {
            doc,
            page: 2,
            max_edge_px: 256,
            rotation: RotationQuarter::None,
            generation: Generation(1),
        })
        .await;
    let slot = match reply {
        Response::TileReady { slot, .. } => slot,
        other => panic!("expected TileReady, got {other:?}"),
    };
    let expected = f64::from(w) / f64::from(ph);
    let drawn = f64::from(slot.width) / f64::from(slot.height);
    assert!(
        (expected - drawn).abs() < 0.02,
        "layout says {expected:.3}, render drew {drawn:.3}: the page tree and the \
         tile matrix disagree about /Rotate"
    );
    h.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn the_tiles_of_a_page_cover_it_and_line_up_with_their_neighbours() {
    let Some(fixture) = viewer_fixture() else {
        return skip("test-fixtures/viewer-10p.pdf");
    };
    let Some(mut h) = spawn_worker(4, 0xB4).await else {
        return skip("izul-worker atau PDFium tidak ada");
    };
    let doc = DocId(1);
    let (_, sizes) = h.open(doc, &fixture).await;
    let (w, ph) = sizes.first().copied().expect("a first page");

    // 1.5 pixels per point: 918 x 1188 px, so two columns and three rows, with
    // the far ones clipped. Every tile must come back at exactly the size the
    // grid predicted, or the page would show seams.
    let ppp = 1.5f32;
    let cols = ((w * ppp).round() as u32).div_ceil(TILE_EDGE);
    let rows = ((ph * ppp).round() as u32).div_ceil(TILE_EDGE);
    assert!(
        cols >= 2 && rows >= 2,
        "the fixture should need a real grid"
    );

    let mut painted = 0.0f64;
    for row in 0..rows {
        for col in 0..cols {
            let (source, dest_w, dest_h) = tile_source(w, ph, ppp, col, row);
            let reply = h
                .call(Request::RenderTile {
                    doc,
                    page: 0,
                    source,
                    dest_w,
                    dest_h,
                    rotation: RotationQuarter::None,
                    quality: RenderQuality::Sharp,
                    generation: Generation(1),
                })
                .await;
            let slot = match reply {
                Response::TileReady { slot, .. } => slot,
                other => panic!("tile {col},{row}: expected TileReady, got {other:?}"),
            };
            assert_eq!(
                (slot.width, slot.height),
                (dest_w, dest_h),
                "tile {col},{row} came back at the wrong size"
            );
            painted += ink(&h.take(&slot));
        }
    }
    assert!(painted > 0.0, "a page of text must have ink on some tile");
    h.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn work_from_a_superseded_generation_is_dropped_rather_than_rendered() {
    let Some(fixture) = viewer_fixture() else {
        return skip("test-fixtures/viewer-10p.pdf");
    };
    let Some(mut h) = spawn_worker(5, 0xB5).await else {
        return skip("izul-worker atau PDFium tidak ada");
    };
    let doc = DocId(1);
    let (_, sizes) = h.open(doc, &fixture).await;
    let (w, ph) = sizes.first().copied().expect("a first page");

    // The user zooms: the layout epoch moves to 5.
    assert!(matches!(
        h.call(Request::Cancel {
            doc,
            generation: Generation(5)
        })
        .await,
        Response::Superseded { .. }
    ));

    // A tile from the view they have left must not be rendered.
    let stale = h
        .call(Request::RenderTile {
            doc,
            page: 0,
            source: PdfRectF::new(0.0, 0.0, w, ph),
            dest_w: 256,
            dest_h: 256,
            rotation: RotationQuarter::None,
            quality: RenderQuality::Sharp,
            generation: Generation(4),
        })
        .await;
    assert!(
        matches!(stale, Response::Superseded { .. }),
        "expected the old generation to be dropped, got {stale:?}"
    );

    // The current one still is.
    let current = h
        .call(Request::RenderTile {
            doc,
            page: 0,
            source: PdfRectF::new(0.0, 0.0, w, ph),
            dest_w: 256,
            dest_h: 256,
            rotation: RotationQuarter::None,
            quality: RenderQuality::Sharp,
            generation: Generation(5),
        })
        .await;
    assert!(matches!(current, Response::TileReady { .. }));

    // A preview is exempt: it is what keeps a page from being white, and it
    // does not depend on the zoom that changed.
    let preview = h
        .call(Request::RenderPreview {
            doc,
            page: 1,
            max_edge_px: 128,
            rotation: RotationQuarter::None,
            generation: Generation(1),
        })
        .await;
    assert!(
        matches!(preview, Response::TileReady { .. }),
        "a preview must survive a superseded generation, got {preview:?}"
    );
    h.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn the_outline_comes_back_as_a_navigable_tree() {
    let Some(fixture) = viewer_fixture() else {
        return skip("test-fixtures/viewer-10p.pdf");
    };
    let Some(mut h) = spawn_worker(6, 0xB6).await else {
        return skip("izul-worker atau PDFium tidak ada");
    };
    let doc = DocId(1);
    let (page_count, _) = h.open(doc, &fixture).await;

    let nodes = match h.call(Request::Outline { doc }).await {
        Response::OutlineReady { nodes, .. } => nodes,
        other => panic!("expected OutlineReady, got {other:?}"),
    };
    assert!(!nodes.is_empty(), "the fixture has bookmarks");
    assert!(
        nodes.iter().any(|n| n.depth > 0),
        "the fixture nests its bookmarks, and the depth must survive the wire"
    );
    for node in &nodes {
        assert!(!node.title.is_empty(), "every entry must be clickable text");
        if let Some(page) = node.page {
            assert!(
                page < page_count,
                "a destination must be inside the document"
            );
        }
    }
    h.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn text_boxes_land_on_the_page_they_describe() {
    let Some(fixture) = viewer_fixture() else {
        return skip("test-fixtures/viewer-10p.pdf");
    };
    let Some(mut h) = spawn_worker(7, 0xB7).await else {
        return skip("izul-worker atau PDFium tidak ada");
    };
    let doc = DocId(1);
    let (_, sizes) = h.open(doc, &fixture).await;
    let (w, ph) = sizes.first().copied().expect("a first page");

    let (text, chars) = match h
        .call(Request::ExtractText {
            doc,
            page: 0,
            with_boxes: true,
            rotation: RotationQuarter::None,
        })
        .await
    {
        Response::TextReady { text, chars, .. } => (text, chars),
        other => panic!("expected TextReady, got {other:?}"),
    };
    assert!(!text.is_empty(), "the fixture's first page has text");
    assert!(
        !chars.is_empty(),
        "the selection layer needs character boxes"
    );
    for c in &chars {
        assert!(
            c.rect.left >= -1.0
                && c.rect.bottom >= -1.0
                && c.rect.right <= w + 1.0
                && c.rect.top <= ph + 1.0,
            "character {:?} at {:?} falls outside the {w}x{ph} page",
            c.unicode,
            c.rect
        );
    }

    // Turned a quarter, the same characters must be inside the *turned* page,
    // which is the page's height by its width.
    let turned = match h
        .call(Request::ExtractText {
            doc,
            page: 0,
            with_boxes: true,
            rotation: RotationQuarter::Cw90,
        })
        .await
    {
        Response::TextReady { chars, .. } => chars,
        other => panic!("expected TextReady, got {other:?}"),
    };
    assert_eq!(
        turned.len(),
        chars.len(),
        "rotation must not lose characters"
    );
    for c in &turned {
        assert!(
            c.rect.right <= ph + 1.0 && c.rect.top <= w + 1.0,
            "rotated character {:?} at {:?} falls outside the {ph}x{w} page",
            c.unicode,
            c.rect
        );
    }
    h.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn a_thumbnail_sweep_does_not_leave_the_whole_document_resident() {
    let Some(fixture) = viewer_fixture() else {
        return skip("test-fixtures/viewer-10p.pdf");
    };
    let Some(mut h) = spawn_worker(8, 0xB8).await else {
        return skip("izul-worker atau PDFium tidak ada");
    };
    let doc = DocId(1);
    let (page_count, _) = h.open(doc, &fixture).await;

    // Every page, as the sidebar asks for them. The worker releases each page
    // after drawing its preview; without that, opening the sidebar on a
    // 500-page document would hold 500 parsed pages in the worker.
    for page in 0..page_count {
        let reply = h
            .call(Request::RenderPreview {
                doc,
                page,
                max_edge_px: 128,
                rotation: RotationQuarter::None,
                generation: Generation(1),
            })
            .await;
        assert!(
            matches!(reply, Response::TileReady { .. }),
            "page {page}: {reply:?}"
        );
    }
    // The ring has 16 slots and we asked for more previews than that, so this
    // also proves slots are being recycled rather than exhausted.
    assert!(page_count > SLOTS / 2);
    h.kill();
}

/// The whole pipeline, at the zoom levels that used to render white.
///
/// `izul-pdf` pins the matrix itself with pixel tests, but the tile grid, the
/// source rect it derives, and the IPC round trip all sit between the viewport
/// and that matrix. A tile of the page's top-left corner — the first thing a
/// reader sees — must carry ink at every zoom, not just at 100 %.
#[cfg(unix)]
#[tokio::test]
async fn a_zoomed_tile_of_the_top_left_corner_is_never_blank() {
    let Some(fixture) = viewer_fixture() else {
        return skip("fixture");
    };
    let Some(mut h) = spawn_worker(30, 0xC1).await else {
        return skip("worker");
    };
    let doc = DocId(1);
    let (_, sizes) = h.open(doc, &fixture).await;
    let (w, ph) = sizes.first().copied().expect("page");

    for ppp in [1.0f32, 1.5, 2.0, 3.0] {
        let (source, dw, dh) = tile_source(w, ph, ppp, 0, 0);
        let reply = h
            .call(Request::RenderTile {
                doc,
                page: 0,
                source,
                dest_w: dw,
                dest_h: dh,
                rotation: RotationQuarter::None,
                quality: RenderQuality::Sharp,
                generation: Generation(1),
            })
            .await;
        match reply {
            Response::TileReady { slot, .. } => {
                let bytes = h.take(&slot);
                let got = ink(&bytes);
                assert!(
                    got > 0.001,
                    "zoom {ppp}x: ubin kiri-atas kosong (tinta {got:.4}), source {source:?}, \
                     dest {dw}x{dh}"
                );
            }
            other => panic!("zoom {ppp}x: {other:?}"),
        }
    }
    h.kill();
}
