//! Phase 1 measurement: the viewer pipeline end to end (SPEC 13, SPEC 17).
//!
//! What is measured is the shipping pipeline, not a model of it: a real
//! sandboxed worker process, the real shared-memory ring, the real tile cache,
//! the real priority queue and the real cancellation, driven the way the
//! viewport drives them. The only missing piece is the webview, which cannot
//! run headless — so the claims that depend on a real compositor (frame pacing,
//! dropped frames) are reported as unverified rather than guessed at, and what
//! *can* be measured about them — whether a tile was ready when the frame
//! wanted it — is measured.
//!
//! Sections:
//!
//!   first-page  document open -> the first thing the user can look at, and
//!               then -> a sharp first screen. SPEC 13's 400 ms.
//!   fill        a whole viewport of tiles, cold cache and warm.
//!   zoom        after a zoom step: how soon there is something to draw, and
//!               how soon it is sharp. SPEC 13's 16 ms and 150 ms.
//!   scroll      a 60 fps scroll, counting the frames that would have shown a
//!               white page and the frames whose sharp tiles were not ready.
//!   cancel      what a fast zoom costs when superseded tiles are dropped.
//!   memory      ten documents open with the pipeline live, against 1.5 GB.
//!
//! ```text
//! cargo build --release -p izul-bench
//! ./target/release/viewport --json bench/results/phase1.json
//! ./target/release/viewport --only scroll,zoom
//! ```

mod stats;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use izul_ipc::codec::{read_frame, write_frame};
use izul_ipc::message::{DocId, Envelope, Generation, Request, RequestId, Response, SlotRef};
use izul_ipc::ring::TileRing;
use izul_ipc::{region_bytes, ChannelName, Listener, SharedRegion};
use izul_render::{
    tile_source, CachedTile, DocInfo, Priority, RenderError, RenderService, TileBackend, TileKey,
    TileKind, TILE_EDGE,
};
use parking_lot::Mutex;
use serde::Serialize;
use stats::Stats;
use tokio::sync::{mpsc, oneshot};

/// Slots per ring. The UI side copies a tile out and frees the slot at once, so
/// this only has to cover what is in flight.
const SLOTS: u32 = 64;
/// The viewport every measurement assumes: a 1600x1000 CSS window at 1x.
const VIEW_W: f32 = 1600.0;
const VIEW_H: f32 = 1000.0;
/// Maximum edge of a preview, matching `PREVIEW_EDGE` in the viewport.
const PREVIEW_EDGE: u32 = 256;
/// Cache budget for the measurements. Deliberately not the 2 GiB default: a
/// budget nothing ever reaches would measure an unbounded cache.
const CACHE_BUDGET: usize = 512 * 1024 * 1024;
const GAP: f32 = 16.0;
const PADDING: f32 = 24.0;

#[derive(Debug, Serialize)]
struct Report {
    host: HostInfo,
    results: Vec<Measurement>,
}

#[derive(Debug, Serialize)]
struct HostInfo {
    os: String,
    arch: String,
    cpu_model: String,
    logical_cores: usize,
    total_ram_mb: u64,
    profile: String,
}

#[derive(Debug, Serialize)]
struct Measurement {
    section: String,
    fixture: String,
    unit: String,
    detail: String,
    stats: Stats,
    /// Present when the measurement has a SPEC 13 target to be judged against.
    target_ms: Option<f64>,
    verdict: Option<String>,
}

impl Measurement {
    fn ms(section: &str, fixture: &str, detail: &str, stats: Stats) -> Self {
        Measurement {
            section: section.into(),
            fixture: fixture.into(),
            unit: "ms".into(),
            detail: detail.into(),
            stats,
            target_ms: None,
            verdict: None,
        }
    }

    /// Judged on p95, never on the mean: the slow frame is the one the user
    /// notices.
    fn against(mut self, target_ms: f64) -> Self {
        let pass = self.stats.p95_ms <= target_ms;
        self.verdict = Some(if pass {
            format!("LULUS vs {target_ms:.0}ms")
        } else {
            format!("GAGAL vs {target_ms:.0}ms")
        });
        self.target_ms = Some(target_ms);
        self
    }

    fn count(section: &str, fixture: &str, detail: &str, unit: &str, value: f64) -> Self {
        Measurement {
            section: section.into(),
            fixture: fixture.into(),
            unit: unit.into(),
            detail: detail.into(),
            stats: Stats::one(value),
            target_ms: None,
            verdict: None,
        }
    }
}

// ---------------------------------------------------------------------------
// A worker process, spoken to directly. This is the benchmark's `TileBackend`.
// ---------------------------------------------------------------------------

