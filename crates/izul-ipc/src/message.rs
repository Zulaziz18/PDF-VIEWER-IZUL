//! Wire messages for the command channel (SPEC 6).
//!
//! Serialised with `postcard`, which produces a compact non-self-describing
//! encoding. That is a deliberate trade: both ends are built from this crate in
//! the same build, so there is no version skew to tolerate, and we get framing
//! costs measured in nanoseconds instead of a JSON parse per tile.

use serde::{Deserialize, Serialize};

use izul_model::geom::{PageFrame, PdfRectF, RotationQuarter};

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
pub const PROTOCOL_VERSION: u32 = 12;

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
    /// Matches for `query` on **one** page, with the boxes to highlight them.
    ///
    /// Page-scoped for the same reason [`Request::RenderTile`] is: a worker that
    /// went away to search five hundred pages is a worker that stopped answering
    /// heartbeats, and the supervisor kills it at six seconds (SPEC 3.4). It is
    /// also what makes the search cancellable — `generation` lets a query the
    /// user has already typed past be dropped rather than finished.
    ///
    /// Which pages are worth asking about is a question for the FTS5 index,
    /// which answers it for the whole library at once; this fills in the
    /// geometry the index cannot know.
    Search {
        doc: DocId,
        page: u32,
        query: String,
        opts: SearchOptions,
        /// Display rotation the boxes should come back in, matching the tiles.
        rotation: RotationQuarter,
        generation: Generation,
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
    /// Advance widths and face metrics for one of the standard-14 faces.
    ///
    /// The annotation model lays text out from these numbers and both backends
    /// position every glyph from that layout, so they must be the metrics the
    /// renderer will actually draw with (SPEC 3.2). PDFium is what knows them —
    /// and PDFium lives here, in the sandbox, not in the UI process. Hence a
    /// request: the UI asks, caches the answer, and never links a PDF parser
    /// into the process that owns the window (SPEC 5).
    FontMetrics {
        family: String,
        bold: bool,
        italic: bool,
        /// The characters actually needed. Asking for a whole face would be a
        /// wire message per annotation for glyphs nobody is going to draw.
        chars: String,
    },
    Ping {
        nonce: u64,
    },
    Shutdown,

    // ---- Phase 4: saving and exporting. Appended at the end, like every
    // variant since Phase 1's `Ping`-read-as-`Shutdown` (see
    // `PROTOCOL_VERSION`). ----------------------------------------------
    /// This application's own annotations on one page of an open document,
    /// taken out of the page the first time it loaded (see `izul-pdf`'s
    /// `izul` module). Answered with `PageAnnotsReady`.
    PageAnnots {
        doc: DocId,
        page: u32,
    },
    /// Opens a *working copy* of a file for saving or exporting: read into
    /// memory, with our annotations left in place. One per document; opening
    /// another replaces it. Answered with `WorkReady`.
    WorkOpen {
        doc: DocId,
        path: String,
    },
    /// On the working copy: replaces our annotations on each listed page with
    /// placeholders `/NM (izul-<id>)`, one per `(page, id)`. Pages not listed
    /// keep what they have.
    WorkPlaceholders {
        doc: DocId,
        pages: Vec<u32>,
        placeholders: Vec<(u32, u64)>,
    },
    /// On the working copy: burns the annotations of `count` pages from
    /// `first` into their content. Batched so that a 2 000-page document
    /// cannot keep the worker silent past the heartbeat (SPEC 3.4).
    WorkFlatten {
        doc: DocId,
        first: u32,
        count: u32,
    },
    /// Replaces the working copy with a new document of just these pages.
    WorkExtract {
        doc: DocId,
        pages: Vec<u32>,
    },
    /// Renders one page of the working copy, annotations included, and
    /// encodes it. Answered with `BlobReady`.
    WorkRender {
        doc: DocId,
        page: u32,
        /// Pixels per point.
        scale: f32,
        /// `None` for PNG, `Some(quality)` for JPEG.
        jpeg_quality: Option<u8>,
    },
    /// Writes the working copy with PDFium. Answered with `BlobReady`.
    WorkSave {
        doc: DocId,
    },
    WorkClose {
        doc: DocId,
    },
    /// Up to `len` bytes of a blob from `offset`. Blobs travel in pieces
    /// because a saved PDF is larger than the frame limit, and because the UI
    /// process — which owns the user's files — is the one that writes them.
    BlobRead {
        blob: u64,
        offset: u64,
        len: u32,
    },
    BlobDrop {
        blob: u64,
    },
    /// Opens a file as a reader would and counts our annotations on `pages`.
    /// The last check before an atomic save replaces the original.
    VerifyFile {
        path: String,
        pages: Vec<u32>,
    },
    // ---- Phase 5 (appended; see `PROTOCOL_VERSION`). ---------------------
    /// On the working copy: makes its pages exactly `pages`, in order —
    /// deleted, moved, duplicated, turned, blank pages added, pages copied in
    /// from other files. Applied in place, so the document's bookmarks and
    /// metadata survive (`izul-pdf`'s `arrange`). `sources[0]` is the file
    /// the working copy was opened from; the others are the files pages came
    /// from. Answered with `WorkReady`.
    WorkArrange {
        doc: DocId,
        pages: Vec<ArrangePage>,
        sources: Vec<String>,
    },
    // ---- Phase 6 (appended; see `PROTOCOL_VERSION`). ---------------------
    /// On the working copy: takes out everything inside the areas (true
    /// redaction, `izul-redact`), then checks the result with PDFium before
    /// it becomes the working copy. Answered with `WorkRedacted`.
    WorkRedact {
        doc: DocId,
        pages: Vec<RedactPageWire>,
    },
    /// Opens a written file and checks that no character is left in the
    /// areas — the same check, on the bytes that are about to replace the
    /// user's file. Answered with `RedactionVerified`.
    VerifyRedacted {
        path: String,
        /// Page, and its areas in user space as `WorkRedacted` returned them.
        pages: Vec<(u32, Vec<PdfRectF>)>,
    },
    // ---- Phase 7 (appended; see `PROTOCOL_VERSION`). ---------------------
    /// Where each page of the working copy puts display space in user space
    /// — asked after the pages are in their final order, because annotations
    /// are kept in display space and must be written in user space.
    /// Answered with `WorkFramesReady`.
    WorkFrames {
        doc: DocId,
    },
    /// On the working copy: reads one page with local OCR and writes what it
    /// read onto the page as invisible text (`izul_ocr::ocr_page`). One page
    /// per request, so a worker is never silent long enough for the heartbeat
    /// to take it for hung, and the UI can report progress and stop between
    /// pages. `models` is the folder holding the two `.rten` files. A page
    /// that already has text is left alone unless `force`. Answered with
    /// `WorkOcrDone`.
    WorkOcr {
        doc: DocId,
        page: u32,
        models: String,
        force: bool,
    },
    /// Adds bytes to a blob in the worker, for input too large for one frame
    /// (a picture). `blob` 0 starts a new one. Answered with `BlobAppended`.
    BlobAppend {
        blob: u64,
        data: Vec<u8>,
    },
    /// Makes the background of the picture in `blob` (PNG or JPEG bytes)
    /// transparent with the segmentation model (`izul_ocr::background`). The
    /// input blob is consumed. Answered with `BackgroundRemoved`.
    RemoveBackground {
        blob: u64,
        /// The ONNX Runtime library, and the model.
        runtime: String,
        model: String,
        kind: BackgroundKind,
    },
    /// Every form widget of the open document (`izul_pdf::forms`). Answered
    /// with `FormFieldsReady`.
    FormFields {
        doc: DocId,
    },
    /// Fills fields through PDFium's form-fill environment, which builds the
    /// widgets' appearances again. `working`: on the working copy (a save);
    /// otherwise on the open document itself, so the page shows the value
    /// before it is saved. Answered with `FormFilled`.
    FillForm {
        doc: DocId,
        working: bool,
        values: Vec<(String, izul_model::FormValue)>,
    },
    /// Replaces the text in `rect` (display space) on `page` of the working
    /// copy with `text`, within one line (`izul_pdf::textedit`), checked with
    /// PDFium before it answers `TextReplaced`.
    WorkReplaceText {
        doc: DocId,
        page: u32,
        rect: PdfRectF,
        text: String,
    },
}

