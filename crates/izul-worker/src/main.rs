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
use izul_ipc::message::{
    DocId, Envelope, ErrorKind, OutlineEntry, Request, RequestId, Response, SearchHitWire, SlotRef,
};
use izul_ipc::ring::{TileRing, TILE_EDGE};
use izul_ipc::{region_bytes, ChannelName, SharedRegion};
use izul_pdf::render::Quality;
use izul_pdf::{Engine, FindOptions, PdfError, PdfRectF, TileRequest};
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

    // Announce the protocol before anything else. The supervisor refuses a
    // worker whose number does not match its own, which is what turns "this
    // binary is a phase out of date" from a silent misparse — a `Ping` read as
    // a `Shutdown` — into a message that says so.
    write_frame(
        &mut channel,
        &Envelope {
            id: RequestId(0),
            payload: Response::Hello {
                protocol: izul_ipc::PROTOCOL_VERSION,
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        },
    )
    .await
    .map_err(|e| format!("salam versi: {e}"))?;

    let mut sess = Session::new(engine);
    tracing::info!(
        worker = boot.worker_id,
        slots = boot.shm_slots,
        protocol = izul_ipc::PROTOCOL_VERSION,
        "siap"
    );

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
            rotation,
            quality,
            generation,
        } => {
            sess.bump_generation(doc, generation);
            // The cheap half of cancellation, and the half that matters: a tile
            // the user has already zoomed past never reaches PDFium at all. The
            // unit of work is one tile, so this check *is* SPEC 6's "at tile
            // boundaries" (SPEC 6).
            if !sess.is_current(doc, generation) {
                return reply(channel, id, Response::Superseded { doc, generation }).await;
            }
            let Some(open) = sess.get(doc) else {
                return fail_kind(channel, id, Some(doc), ErrorKind::BadRequest, "doc.unknown")
                    .await;
            };
            let req = TileRequest {
                page,
                source,
                dest_w,
                dest_h,
                rotation,
                draw_annotations: true,
                quality: quality_of(quality),
                limit_image_cache: false,
            };
            match render_into_ring(ring, &open.doc, doc, &req, generation.0) {
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

        Request::RenderPreview {
            doc,
            page,
            max_edge_px,
            rotation,
            generation,
        } => {
            let Some(open) = sess.get(doc) else {
                return fail_kind(channel, id, Some(doc), ErrorKind::BadRequest, "doc.unknown")
                    .await;
            };
            // A preview is deliberately *not* generation-gated. It depends on
            // neither zoom nor scroll, it is what stops the viewport showing a
            // white page, and it costs a couple of milliseconds — dropping it
            // because the user kept scrolling would defeat its whole purpose.
            let size = match open.doc.page_display_size(page, rotation) {
                Ok(s) => s,
                Err(e) => return fail(channel, id, Some(doc), &e).await,
            };
            let cap = max_edge_px.clamp(16, TILE_EDGE) as f32;
            let scale = size.scale_to_fit(cap, cap);
            let req = TileRequest {
                page,
                source: PdfRectF::new(0.0, 0.0, size.width, size.height),
                dest_w: ((size.width * scale).round() as u32).clamp(1, TILE_EDGE),
                dest_h: ((size.height * scale).round() as u32).clamp(1, TILE_EDGE),
                rotation,
                draw_annotations: true,
                // The first tier is about arriving, not about being beautiful.
                quality: Quality::Fast,
                limit_image_cache: false,
            };
            let rendered = render_into_ring(ring, &open.doc, doc, &req, generation.0);
            // A preview sweep over 500 pages must not leave 500 parsed pages
            // resident, and a preview is by definition a page we are not
            // otherwise working on.
            open.doc.release_page(page);
            match rendered {
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

        Request::ExtractText {
            doc,
            page,
            with_boxes,
            rotation,
        } => {
            let Some(open) = sess.get(doc) else {
                return fail_kind(channel, id, Some(doc), ErrorKind::BadRequest, "doc.unknown")
                    .await;
            };
            let resp = if with_boxes {
                match open.doc.page_text_boxed(page, rotation) {
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
            reply(channel, id, resp).await
        }

        Request::Outline { doc } => {
            let Some(open) = sess.get(doc) else {
                return fail_kind(channel, id, Some(doc), ErrorKind::BadRequest, "doc.unknown")
                    .await;
            };
            match open.doc.outline() {
                Ok(tree) => {
                    let mut nodes = Vec::new();
                    flatten_outline(&tree, 0, &mut nodes);
                    reply(channel, id, Response::OutlineReady { doc, nodes }).await
                }
                Err(e) => fail(channel, id, Some(doc), &e).await,
            }
        }

        Request::Search {
            doc,
            page,
            query,
            opts,
            rotation,
            generation,
        } => {
            sess.bump_generation(doc, generation);
            // Same cancellation point as a tile: a query the user has already
            // typed past never reaches PDFium. Typing into a search box produces
            // a new generation per keystroke, so without this the worker would
            // finish every prefix of the word.
            if !sess.is_current(doc, generation) {
                return reply(channel, id, Response::Superseded { doc, generation }).await;
            }
            let Some(open) = sess.get(doc) else {
                return fail_kind(channel, id, Some(doc), ErrorKind::BadRequest, "doc.unknown")
                    .await;
            };
            let find_opts = FindOptions {
                case_sensitive: opts.case_sensitive,
                whole_word: opts.whole_word,
                max_hits: opts.max_hits,
            };
            match open.doc.find_on_page(page, &query, find_opts, rotation) {
                Ok(matches) => {
                    let hits = matches
                        .into_iter()
                        .map(|m| SearchHitWire {
                            char_index: m.char_index,
                            char_count: m.char_count,
                            rects: m.rects,
                        })
                        .collect();
                    reply(
                        channel,
                        id,
                        Response::SearchReady {
                            doc,
                            page,
                            hits,
                            generation,
                        },
                    )
                    .await
                }
                Err(e) => fail(channel, id, Some(doc), &e).await,
            }
        }

        Request::Shutdown => Ok(()),
    }
}

/// Flattens the outline tree onto the wire's depth-tagged list.
fn flatten_outline(nodes: &[izul_pdf::OutlineNode], depth: u16, out: &mut Vec<OutlineEntry>) {
    for node in nodes {
        out.push(OutlineEntry {
            title: node.title.clone(),
            depth,
            page: node.page,
            y: node.y,
        });
        flatten_outline(&node.children, depth.saturating_add(1), out);
    }
}

/// Renders one tile into a ring slot and publishes it.
fn render_into_ring(
    ring: &TileRing,
    doc: &izul_pdf::Document,
    doc_id: DocId,
    req: &TileRequest,
    generation: u64,
) -> Result<SlotRef, PdfError> {
    // `claim_or_reclaim` rather than `claim`: when the ring is full of tiles the
    // UI has not collected, the tile the user is waiting for now is worth more
    // than the oldest one they already scrolled past.
    let mut slot = ring.claim_or_reclaim().map_err(|_| PdfError::Cancelled)?;
    let clamped = TileRequest {
        dest_w: req.dest_w.min(TILE_EDGE),
        dest_h: req.dest_h.min(TILE_EDGE),
        ..*req
    };
    let geom = doc.render_tile_into(&clamped, slot.bytes())?;
    slot.publish(
        doc_id.0,
        req.page,
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
