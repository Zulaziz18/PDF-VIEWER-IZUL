//! Per-worker document table and the render loop's job handling.
//!
//! Everything here runs on the worker's single PDFium thread. PDFium's `FPDF_*`
//! calls are not safe to make concurrently, so parallelism comes from having
//! several worker processes (SPEC 3.4), never from threads inside one.

use std::collections::HashMap;

use izul_ipc::message::{DocId, ErrorKind, Generation, RenderQuality};
use izul_pdf::render::Quality;
use izul_pdf::{Document, Engine, PdfError};

/// Newest layout epoch seen per document.
///
/// Split out from the document table so the cancellation rule — the part most
/// likely to be got subtly wrong — is pure data with no PDFium anywhere near
/// it, and can be tested directly.
#[derive(Debug, Default)]
pub struct Epochs {
    current: HashMap<DocId, Generation>,
}

impl Epochs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a new layout epoch, never moving backwards. Messages can be
    /// reordered under load, and an out-of-order older generation must not
    /// resurrect work the UI has already superseded.
    pub fn bump(&mut self, id: DocId, generation: Generation) {
        let slot = self.current.entry(id).or_default();
        if generation > *slot {
            *slot = generation;
        }
    }

    /// Whether a job should still run.
    ///
    /// A document we have no epoch for is treated as current: an `Open` racing
    /// its first `RenderTile` must not have that first tile dropped.
    pub fn is_current(&self, id: DocId, generation: Generation) -> bool {
        self.current
            .get(&id)
            .is_none_or(|newest| generation >= *newest)
    }

    pub fn forget(&mut self, id: DocId) {
        self.current.remove(&id);
    }
}

/// One open document.
pub struct OpenDoc {
    pub doc: Document,
}

/// The worker's document table.
pub struct Session {
    engine: &'static Engine,
    docs: HashMap<DocId, OpenDoc>,
    epochs: Epochs,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("open_docs", &self.docs.len())
            .finish_non_exhaustive()
    }
}

impl Session {
    pub fn new(engine: &'static Engine) -> Self {
        Self {
            engine,
            docs: HashMap::new(),
            epochs: Epochs::new(),
        }
    }

    pub fn open(
        &mut self,
        id: DocId,
        path: &str,
        password: Option<&str>,
    ) -> Result<&OpenDoc, PdfError> {
        let doc = self.engine.open(path, password)?;
        self.docs.insert(id, OpenDoc { doc });
        self.docs.get(&id).ok_or(PdfError::Cancelled)
    }

    pub fn close(&mut self, id: DocId) -> bool {
        self.epochs.forget(id);
        self.docs.remove(&id).is_some()
    }

    pub fn get(&self, id: DocId) -> Option<&OpenDoc> {
        self.docs.get(&id)
    }

    pub fn open_count(&self) -> usize {
        self.docs.len()
    }

    /// Records a new layout epoch for a document.
    pub fn bump_generation(&mut self, id: DocId, generation: Generation) {
        self.epochs.bump(id, generation);
    }

    /// Whether a job should still run.
    ///
    /// This is the cheap half of cancellation: a queued job whose generation has
    /// been superseded is dropped without touching PDFium at all. SPEC 6 calls
    /// for the same check at tile boundaries inside long jobs, which the render
    /// loop applies per tile.
    pub fn is_current(&self, id: DocId, generation: Generation) -> bool {
        self.epochs.is_current(id, generation)
    }

    /// Drops full-resolution page handles for a document whose tab went
    /// inactive, keeping the document itself open (SPEC 10).
    pub fn trim(&self, id: DocId) {
        if let Some(entry) = self.docs.get(&id) {
            entry.doc.release_all_pages();
        }
    }
}

/// Maps a model-level error onto the coarse class the UI switches on.
pub fn classify(err: &PdfError) -> ErrorKind {
    match err {
        PdfError::PasswordRequired => ErrorKind::PasswordRequired,
        PdfError::PasswordWrong => ErrorKind::PasswordWrong,
        PdfError::FileNotReadable { .. } | PdfError::LibraryLoad { .. } => {
            ErrorKind::FileUnavailable
        }
        PdfError::Corrupt { .. } => ErrorKind::Corrupt,
        PdfError::PageOutOfRange { .. }
        | PdfError::BadTileSize { .. }
        | PdfError::BadSourceRect { .. }
        | PdfError::BufferTooSmall { .. } => ErrorKind::BadRequest,
        PdfError::EnginePanic { .. } | PdfError::LibraryAlreadyLoaded => ErrorKind::EngineFault,
        PdfError::Cancelled => ErrorKind::Timeout,
        PdfError::Pdfium { .. } => ErrorKind::Corrupt,
    }
}

pub fn quality_of(q: RenderQuality) -> Quality {
    match q {
        RenderQuality::Fast => Quality::Fast,
        RenderQuality::Sharp => Quality::Sharp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_document_is_treated_as_current() {
        // An Open racing its own first RenderTile must not lose that tile.
        let e = Epochs::new();
        assert!(e.is_current(DocId(1), Generation(0)));
        assert!(e.is_current(DocId(1), Generation(99)));
    }

    #[test]
    fn a_job_from_a_superseded_epoch_is_dropped() {
        let mut e = Epochs::new();
        e.bump(DocId(1), Generation(5));
        assert!(!e.is_current(DocId(1), Generation(4)));
        assert!(e.is_current(DocId(1), Generation(5)));
        assert!(e.is_current(DocId(1), Generation(6)));
    }

    #[test]
    fn epochs_never_move_backwards() {
        // Messages can arrive out of order under load; an older generation must
        // not resurrect work the UI has already superseded.
        let mut e = Epochs::new();
        e.bump(DocId(1), Generation(9));
        e.bump(DocId(1), Generation(3));
        assert!(!e.is_current(DocId(1), Generation(8)));
    }

    #[test]
    fn documents_have_independent_epochs() {
        let mut e = Epochs::new();
        e.bump(DocId(1), Generation(7));
        assert!(!e.is_current(DocId(1), Generation(2)));
        assert!(e.is_current(DocId(2), Generation(2)));
    }

    #[test]
    fn closing_a_document_forgets_its_epoch() {
        let mut e = Epochs::new();
        e.bump(DocId(1), Generation(7));
        e.forget(DocId(1));
        assert!(e.is_current(DocId(1), Generation(1)));
    }

    #[test]
    fn errors_map_onto_the_class_the_ui_switches_on() {
        assert_eq!(
            classify(&PdfError::PasswordRequired),
            ErrorKind::PasswordRequired
        );
        assert_eq!(classify(&PdfError::PasswordWrong), ErrorKind::PasswordWrong);
        assert_eq!(
            classify(&PdfError::EnginePanic {
                operation: "render"
            }),
            ErrorKind::EngineFault
        );
        assert_eq!(
            classify(&PdfError::PageOutOfRange {
                page: 9,
                page_count: 2
            }),
            ErrorKind::BadRequest
        );
        assert_eq!(
            classify(&PdfError::Corrupt {
                detail: String::new()
            }),
            ErrorKind::Corrupt
        );
    }

    #[test]
    fn quality_maps_straight_through() {
        assert_eq!(quality_of(RenderQuality::Fast), Quality::Fast);
        assert_eq!(quality_of(RenderQuality::Sharp), Quality::Sharp);
    }
}
