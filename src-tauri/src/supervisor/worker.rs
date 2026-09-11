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
    #[error(
        "pekerja bicara protokol {found} sementara aplikasi memakai {expected}. \
         Binari izul-worker sudah usang — jalankan `cargo build --workspace` \
         lalu jalankan aplikasi lagi."
    )]
    ProtocolMismatch { expected: u32, found: u32 },
    #[error(
        "pekerja tidak mengirim salam versi dalam {0:?}. Binari izul-worker \
         kemungkinan usang — jalankan `cargo build --workspace`."
    )]
    NoHandshake(Duration),
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
///
/// Every method takes `&self`, including the ones that kill the process, so the
/// supervisor can hand out an `Arc<Worker>` and let a render request await its
/// reply *without* holding the pool lock. That matters: the heartbeat sweep
/// needs the write lock every two seconds, and a lock held across a render
/// would delay the sweep by exactly as long as the render takes.
pub struct Worker {
    pub id: u32,
    child: parking_lot::Mutex<Child>,
    pid: u32,
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
            .field("pid", &self.pid)
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

        let mut stream = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .map_err(|_| WorkerError::Unresponsive(Duration::from_secs(10)))?
            .map_err(WorkerError::Channel)?;

        // The worker's first frame is its protocol number, sent unprompted. A
        // binary from another build either sends nothing (it does not know to)
        // or sends a number that does not match; either way it is refused here
        // rather than left to misread every later message.
        const HANDSHAKE: Duration = Duration::from_secs(10);
        let hello: Envelope<Response> = tokio::time::timeout(HANDSHAKE, read_frame(&mut stream))
            .await
            .map_err(|_| WorkerError::NoHandshake(HANDSHAKE))??;
        match hello.payload {
            Response::Hello { protocol, version } if protocol == izul_ipc::PROTOCOL_VERSION => {
                tracing::debug!(worker = id, %version, protocol, "salam pekerja diterima");
            }
            Response::Hello { protocol, .. } => {
                return Err(WorkerError::ProtocolMismatch {
                    expected: izul_ipc::PROTOCOL_VERSION,
                    found: protocol,
                })
            }
            _ => return Err(WorkerError::NoHandshake(HANDSHAKE)),
        }

        let (tx, rx) = mpsc::channel::<Job>(256);
        let last_beat = Arc::new(Mutex::new(Instant::now()));
        tokio::spawn(pump(stream, rx, Arc::clone(&last_beat)));

