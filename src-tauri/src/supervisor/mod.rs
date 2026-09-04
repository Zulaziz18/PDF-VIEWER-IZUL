//! The worker pool and its supervisor (SPEC 3.4).
//!
//! A fixed pool of sandboxed PDFium processes, each holding several documents.
//! The supervisor's job is to make a worker crash survivable: it heartbeats
//! every worker, restarts dead ones with exponential backoff, and quarantines
//! documents that keep taking workers down.
//!
//! The property that matters, and that the Phase 0 exit criteria name: killing a
//! worker by force must cost the user the affected tabs, briefly, and nothing
//! else. The UI process keeps running, and unsaved work is recoverable because
//! the draft lives in SQLite rather than in the dead process.

pub mod poison;
pub mod policy;
pub mod sandbox;
pub mod worker;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use izul_ipc::message::{DocId, Request, Response};
use tokio::sync::RwLock;

use poison::{DocKey, PoisonList};
use policy::{pool_size, restart_backoff, Placement, HEARTBEAT_TIMEOUT};
use sandbox::Sandbox;
use worker::{Worker, WorkerError, WorkerPaths};

/// What one heartbeat probe concluded about a worker.
#[derive(Debug, Clone, Copy)]
enum Health {
    /// Answered its ping.
    Alive,
    /// Missed a ping but has been heard from recently — busy, not hung.
    Slow,
    /// The OS says the process is gone.
    Dead,
    /// Missed a ping and has been silent past the heartbeat window.
    Hung { silent: Duration },
}

/// A document's placement and identity within the pool.
#[derive(Debug, Clone)]
struct DocSlot {
    worker: usize,
    key: DocKey,
    path: String,
}

/// State a restarted worker needs in order to put the user back where they were.
#[derive(Debug, Clone)]
pub struct RecoveredDoc {
    pub doc: DocId,
    pub path: String,
    pub quarantined: bool,
}

pub struct Pool {
    session: u64,
    paths: WorkerPaths,
    sandbox: Sandbox,
    workers: Vec<Option<Arc<Worker>>>,
    failures: Vec<u32>,
    docs: HashMap<DocId, DocSlot>,
    poison: PoisonList,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("workers", &self.workers.len())
            .field("live", &self.workers.iter().filter(|w| w.is_some()).count())
            .field("docs", &self.docs.len())
            .finish_non_exhaustive()
    }
}

impl Pool {
    /// Starts the pool. Every worker is spawned up front so the first document
    /// the user opens does not pay process-creation cost.
    pub async fn start(paths: WorkerPaths) -> Result<Self, WorkerError> {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        let size = pool_size(cores);
        let session = session_id();
        let sandbox = Sandbox::create().map_err(WorkerError::Spawn)?;

        tracing::info!(
            size,
            cores,
            security_boundary = sandbox.is_security_boundary(),
            "memulai kolam pekerja"
        );

        let mut workers = Vec::with_capacity(size);
        for id in 0..size as u32 {
            let w = Worker::spawn(id, session, &paths, &sandbox).await?;
            tracing::info!(worker = id, pid = w.pid(), "pekerja siap");
            workers.push(Some(Arc::new(w)));
        }
        Ok(Pool {
            session,
            paths,
            sandbox,
            workers,
            failures: vec![0; size],
            docs: HashMap::new(),
            poison: PoisonList::new(),
        })
    }

    pub fn size(&self) -> usize {
        self.workers.len()
    }

    pub fn live_workers(&self) -> usize {
        self.workers.iter().filter(|w| w.is_some()).count()
    }

    fn loads(&self) -> Vec<usize> {
        (0..self.workers.len())
            .map(|i| self.docs.values().filter(|d| d.worker == i).count())
            .collect()
    }

