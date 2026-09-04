//! Wire messages for the command channel (SPEC 6).
//!
//! Serialised with `postcard`, which produces a compact non-self-describing
//! encoding. That is a deliberate trade: both ends are built from this crate in
//! the same build, so there is no version skew to tolerate, and we get framing
//! costs measured in nanoseconds instead of a JSON parse per tile.

use serde::{Deserialize, Serialize};

use izul_model::geom::{PdfRectF, RotationQuarter};

/// Version of the wire protocol in this build.
///
/// `postcard` is not self-describing: an enum travels as the *index* of its
/// variant, so two builds whose `Request` enums differ by one variant will
/// happily talk past each other — a `Ping` decoded as a `Shutdown`, and a
/// worker that exits instead of answering. That failure is silent, and it cost
/// a long debugging session on a machine where `izul-worker.exe` was one phase
/// older than the application that spawned it.
///
/// So the worker announces this number the moment it connects, and the
/// supervisor refuses a worker that does not match. Bump it whenever anything
/// in [`Request`] or [`Response`] changes shape.
pub const PROTOCOL_VERSION: u32 = 2;

/// Identifies one open document within a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DocId(pub u64);

/// Correlates a reply with its request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(pub u64);

/// Layout epoch.
///
/// Incremented by the UI whenever zoom, rotation, or page layout changes. A
/// worker drops any queued job whose generation is behind the newest one it has
/// seen for that document, and in-flight jobs check it at tile boundaries. This
/// is what stops a fast zoom from burning a worker on tiles nobody will ever
/// look at (SPEC 6, SPEC 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct Generation(pub u64);

impl Generation {
    pub fn next(self) -> Self {
        Generation(self.0.wrapping_add(1))
    }
}

/// Quality tier requested for a render (SPEC 9's two-tier pipeline).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RenderQuality {
    /// Low resolution, no smoothing. Shown immediately during scroll.
    Fast,
    #[default]
    Sharp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub max_hits: u32,
}

/// UI process -> worker process.
///
/// Every request has exactly one response. A request that streamed several
/// replies under one id — the shape Phase 0 used for thumbnail sweeps — cannot
/// be routed by the supervisor's reply table, and the extra replies were
/// silently dropped. Phase 1 pulls instead: the viewport asks for the one page
/// it is about to draw, which is also what makes cancellation meaningful.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    Open {
        doc: DocId,
        path: String,
        password: Option<String>,
    },
    Close {
        doc: DocId,
    },
    /// One tile of one page.
    ///
    /// `source` is in display space: points, origin bottom-left, with the
    /// page's own `/Rotate` and `rotation` already applied, so its extents are
    /// the page size the viewport laid out with.
    RenderTile {
        doc: DocId,
        page: u32,
        source: PdfRectF,
        dest_w: u32,
        dest_h: u32,
        rotation: RotationQuarter,
        quality: RenderQuality,
        generation: Generation,
    },
    /// A whole page at thumbnail resolution: the low-resolution first tier of
    /// SPEC 9's two-tier render, and the sidebar's thumbnail, which are the
    /// same bitmap and therefore rendered once.
    RenderPreview {
        doc: DocId,
        page: u32,
        max_edge_px: u32,
        rotation: RotationQuarter,
        generation: Generation,
    },
    /// Page text, and — when `with_boxes` is set — the per-character boxes the
    /// selection layer needs, in display space at `rotation`.
    ExtractText {
        doc: DocId,
        page: u32,
        with_boxes: bool,
        rotation: RotationQuarter,
    },
    /// The document's own bookmark tree, for the outline sidebar.
    Outline {
        doc: DocId,
    },
    Search {
        doc: DocId,
        query: String,
        opts: SearchOptions,
    },
    /// Drop full-resolution page handles for a document whose tab went inactive,
    /// keeping the document itself open (SPEC 10).
    Trim {
        doc: DocId,
    },
    /// Cancel every job for `doc` older than `generation`.
    Cancel {
        doc: DocId,
        generation: Generation,
    },
    Ping {
        nonce: u64,
    },
    Shutdown,
}

/// Worker process -> UI process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Response {
    /// Sent unprompted the moment the worker connects, before anything is
    /// asked of it. A worker that does not send this is from a different
    /// build and cannot be talked to (see [`PROTOCOL_VERSION`]).
    Hello {
        protocol: u32,
        /// The worker's own `CARGO_PKG_VERSION`, for the log line.
        version: String,
    },
    Opened {
        doc: DocId,
        page_count: u32,
        /// Display size of each page in points, with the page's own `/Rotate`
        /// applied. Read from the page tree, which the Phase 0 spike measured
        /// at ~6 ms for 500 pages against ~495 ms for loading every page — and
        /// the scroll bar cannot have the right size until this is known.
        page_sizes: Vec<(f32, f32)>,
        permissions: u32,
        encrypted: bool,
    },
    /// The pixels are in shared memory; this message carries only the address.
    TileReady {
        doc: DocId,
        page: u32,
        slot: SlotRef,
        generation: Generation,
    },
    TextReady {
        doc: DocId,
        page: u32,
        text: String,
        chars: Vec<CharBoxWire>,
    },
    OutlineReady {
        doc: DocId,
        nodes: Vec<OutlineEntry>,
    },
    /// A job was dropped because a newer generation superseded it. Not an error;
    /// the UI uses it to clear its pending set.
    Superseded {
        doc: DocId,
        generation: Generation,
    },
    Closed {
        doc: DocId,
    },
    Error {
        doc: Option<DocId>,
        kind: ErrorKind,
        message_id: String,
        detail: String,
    },
    Pong {
        nonce: u64,
    },
}

/// One outline entry, flattened for the wire.
///
/// A tree would need a recursive `postcard` schema and a recursive decoder on
/// an untrusted-ish path; a flat list with a depth column carries exactly the
/// same information and cannot recurse at all. The sidebar rebuilds the nesting
/// from `depth`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlineEntry {
    pub title: String,
    /// Nesting level, 0 for a top-level entry.
    pub depth: u16,
    /// Target page, when the destination resolves to one in this document.
    pub page: Option<u32>,
    /// Target y in PDF points, when the destination names one.
    pub y: Option<f32>,
}

/// Where a rendered bitmap lives in the pixel channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotRef {
    /// Index of the slot within the ring.
    pub slot: u32,
    /// Byte offset of the first scanline from the start of the shared region.
    pub offset: u64,
    pub stride: u32,
    pub width: u32,
    pub height: u32,
    /// Bumped every time a slot is reused, so a stale `SlotRef` can be detected
    /// rather than read as live pixels.
    pub epoch: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CharBoxWire {
    pub unicode: char,
    pub rect: PdfRectF,
}

/// Coarse error class. The UI picks a recovery affordance from this; the exact
/// wording comes from `message_id` through the i18n table (SPEC 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    /// Ask the user for a password and retry.
    PasswordRequired,
    PasswordWrong,
    /// File is gone, renamed, or unreadable.
    FileUnavailable,
    /// Structurally broken. Partial recovery may be possible.
    Corrupt,
    /// The request itself was malformed. A bug on our side.
    BadRequest,
    /// PDFium unwound. The supervisor counts these per document.
    EngineFault,
    /// Worker exceeded its per-page render budget.
    Timeout,
    /// The worker is shutting down or already gone.
    WorkerGone,
}

/// One framed message on the command channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub id: RequestId,
    pub payload: T,
}

pub type RequestEnvelope = Envelope<Request>;
pub type ResponseEnvelope = Envelope<Response>;