/// One widget of a form field, as `FormFields` reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormWidgetWire {
    pub page: u32,
    /// Display space.
    pub rect: PdfRectF,
    pub name: String,
    pub kind: FormKindWire,
    pub read_only: bool,
    pub value: String,
    pub checked: bool,
    /// A checkbox or radio widget's "on" name.
    pub export: String,
    pub options: Vec<String>,
    pub selected: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormKindWire {
    Text { multiline: bool },
    CheckBox,
    Radio,
    ComboBox { editable: bool },
    ListBox { multiple: bool },
    Other,
}

/// What a picture is, for [`Request::RemoveBackground`]. The user says; it
/// cannot be told from the picture (see `izul_ocr::background::Kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundKind {
    Photo,
    OnPaper,
}

/// The areas to redact on one page of a [`Request::WorkRedact`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RedactPageWire {
    pub page: u32,
    pub areas: Vec<RedactAreaWire>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RedactAreaWire {
    /// In display space, as annotations are (origin bottom-left of the page
    /// as shown, its own `/Rotate` applied).
    pub rect: PdfRectF,
    /// Fill colour afterwards, 0..1 RGB; `None` leaves the area empty.
    pub fill: Option<[f32; 3]>,
}

/// What was taken out of one page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RedactedPageWire {
    pub page: u32,
    /// The areas as redacted: user space, widened to whole characters.
    pub areas: Vec<PdfRectF>,
    pub glyphs: u32,
    pub images_removed: u32,
    pub images_cleared: u32,
    /// Images removed whole because their format cannot be partly cleared.
    pub images_unsupported: u32,
    pub paths: u32,
    pub forms: u32,
    pub annotations: u32,
    pub marked_content: u32,
}