struct Job {
    request: Request,
    reply: oneshot::Sender<Result<Response, RenderError>>,
}

struct WorkerBackend {
    tx: mpsc::Sender<Job>,
    ring: Arc<TileRing>,
    _region: SharedRegion,
    child: Mutex<Child>,
    pid: u32,
}

impl Drop for WorkerBackend {
    fn drop(&mut self) {
        let mut child = self.child.lock();
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl WorkerBackend {
    async fn spawn(worker: &Path, pdfium: &Path, id: u32) -> Arc<Self> {
        let session = (std::process::id() as u64) << 16 ^ u64::from(id) ^ nanos();
        let channel = ChannelName::for_worker(session, id);
        let mut listener = Listener::bind(channel).expect("bind channel");

        let shm_name = format!("bench-{session:x}-w{id}");
        let len = region_bytes(SLOTS);
        let region = SharedRegion::create(&shm_name, len).expect("create shm");
        // SAFETY: freshly created, zeroed, correctly sized, not yet shared.
        let ring = unsafe { TileRing::initialise(region.as_ptr(), len, SLOTS) }.expect("ring");

        let child = Command::new(worker)
            .env("IZUL_WORKER_ID", id.to_string())
            .env("IZUL_SESSION_ID", session.to_string())
            .env("IZUL_SHM_NAME", &shm_name)
            .env("IZUL_SHM_SLOTS", SLOTS.to_string())
            .env("IZUL_PDFIUM_PATH", pdfium)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn worker");
        let pid = child.id();

        let mut stream = tokio::time::timeout(Duration::from_secs(20), listener.accept())
            .await
            .expect("worker connected")
            .expect("accept");

        let (tx, mut rx) = mpsc::channel::<Job>(512);
        tokio::spawn(async move {
            // The listener stays alive with the task; the worker holds the only
            // other end.
            let _listener = listener;
            let mut next = 1u64;
            while let Some(job) = rx.recv().await {
                let id = RequestId(next);
                next += 1;
                if write_frame(
                    &mut stream,
                    &Envelope {
                        id,
                        payload: job.request,
                    },
                )
                .await
                .is_err()
                {
                    let _ = job
                        .reply
                        .send(Err(RenderError::Worker("tulis gagal".into())));
                    break;
                }
                let env: Result<Envelope<Response>, _> = read_frame(&mut stream).await;
                let out = match env {
                    Ok(env) => Ok(env.payload),
                    Err(e) => Err(RenderError::Worker(e.to_string())),
                };
                let _ = job.reply.send(out);
            }
        });

        Arc::new(WorkerBackend {
            tx,
            ring: Arc::new(ring),
            _region: region,
            child: Mutex::new(child),
            pid,
        })
    }

    async fn call(&self, request: Request) -> Result<Response, RenderError> {
        let (reply, wait) = oneshot::channel();
        self.tx
            .send(Job { request, reply })
            .await
            .map_err(|_| RenderError::Worker("pekerja hilang".into()))?;
        wait.await
            .map_err(|_| RenderError::Worker("balasan hilang".into()))?
    }

    /// Copies a published tile out of the ring, exactly as the UI process does.
    fn take(&self, slot: &SlotRef) -> Result<CachedTile, RenderError> {
        let held = self.ring.acquire(slot).map_err(|_| RenderError::SlotGone)?;
        let used = slot.stride as usize * slot.height as usize;
        let bytes = held.bytes().get(..used).ok_or(RenderError::SlotGone)?;
        Ok(CachedTile {
            bytes: Arc::from(bytes),
            width: slot.width,
            height: slot.height,
            stride: slot.stride,
        })
    }

    async fn open(&self, doc: DocId, path: &Path) -> (u32, Vec<(f32, f32)>) {
        match self
            .call(Request::Open {
                doc,
                path: path.to_string_lossy().into_owned(),
                password: None,
            })
            .await
        {
            Ok(Response::Opened {
                page_count,
                page_sizes,
                ..
            }) => (page_count, page_sizes),
            other => panic!("open gagal: {other:?}"),
        }
    }
}

impl TileBackend for WorkerBackend {
    fn render<'a>(
        &'a self,
        _doc: DocId,
        request: Request,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<CachedTile, RenderError>> + Send + 'a>,
    > {
        Box::pin(async move {
            match self.call(request).await? {
                Response::TileReady { slot, .. } => self.take(&slot),
                Response::Superseded { .. } => Err(RenderError::Superseded),
                Response::Error {
                    message_id, detail, ..
                } => Err(RenderError::Worker(format!("{message_id}: {detail}"))),
                other => Err(RenderError::Worker(format!("tak terduga: {other:?}"))),
            }
        })
    }

    fn cancel<'a>(
        &'a self,
        doc: DocId,
        generation: Generation,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let _ = self.call(Request::Cancel { doc, generation }).await;
        })
    }
}

