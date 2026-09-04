//! Wire messages for the command channel (SPEC 6).
//!
//! Serialised with `postcard`, which produces a compact non-self-describing
//! encoding. That is a deliberate trade: both ends are built from this crate in
//! the same build, so there is no version skew to tolerate, and we get framing
//! costs measured in nanoseconds instead of a JSON parse per tile.

use serde::{Deserialize, Serialize};

use izul_model::geom::{PdfRectF, RotationQuarter};

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
pub struct PageRange {
    pub first: u32,
    /// Exclusive.
    pub last: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub max_hits: u32,
}

/// UI process -> worker process.
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
    RenderTile {
        doc: DocId,
        page: u32,
        /// Region of the page to draw, in PDF points.
        source: PdfRectF,
        dest_w: u32,
        dest_h: u32,
        rotation: RotationQuarter,
        quality: RenderQuality,
        generation: Generation,
    },
    RenderThumb {
        doc: DocId,
        pages: PageRange,
        max_edge_px: u32,
        generation: Generation,
    },
    ExtractText {
        doc: DocId,
        pages: PageRange,
        with_boxes: bool,
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
    Opened {
        doc: DocId,
        page_count: u32,
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
    ThumbReady {
        doc: DocId,
        page: u32,
        slot: SlotRef,
    },
    TextReady {
        doc: DocId,
        page: u32,
        text: String,
        chars: Vec<CharBoxWire>,
    },
    SearchHit {
        doc: DocId,
        page: u32,
        rects: Vec<PdfRectF>,
    },
    Progress {
        doc: DocId,
        done: u32,
        total: u32,
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
