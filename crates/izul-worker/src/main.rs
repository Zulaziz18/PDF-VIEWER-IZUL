//! The sandboxed PDFium worker process (SPEC 3.4).
//!
//! One process holds one PDFium engine and several documents. It connects back
//! to the supervisor's command channel, maps the supervisor's shared-memory
//! ring, and then serves requests until told to stop or until it is killed.
//!
//! Two things it deliberately does *not* do:
//!
//! * it never renders on more than one thread, because PDFium's `FPDF_*`
//!   entry points are not safe to call concurrently — parallelism comes from
//!   running several of these;
//! * it never decides its own limits. Memory cap, privileges and network
//!   isolation are imposed from outside by the supervisor's Job Object, so a
//!   compromised worker cannot lift them.

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

mod session;

use std::path::PathBuf;

use izul_ipc::codec::{read_frame, write_frame};
use izul_ipc::message::{DocId, Envelope, ErrorKind, Request, RequestId, Response, SlotRef};
use izul_ipc::ring::{TileRing, TILE_EDGE};
use izul_ipc::{region_bytes, ChannelName, SharedRegion};
use izul_pdf::render::Quality;
use izul_pdf::{Engine, PdfError, PdfRectF, TileRequest};
use session::{classify, quality_of, Session};
use tokio::io::{AsyncRead, AsyncWrite};

/// How the supervisor tells a freshly spawned worker where to attach.
#[derive(Debug)]
struct Bootstrap {
    channel: ChannelName,
    shm_name: String,
    shm_slots: u32,
    pdfium: PathBuf,
    worker_id: u32,
}

impl Bootstrap {
    /// Read from the environment rather than argv: a command line is visible to
    /// every process on the machine, and the channel name is the one thing that
    /// should not be.
    fn from_env() -> Result<Self, String> {
        fn need(key: &str) -> Result<String, String> {
            std::env::var(key).map_err(|_| format!("variabel {key} tidak diset"))
        }
        let worker_id: u32 = need("IZUL_WORKER_ID")?
            .parse()
            .map_err(|_| "IZUL_WORKER_ID bukan angka".to_string())?;
        let session: u64 = need("IZUL_SESSION_ID")?
            .parse()
            .map_err(|_| "IZUL_SESSION_ID bukan angka".to_string())?;
        Ok(Bootstrap {
            channel: ChannelName::for_worker(session, worker_id),
            shm_name: need("IZUL_SHM_NAME")?,
            shm_slots: need("IZUL_SHM_SLOTS")?
                .parse()
                .map_err(|_| "IZUL_SHM_SLOTS bukan angka".to_string())?,
            pdfium: PathBuf::from(need("IZUL_PDFIUM_PATH")?),
            worker_id,
        })
    }
}