    /// Assigns a document to a worker and opens it there.
    pub async fn open(
        &mut self,
        doc: DocId,
        path: &str,
        key: DocKey,
        active_worker: Option<usize>,
    ) -> Result<Response, WorkerError> {
        let quarantined = self.poison.is_quarantined(&key);
        let placement = policy::place(&self.loads(), active_worker, quarantined);
        let index = match placement {
            Placement::Pooled(i) => i,
            // Phase 0 places a quarantined document on the least-loaded worker
            // and records the decision; the dedicated read-only isolated worker
            // it deserves arrives with the tab lifecycle in Phase 2.
            Placement::Isolated => {
                tracing::warn!(?doc, "dokumen dikarantina");
                self.loads()
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, l)| **l)
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            }
        };
        self.docs.insert(
            doc,
            DocSlot {
                worker: index,
                key,
                path: path.to_string(),
            },
        );
        let req = Request::Open {
            doc,
            path: path.to_string(),
            password: None,
        };
        self.request_on(index, req).await
    }

    /// The worker holding a document, as a handle that outlives the pool lock.
    ///
    /// This is how the render pipeline avoids serialising every tile behind the
    /// supervisor's lock: it takes the handle, releases the lock, and only then
    /// waits for pixels.
    pub fn worker_for(&self, doc: DocId) -> Option<Arc<Worker>> {
        let slot = self.docs.get(&doc)?;
        self.workers.get(slot.worker)?.clone()
    }

    /// Sends one request to a worker under the standard timeout.
    ///
    /// A bounded wait, so a worker that wedges on one pathological page cannot
    /// leave a UI call pending forever. The heartbeat sweep is what then
    /// notices the worker is gone and replaces it.
    pub async fn ask(worker: &Worker, req: Request) -> Result<Response, WorkerError> {
        match tokio::time::timeout(REQUEST_TIMEOUT, worker.request(req)).await {
            Ok(result) => result,
            Err(_) => Err(WorkerError::Unresponsive(REQUEST_TIMEOUT)),
        }
    }

    async fn request_on(&self, index: usize, req: Request) -> Result<Response, WorkerError> {
        let worker = self
            .workers
            .get(index)
            .and_then(|w| w.clone())
            .ok_or(WorkerError::Gone)?;
        Self::ask(&worker, req).await
    }

    pub fn worker_ring(&self, doc: DocId) -> Option<Arc<izul_ipc::TileRing>> {
        let slot = self.docs.get(&doc)?;
        self.workers
            .get(slot.worker)?
            .as_ref()
            .map(|w| Arc::clone(&w.ring))
    }

    /// One supervision sweep: heartbeat every worker, replace the dead ones.
    ///
    /// Returns the documents that need reopening, so the caller can restore each
    /// affected tab to its last reading position.
    pub async fn sweep(&mut self) -> Vec<RecoveredDoc> {
        let mut casualties: Vec<usize> = Vec::new();
        let mut healthy: Vec<usize> = Vec::new();

        // Every live worker is probed at once. Sequentially, one hung worker
        // would delay the check on the next by the whole heartbeat timeout,
        // and the pool's write lock is held for all of it — which stops the
        // viewport from rendering anything in the meantime.
        let live: Vec<(usize, Arc<Worker>)> = (0..self.workers.len())
            .filter_map(|i| match self.workers.get(i).and_then(|w| w.clone()) {
                Some(worker) => Some((i, worker)),
                None => {
                    casualties.push(i);
                    None
                }
            })
            .collect();

        let mut probes = Vec::with_capacity(live.len());
        for (index, worker) in live {
            probes.push(tokio::spawn(async move {
                if worker.has_exited() {
                    return (index, Health::Dead);
                }
                // Ping *first*, and judge silence only if it fails. Silence on
                // its own means "we have not spoken to it", not "it is hung" —
                // and the supervisor is itself the reason for most of it, since
                // starting eight workers takes several seconds each on a cold
                // debug build. Killing on silence alone executed every worker
                // on the first sweep and put the pool in a restart loop it
                // never left.
                if worker.ping().await.is_ok() {
                    return (index, Health::Alive);
                }
                let silent = worker.silent_for().await;
                if silent > HEARTBEAT_TIMEOUT {
                    worker.kill();
                    (index, Health::Hung { silent })
                } else {
                    // One missed ping while a long render is in flight is not
                    // a death sentence; the next sweep asks again.
                    (index, Health::Slow)
                }
            }));
        }

        for probe in probes {
            let Ok((index, health)) = probe.await else {
                continue;
            };
            match health {
                Health::Alive => healthy.push(index),
                Health::Slow => {
                    tracing::debug!(
                        worker = index,
                        "pekerja sibuk, dicoba lagi sapuan berikutnya"
                    );
                }
                Health::Dead => {
                    tracing::warn!(worker = index, "pekerja mati");
                    casualties.push(index);
                }
                Health::Hung { silent } => {
                    tracing::warn!(
                        worker = index,
                        ?silent,
                        "pekerja tidak menjawab dan diam terlalu lama, dimatikan"
                    );
                    casualties.push(index);
                }
            }
        }
        for index in healthy {
            self.note_healthy(index);
        }

        let mut recovered = Vec::new();
        for index in casualties {
            recovered.extend(self.replace(index).await);
        }
        recovered
    }

    /// Replaces a dead worker and reports what it was holding.
    async fn replace(&mut self, index: usize) -> Vec<RecoveredDoc> {
        let orphans: Vec<(DocId, DocSlot)> = self
            .docs
            .iter()
            .filter(|(_, d)| d.worker == index)
            .map(|(k, v)| (*k, v.clone()))
            .collect();

        // Every document that was resident takes a strike: from outside a dead
        // process there is no way to tell which one was responsible.
        let keys: Vec<DocKey> = orphans.iter().map(|(_, d)| d.key).collect();
        self.poison.worker_died_holding(&keys);

        if let Some(slot) = self.workers.get_mut(index) {
            if let Some(old) = slot.take() {
                old.kill();
            }
        }
        if let Some(f) = self.failures.get_mut(index) {
            *f = f.saturating_add(1);
        }
        let delay = restart_backoff(self.failures.get(index).copied().unwrap_or(0));
        tracing::info!(worker = index, ?delay, "menjadwalkan restart pekerja");
        tokio::time::sleep(delay).await;

        match Worker::spawn(index as u32, self.session, &self.paths, &self.sandbox).await {
            Ok(w) => {
                tracing::info!(worker = index, pid = w.pid(), "pekerja hidup kembali");
                if let Some(slot) = self.workers.get_mut(index) {
                    *slot = Some(Arc::new(w));
                }
            }
            Err(e) => {
                tracing::error!(worker = index, error = %e, "restart pekerja gagal");
            }
        }

        for (doc, _) in &orphans {
            self.docs.remove(doc);
        }
        orphans
            .into_iter()
            .map(|(doc, slot)| RecoveredDoc {
                doc,
                path: slot.path,
                quarantined: self.poison.is_quarantined(&slot.key),
            })
            .collect()
    }

    /// Clears the restart backoff for a worker that has proven itself.
    fn note_healthy(&mut self, index: usize) {
        if let Some(f) = self.failures.get_mut(index) {
            *f = 0;
        }
    }

    /// How many documents are currently quarantined. Surfaced in the status bar
    /// so an isolated document is visible rather than silently degraded.
    pub fn quarantined_count(&self) -> usize {
        self.poison.quarantined_count()
    }

    pub async fn shutdown(&mut self) {
        for slot in self.workers.iter_mut() {
            if let Some(w) = slot.take() {
                w.shutdown().await;
            }
        }
        self.workers.clear();
    }
}