// ---------------------------------------------------------------------------
// The layout the measurements scroll through: single-page continuous, the
// default view mode and the one the targets are written about.
// ---------------------------------------------------------------------------

struct Layout {
    tops: Vec<f32>,
    sizes: Vec<(f32, f32)>,
    zoom: f32,
    height: f32,
}

impl Layout {
    fn new(sizes: &[(f32, f32)], zoom: f32) -> Self {
        let mut tops = Vec::with_capacity(sizes.len());
        let mut y = PADDING;
        for (_, h) in sizes {
            tops.push(y);
            y += h * zoom + GAP;
        }
        Layout {
            tops,
            sizes: sizes.to_vec(),
            zoom,
            height: (y - GAP + PADDING).max(1.0),
        }
    }

    fn visible(&self, scroll_y: f32) -> Vec<u32> {
        let bottom = scroll_y + VIEW_H;
        let mut out = Vec::new();
        for (i, top) in self.tops.iter().enumerate() {
            if *top > bottom {
                break;
            }
            let h = self.sizes.get(i).map(|s| s.1 * self.zoom).unwrap_or(0.0);
            if top + h >= scroll_y {
                out.push(i as u32);
            }
        }
        out
    }

    /// The tiles of `page` a viewport at `scroll_y` touches, as the renderer
    /// computes them.
    fn tiles(&self, page: u32, scroll_y: f32, ppp: f32) -> Vec<(u16, u16)> {
        let Some((w, h)) = self.sizes.get(page as usize).copied() else {
            return Vec::new();
        };
        let Some(top) = self.tops.get(page as usize).copied() else {
            return Vec::new();
        };
        let page_px_h = (h * ppp).round().max(1.0);
        let page_px_w = (w * ppp).round().max(1.0);
        // The strip of this page inside the window, in page pixels.
        let scale = ppp / self.zoom;
        let from_y = ((scroll_y - top).max(0.0) * scale).min(page_px_h);
        let to_y = ((scroll_y + VIEW_H - top).max(0.0) * scale).min(page_px_h);
        if to_y <= from_y {
            return Vec::new();
        }
        let visible_w = (VIEW_W * scale).min(page_px_w);
        let cols = (visible_w.ceil() as u32).div_ceil(TILE_EDGE).max(1);
        let first_row = from_y as u32 / TILE_EDGE;
        let last_row = (to_y.ceil() as u32).saturating_sub(1) / TILE_EDGE;
        let mut out = Vec::new();
        for row in first_row..=last_row {
            for col in 0..cols {
                if tile_source(w, h, ppp, col as u16, row as u16).is_ok() {
                    out.push((col as u16, row as u16));
                }
            }
        }
        out
    }
}

fn sharp_key(doc: u64, page: u32, ppp_milli: u32, col: u16, row: u16) -> TileKey {
    TileKey {
        doc,
        page,
        rotation: 0,
        ppp_milli,
        col,
        row,
        kind: TileKind::Sharp,
        invert: false,
    }
}

fn preview_key(doc: u64, page: u32) -> TileKey {
    TileKey {
        doc,
        page,
        rotation: 0,
        ppp_milli: PREVIEW_EDGE,
        col: 0,
        row: 0,
        kind: TileKind::Preview,
        invert: false,
    }
}

fn ppp_milli(ppp: f32) -> u32 {
    (ppp * 1000.0).round().clamp(1.0, 64_000.0) as u32
}

/// The viewport's own record of what it has: the frontend's bitmap map, which
/// is what makes a frame's decision "draw or request" without awaiting.
#[derive(Default)]
struct Held {
    ready: Mutex<HashSet<TileKey>>,
    pending: Mutex<HashSet<TileKey>>,
}

impl Held {
    fn has(&self, key: &TileKey) -> bool {
        self.ready.lock().contains(key)
    }

    fn clear(&self) {
        self.ready.lock().clear();
        self.pending.lock().clear();
    }

    /// Issues a request unless the tile is already held or in flight.
    fn want(
        self: &Arc<Self>,
        service: &Arc<RenderService>,
        key: TileKey,
        generation: u64,
        priority: Priority,
    ) {
        if self.ready.lock().contains(&key) || !self.pending.lock().insert(key) {
            return;
        }
        let held = Arc::clone(self);
        let service = Arc::clone(service);
        tokio::spawn(async move {
            let outcome = service.fetch(key, generation, priority).await;
            held.pending.lock().remove(&key);
            if outcome.is_ok() {
                held.ready.lock().insert(key);
            }
        });
    }

