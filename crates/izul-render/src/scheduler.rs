//! Scheduling, coalescing and cancellation for the render pipeline (SPEC 9).
//!
//! Between the viewport and the workers sits one decision: of everything that
//! has been asked for, what should a worker do next? Four rules answer it.
//!
//! * **Cache first.** A tile that is already resident never becomes a job.
//! * **Coalesce.** Two requests for the same tile share one render. The
//!   viewport asks for the same tile from several places — the visible pass and
//!   the prefetch pass overlap by design — and rendering it twice would be pure
//!   waste.
//! * **Priority.** What is on screen outruns what is merely predicted. A
//!   prefetch that has not started yet loses its place to a visible tile every
//!   time.
//! * **Generation.** A job whose layout epoch has been superseded is dropped
//!   here, in the UI process, before it costs an IPC round trip — the cheapest
//!   possible place to cancel. The worker applies the same rule again for jobs
//!   already in its queue (SPEC 6).
//!
//! Concurrency is capped at one outstanding request per worker, because a
//! worker renders on one thread: a second request would only queue inside it,
//! where we can no longer reprioritise it.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use izul_ipc::message::{DocId, Generation, RenderQuality, Request};
use izul_model::geom::{PdfRectF, RotationQuarter};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{oneshot, Notify, Semaphore};

use crate::cache::{CacheStats, CachedTile, TileCache, TileKey, TileKind};

/// What the scheduler needs from its host in order to draw anything.
///
/// One method for pixels and one for cancellation, because that is genuinely
/// all of it. The UI process implements this over the sandboxed worker pool;
/// the benchmark harness implements it over a single worker it spawned itself.
/// Everything above — caching, priority, coalescing, cancellation — is
/// identical in both, which is the point: the numbers the benchmark reports are
/// measured through the code the application actually runs.
pub trait TileBackend: Send + Sync + 'static {
    /// Renders one request and returns the bitmap.
    ///
    /// A hand-written boxed future rather than an `async fn` in a trait, so the
    /// trait stays object-safe and the scheduler can hold an
    /// `Arc<dyn TileBackend>`.
    fn render<'a>(
        &'a self,
        doc: DocId,
        request: Request,
    ) -> Pin<Box<dyn Future<Output = Result<CachedTile, RenderError>> + Send + 'a>>;

    /// Tells the worker holding `doc` to drop work older than `generation`.
    ///
    /// Best-effort: the scheduler already refuses to dispatch superseded jobs,
    /// so this only saves work a worker has already accepted.
    fn cancel<'a>(
        &'a self,
        doc: DocId,
        generation: Generation,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// Edge length of a tile in pixels. Must match `izul_ipc::ring::TILE_EDGE` and
/// `TILE_EDGE` in `src/viewport/geometry.ts`; the three are checked against
/// each other by a test in this module and one in the frontend suite.
pub const TILE_EDGE: u32 = izul_ipc::ring::TILE_EDGE;

/// Longest queue we will hold before shedding work.
///
/// A viewport-full of tiles at 1600 % is a few dozen; the rest of this is
/// prefetch. Beyond it the user has scrolled far enough that the oldest
/// predictions are worthless, so holding them costs memory and delays the
/// tiles that are actually on screen.
const MAX_QUEUE: usize = 512;

/// How urgent a request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[repr(u8)]
pub enum Priority {
    /// Rendered ahead of where the user is looking.
    Prefetch = 0,
    /// The low-resolution tier for a page coming into view.
    Preview = 1,
    /// On screen now.
    Visible = 2,
}

impl Priority {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Priority::Prefetch,
            1 => Priority::Preview,
            _ => Priority::Visible,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("dokumen {0} tidak terbuka")]
    UnknownDocument(u64),
    #[error("halaman {page} di luar jangkauan ({page_count} halaman)")]
    PageOutOfRange { page: u32, page_count: u32 },
    #[error("permintaan sudah kedaluwarsa")]
    Superseded,
    #[error("antrean render penuh")]
    Busy,
    #[error("ubin tidak lagi ada di memori bersama")]
    SlotGone,
    #[error("pekerja: {0}")]
    Worker(String),
    #[error("permintaan dibatalkan")]
    Cancelled,
}