/// Distinguishes this run's channel and shared-memory names from a previous
/// run's leftovers, which matters when the last run was killed rather than
/// closed.
fn session_id() -> u64 {
    let pid = std::process::id() as u64;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    pid.rotate_left(32) ^ nanos
}

/// Called with the documents that need reopening after a worker died.
pub type RecoverySink = Box<dyn Fn(Vec<RecoveredDoc>) + Send + Sync>;

/// Runs the supervision loop until `stop` resolves.
///
/// `on_recovered` is how the UI learns that a tab must be reloaded. It is a
/// callback rather than a channel so the supervisor has no opinion about how the
/// UI is structured.
pub async fn supervise(
    pool: Arc<RwLock<Pool>>,
    on_recovered: Option<RecoverySink>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) {
    let mut ticker = tokio::time::interval(worker::SWEEP_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let recovered = pool.write().await.sweep().await;
                for doc in &recovered {
                    // Named individually: after a crash report arrives, "which
                    // document was it" is the first question, and a count cannot
                    // answer it.
                    tracing::warn!(
                        doc = doc.doc.0,
                        path = %doc.path,
                        quarantined = doc.quarantined,
                        "dokumen perlu dimuat ulang setelah pekerja mati"
                    );
                }
                if let Some(sink) = &on_recovered {
                    sink(recovered);
                }
            }
            _ = &mut stop => {
                pool.write().await.shutdown().await;
                return;
            }
        }
    }
}

/// Timeout used when the UI waits on a single worker request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