    /// Waits until every key is held, or the deadline passes.
    async fn wait_for(&self, keys: &[TileKey], limit: Duration) -> bool {
        let started = Instant::now();
        loop {
            if keys.iter().all(|k| self.has(k)) {
                return true;
            }
            if started.elapsed() > limit {
                return false;
            }
            tokio::time::sleep(Duration::from_micros(200)).await;
        }
    }
}

// ---------------------------------------------------------------------------

struct Bench {
    /// Held so the worker process outlives the measurements; the service keeps
    /// its own handle, but the process must not be reaped between sections.
    _backend: Arc<WorkerBackend>,
    service: Arc<RenderService>,
    held: Arc<Held>,
    sizes: Vec<(f32, f32)>,
    page_count: u32,
    fixture: String,
    doc: u64,
    /// Zoom that fits the first page's width into the benchmark's viewport.
    fit_zoom: f32,
    generation: u64,
}

impl Bench {
    async fn start(worker: &Path, pdfium: &Path, fixture: &Path, id: u32) -> Self {
        let backend = WorkerBackend::spawn(worker, pdfium, id).await;
        let doc = DocId(1);
        let (page_count, sizes) = backend.open(doc, fixture).await;
        let service = RenderService::new(
            Arc::clone(&backend) as Arc<dyn TileBackend>,
            CACHE_BUDGET,
            1,
        );
        service.register(
            doc.0,
            DocInfo {
                path: fixture.to_string_lossy().into_owned(),
                page_sizes: sizes.clone(),
                generation: 0,
            },
        );
        service.spawn_dispatcher();
        let fit_zoom = sizes
            .first()
            .map(|(w, _)| (VIEW_W - 2.0 * PADDING) / w)
            .unwrap_or(1.0);
        Bench {
            _backend: backend,
            service,
            held: Arc::new(Held::default()),
            sizes,
            page_count,
            fixture: fixture.to_string_lossy().into_owned(),
            doc: doc.0,
            fit_zoom,
            generation: 1,
        }
    }

    /// Drops everything cached, as a freshly opened document would be.
    fn cold(&self) {
        self.service.forget(self.doc);
        self.service.register(
            self.doc,
            DocInfo {
                path: self.fixture.clone(),
                page_sizes: self.sizes.clone(),
                generation: self.generation,
            },
        );
        self.held.clear();
    }

    /// The keys one screenful needs: the previews of the visible pages and
    /// their sharp tiles.
    fn screen(&self, layout: &Layout, scroll_y: f32, ppp: f32) -> (Vec<TileKey>, Vec<TileKey>) {
        let mut previews = Vec::new();
        let mut sharp = Vec::new();
        for page in layout.visible(scroll_y) {
            previews.push(preview_key(self.doc, page));
            for (col, row) in layout.tiles(page, scroll_y, ppp) {
                sharp.push(sharp_key(self.doc, page, ppp_milli(ppp), col, row));
            }
        }
        (previews, sharp)
    }

    /// Requests a screenful and waits for it, returning how long it took.
    async fn draw_screen(&self, layout: &Layout, scroll_y: f32, ppp: f32) -> (f64, f64) {
        let (previews, sharp) = self.screen(layout, scroll_y, ppp);
        let started = Instant::now();
        for key in &previews {
            self.held
                .want(&self.service, *key, self.generation, Priority::Preview);
        }
        for key in &sharp {
            self.held
                .want(&self.service, *key, self.generation, Priority::Visible);
        }
        self.held.wait_for(&previews, Duration::from_secs(10)).await;
        let previewed = started.elapsed().as_secs_f64() * 1000.0;
        self.held.wait_for(&sharp, Duration::from_secs(30)).await;
        (previewed, started.elapsed().as_secs_f64() * 1000.0)
    }