/// One page of a [`Request::WorkArrange`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ArrangePage {
    /// Page `page` of `sources[source]`, turned `rotation` quarter turns
    /// further than its own `/Rotate`.
    Page {
        source: u32,
        page: u32,
        rotation: u8,
    },
    /// A new empty page, in points.
    Blank {
        width: f32,
        height: f32,
        rotation: u8,
    },
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
    /// Appended at the end of the enum on purpose: `postcard` identifies a
    /// variant by its index, so adding one anywhere else renumbers the ones
    /// after it. Phase 1 shipped a worker binary that read `Ping` as `Shutdown`
    /// for exactly that reason.
    SearchReady {
        doc: DocId,
        page: u32,
        hits: Vec<SearchHitWire>,
        generation: Generation,
    },
    /// Metrics for the face that was asked about. Appended at the end of the
    /// enum, like every variant since Phase 1's `Ping`-read-as-`Shutdown`.
    FontMetricsReady {
        /// The standard-14 face the request resolved to, for the log and for
        /// the cache key.
        base_font: String,
        ascent_milli: i16,
        descent_milli: i16,
        /// One entry per character asked for that the face actually has.
        advances: Vec<(char, u16)>,
        /// Characters the face has no glyph for. The caller refuses the text
        /// and names them rather than drawing empty boxes (SPEC 11.2).
        missing: Vec<char>,
    },

    // ---- Phase 4, appended ------------------------------------------------
    PageAnnotsReady {
        doc: DocId,
        page: u32,
        annots: Vec<SavedAnnotWire>,
    },
    WorkReady {
        doc: DocId,
        page_count: u32,
    },
    /// A blob is waiting in the worker; fetch it with `BlobRead`.
    BlobReady {
        blob: u64,
        len: u64,
    },
    BlobBytes {
        blob: u64,
        data: Vec<u8>,
    },
    Verified {
        page_count: u32,
        izul_annots: u32,
    },
    // ---- Phase 6, appended ------------------------------------------------
    WorkRedacted {
        doc: DocId,
        pages: Vec<RedactedPageWire>,
    },
    RedactionVerified {
        pages: u32,
    },
    // ---- Phase 7, appended ------------------------------------------------
    WorkFramesReady {
        doc: DocId,
        /// One per page, in page order.
        frames: Vec<PageFrame>,
    },
    WorkOcrDone {
        doc: DocId,
        page: u32,
        /// Words written, or `None` when the page already had text.
        words: Option<u32>,
    },
    BlobAppended {
        blob: u64,
        len: u64,
    },
    /// The picture as a PNG with alpha, waiting in `blob`.
    BackgroundRemoved {
        blob: u64,
        len: u64,
        /// Where the model ran ("DirectML", or "CPU" with the reason).
        device: String,
        /// The paper refinement ran (`BackgroundKind::OnPaper` on a plain,
        /// light background).
        paper: bool,
    },
    FormFieldsReady {
        doc: DocId,
        widgets: Vec<FormWidgetWire>,
    },
    FormFilled {
        doc: DocId,
        /// Widgets changed.
        changed: u32,
        /// Pages whose widgets changed, for repainting.
        pages: Vec<u32>,
    },
    TextReplaced {
        doc: DocId,
        /// What was there, as PDFium read it.
        before: String,
        /// Glyphs taken out.
        glyphs: u32,
    },
}

/// One saved annotation as the worker found it: its `/IzulObj` value, and —
/// for a picture — the blob holding the picture's bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedAnnotWire {
    pub metadata: String,
    pub image: Option<(u64, u64, String)>,
}

/// The largest piece `BlobRead` hands out: half the frame limit, leaving room
/// for the envelope.
pub const BLOB_CHUNK: u32 = 4 * 1024 * 1024;