/// What a document contributes to scheduling: where it is and how big its pages
/// are.
///
/// Page sizes live here rather than in the frontend's request because the tile
/// grid must be derived identically on both sides. Sending a floating-point
/// source rectangle over the wire would let the two drift apart by a rounding
/// step, which shows on screen as a hairline seam between tiles.
#[derive(Debug, Clone)]
pub struct DocInfo {
    pub path: String,
    /// Display size of each page in points, with the page's own `/Rotate`
    /// already applied.
    pub page_sizes: Vec<(f32, f32)>,
    pub generation: u64,
}

/// What the worker is being asked to draw.
#[derive(Debug, Clone, Copy)]
enum JobSpec {
    Tile {
        source: PdfRectF,
        dest_w: u32,
        dest_h: u32,
        rotation: RotationQuarter,
        quality: RenderQuality,
    },
    Preview {
        max_edge_px: u32,
        rotation: RotationQuarter,
    },
}

#[derive(Debug)]
struct QueuedJob {
    key: TileKey,
    generation: u64,
    spec: JobSpec,
}

type Waiter = oneshot::Sender<Result<CachedTile, RenderError>>;

#[derive(Debug, Default)]
struct Queue {
    /// Keyed by `(255 - priority, arrival)`, so the first entry is the most
    /// urgent job that has waited longest and the last is the least urgent.
    pending: BTreeMap<(u8, u64), QueuedJob>,
    inflight: HashMap<TileKey, Vec<Waiter>>,
    next_seq: u64,
}

#[derive(Debug, Default, Serialize)]
struct Counters {
    rendered: AtomicU64,
    superseded: AtomicU64,
    shed: AtomicU64,
    failed: AtomicU64,
    coalesced: AtomicU64,
}

/// What the status bar and the benchmark harness read.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct RenderStats {
    pub cache: CacheStats,
    pub queued: usize,
    pub inflight: usize,
    pub rendered: u64,
    pub superseded: u64,
    pub shed: u64,
    pub failed: u64,
    pub coalesced: u64,
}

pub struct RenderService {
    backend: Arc<dyn TileBackend>,
    docs: Mutex<HashMap<u64, DocInfo>>,
    cache: Mutex<TileCache>,
    queue: Mutex<Queue>,
    wake: Notify,
    permits: Arc<Semaphore>,
    counters: Counters,
}

impl std::fmt::Debug for RenderService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderService")
            .field("docs", &self.docs.lock().len())
            .field("queued", &self.queue.lock().pending.len())
            .finish_non_exhaustive()
    }
}