    /// SPEC 13: a 50 MB / 500 page document must reach the first page in
    /// 400 ms.
    ///
    /// Measured from the `Open` request, which is where the user's wait starts:
    /// the worker pool is already running when a file is opened, so process
    /// start belongs to cold start, not here. Everything after that is counted —
    /// parsing the document, reading every page's size for the scroll bar, the
    /// preview, and then a full screen of sharp tiles.
    async fn first_page(&self) -> Vec<Measurement> {
        let layout = Layout::new(&self.sizes, self.fit_zoom);
        let mut open_ms = Vec::new();
        let mut preview_ms = Vec::new();
        let mut sharp_ms = Vec::new();
        for _ in 0..5 {
            self.cold();
            let doc = DocId(self.doc);
            let _ = self._backend.call(Request::Close { doc }).await;
            let started = Instant::now();
            self._backend.open(doc, &PathBuf::from(&self.fixture)).await;
            let opened = started.elapsed().as_secs_f64() * 1000.0;
            open_ms.push(opened);
            let (p, s) = self.draw_screen(&layout, 0.0, self.fit_zoom).await;
            preview_ms.push(opened + p);
            sharp_ms.push(opened + s);
        }
        vec![
            Measurement::ms(
                "first-page",
                &self.fixture,
                "buka dokumen (termasuk ukuran semua halaman)",
                Stats::from_millis(open_ms),
            ),
            Measurement::ms(
                "first-page",
                &self.fixture,
                "buka -> pratinjau halaman 1",
                Stats::from_millis(preview_ms),
            )
            .against(400.0),
            Measurement::ms(
                "first-page",
                &self.fixture,
                "buka -> layar pertama tajam",
                Stats::from_millis(sharp_ms),
            )
            .against(400.0),
        ]
    }

    /// A viewport of tiles, cold and warm. The warm number is what scrolling
    /// back to a page costs.
    async fn fill(&self) -> Vec<Measurement> {
        let layout = Layout::new(&self.sizes, self.fit_zoom);
        let mut cold = Vec::new();
        let mut warm = Vec::new();
        for i in 0..8 {
            self.cold();
            let scroll = layout.height * 0.1 + i as f32 * VIEW_H;
            let (_, c) = self.draw_screen(&layout, scroll, self.fit_zoom).await;
            cold.push(c);
            // Same screen again, everything resident: a cache hit is a copy.
            self.held.clear();
            let (_, w) = self.draw_screen(&layout, scroll, self.fit_zoom).await;
            warm.push(w);
        }
        vec![
            Measurement::ms(
                "fill",
                &self.fixture,
                "layar penuh, cache dingin",
                Stats::from_millis(cold),
            ),
            Measurement::ms(
                "fill",
                &self.fixture,
                "layar penuh, cache panas",
                Stats::from_millis(warm),
            )
            .against(16.0),
        ]
    }