        let pid = child.id();
        Ok(Worker {
            id,
            child: parking_lot::Mutex::new(child),
            pid,
            _region: region,
            ring: Arc::new(ring),
            tx,
            last_beat,
            next_request: AtomicU64::new(1),
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
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
    pub fn has_exited(&self) -> bool {
        matches!(self.child.lock().try_wait(), Ok(Some(_)) | Err(_))
    }

    /// Asks politely, then insists.
    pub async fn shutdown(&self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.request(Request::Shutdown)).await;
        for _ in 0..20 {
            if self.has_exited() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.kill();
    }

    pub fn kill(&self) {
        let mut child = self.child.lock();
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Drives one worker's channel: serialises requests onto it and routes replies
/// back by request id.
///
/// Replies are matched by id rather than assumed to arrive in order, because
/// several requests are in flight at once whenever the viewport asks for a
/// screenful of tiles.
///
/// ## Why the reading happens in its own task
///
/// The obvious shape — one `select!` waiting on both "a new request to send"
/// and "a frame to read" — is wrong, and wrong in a way that only shows under
/// real load. `select!` drops the future of every branch that does not win, and
/// [`read_frame`] is **not cancel-safe**: it reads a four-byte length, then the
/// body. Cancel it in between and the length bytes are gone from the stream for
/// good. The next read starts in the middle of a frame, reads body bytes as a
/// length, and waits for a frame that will never come. The worker then looks
/// silent, the supervisor kills it at the heartbeat timeout, the replacement
/// meets the same fate on the next burst of tiles, and the application renders
/// nothing while the log fills with `pekerja diam terlalu lama`.
///
/// So the stream is read by a task that is never cancelled, and hands whole
/// frames over an `mpsc` channel. Both arms of the `select!` below are then
/// cancel-safe — `Receiver::recv` documents itself so — and no byte can be lost
/// between iterations.
async fn pump<S>(stream: S, mut rx: mpsc::Receiver<Job>, last_beat: Arc<Mutex<Instant>>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut pending: HashMap<u64, oneshot::Sender<Result<Response, WorkerError>>> = HashMap::new();
    let mut next_id: u64 = 1;

    // Bounded, because an unbounded queue here would let a worker that answers
    // faster than the UI consumes grow memory without limit. 64 frames is far
    // more than the pool ever has in flight.
    let (frames_tx, mut frames) = mpsc::channel::<Result<Envelope<Response>, ()>>(64);
    let reader_task = tokio::spawn(async move {
        let mut reader = reader;
        loop {
            match read_frame::<_, Envelope<Response>>(&mut reader).await {
                Ok(env) => {
                    if frames_tx.send(Ok(env)).await.is_err() {
                        return; // the pump is gone
                    }
                }
                Err(_) => {
                    // The channel died. Say so once and stop; the pump treats
                    // this as the end of the worker.
                    let _ = frames_tx.send(Err(())).await;
                    return;
                }
            }
        }
    });

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
            incoming = frames.recv() => {
                match incoming {
                    Some(Ok(env)) => {
                        *last_beat.lock().await = Instant::now();
                        if let Some(reply) = pending.remove(&env.id.0) {
                            let _ = reply.send(Ok(env.payload));
                        }
                        // A reply with no waiter is one whose caller gave up
                        // first — a timed-out request, or a cancelled fetch.
                        // Dropping it is correct.
                    }
                    // Read error, or the reader task ended: the worker is gone.
                    Some(Err(())) | None => break,
                }
            }
        }
    }

    reader_task.abort();

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

#[cfg(test)]
mod tests {
    use super::*;
    use izul_ipc::message::Request;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Sends a request through the pump the way `Worker::request` does.
    async fn ask(tx: &mpsc::Sender<Job>, request: Request) -> Result<Response, WorkerError> {
        let (reply, wait) = oneshot::channel();
        tx.send(Job { request, reply }).await.expect("queue job");
        wait.await.expect("pump answered")
    }

    /// The regression this exists for: a frame split across two writes, with a
    /// new request arriving in the gap.
    ///
    /// That is the shape that broke a real Windows build — the viewport asks
    /// for a screenful of tiles while replies are streaming back, so a request
    /// lands mid-frame constantly. With a cancel-unsafe read inside `select!`,
    /// the length bytes of the half-read frame are dropped, every later frame
    /// is misread, and the worker goes silent until the supervisor kills it.
    /// Nothing in the integration suite caught it, because those tests talk to
    /// a worker sequentially and never go through the pump at all.
    #[tokio::test]
    async fn a_request_arriving_mid_frame_does_not_desynchronise_the_channel() {
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        let (tx, rx) = mpsc::channel::<Job>(16);
        let beat = Arc::new(Mutex::new(Instant::now()));
        tokio::spawn(pump(ours, rx, Arc::clone(&beat)));

        // The far side plays the worker: it answers every ping, but writes the
        // length and the body of each reply as two separate writes with a pause
        // between them.
        tokio::spawn(async move {
            let mut peer = theirs;
            loop {
                let mut len_buf = [0u8; 4];
                if peer.read_exact(&mut len_buf).await.is_err() {
                    return;
                }
                let len = u32::from_le_bytes(len_buf) as usize;
                let mut body = vec![0u8; len];
                if peer.read_exact(&mut body).await.is_err() {
                    return;
                }
                let env: Envelope<Request> = match postcard::from_bytes(&body) {
                    Ok(env) => env,
                    Err(_) => return,
                };
                let nonce = match env.payload {
                    Request::Ping { nonce } => nonce,
                    _ => 0,
                };
                let reply = Envelope {
                    id: env.id,
                    payload: Response::Pong { nonce },
                };
                let out = postcard::to_allocvec(&reply).expect("encode");
                // Header first...
                if peer
                    .write_all(&(out.len() as u32).to_le_bytes())
                    .await
                    .is_err()
                {
                    return;
                }
                let _ = peer.flush().await;
                // ...then a gap in which the pump is parked mid-frame, and
                // then the body.
                tokio::time::sleep(Duration::from_millis(30)).await;
                if peer.write_all(&out).await.is_err() {
                    return;
                }
                let _ = peer.flush().await;
            }
        });

        // Ten requests, each issued while the previous reply is half-written.
        for nonce in 0..10u64 {
            let sender = tx.clone();
            let answering =
                tokio::spawn(async move { ask(&sender, Request::Ping { nonce }).await });
            tokio::time::sleep(Duration::from_millis(10)).await;
            let next = tx.clone();
            let overlapping =
                tokio::spawn(async move { ask(&next, Request::Ping { nonce: nonce + 100 }).await });

            let first = tokio::time::timeout(Duration::from_secs(5), answering)
                .await
                .expect("the channel must not go silent")
                .expect("task");
            let second = tokio::time::timeout(Duration::from_secs(5), overlapping)
                .await
                .expect("the channel must not go silent")
                .expect("task");

            assert_eq!(first.expect("reply"), Response::Pong { nonce });
            assert_eq!(
                second.expect("reply"),
                Response::Pong { nonce: nonce + 100 }
            );
        }
    }

    /// Replies are matched by id, not by arrival order.
    #[tokio::test]
    async fn replies_are_routed_by_id_even_when_they_arrive_out_of_order() {
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        let (tx, rx) = mpsc::channel::<Job>(16);
        let beat = Arc::new(Mutex::new(Instant::now()));
        tokio::spawn(pump(ours, rx, beat));

        tokio::spawn(async move {
            let mut peer = theirs;
            let mut seen: Vec<Envelope<Request>> = Vec::new();
            // Collect two requests, then answer them backwards.
            while seen.len() < 2 {
                let mut len_buf = [0u8; 4];
                if peer.read_exact(&mut len_buf).await.is_err() {
                    return;
                }
                let mut body = vec![0u8; u32::from_le_bytes(len_buf) as usize];
                if peer.read_exact(&mut body).await.is_err() {
                    return;
                }
                match postcard::from_bytes(&body) {
                    Ok(env) => seen.push(env),
                    Err(_) => return,
                }
            }
            for env in seen.into_iter().rev() {
                let nonce = match env.payload {
                    Request::Ping { nonce } => nonce,
                    _ => 0,
                };
                let out = postcard::to_allocvec(&Envelope {
                    id: env.id,
                    payload: Response::Pong { nonce },
                })
                .expect("encode");
                let _ = peer.write_all(&(out.len() as u32).to_le_bytes()).await;
                let _ = peer.write_all(&out).await;
                let _ = peer.flush().await;
            }
        });

        let a = tokio::spawn({
            let tx = tx.clone();
            async move { ask(&tx, Request::Ping { nonce: 1 }).await }
        });
        let b = tokio::spawn({
            let tx = tx.clone();
            async move { ask(&tx, Request::Ping { nonce: 2 }).await }
        });

        let (a, b) = tokio::join!(a, b);
        assert_eq!(
            a.expect("task").expect("reply"),
            Response::Pong { nonce: 1 }
        );
        assert_eq!(
            b.expect("task").expect("reply"),
            Response::Pong { nonce: 2 }
        );
    }

    /// A worker that dies must not leave callers waiting for their own timeout.
    #[tokio::test]
    async fn every_waiter_hears_about_it_when_the_channel_dies() {
        let (ours, theirs) = tokio::io::duplex(1024);
        let (tx, rx) = mpsc::channel::<Job>(16);
        let beat = Arc::new(Mutex::new(Instant::now()));
        tokio::spawn(pump(ours, rx, beat));

        // Read the request, then hang up without answering.
        tokio::spawn(async move {
            let mut peer = theirs;
            let mut len_buf = [0u8; 4];
            let _ = peer.read_exact(&mut len_buf).await;
            drop(peer);
        });

        let outcome =
            tokio::time::timeout(Duration::from_secs(5), ask(&tx, Request::Ping { nonce: 7 }))
                .await
                .expect("a dead channel must fail fast, not hang");
        assert!(matches!(outcome, Err(WorkerError::Gone)), "{outcome:?}");
    }
}