impl RenderService {
    pub fn new(
        backend: Arc<dyn TileBackend>,
        budget_bytes: usize,
        concurrency: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            backend,
            docs: Mutex::new(HashMap::new()),
            cache: Mutex::new(TileCache::new(budget_bytes)),
            queue: Mutex::new(Queue::default()),
            wake: Notify::new(),
            permits: Arc::new(Semaphore::new(concurrency.max(1))),
            counters: Counters::default(),
        })
    }

    /// Starts the dispatcher. One task; the work itself runs in spawned jobs.
    pub fn spawn_dispatcher(self: &Arc<Self>) {
        let me = Arc::clone(self);
        tokio::spawn(async move { me.dispatch_loop().await });
    }

    // ---- document registry -------------------------------------------------

    pub fn register(&self, doc: u64, info: DocInfo) {
        self.docs.lock().insert(doc, info);
    }

    pub fn forget(&self, doc: u64) {
        self.docs.lock().remove(&doc);
        let dropped = self.cache.lock().forget_document(doc);
        tracing::debug!(doc, dropped, "cache dokumen dilepas");
    }

    /// Forgets the cached tiles of one page whose content changed.
    pub fn invalidate_page(&self, doc: u64, page: u32) -> usize {
        self.cache.lock().forget_page(doc, page)
    }

    /// Releases a tab's full-resolution bitmaps, keeping its previews (SPEC 10).
    pub fn trim(&self, doc: u64) -> usize {
        self.cache.lock().trim_document(doc)
    }

    pub fn path_of(&self, doc: u64) -> Option<String> {
        self.docs.lock().get(&doc).map(|d| d.path.clone())
    }

    /// Records a new layout epoch and tells the worker to drop older jobs.
    ///
    /// Both halves matter: ours stops queued jobs from being dispatched, the
    /// worker's stops jobs it has already accepted.
    pub fn bump_generation(self: &Arc<Self>, doc: u64, generation: u64) {
        {
            let mut docs = self.docs.lock();
            let Some(info) = docs.get_mut(&doc) else {
                return;
            };
            if generation <= info.generation {
                return;
            }
            info.generation = generation;
        }
        let backend = Arc::clone(&self.backend);
        tokio::spawn(async move {
            backend.cancel(DocId(doc), Generation(generation)).await;
        });
    }

    fn current_generation(&self, doc: u64) -> u64 {
        self.docs.lock().get(&doc).map_or(0, |d| d.generation)
    }

    /// Display size of a page at an extra rotation, in points.
    pub fn display_size(
        &self,
        doc: u64,
        page: u32,
        rotation: RotationQuarter,
    ) -> Option<(f32, f32)> {
        let docs = self.docs.lock();
        let info = docs.get(&doc)?;
        let (w, h) = *info.page_sizes.get(page as usize)?;
        Some(if rotation.swaps_axes() {
            (h, w)
        } else {
            (w, h)
        })
    }

    // ---- the public request path -------------------------------------------

    /// Returns a tile, rendering it if it is not already cached.
    pub async fn fetch(
        self: &Arc<Self>,
        key: TileKey,
        generation: u64,
        priority: Priority,
    ) -> Result<CachedTile, RenderError> {
        if let Some(hit) = self.cache.lock().get(&key) {
            return Ok(hit);
        }
        // A preview is exempt from the generation gate on purpose: it depends on
        // neither zoom nor scroll, and it is the thing that stops the viewport
        // showing a white page. Dropping it because the user kept scrolling
        // would defeat the tier.
        if key.kind != TileKind::Preview && generation < self.current_generation(key.doc) {
            self.counters.superseded.fetch_add(1, Ordering::Relaxed);
            return Err(RenderError::Superseded);
        }
        let spec = self.job_spec(&key)?;

        let (tx, rx) = oneshot::channel();
        {
            let mut q = self.queue.lock();
            if let Some(waiters) = q.inflight.get_mut(&key) {
                waiters.push(tx);
                self.counters.coalesced.fetch_add(1, Ordering::Relaxed);
            } else {
                q.inflight.insert(key, vec![tx]);
                let seq = q.next_seq;
                q.next_seq += 1;
                q.pending.insert(
                    (255 - priority as u8, seq),
                    QueuedJob {
                        key,
                        generation,
                        spec,
                    },
                );
                self.shed_if_overfull(&mut q);
            }
        }
        self.wake.notify_one();
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(RenderError::Cancelled),
        }
    }

    /// Drops the least urgent oldest job when the queue is over its cap.
    ///
    /// The job that loses is the oldest of the lowest-priority class: a
    /// prediction made for a scroll position the user has already left.
    fn shed_if_overfull(&self, q: &mut Queue) {
        while q.pending.len() > MAX_QUEUE {
            let Some((&(bucket, _), _)) = q.pending.iter().next_back() else {
                return;
            };
            let victim = q
                .pending
                .range((bucket, 0)..=(bucket, u64::MAX))
                .next()
                .map(|(k, _)| *k);
            let Some(victim) = victim else { return };
            let Some(job) = q.pending.remove(&victim) else {
                return;
            };
            self.counters.shed.fetch_add(1, Ordering::Relaxed);
            Self::resolve(q, job.key, Err(RenderError::Busy));
        }
    }

    /// Builds the render request for a tile from its key and the document's
    /// page sizes.
    ///
    /// This is the one place the tile grid is defined on the backend, and it
    /// mirrors `tilesCovering` in `src/viewport/geometry.ts` exactly.
    fn job_spec(&self, key: &TileKey) -> Result<JobSpec, RenderError> {
        let rotation = RotationQuarter::from_degrees(i32::from(key.rotation) * 90);
        let docs = self.docs.lock();
        let info = docs
            .get(&key.doc)
            .ok_or(RenderError::UnknownDocument(key.doc))?;
        let page_count = info.page_sizes.len() as u32;
        let (w, h) =
            *info
                .page_sizes
                .get(key.page as usize)
                .ok_or(RenderError::PageOutOfRange {
                    page: key.page,
                    page_count,
                })?;
        drop(docs);
        let (dw, dh) = if rotation.swaps_axes() {
            (h, w)
        } else {
            (w, h)
        };
        if !(dw.is_finite() && dh.is_finite() && dw > 0.0 && dh > 0.0) {
            return Err(RenderError::PageOutOfRange {
                page: key.page,
                page_count,
            });
        }

        if key.kind == TileKind::Preview {
            return Ok(JobSpec::Preview {
                max_edge_px: key.ppp_milli.clamp(16, TILE_EDGE),
                rotation,
            });
        }

        let ppp = key.ppp_milli as f32 / 1000.0;
        let (source, dest_w, dest_h) = tile_source(dw, dh, ppp, key.col, key.row)?;
        Ok(JobSpec::Tile {
            source,
            dest_w,
            dest_h,
            rotation,
            quality: if key.kind == TileKind::Sharp {
                RenderQuality::Sharp
            } else {
                RenderQuality::Fast
            },
        })
    }

    // ---- the dispatcher ----------------------------------------------------

    async fn dispatch_loop(self: Arc<Self>) {
        loop {
            let Ok(permit) = Arc::clone(&self.permits).acquire_owned().await else {
                return; // the semaphore was closed: shutting down
            };
            // Subscribe before checking, so a job queued between the check and
            // the wait still wakes us.
            let waiting = self.wake.notified();
            let Some(job) = self.take_next() else {
                drop(permit);
                waiting.await;
                continue;
            };
            let me = Arc::clone(&self);
            tokio::spawn(async move {
                me.run(job).await;
                drop(permit);
            });
        }
    }

    /// Pops the most urgent live job, discarding superseded ones on the way.
    ///
    /// Exactly one lock is held at a time, deliberately. Every other path takes
    /// the document table before the queue; holding the queue while asking the
    /// document table for a generation would be the opposite order, and two
    /// opposite orders are how a deadlock is built.
    fn take_next(&self) -> Option<QueuedJob> {
        loop {
            let job = {
                let mut q = self.queue.lock();
                let key = *q.pending.keys().next()?;
                q.pending.remove(&key)?
            };
            let stale = job.key.kind != TileKind::Preview
                && job.generation < self.current_generation(job.key.doc);
            if stale {
                self.counters.superseded.fetch_add(1, Ordering::Relaxed);
                let mut q = self.queue.lock();
                Self::resolve(&mut q, job.key, Err(RenderError::Superseded));
                continue;
            }
            return Some(job);
        }
    }

    async fn run(self: &Arc<Self>, job: QueuedJob) {
        let result = self.render(&job).await;
        match &result {
            Ok(tile) => {
                self.counters.rendered.fetch_add(1, Ordering::Relaxed);
                self.cache.lock().insert(job.key, tile.clone());
            }
            Err(RenderError::Superseded) => {
                self.counters.superseded.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                self.counters.failed.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(doc = job.key.doc, page = job.key.page, error = %e, "render gagal");
            }
        }
        let mut q = self.queue.lock();
        Self::resolve(&mut q, job.key, result);
    }

    /// Hands one outcome to everyone waiting on a tile.
    fn resolve(q: &mut Queue, key: TileKey, result: Result<CachedTile, RenderError>) {
        let Some(waiters) = q.inflight.remove(&key) else {
            return;
        };
        let mut result = Some(result);
        let last = waiters.len().saturating_sub(1);
        for (i, waiter) in waiters.into_iter().enumerate() {
            let payload = if i == last {
                result.take()
            } else {
                result.as_ref().map(clone_outcome)
            };
            if let Some(payload) = payload {
                let _ = waiter.send(payload);
            }
        }
    }

    async fn render(&self, job: &QueuedJob) -> Result<CachedTile, RenderError> {
        let doc = DocId(job.key.doc);
        let request = match job.spec {
            JobSpec::Tile {
                source,
                dest_w,
                dest_h,
                rotation,
                quality,
            } => Request::RenderTile {
                doc,
                page: job.key.page,
                source,
                dest_w,
                dest_h,
                rotation,
                quality,
                generation: Generation(job.generation),
                invert: job.key.invert,
            },
            JobSpec::Preview {
                max_edge_px,
                rotation,
            } => Request::RenderPreview {
                doc,
                page: job.key.page,
                max_edge_px,
                rotation,
                generation: Generation(job.generation),
                invert: job.key.invert,
            },
        };
        self.backend.render(doc, request).await
    }

    pub fn stats(&self) -> RenderStats {
        let q = self.queue.lock();
        RenderStats {
            cache: self.cache.lock().stats(),
            queued: q.pending.len(),
            inflight: q.inflight.len(),
            rendered: self.counters.rendered.load(Ordering::Relaxed),
            superseded: self.counters.superseded.load(Ordering::Relaxed),
            shed: self.counters.shed.load(Ordering::Relaxed),
            failed: self.counters.failed.load(Ordering::Relaxed),
            coalesced: self.counters.coalesced.load(Ordering::Relaxed),
        }
    }
}