    /// SPEC 13: zoom responds in 16 ms and is sharp within 150 ms. The first
    /// number is the two-tier promise — the preview is already resident, so
    /// there is something to draw immediately — and the second is the sharp
    /// tiles at the new scale.
    async fn zoom(&mut self) -> Vec<Measurement> {
        let mut response = Vec::new();
        let mut sharp = Vec::new();
        let steps = [1.25f32, 1.5, 2.0, 3.0, 1.5, 1.0, 0.75, 1.0];

        self.cold();
        let base = Layout::new(&self.sizes, self.fit_zoom);
        // Settle at the starting view first, so previews are resident exactly
        // as they would be after any reading.
        self.draw_screen(&base, 0.0, self.fit_zoom).await;

        for factor in steps {
            let zoom = self.fit_zoom * factor;
            let layout = Layout::new(&self.sizes, zoom);
            self.generation += 1;
            self.service.bump_generation(self.doc, self.generation);
            let (previews, tiles) = self.screen(&layout, 0.0, zoom);

            let started = Instant::now();
            for key in &previews {
                self.held
                    .want(&self.service, *key, self.generation, Priority::Preview);
            }
            self.held.wait_for(&previews, Duration::from_secs(5)).await;
            response.push(started.elapsed().as_secs_f64() * 1000.0);

            for key in &tiles {
                self.held
                    .want(&self.service, *key, self.generation, Priority::Visible);
            }
            self.held.wait_for(&tiles, Duration::from_secs(30)).await;
            sharp.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        vec![
            Measurement::ms(
                "zoom",
                &self.fixture,
                "zoom -> ada yang bisa digambar",
                Stats::from_millis(response),
            )
            .against(16.0),
            Measurement::ms(
                "zoom",
                &self.fixture,
                "zoom -> versi tajam",
                Stats::from_millis(sharp),
            )
            .against(150.0),
        ]
    }

    /// A 60 fps scroll, which is the only honest thing that can be said about
    /// scrolling without a compositor: at each frame, did the pipeline have
    /// something to draw for every visible page, and did it have the sharp
    /// version?
    async fn scroll(&self) -> Vec<Measurement> {
        const FRAME: Duration = Duration::from_micros(16_667);
        const SPEED: f32 = 2500.0; // CSS px per second: a brisk, sustained scroll
        const FRAMES: usize = 240; // four seconds

        self.cold();
        let before = self.service.stats();
        let layout = Layout::new(&self.sizes, self.fit_zoom);
        let ppp = self.fit_zoom;
        let mut scroll_y = 0.0f32;
        let mut white_frames = 0usize;
        let mut unsharp_frames = 0usize;
        let mut frame_ms = Vec::with_capacity(FRAMES);

        for _ in 0..FRAMES {
            let frame_started = Instant::now();
            let visible = layout.visible(scroll_y);

            // The frame's own work: decide what to draw and ask for what is
            // missing. This is the part that must fit in 16 ms.
            let mut white = false;
            let mut unsharp = false;
            for page in &visible {
                let preview = preview_key(self.doc, *page);
                let has_preview = self.held.has(&preview);
                if !has_preview {
                    self.held
                        .want(&self.service, preview, self.generation, Priority::Preview);
                }
                let mut page_has_sharp = false;
                for (col, row) in layout.tiles(*page, scroll_y, ppp) {
                    let key = sharp_key(self.doc, *page, ppp_milli(ppp), col, row);
                    if self.held.has(&key) {
                        page_has_sharp = true;
                    } else {
                        unsharp = true;
                        self.held
                            .want(&self.service, key, self.generation, Priority::Visible);
                    }
                }
                if !has_preview && !page_has_sharp {
                    white = true;
                }
            }
            // Prefetch, as the viewport does: previews ahead of the scroll.
            if let Some(last) = visible.last() {
                for ahead in 1..=6u32 {
                    let page = last + ahead;
                    if page < self.page_count {
                        self.held.want(
                            &self.service,
                            preview_key(self.doc, page),
                            self.generation,
                            Priority::Prefetch,
                        );
                    }
                }
            }
            frame_ms.push(frame_started.elapsed().as_secs_f64() * 1000.0);
            if white {
                white_frames += 1;
            }
            if unsharp {
                unsharp_frames += 1;
            }

            scroll_y = (scroll_y + SPEED * FRAME.as_secs_f32()).min(layout.height - VIEW_H);
            let spent = frame_started.elapsed();
            if spent < FRAME {
                tokio::time::sleep(FRAME - spent).await;
            }
        }

        let stats = self.service.stats();
        // Deltas, so the figures describe this scroll rather than everything the
        // harness did before it.

        vec![
            // Scheduling decisions only. Painting happens in the webview, which
            // cannot run here, so this is a floor on the frame cost and is
            // reported as one — it says the bookkeeping is free, not that the
            // frame is.
            Measurement::ms(
                "scroll",
                &self.fixture,
                "keputusan per frame (tanpa menggambar)",
                Stats::from_millis(frame_ms),
            )
            .against(16.0),
            Measurement::count(
                "scroll",
                &self.fixture,
                "frame dengan halaman putih",
                "frame",
                white_frames as f64,
            ),
            Measurement::count(
                "scroll",
                &self.fixture,
                "frame tanpa ubin tajam",
                "frame",
                unsharp_frames as f64,
            ),
            // A forward scroll asks for each tile once, so the backend cache
            // cannot help it — what the cache is worth is measured by "layar
            // penuh, cache panas" above. What matters here is that the number
            // of renders the scroll demanded was one per tile and no more.
            Measurement::count(
                "scroll",
                &self.fixture,
                "ubin dirender selama scroll",
                "ubin",
                (stats.rendered - before.rendered) as f64,
            ),
            Measurement::count(
                "scroll",
                &self.fixture,
                "cache terpakai",
                "MB",
                stats.cache.bytes as f64 / (1024.0 * 1024.0),
            ),
        ]
    }

    /// A fast zoom: eight scale changes in quick succession, each superseding
    /// the last. What is measured is how much work the pipeline refuses to do.
    async fn cancel_storm(&mut self) -> Vec<Measurement> {
        self.cold();
        let before = self.service.stats();
        let started = Instant::now();
        // A wheel being spun: a step every 8 ms, at zoom levels high enough that
        // one screen is dozens of tiles. Without cancellation the pipeline would
        // owe every step's tiles; with it, only the last step's are drawn.
        for step in 0..8u32 {
            let zoom = self.fit_zoom * (1.0 + step as f32 * 0.9);
            let layout = Layout::new(&self.sizes, zoom);
            self.generation += 1;
            self.service.bump_generation(self.doc, self.generation);
            let (_, tiles) = self.screen(&layout, 0.0, zoom);
            for key in &tiles {
                self.held
                    .want(&self.service, *key, self.generation, Priority::Visible);
            }
            tokio::time::sleep(Duration::from_millis(8)).await;
        }
        // Let the final generation settle.
        let zoom = self.fit_zoom * (1.0 + 7.0 * 0.9);
        let layout = Layout::new(&self.sizes, zoom);
        let (_, final_tiles) = self.screen(&layout, 0.0, zoom);
        self.held
            .wait_for(&final_tiles, Duration::from_secs(30))
            .await;
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let after = self.service.stats();

        vec![
            Measurement::ms(
                "cancel",
                &self.fixture,
                "delapan langkah zoom -> tajam",
                Stats::one(elapsed),
            ),
            Measurement::count(
                "cancel",
                &self.fixture,
                "ubin diminta seluruhnya",
                "ubin",
                (after.rendered - before.rendered + after.superseded - before.superseded) as f64,
            ),
            Measurement::count(
                "cancel",
                &self.fixture,
                "ubin dibuang tanpa dirender",
                "ubin",
                (after.superseded - before.superseded) as f64,
            ),
            Measurement::count(
                "cancel",
                &self.fixture,
                "ubin benar-benar dirender",
                "ubin",
                (after.rendered - before.rendered) as f64,
            ),
        ]
    }
}

/// The part of SPEC 13's one-second cold start that can be measured without a
/// window: spawning a sandboxed worker, connecting its channel, mapping the
/// ring, loading PDFium, and getting an answer back.
///
/// The window, the webview and the first frame are the rest of that budget and
/// belong to a Windows run; this is the floor the rest is built on.
async fn startup(worker: &Path, pdfium: &Path) -> Vec<Measurement> {
    let mut ms = Vec::new();
    for i in 0..5u32 {
        let started = Instant::now();
        let backend = WorkerBackend::spawn(worker, pdfium, 200 + i).await;
        let _ = backend.call(Request::Ping { nonce: 1 }).await;
        ms.push(started.elapsed().as_secs_f64() * 1000.0);
        drop(backend);
    }
    vec![Measurement::ms(
        "startup",
        "pekerja",
        "spawn + kanal + PDFium + ping pertama",
        Stats::from_millis(ms),
    )
    .against(1000.0)]
}

/// SPEC 13: ten documents open, under 1.5 GB including the cache.
///
/// Both sides are counted: the worker process, which holds the mapped files and
/// PDFium's own state, and this process, which holds the tile cache and stands
/// in for the UI process.
async fn memory(worker: &Path, pdfium: &Path, fixtures: &[PathBuf]) -> Vec<Measurement> {
    let backend = WorkerBackend::spawn(worker, pdfium, 90).await;
    let service = RenderService::new(
        Arc::clone(&backend) as Arc<dyn TileBackend>,
        CACHE_BUDGET,
        1,
    );
    service.spawn_dispatcher();
    let held = Arc::new(Held::default());
    let before_self = rss_mb();
    let before_worker = rss_of(backend.pid);

    // Ten documents, cycling through whatever fixtures exist so the mix is
    // heavy rather than ten copies of the cheapest one.
    for i in 0..10u64 {
        let Some(path) = fixtures.get(i as usize % fixtures.len()) else {
            continue;
        };
        let doc = DocId(i + 1);
        let (_, sizes) = backend.open(doc, path).await;
        service.register(
            doc.0,
            DocInfo {
                path: path.to_string_lossy().into_owned(),
                page_sizes: sizes.clone(),
                generation: 0,
            },
        );
        // Each document gets a screenful rendered, as an opened tab would.
        let fit = sizes
            .first()
            .map(|(w, _)| (VIEW_W - 2.0 * PADDING) / w)
            .unwrap_or(1.0);
        let layout = Layout::new(&sizes, fit);
        let mut keys = Vec::new();
        for page in layout.visible(0.0) {
            keys.push(preview_key(doc.0, page));
            for (col, row) in layout.tiles(page, 0.0, fit) {
                keys.push(sharp_key(doc.0, page, ppp_milli(fit), col, row));
            }
        }
        for key in &keys {
            held.want(&service, *key, 0, Priority::Visible);
        }
        held.wait_for(&keys, Duration::from_secs(60)).await;
    }

    let after_self = rss_mb();
    let after_worker = rss_of(backend.pid);
    let stats = service.stats();
    vec![
        Measurement::count(
            "memory",
            "10 dokumen",
            "proses UI (termasuk cache ubin)",
            "MB",
            after_self as f64,
        ),
        Measurement::count(
            "memory",
            "10 dokumen",
            "proses pekerja",
            "MB",
            after_worker as f64,
        ),
        Measurement::count(
            "memory",
            "10 dokumen",
            "total",
            "MB",
            (after_self + after_worker) as f64,
        ),
        Measurement::count(
            "memory",
            "10 dokumen",
            "cache ubin di dalamnya",
            "MB",
            stats.cache.bytes as f64 / (1024.0 * 1024.0),
        ),
        Measurement::count(
            "memory",
            "10 dokumen",
            "kenaikan sejak kosong",
            "MB",
            (after_self + after_worker).saturating_sub(before_self + before_worker) as f64,
        ),
    ]
}

// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json_out = arg_value(&args, "--json").map(PathBuf::from);
    let only: Vec<String> = arg_value(&args, "--only")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let wants = |name: &str| only.is_empty() || only.iter().any(|o| o == name);

