//! One supervised worker process: spawn, converse, notice death, restart.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use izul_ipc::codec::{read_frame, write_frame};
use izul_ipc::message::{Envelope, Request, RequestId, Response};
use izul_ipc::ring::TileRing;
use izul_ipc::{region_bytes, ChannelName, Listener, SharedRegion};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::policy::{HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT, RING_SLOTS};
use super::sandbox::Sandbox;

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("tidak dapat menjalankan proses pekerja: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("tidak dapat menyiapkan kanal: {0}")]
    Channel(#[source] std::io::Error),
    #[error("tidak dapat menyiapkan memori bersama: {0}")]
    Shm(#[source] std::io::Error),
    #[error("ring: {0}")]
    Ring(#[from] izul_ipc::RingError),
    #[error("pekerja tidak menjawab dalam {0:?}")]
    Unresponsive(Duration),
    #[error("pekerja sudah mati")]
    Gone,
    #[error("kanal: {0}")]
    Codec(#[from] izul_ipc::CodecError),
}

/// Where the worker executable and the PDFium library live.
#[derive(Debug, Clone)]
pub struct WorkerPaths {
    pub executable: PathBuf,
    pub pdfium: PathBuf,
}

impl WorkerPaths {
    /// Resolves both next to the running executable, which is how a shipped
    /// build is laid out and the only layout we support at run time.
    pub fn beside_current_exe() -> std::io::Result<Self> {
        let exe = std::env::current_exe()?;
        let dir = exe
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        let worker_name = if cfg!(windows) {
            "izul-worker.exe"
        } else {
            "izul-worker"
        };
        let pdfium_name = if cfg!(windows) {
            "pdfium.dll"
        } else {
            "libpdfium.so"
        };
        Ok(WorkerPaths {
            executable: dir.join(worker_name),
            pdfium: dir.join(pdfium_name),
        })
    }
}

/// A live worker: its process, its channel, and its slice of shared memory.
pub struct Worker {
    pub id: u32,
    child: Child,
    /// Kept alive because the ring points into it.
    _region: SharedRegion,
    pub ring: Arc<TileRing>,
    tx: mpsc::Sender<Job>,
    pub last_beat: Arc<Mutex<Instant>>,
    next_request: AtomicU64,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            .field("id", &self.id)
            .field("pid", &self.child.id())
            .finish_non_exhaustive()
    }
}

struct Job {
    request: Request,
    reply: oneshot::Sender<Result<Response, WorkerError>>,
}

impl Worker {
    /// Spawns a worker and waits for it to connect.
    ///
    /// The listener is bound *before* the process is spawned, so the worker
    /// cannot lose a race against an endpoint that does not exist yet.
    pub async fn spawn(
        id: u32,
        session: u64,
        paths: &WorkerPaths,
        sandbox: &Sandbox,
    ) -> Result<Self, WorkerError> {
        let channel = ChannelName::for_worker(session, id);
        let mut listener = Listener::bind(channel.clone()).map_err(WorkerError::Channel)?;

        let shm_name = format!("{session:016x}-w{id}-{}", std::process::id());
        let region_len = region_bytes(RING_SLOTS);
        let region = SharedRegion::create(&shm_name, region_len).map_err(WorkerError::Shm)?;
        // SAFETY: the region was just created, is zero-filled and correctly
        // sized, and no other process has been told its name yet.
        let ring = unsafe { TileRing::initialise(region.as_ptr(), region_len, RING_SLOTS) }?;

        let mut cmd = Command::new(&paths.executable);
        cmd.env("IZUL_WORKER_ID", id.to_string())
            .env("IZUL_SESSION_ID", session.to_string())
            .env("IZUL_SHM_NAME", &shm_name)
            .env("IZUL_SHM_SLOTS", RING_SLOTS.to_string())
            .env("IZUL_PDFIUM_PATH", &paths.pdfium)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        sandbox.prepare(&mut cmd);
        let child = cmd.spawn().map_err(WorkerError::Spawn)?;
        sandbox.adopt(&child).map_err(WorkerError::Spawn)?;

        let stream = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .map_err(|_| WorkerError::Unresponsive(Duration::from_secs(10)))?
            .map_err(WorkerError::Channel)?;

        let (tx, rx) = mpsc::channel::<Job>(256);
        let last_beat = Arc::new(Mutex::new(Instant::now()));
        tokio::spawn(pump(stream, rx, Arc::clone(&last_beat)));

        Ok(Worker {
            id,
            child,
            _region: region,
            ring: Arc::new(ring),
            tx,
            last_beat,
            next_request: AtomicU64::new(1),
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Sends a request and waits for its reply.
    pub async fn request(&self, request: Request) -> Result<Response, WorkerError> {
        let _ = self.next_request.fetch_add(1, Ordering::Relaxed);
        let (reply, wait) = oneshot::channel();
        self.tx
            .send(Job { request, reply })
            .await
            .map_err(|_| WorkerError::Gone)?;
        wait.await.map_err(|_| WorkerError::Gone)?
    }

    /// One heartbeat. Returns an error if the worker does not answer in time,
    /// which the supervisor treats as a hang.
    pub async fn ping(&self) -> Result<(), WorkerError> {
        let nonce = self.next_request.load(Ordering::Relaxed);
        let fut = self.request(Request::Ping { nonce });
        match tokio::time::timeout(HEARTBEAT_TIMEOUT, fut).await {
            Ok(Ok(Response::Pong { .. })) => {
                *self.last_beat.lock().await = Instant::now();
                Ok(())
            }
            Ok(Ok(_)) => Err(WorkerError::Gone),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(WorkerError::Unresponsive(HEARTBEAT_TIMEOUT)),
        }
    }

    pub async fn silent_for(&self) -> Duration {
        self.last_beat.lock().await.elapsed()
    }

    /// True when the OS says the process has exited.
    pub fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)) | Err(_))
    }

    /// Asks politely, then insists.
    pub async fn shutdown(&mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.request(Request::Shutdown)).await;
        for _ in 0..20 {
            if self.has_exited() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.kill();
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Drives one worker's channel: serialises requests onto it and routes replies
/// back by request id.
///
/// Replies are matched by id rather than assumed to arrive in order, because a
/// thumbnail sweep streams many responses under one id while other requests are
/// still in flight.
async fn pump<S>(stream: S, mut rx: mpsc::Receiver<Job>, last_beat: Arc<Mutex<Instant>>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reader, writer) = tokio::io::split(stream);
    let mut reader = reader;
    let mut writer = writer;
    let mut pending: HashMap<u64, oneshot::Sender<Result<Response, WorkerError>>> = HashMap::new();
    let mut next_id: u64 = 1;

    loop {
        tokio::select! {
            job = rx.recv() => {
                let Some(job) = job else { break };
                let id = next_id;
                next_id = next_id.wrapping_add(1);
                let env = Envelope { id: RequestId(id), payload: job.request };
                if let Err(e) = write_frame(&mut writer, &env).await {
                    let _ = job.reply.send(Err(WorkerError::Codec(e)));
                    break;
                }
                pending.insert(id, job.reply);
            }
            incoming = read_frame::<_, Envelope<Response>>(&mut reader) => {
                match incoming {
                    Ok(env) => {
                        *last_beat.lock().await = Instant::now();
                        if let Some(reply) = pending.remove(&env.id.0) {
                            let _ = reply.send(Ok(env.payload));
                        }
                        // A response with no waiter is a streamed progress or
                        // thumbnail message; Phase 1 routes those to the
                        // viewport. Dropping it here is correct for Phase 0.
                    }
                    Err(_) => break,
                }
            }
        }
    }

    // The channel is gone. Everyone still waiting hears about it now rather than
    // hanging until their own timeout.
    for (_, reply) in pending.drain() {
        let _ = reply.send(Err(WorkerError::Gone));
    }
    while let Ok(job) = rx.try_recv() {
        let _ = job.reply.send(Err(WorkerError::Gone));
    }
}

/// How often the supervisor should sweep the pool.
pub const SWEEP_INTERVAL: Duration = HEARTBEAT_INTERVAL;