/// `Result` is not `Clone` because `RenderError` carries a `String`; this is the
/// explicit, lossless copy used to answer several waiters at once.
fn clone_outcome(r: &Result<CachedTile, RenderError>) -> Result<CachedTile, RenderError> {
    match r {
        Ok(t) => Ok(t.clone()),
        Err(RenderError::UnknownDocument(d)) => Err(RenderError::UnknownDocument(*d)),
        Err(RenderError::PageOutOfRange { page, page_count }) => Err(RenderError::PageOutOfRange {
            page: *page,
            page_count: *page_count,
        }),
        Err(RenderError::Superseded) => Err(RenderError::Superseded),
        Err(RenderError::Busy) => Err(RenderError::Busy),
        Err(RenderError::SlotGone) => Err(RenderError::SlotGone),
        Err(RenderError::Worker(m)) => Err(RenderError::Worker(m.clone())),
        Err(RenderError::Cancelled) => Err(RenderError::Cancelled),
    }
}

/// The source rectangle and pixel size of one tile.
///
/// Pure, and the exact mirror of `tilesCovering` in
/// `src/viewport/geometry.ts`. Both sides must agree to the pixel: if the
/// backend thinks a tile is one pixel wider than the frontend does, the seam
/// shows as a hairline on screen at every tile boundary.
///
/// `page_w`/`page_h` are display-space points; `ppp` is pixels per point.
pub fn tile_source(
    page_w: f32,
    page_h: f32,
    ppp: f32,
    col: u16,
    row: u16,
) -> Result<(PdfRectF, u32, u32), RenderError> {
    let pw = (page_w * ppp).round().max(1.0);
    let ph = (page_h * ppp).round().max(1.0);
    let x = f64::from(col) * f64::from(TILE_EDGE);
    let y = f64::from(row) * f64::from(TILE_EDGE);
    if x >= f64::from(pw) || y >= f64::from(ph) {
        return Err(RenderError::Superseded);
    }
    let w = (f64::from(pw) - x).min(f64::from(TILE_EDGE));
    let h = (f64::from(ph) - y).min(f64::from(TILE_EDGE));
    let k = f64::from(ppp);
    let left = x / k;
    let right = (x + w) / k;
    // Device y grows down from the page top; PDF y grows up from the bottom.
    let top = f64::from(page_h) - y / k;
    let bottom = f64::from(page_h) - (y + h) / k;
    let source = PdfRectF::new(left as f32, bottom as f32, right as f32, top as f32);
    if !source.is_valid() {
        return Err(RenderError::Superseded);
    }
    Ok((source, w.round() as u32, h.round() as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tile_edge_agrees_with_the_shared_memory_ring() {
        // A mismatch would mean a tile that does not fit its slot, caught at
        // run time as a truncated render rather than here.
        assert_eq!(TILE_EDGE, izul_ipc::ring::TILE_EDGE);
        assert_eq!(
            izul_ipc::ring::SLOT_BYTES,
            (TILE_EDGE * TILE_EDGE) as usize * 4
        );
    }

    #[test]
    fn the_first_tile_starts_at_the_page_top_left() {
        let (src, w, h) = tile_source(612.0, 792.0, 1.0, 0, 0).expect("tile");
        assert_eq!((w, h), (512, 512));
        assert!((src.left - 0.0).abs() < 1e-3);
        assert!((src.top - 792.0).abs() < 1e-3, "top = {}", src.top);
        assert!((src.bottom - 280.0).abs() < 1e-3, "bottom = {}", src.bottom);
    }

    #[test]
    fn the_last_tile_is_clipped_to_the_page() {
        // 612x792 at 1 ppp is 612x792 px: two columns, two rows, and the far
        // ones are stubs. A tile that claimed a full 512 would ask the worker
        // to draw past the page edge.
        let (src, w, h) = tile_source(612.0, 792.0, 1.0, 1, 1).expect("tile");
        assert_eq!((w, h), (100, 280));
        assert!((src.right - 612.0).abs() < 1e-3);
        assert!((src.bottom - 0.0).abs() < 1e-3);
    }

    #[test]
    fn tiles_tile_the_page_without_gaps_or_overlap() {
        let (pw, ph, ppp) = (612.0f32, 792.0f32, 1.5f32);
        let cols = ((pw * ppp).round() as u32).div_ceil(TILE_EDGE);
        let rows = ((ph * ppp).round() as u32).div_ceil(TILE_EDGE);
        let mut covered = 0.0f64;
        for row in 0..rows {
            for col in 0..cols {
                let (src, w, h) = tile_source(pw, ph, ppp, col as u16, row as u16).expect("tile");
                covered += f64::from(src.width()) * f64::from(src.height());
                assert!(src.left >= -1e-3 && src.right <= f32::from(612u16) + 1e-3);
                assert!(w <= TILE_EDGE && h <= TILE_EDGE);
            }
        }
        let page_area = f64::from(pw) * f64::from(ph);
        assert!(
            (covered - page_area).abs() / page_area < 1e-3,
            "tiles covered {covered} of {page_area}"
        );
    }

    #[test]
    fn a_tile_beyond_the_page_is_refused() {
        assert!(tile_source(612.0, 792.0, 1.0, 9, 0).is_err());
        assert!(tile_source(612.0, 792.0, 1.0, 0, 9).is_err());
    }

    #[test]
    fn priority_orders_visible_ahead_of_prefetch() {
        assert!(Priority::Visible > Priority::Preview);
        assert!(Priority::Preview > Priority::Prefetch);
        // The queue's key is `255 - priority`, so a smaller key is more urgent.
        assert!((255 - Priority::Visible as u8) < (255 - Priority::Prefetch as u8));
    }

    // ---- the scheduler itself, against a controllable backend --------------

    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    /// A backend that counts renders and can be made slow, so the ordering and
    /// coalescing rules can be observed rather than assumed.
    struct FakeBackend {
        rendered: AtomicUsize,
        cancels: AtomicUsize,
        delay: Duration,
        order: Mutex<Vec<u32>>,
    }

    impl FakeBackend {
        fn new(delay_ms: u64) -> Arc<Self> {
            Arc::new(FakeBackend {
                rendered: AtomicUsize::new(0),
                cancels: AtomicUsize::new(0),
                delay: Duration::from_millis(delay_ms),
                order: Mutex::new(Vec::new()),
            })
        }

        fn count(&self) -> usize {
            self.rendered.load(Ordering::Relaxed)
        }
    }

    impl TileBackend for FakeBackend {
        fn render<'a>(
            &'a self,
            _doc: DocId,
            request: Request,
        ) -> Pin<Box<dyn Future<Output = Result<CachedTile, RenderError>> + Send + 'a>> {
            Box::pin(async move {
                let page = match request {
                    Request::RenderTile { page, .. } | Request::RenderPreview { page, .. } => page,
                    _ => 0,
                };
                if !self.delay.is_zero() {
                    tokio::time::sleep(self.delay).await;
                }
                self.rendered.fetch_add(1, Ordering::Relaxed);
                self.order.lock().push(page);
                Ok(CachedTile {
                    bytes: Arc::from(vec![7u8; 64].as_slice()),
                    width: 4,
                    height: 4,
                    stride: 16,
                })
            })
        }

        fn cancel<'a>(
            &'a self,
            _doc: DocId,
            _generation: Generation,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            self.cancels.fetch_add(1, Ordering::Relaxed);
            Box::pin(async {})
        }
    }

    fn service(backend: Arc<FakeBackend>, concurrency: usize) -> Arc<RenderService> {
        let service = RenderService::new(backend, 1 << 20, concurrency);
        service.register(
            1,
            DocInfo {
                path: "/x.pdf".into(),
                page_sizes: vec![(612.0, 792.0); 20],
                generation: 0,
            },
        );
        service.spawn_dispatcher();
        service
    }

    fn key(page: u32) -> TileKey {
        TileKey {
            doc: 1,
            page,
            rotation: 0,
            ppp_milli: 1000,
            col: 0,
            row: 0,
            kind: TileKind::Sharp,
            invert: false,
        }
    }

    #[tokio::test]
    async fn a_second_request_for_a_rendered_tile_is_served_from_the_cache() {
        let backend = FakeBackend::new(0);
        let service = service(Arc::clone(&backend), 2);
        service
            .fetch(key(0), 1, Priority::Visible)
            .await
            .expect("first");
        service
            .fetch(key(0), 1, Priority::Visible)
            .await
            .expect("second");
        assert_eq!(backend.count(), 1, "the second request must not re-render");
        let stats = service.stats();
        assert_eq!((stats.cache.hits, stats.rendered), (1, 1));
    }

    #[tokio::test]
    async fn simultaneous_requests_for_one_tile_share_a_single_render() {
        // The visible pass and the prefetch pass overlap by design, so this is
        // not a corner case: it is what happens on every frame.
        let backend = FakeBackend::new(20);
        let service = service(Arc::clone(&backend), 4);
        let a = Arc::clone(&service);
        let b = Arc::clone(&service);
        let (x, y) = tokio::join!(
            async move { a.fetch(key(3), 1, Priority::Visible).await },
            async move { b.fetch(key(3), 1, Priority::Prefetch).await }
        );
        assert!(x.is_ok() && y.is_ok());
        assert_eq!(backend.count(), 1, "one render for two waiters");
        assert_eq!(service.stats().coalesced, 1);
    }

    #[tokio::test]
    async fn a_request_from_a_superseded_epoch_never_reaches_a_worker() {
        let backend = FakeBackend::new(0);
        let service = service(Arc::clone(&backend), 2);
        service.bump_generation(1, 5);
        let err = service.fetch(key(0), 4, Priority::Visible).await;
        assert!(matches!(err, Err(RenderError::Superseded)), "{err:?}");
        assert_eq!(backend.count(), 0, "cancelled work must cost nothing");
    }

    #[tokio::test]
    async fn a_preview_survives_a_superseded_epoch() {
        // The preview tier is what stops a page being white; it depends on
        // neither zoom nor scroll, so a stale generation must not drop it.
        let backend = FakeBackend::new(0);
        let service = service(Arc::clone(&backend), 2);
        service.bump_generation(1, 9);
        let preview = TileKey {
            kind: TileKind::Preview,
            ppp_milli: 256,
            ..key(2)
        };
        assert!(service.fetch(preview, 1, Priority::Preview).await.is_ok());
        assert_eq!(backend.count(), 1);
    }

    #[tokio::test]
    async fn what_is_on_screen_outruns_what_is_merely_predicted() {
        // One worker, a slow render, and a prefetch queued first. The visible
        // tile must still be rendered before the prefetched one.
        let backend = FakeBackend::new(30);
        let service = service(Arc::clone(&backend), 1);
        let blocker = Arc::clone(&service);
        let busy = tokio::spawn(async move { blocker.fetch(key(0), 1, Priority::Visible).await });
        tokio::time::sleep(Duration::from_millis(5)).await;

        let prefetch = Arc::clone(&service);
        let visible = Arc::clone(&service);
        let low = tokio::spawn(async move { prefetch.fetch(key(1), 1, Priority::Prefetch).await });
        tokio::time::sleep(Duration::from_millis(5)).await;
        let high = tokio::spawn(async move { visible.fetch(key(2), 1, Priority::Visible).await });

        let _ = tokio::join!(busy, low, high);
        let order = backend.order.lock().clone();
        assert_eq!(
            order,
            vec![0, 2, 1],
            "the visible tile queued last must still be rendered before the prefetch"
        );
    }

    #[tokio::test]
    async fn a_zoom_drops_the_queue_it_left_behind() {
        let backend = FakeBackend::new(40);
        let service = service(Arc::clone(&backend), 1);
        let blocker = Arc::clone(&service);
        let busy = tokio::spawn(async move { blocker.fetch(key(0), 1, Priority::Visible).await });
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Queued behind the slow render, then superseded before it starts.
        let queued = Arc::clone(&service);
        let stale = tokio::spawn(async move { queued.fetch(key(1), 1, Priority::Visible).await });
        tokio::time::sleep(Duration::from_millis(5)).await;
        service.bump_generation(1, 2);

        let (_, stale) = tokio::join!(busy, stale);
        assert!(
            matches!(stale.expect("joined"), Err(RenderError::Superseded)),
            "a queued tile from the old epoch must be dropped, not drawn"
        );
        assert_eq!(backend.count(), 1, "only the in-flight render finished");
        assert!(service.stats().superseded >= 1);
    }

    #[tokio::test]
    async fn closing_a_document_frees_its_cache() {
        let backend = FakeBackend::new(0);
        let service = service(Arc::clone(&backend), 2);
        service
            .fetch(key(0), 1, Priority::Visible)
            .await
            .expect("render");
        assert!(service.stats().cache.bytes > 0);
        service.forget(1);
        assert_eq!(service.stats().cache.bytes, 0);
        // And a request for the closed document is refused rather than queued.
        assert!(matches!(
            service.fetch(key(0), 1, Priority::Visible).await,
            Err(RenderError::UnknownDocument(1))
        ));
    }

    #[tokio::test]
    async fn a_page_that_does_not_exist_is_refused_before_it_is_queued() {
        let backend = FakeBackend::new(0);
        let service = service(Arc::clone(&backend), 2);
        let err = service.fetch(key(99), 1, Priority::Visible).await;
        assert!(
            matches!(err, Err(RenderError::PageOutOfRange { .. })),
            "{err:?}"
        );
        assert_eq!(backend.count(), 0);
    }

    #[tokio::test]
    async fn an_inactive_tab_gives_back_its_tiles_but_keeps_its_thumbnails() {
        let backend = FakeBackend::new(0);
        let service = service(Arc::clone(&backend), 2);
        let preview = TileKey {
            kind: TileKind::Preview,
            ppp_milli: 256,
            ..key(0)
        };
        service
            .fetch(key(0), 1, Priority::Visible)
            .await
            .expect("tile");
        service
            .fetch(preview, 1, Priority::Preview)
            .await
            .expect("preview");
        assert_eq!(service.trim(1), 1, "one sharp tile released");
        assert_eq!(service.stats().cache.entries, 1, "the preview stays");
    }
}