    let root = repo_root();
    let (worker, pdfium) = match (worker_binary(&root), pdfium_path(&root)) {
        (Some(w), Some(p)) => (w, p),
        _ => {
            eprintln!(
                "izul-worker atau PDFium tidak ditemukan.\n\
                 Jalankan: cargo build --release && ./vendor/pdfium/fetch.sh"
            );
            std::process::exit(2);
        }
    };
    let fixtures = fixtures(&root);
    if fixtures.is_empty() {
        eprintln!("test-fixtures kosong. Jalankan: python3 bench/make_fixtures.py test-fixtures");
        std::process::exit(2);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio");

    let mut results = Vec::new();
    if wants("startup") {
        results.extend(rt.block_on(startup(&worker, &pdfium)));
    }
    for (i, fixture) in fixtures.iter().enumerate() {
        rt.block_on(async {
            let mut bench = Bench::start(&worker, &pdfium, fixture, 10 + i as u32).await;
            if wants("first-page") {
                results.extend(bench.first_page().await);
            }
            if wants("fill") {
                results.extend(bench.fill().await);
            }
            if wants("zoom") {
                results.extend(bench.zoom().await);
            }
            if wants("scroll") {
                results.extend(bench.scroll().await);
            }
            if wants("cancel") {
                results.extend(bench.cancel_storm().await);
            }
        });
    }
    if wants("memory") {
        results.extend(rt.block_on(memory(&worker, &pdfium, &fixtures)));
    }

    let report = Report {
        host: host_info(),
        results,
    };
    print_table(&report);
    if let Some(path) = json_out {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                let _ = std::fs::write(&path, json);
                println!("\nhasil mentah -> {}", path.display());
            }
            Err(e) => eprintln!("tidak dapat menulis JSON: {e}"),
        }
    }
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn worker_binary(root: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "izul-worker.exe"
    } else {
        "izul-worker"
    };
    for profile in ["release", "debug"] {
        let p = root.join("target").join(profile).join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn pdfium_path(root: &Path) -> Option<PathBuf> {
    let p = if cfg!(windows) {
        root.join("vendor/pdfium/win-x64/bin/pdfium.dll")
    } else {
        root.join("vendor/pdfium/linux-x64/lib/libpdfium.so")
    };
    p.exists().then_some(p)
}

fn fixtures(root: &Path) -> Vec<PathBuf> {
    let dir = root.join("test-fixtures");
    let mut out = Vec::new();
    for name in [
        "text-500p.pdf",
        "mixed-500p.pdf",
        "scan-realistic-50mb-500p.pdf",
    ] {
        let p = dir.join(name);
        if p.exists() {
            out.push(p);
        }
    }
    out
}

fn rss_mb() -> u64 {
    rss_of(std::process::id())
}

fn rss_of(pid: u32) -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    if let Some(kb) = rest.split_whitespace().next() {
                        return kb.parse::<u64>().unwrap_or(0) / 1024;
                    }
                }
            }
        }
    }
    let _ = pid;
    0
}