/// One match, with the boxes to draw over it.
///
/// `rects` is a list because a match broken across a line has one box per line;
/// a single box spanning them would paint over everything in between.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHitWire {
    /// Index of the first matching character in the page's extracted text, so a
    /// hit can be lined up with a text layer the frontend already holds.
    pub char_index: u32,
    pub char_count: u32,
    pub rects: Vec<PdfRectF>,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    /// `postcard` sends a variant as its index, so an index that moves is a
    /// silent protocol break (CLAUDE.md, bug #4). These are the indices older
    /// variants had when Phase 4 appended its own; adding a variant anywhere
    /// but the end changes one of them and fails here.
    #[test]
    fn existing_variants_keep_their_wire_index() {
        let index = |bytes: Vec<u8>| bytes[0];
        assert_eq!(
            index(postcard::to_allocvec(&Request::Ping { nonce: 0 }).unwrap()),
            10
        );
        assert_eq!(
            index(postcard::to_allocvec(&Request::Shutdown).unwrap()),
            11
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Request::PageAnnots {
                    doc: DocId(1),
                    page: 0
                })
                .unwrap()
            ),
            12
        );
        assert_eq!(
            index(postcard::to_allocvec(&Response::Pong { nonce: 0 }).unwrap()),
            8
        );
        assert_eq!(
            index(postcard::to_allocvec(&Response::BlobReady { blob: 1, len: 2 }).unwrap()),
            13
        );
        // Phase 4's last variant, pinned when Phase 5 appended after it.
        assert_eq!(
            index(
                postcard::to_allocvec(&Request::VerifyFile {
                    path: String::new(),
                    pages: vec![]
                })
                .unwrap()
            ),
            22
        );
        // Phase 5's last variants, pinned when Phase 6 appended after them.
        assert_eq!(
            index(
                postcard::to_allocvec(&Response::Verified {
                    page_count: 0,
                    izul_annots: 0
                })
                .unwrap()
            ),
            15
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Request::WorkRedact {
                    doc: DocId(1),
                    pages: vec![]
                })
                .unwrap()
            ),
            24
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Request::WorkArrange {
                    doc: DocId(1),
                    pages: vec![],
                    sources: vec![]
                })
                .unwrap()
            ),
            23
        );
    }

    #[test]
    fn phase_seven_variants_are_appended() {
        let index = |bytes: Vec<u8>| bytes[0];
        assert_eq!(
            index(postcard::to_allocvec(&Request::WorkFrames { doc: DocId(1) }).unwrap()),
            26
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Request::WorkOcr {
                    doc: DocId(1),
                    page: 0,
                    models: String::new(),
                    force: false
                })
                .unwrap()
            ),
            27
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Request::BlobAppend {
                    blob: 0,
                    data: vec![]
                })
                .unwrap()
            ),
            28
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Request::RemoveBackground {
                    blob: 1,
                    runtime: String::new(),
                    model: String::new(),
                    kind: BackgroundKind::Photo
                })
                .unwrap()
            ),
            29
        );
        assert_eq!(
            index(postcard::to_allocvec(&Response::BlobAppended { blob: 1, len: 0 }).unwrap()),
            20
        );
        assert_eq!(
            index(postcard::to_allocvec(&Request::FormFields { doc: DocId(1) }).unwrap()),
            30
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Request::FillForm {
                    doc: DocId(1),
                    working: false,
                    values: vec![]
                })
                .unwrap()
            ),
            31
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Response::FormFieldsReady {
                    doc: DocId(1),
                    widgets: vec![]
                })
                .unwrap()
            ),
            22
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Response::FormFilled {
                    doc: DocId(1),
                    changed: 0,
                    pages: vec![]
                })
                .unwrap()
            ),
            23
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Request::WorkReplaceText {
                    doc: DocId(1),
                    page: 0,
                    rect: PdfRectF::new(0.0, 0.0, 1.0, 1.0),
                    text: String::new()
                })
                .unwrap()
            ),
            32
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Response::TextReplaced {
                    doc: DocId(1),
                    before: String::new(),
                    glyphs: 0
                })
                .unwrap()
            ),
            24
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Response::BackgroundRemoved {
                    blob: 1,
                    len: 0,
                    device: String::new(),
                    paper: false
                })
                .unwrap()
            ),
            21
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Response::WorkOcrDone {
                    doc: DocId(1),
                    page: 0,
                    words: None
                })
                .unwrap()
            ),
            19
        );
        assert_eq!(
            index(
                postcard::to_allocvec(&Response::WorkFramesReady {
                    doc: DocId(1),
                    frames: vec![]
                })
                .unwrap()
            ),
            18
        );
    }

    #[test]
    fn the_new_messages_round_trip() {
        let r = Response::PageAnnotsReady {
            doc: DocId(3),
            page: 7,
            annots: vec![SavedAnnotWire {
                metadata: "{\"izul\":1}".into(),
                image: Some((9, 1024, "image/png".into())),
            }],
        };
        let bytes = postcard::to_allocvec(&r).unwrap();
        assert_eq!(postcard::from_bytes::<Response>(&bytes).unwrap(), r);
    }
}