fn main() -> std::process::ExitCode {
    init_tracing();
    let boot = match Bootstrap::from_env() {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "bootstrap tidak lengkap");
            return std::process::ExitCode::from(2);
        }
    };
    tracing::info!(worker = boot.worker_id, "worker mulai");

    // A current-thread runtime: all I/O is one channel, and the PDFium work must
    // stay on one thread anyway. A multi-thread runtime would only add the
    // temptation to render off-thread.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(error = %e, "tokio runtime gagal dibuat");
            return std::process::ExitCode::from(3);
        }
    };

    match rt.block_on(run(boot)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "worker berhenti karena galat");
            std::process::ExitCode::from(1)
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("IZUL_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    // stderr, not stdout: the supervisor reads stderr into the application log
    // and leaves stdout free.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

async fn run(boot: Bootstrap) -> Result<(), String> {
    let engine = Engine::load_from(&boot.pdfium).map_err(|e| format!("PDFium: {e}"))?;

    let region_len = region_bytes(boot.shm_slots);
    let region = SharedRegion::open(&boot.shm_name, region_len)
        .map_err(|e| format!("shm {}: {e}", boot.shm_name))?;
    // SAFETY: the supervisor created and initialised this region before spawning
    // us, and `region` keeps it mapped for as long as `ring` is alive because
    // both live for the rest of this function.
    let ring = unsafe { TileRing::attach(region.as_ptr(), region_len) }
        .map_err(|e| format!("ring: {e}"))?;

    let mut channel = izul_ipc::connect(&boot.channel)
        .await
        .map_err(|e| format!("kanal perintah: {e}"))?;

    let mut sess = Session::new(engine);
    tracing::info!(worker = boot.worker_id, slots = boot.shm_slots, "siap");

    loop {
        let env: Envelope<Request> = match read_frame(&mut channel).await {
            Ok(e) => e,
            Err(izul_ipc::CodecError::PeerClosed) => {
                tracing::info!("supervisor menutup kanal, worker keluar");
                return Ok(());
            }
            Err(e) => return Err(format!("baca perintah: {e}")),
        };
        if matches!(env.payload, Request::Shutdown) {
            tracing::info!("perintah shutdown diterima");
            return Ok(());
        }
        handle(&mut channel, &ring, &mut sess, env).await?;
    }
}

async fn handle<C>(
    channel: &mut C,
    ring: &TileRing,
    sess: &mut Session,
    env: Envelope<Request>,
) -> Result<(), String>
where
    C: AsyncRead + AsyncWrite + Unpin,
{
    let id = env.id;
    match env.payload {
        Request::Ping { nonce } => reply(channel, id, Response::Pong { nonce }).await,

        Request::Open {
            doc,
            path,
            password,
        } => match sess.open(doc, &path, password.as_deref()) {
            Ok(open) => {
                let d = &open.doc;
                let sizes = match d.page_sizes() {
                    Ok(s) => s.into_iter().map(|s| (s.width, s.height)).collect(),
                    Err(e) => return fail(channel, id, Some(doc), &e).await,
                };
                let resp = Response::Opened {
                    doc,
                    page_count: d.page_count(),
                    page_sizes: sizes,
                    permissions: d.permissions(),
                    encrypted: d.is_encrypted(),
                };
                let page_count = d.page_count();
                let mapped = d.is_mapped();
                let bytes = d.byte_len();
                reply(channel, id, resp).await?;
                // The residency line is what makes a worker's memory behaviour
                // legible in the log after a crash report comes in.
                tracing::info!(
                    ?doc,
                    page_count,
                    bytes,
                    mapped,
                    open_docs = sess.open_count(),
                    "dokumen dibuka"
                );
                Ok(())
            }
            Err(e) => fail(channel, id, Some(doc), &e).await,
        },

        Request::Close { doc } => {
            sess.close(doc);
            reply(channel, id, Response::Closed { doc }).await
        }

        Request::Trim { doc } => {
            sess.trim(doc);
            reply(channel, id, Response::Closed { doc }).await
        }

        Request::Cancel { doc, generation } => {
            sess.bump_generation(doc, generation);
            reply(channel, id, Response::Superseded { doc, generation }).await
        }

        Request::RenderTile {
            doc,
            page,
            source,
            dest_w,
            dest_h,
            quality,
            generation,
            ..
        } => {
            sess.bump_generation(doc, generation);
            if !sess.is_current(doc, generation) {
                return reply(channel, id, Response::Superseded { doc, generation }).await;
            }
            let Some(open) = sess.get(doc) else {
                return fail_kind(channel, id, Some(doc), ErrorKind::BadRequest, "doc.unknown")
                    .await;
            };
            match render_tile(
                ring,
                &open.doc,
                doc,
                page,
                source,
                dest_w,
                dest_h,
                quality_of(quality),
                generation.0,
            ) {
                Ok(slot) => {
                    reply(
                        channel,
                        id,
                        Response::TileReady {
                            doc,
                            page,
                            slot,
                            generation,
                        },
                    )
                    .await
                }
                Err(e) => fail(channel, id, Some(doc), &e).await,
            }
        }

        Request::RenderThumb {
            doc,
            pages,
            max_edge_px,
            generation,
        } => {
            let Some(open) = sess.get(doc) else {
                return fail_kind(channel, id, Some(doc), ErrorKind::BadRequest, "doc.unknown")
                    .await;
            };
            for page in pages.first..pages.last.min(open.doc.page_count()) {
                if !sess.is_current(doc, generation) {
                    return reply(channel, id, Response::Superseded { doc, generation }).await;
                }
                let Ok(size) = open.doc.page_size_fast(page) else {
                    continue;
                };
                let scale = size.scale_to_fit(max_edge_px as f32, max_edge_px as f32);
                let source = PdfRectF::new(0.0, 0.0, size.width, size.height);
                let w = ((size.width * scale).round() as u32).clamp(1, TILE_EDGE);
                let h = ((size.height * scale).round() as u32).clamp(1, TILE_EDGE);
                match render_tile(
                    ring,
                    &open.doc,
                    doc,
                    page,
                    source,
                    w,
                    h,
                    Quality::Fast,
                    generation.0,
                ) {
                    Ok(slot) => {
                        reply(channel, id, Response::ThumbReady { doc, page, slot }).await?
                    }
                    Err(e) => return fail(channel, id, Some(doc), &e).await,
                }
                // Release immediately: a thumbnail sweep over 500 pages must not
                // leave 500 parsed pages resident.
                open.doc.release_page(page);
            }
            reply(
                channel,
                id,
                Response::Progress {
                    doc,
                    done: pages.last,
                    total: pages.last,
                },
            )
            .await
        }

        Request::ExtractText {
            doc,
            pages,
            with_boxes,
        } => {
            let Some(open) = sess.get(doc) else {
                return fail_kind(channel, id, Some(doc), ErrorKind::BadRequest, "doc.unknown")
                    .await;
            };
            for page in pages.first..pages.last.min(open.doc.page_count()) {
                let resp = if with_boxes {
                    match open.doc.page_text_boxed(page) {
                        Ok(pt) => Response::TextReady {
                            doc,
                            page,
                            text: pt.text,
                            chars: pt
                                .chars
                                .into_iter()
                                .map(|c| izul_ipc::CharBoxWire {
                                    unicode: c.unicode,
                                    rect: c.rect,
                                })
                                .collect(),
                        },
                        Err(e) => return fail(channel, id, Some(doc), &e).await,
                    }
                } else {
                    match open.doc.page_text(page) {
                        Ok(text) => Response::TextReady {
                            doc,
                            page,
                            text,
                            chars: Vec::new(),
                        },
                        Err(e) => return fail(channel, id, Some(doc), &e).await,
                    }
                };
                reply(channel, id, resp).await?;
                open.doc.release_page(page);
            }
            Ok(())
        }

        // Search lands in Phase 2, where the FTS5 index it cooperates with is
        // built. Answering with a clear "not yet" beats answering with nothing.
        Request::Search { doc, .. } => {
            fail_kind(
                channel,
                id,
                Some(doc),
                ErrorKind::BadRequest,
                "err.not_in_this_phase",
            )
            .await
        }

        Request::Shutdown => Ok(()),
    }
}

/// Renders one tile into a ring slot and publishes it.
#[allow(clippy::too_many_arguments)]
fn render_tile(
    ring: &TileRing,
    doc: &izul_pdf::Document,
    doc_id: DocId,
    page: u32,
    source: PdfRectF,
    dest_w: u32,
    dest_h: u32,
    quality: Quality,
    generation: u64,
) -> Result<SlotRef, PdfError> {
    let mut slot = ring.claim_or_reclaim().map_err(|_| PdfError::Cancelled)?;
    let req = TileRequest {
        page,
        source,
        dest_w: dest_w.min(TILE_EDGE),
        dest_h: dest_h.min(TILE_EDGE),
        draw_annotations: true,
        quality,
        limit_image_cache: false,
    };
    let geom = doc.render_tile_into(&req, slot.bytes())?;
    slot.publish(
        doc_id.0,
        page,
        generation,
        geom.width,
        geom.height,
        geom.stride as u32,
    )
    .map_err(|_| PdfError::Cancelled)
}

async fn reply<C>(channel: &mut C, id: RequestId, payload: Response) -> Result<(), String>
where
    C: AsyncWrite + Unpin,
{
    write_frame(channel, &Envelope { id, payload })
        .await
        .map_err(|e| format!("tulis balasan: {e}"))
}

async fn fail<C>(
    channel: &mut C,
    id: RequestId,
    doc: Option<DocId>,
    err: &PdfError,
) -> Result<(), String>
where
    C: AsyncWrite + Unpin,
{
    tracing::warn!(error = %err, "permintaan gagal");
    let payload = Response::Error {
        doc,
        kind: classify(err),
        message_id: err.message_id().to_string(),
        detail: err.to_string(),
    };
    reply(channel, id, payload).await
}

async fn fail_kind<C>(
    channel: &mut C,
    id: RequestId,
    doc: Option<DocId>,
    kind: ErrorKind,
    message_id: &str,
) -> Result<(), String>
where
    C: AsyncWrite + Unpin,
{
    let payload = Response::Error {
        doc,
        kind,
        message_id: message_id.to_string(),
        detail: String::new(),
    };
    reply(channel, id, payload).await
}