fn host_info() -> HostInfo {
    let cpu_model = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "tidak diketahui".into());
    let total_ram_mb = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemTotal"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map(|kb| kb / 1024)
        .unwrap_or(0);
    HostInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_model,
        logical_cores: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0),
        total_ram_mb,
        profile: if cfg!(debug_assertions) {
            "debug".into()
        } else {
            "release".into()
        },
    }
}

fn print_table(report: &Report) {
    println!("\n{:=<86}", "");
    println!("HOST");
    println!("{:-<86}", "");
    println!(
        "{} {} · {} · {} core · {} MB RAM · profil {}",
        report.host.os,
        report.host.arch,
        report.host.cpu_model,
        report.host.logical_cores,
        report.host.total_ram_mb,
        report.host.profile
    );
    let mut section = String::new();
    for m in &report.results {
        if m.section != section {
            section = m.section.clone();
            println!("\n{:=<86}", "");
            println!("{}", section.to_uppercase());
            println!("{:-<86}", "");
        }
        let value = if m.unit == "ms" {
            m.stats.to_string()
        } else {
            format!("{:>10.1} {}", m.stats.p50_ms, m.unit)
        };
        println!(
            "{:<26} {:<32} {}  {}",
            shorten(&m.fixture),
            m.detail,
            value,
            m.verdict.clone().unwrap_or_default()
        );
    }
}

fn shorten(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}
