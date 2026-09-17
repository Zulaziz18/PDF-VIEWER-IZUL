//! Annotation state for the UI process, and the font metrics it needs.
//!
//! Two things live here that do not belong together anywhere else:
//!
//! **The edit state.** One [`AnnotDoc`] and one [`CommandStack`] per open
//! document, held for as long as the tab is. Phase 3 keeps them in memory;
//! Phase 4 gives them a home in SQLite (the autosave SPEC 8 asks for) and a way
//! into the file itself. Keeping them here — not in the worker — is deliberate:
//! the worker parses untrusted bytes and can be killed at any moment by the
//! supervisor, and a user's unsaved annotations must not die with it.
//!
//! **The font metric cache.** The model lays text out from metrics, and the
//! metrics come from PDFium, which lives in the sandbox. So the UI asks a
//! worker once per face and caches the answer, which keeps the promise that the
//! process owning the window never links a PDF parser (SPEC 5) without making
//! every text edit a round trip.

use std::collections::HashMap;
use std::sync::Arc;

use izul_ipc::message::{DocId, Request, Response};
use izul_model::annot::{AnnotObject, FontSpec};
use izul_model::display::FontRef;
use izul_model::font::{FaceMetrics, FontCtx, GlyphMetrics};
use izul_model::ops::{AnnotDoc, CommandStack, EditError, Op, Transaction};
use parking_lot::Mutex;
use tokio::sync::RwLock;

use crate::supervisor::Pool;

/// Identifies a face independently of the size it is asked about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FaceKey {
    family: String,
    bold: bool,
    italic: bool,
}

impl FaceKey {
    fn of(spec: &FontSpec) -> Self {
        FaceKey {
            // Case-folded: a user typing "arial" and a stored "Arial" are one
            // face, and two cache entries for them would be two round trips.
            family: spec.family.to_lowercase(),
            bold: spec.bold,
            italic: spec.italic,
        }
    }
}

#[derive(Debug, Default, Clone)]
struct FaceData {
    ascent_milli: i16,
    descent_milli: i16,
    advances: HashMap<char, u16>,
    /// Characters this face was asked about and does not have. Cached too, so a
    /// paragraph full of unsupported glyphs asks once rather than per keystroke.
    missing: Vec<char>,
}

/// Metrics the UI has been told about, and can lay text out from.
#[derive(Debug, Default)]
pub struct FontCache {
    faces: Vec<(FaceKey, FaceData)>,
}

impl FontCache {
    fn index_of(&self, key: &FaceKey) -> Option<usize> {
        self.faces.iter().position(|(k, _)| k == key)
    }

    /// Characters of `text` this cache cannot answer for yet.
    fn unknown(&self, spec: &FontSpec, text: &str) -> String {
        let key = FaceKey::of(spec);
        match self.index_of(&key).and_then(|i| self.faces.get(i)) {
            None => text.chars().collect(),
            Some((_, data)) => text
                .chars()
                .filter(|c| !data.advances.contains_key(c) && !data.missing.contains(c))
                .collect(),
        }
    }

    fn store(
        &mut self,
        spec: &FontSpec,
        face: FaceMetrics,
        advances: Vec<(char, u16)>,
        missing: Vec<char>,
    ) {
        let key = FaceKey::of(spec);
        let index = match self.index_of(&key) {
            Some(i) => i,
            None => {
                self.faces.push((key, FaceData::default()));
                self.faces.len() - 1
            }
        };
        if let Some((_, data)) = self.faces.get_mut(index) {
            data.ascent_milli = face.ascent_milli;
            data.descent_milli = face.descent_milli;
            data.advances.extend(advances);
            for ch in missing {
                if !data.missing.contains(&ch) {
                    data.missing.push(ch);
                }
            }
        }
    }
}

impl FontCtx for FontCache {
    fn resolve(&self, spec: &FontSpec) -> Option<FontRef> {
        self.index_of(&FaceKey::of(spec)).map(|i| FontRef(i as u32))
    }

    fn glyph(&self, font: FontRef, ch: char) -> Option<GlyphMetrics> {
        let (_, data) = self.faces.get(font.0 as usize)?;
        let advance_milli = *data.advances.get(&ch)?;
        Some(GlyphMetrics {
            glyph_id: u32::from(ch) as u16,
            advance_milli,
        })
    }

    fn face(&self, font: FontRef) -> FaceMetrics {
        match self.faces.get(font.0 as usize) {
            Some((_, data)) => FaceMetrics {
                ascent_milli: data.ascent_milli,
                descent_milli: data.descent_milli,
            },
            None => FaceMetrics {
                ascent_milli: 750,
                descent_milli: -250,
            },
        }
    }
}

/// An image an annotation draws, held in memory until Phase 4 can embed it.
#[derive(Debug, Clone)]
pub struct StoredImage {
    /// The file's own bytes, untouched. Re-encoding a PNG into a PNG would be
    /// work that loses quality for nothing, and Phase 4 wants the original to
    /// embed anyway.
    pub bytes: Vec<u8>,
    pub media_type: String,
}

/// Everything the annotation layer keeps for the life of the run.
#[derive(Debug, Default)]
pub struct AnnotState {
    docs: Mutex<HashMap<u64, DocEdits>>,
    fonts: Mutex<FontCache>,
    /// Images by `(document, ImageRef)`.
    ///
    /// In the UI process, not in a worker: a worker can be killed at any moment
    /// by the supervisor (SPEC 3.4), and an image the user just inserted must
    /// not disappear with it.
    images: Mutex<HashMap<(u64, u32), StoredImage>>,
}

#[derive(Debug, Default)]
struct DocEdits {
    doc: AnnotDoc,
    stack: CommandStack,
}

/// What an edit did, for the frontend to reconcile with.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EditResult {
    pub objects: Vec<AnnotObject>,
    pub can_undo: bool,
    pub can_redo: bool,
}

impl AnnotState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops a document's annotations and its history.
    ///
    /// Called when a tab closes. Phase 4 will have to ask about unsaved work
    /// first; until saving exists there is nothing to lose that a warning could
    /// help with, and pretending otherwise would be theatre.
    pub fn forget(&self, doc: u64) {
        self.docs.lock().remove(&doc);
        self.images.lock().retain(|(d, _), _| *d != doc);
    }

    /// Stores an image's bytes and returns the handle the display list uses.
    ///
    /// The media type is taken from the file's own magic numbers rather than
    /// its extension: a `.png` that is really a JPEG is common enough, and the
    /// webview decides how to decode from the `Content-Type` we send.
    pub fn add_image(&self, doc: u64, bytes: Vec<u8>) -> Result<u32, String> {
        let media_type = sniff_media_type(&bytes)
            .ok_or_else(|| "format gambar tidak dikenali (PNG, JPEG, GIF, WebP)".to_string())?;
        let mut images = self.images.lock();
        let next = images
            .keys()
            .filter(|(d, _)| *d == doc)
            .map(|(_, r)| *r + 1)
            .max()
            .unwrap_or(0);
        images.insert(
            (doc, next),
            StoredImage {
                bytes,
                media_type: media_type.to_string(),
            },
        );
        Ok(next)
    }

    pub fn image(&self, doc: u64, image: u32) -> Option<StoredImage> {
        self.images.lock().get(&(doc, image)).cloned()
    }

    fn with<T>(&self, doc: u64, f: impl FnOnce(&mut DocEdits) -> T) -> T {
        let mut docs = self.docs.lock();
        f(docs.entry(doc).or_default())
    }

    pub fn objects(&self, doc: u64, page: Option<u32>) -> Vec<AnnotObject> {
        self.with(doc, |edits| match page {
            Some(p) => edits.doc.page(p).into_iter().cloned().collect(),
            None => edits.doc.iter().cloned().collect(),
        })
    }

    fn result(edits: &DocEdits, page: Option<u32>) -> EditResult {
        EditResult {
            objects: match page {
                Some(p) => edits.doc.page(p).into_iter().cloned().collect(),
                None => edits.doc.iter().cloned().collect(),
            },
            can_undo: edits.stack.can_undo(),
            can_redo: edits.stack.can_redo(),
        }
    }

    /// Applies one transaction and reports the page afterwards.
    pub fn commit(
        &self,
        doc: u64,
        page: Option<u32>,
        ops: Transaction,
    ) -> Result<EditResult, EditError> {
        self.with(doc, |edits| {
            let DocEdits { doc: model, stack } = edits;
            stack.commit(model, ops)?;
            Ok(Self::result(edits, page))
        })
    }

    /// Inserts an object, giving it an id if the caller did not.
    pub fn add(
        &self,
        doc: u64,
        mut obj: AnnotObject,
    ) -> Result<(AnnotObject, EditResult), EditError> {
        self.with(doc, |edits| {
            if obj.id.0 == 0 || edits.doc.get(obj.id).is_some() {
                obj.id = edits.doc.fresh_id();
            }
            obj.recompute_rect();
            let page = obj.page;
            let created = obj.clone();
            edits
                .stack
                .commit(&mut edits.doc, vec![Op::Insert(Box::new(obj))])?;
            Ok((created, Self::result(edits, Some(page))))
        })
    }

    /// Replaces objects wholesale — the shape every editing gesture takes once
    /// it is finished (SPEC 8's "one gesture, one undo step").
    pub fn replace(&self, doc: u64, objects: Vec<AnnotObject>) -> Result<EditResult, EditError> {
        self.with(doc, |edits| {
            let page = objects.first().map(|o| o.page);
            let mut ops: Transaction = Vec::with_capacity(objects.len());
            for mut after in objects {
                let Some(before) = edits.doc.get(after.id).cloned() else {
                    return Err(EditError::NotFound(after.id));
                };
                after.recompute_rect();
                ops.push(Op::Replace {
                    before: Box::new(before),
                    after: Box::new(after),
                });
            }
            edits.stack.commit(&mut edits.doc, ops)?;
            Ok(Self::result(edits, page))
        })
    }

    pub fn delete(&self, doc: u64, ids: &[u64]) -> Result<EditResult, EditError> {
        self.with(doc, |edits| {
            let mut page = None;
            let mut ops: Transaction = Vec::with_capacity(ids.len());
            for id in ids {
                let id = izul_model::annot::AnnotId(*id);
                let Some(obj) = edits.doc.get(id).cloned() else {
                    return Err(EditError::NotFound(id));
                };
                page = Some(obj.page);
                ops.push(Op::Delete(Box::new(obj)));
            }
            edits.stack.commit(&mut edits.doc, ops)?;
            Ok(Self::result(edits, page))
        })
    }

    pub fn undo(&self, doc: u64, page: Option<u32>) -> Result<EditResult, EditError> {
        self.with(doc, |edits| {
            let DocEdits { doc: model, stack } = edits;
            stack.undo(model)?;
            Ok(Self::result(edits, page))
        })
    }

    pub fn redo(&self, doc: u64, page: Option<u32>) -> Result<EditResult, EditError> {
        self.with(doc, |edits| {
            let DocEdits { doc: model, stack } = edits;
            stack.redo(model)?;
            Ok(Self::result(edits, page))
        })
    }

    pub fn history(&self, doc: u64) -> (bool, bool) {
        self.with(doc, |edits| {
            (edits.stack.can_undo(), edits.stack.can_redo())
        })
    }

    /// Makes sure the cache can answer for every character of `text` in `spec`,
    /// asking a worker if it cannot.
    ///
    /// Returns the characters the face does not have, so the caller can refuse
    /// the edit with a message naming them (SPEC 11.2) rather than laying out
    /// text that will draw as boxes.
    pub async fn ensure_metrics(
        &self,
        pool: &Arc<RwLock<Pool>>,
        doc: u64,
        spec: &FontSpec,
        text: &str,
    ) -> Result<Vec<char>, String> {
        let needed = self.fonts.lock().unknown(spec, text);
        if needed.is_empty() {
            let cache = self.fonts.lock();
            let missing = cache
                .index_of(&FaceKey::of(spec))
                .and_then(|i| cache.faces.get(i))
                .map(|(_, d)| {
                    text.chars()
                        .filter(|c| d.missing.contains(c))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            return Ok(missing);
        }
        let worker = { pool.read().await.worker_for(DocId(doc)) };
        let worker = worker.ok_or_else(|| "dokumen tidak terbuka".to_string())?;
        let req = Request::FontMetrics {
            family: spec.family.clone(),
            bold: spec.bold,
            italic: spec.italic,
            chars: needed,
        };
        match Pool::ask(&worker, req).await {
            Ok(Response::FontMetricsReady {
                ascent_milli,
                descent_milli,
                advances,
                missing,
                ..
            }) => {
                self.fonts.lock().store(
                    spec,
                    FaceMetrics {
                        ascent_milli,
                        descent_milli,
                    },
                    advances,
                    missing.clone(),
                );
                Ok(text.chars().filter(|c| missing.contains(c)).collect())
            }
            Ok(Response::Error {
                message_id, detail, ..
            }) => Err(format!("{message_id}: {detail}")),
            Ok(other) => Err(format!("balasan tak terduga: {other:?}")),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Builds the display lists for a page, with the metrics already cached.
    pub fn display_lists(&self, doc: u64, page: u32) -> Vec<(u64, izul_model::DisplayList)> {
        let fonts = self.fonts.lock();
        self.with(doc, |edits| {
            edits
                .doc
                .page(page)
                .into_iter()
                .filter_map(|obj| {
                    match izul_model::build::display_list(obj, &*fonts) {
                        Ok(list) => Some((obj.id.0, list)),
                        Err(e) => {
                            // A text object whose font is unavailable draws
                            // nothing rather than boxes; the panel already
                            // refused the edit that created it, and this is the
                            // belt to that braces.
                            tracing::debug!(id = obj.id.0, error = %e, "anotasi tidak dapat digambar");
                            None
                        }
                    }
                })
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use izul_model::annot::{AnnotId, AnnotKind, AnnotPayload, ShapeStyle};
    use izul_model::geom::PdfRectF;

    fn rect_obj(page: u32) -> AnnotObject {
        AnnotObject::new(
            AnnotId(0),
            page,
            AnnotKind::Rect,
            PdfRectF::new(0.0, 0.0, 10.0, 10.0),
            AnnotPayload::Shape {
                style: ShapeStyle::default(),
            },
        )
    }

    #[test]
    fn adding_gives_an_id_and_reports_the_page() {
        let state = AnnotState::new();
        let (created, result) = state.add(1, rect_obj(0)).expect("tambah");
        assert_ne!(created.id.0, 0, "id diberikan di sini, bukan oleh frontend");
        assert_eq!(result.objects.len(), 1);
        assert!(result.can_undo);
        assert!(!result.can_redo);
    }

    #[test]
    fn undo_and_redo_report_what_the_toolbar_should_show() {
        let state = AnnotState::new();
        state.add(1, rect_obj(0)).expect("tambah");
        let after_undo = state.undo(1, Some(0)).expect("undo");
        assert!(after_undo.objects.is_empty());
        assert!(!after_undo.can_undo);
        assert!(after_undo.can_redo);
        let after_redo = state.redo(1, Some(0)).expect("redo");
        assert_eq!(after_redo.objects.len(), 1);
    }

    #[test]
    fn a_page_only_reports_its_own_annotations() {
        let state = AnnotState::new();
        state.add(1, rect_obj(0)).expect("tambah");
        state.add(1, rect_obj(3)).expect("tambah");
        assert_eq!(state.objects(1, Some(0)).len(), 1);
        assert_eq!(state.objects(1, Some(3)).len(), 1);
        assert_eq!(state.objects(1, None).len(), 2);
    }

    #[test]
    fn documents_do_not_see_each_others_annotations() {
        let state = AnnotState::new();
        state.add(1, rect_obj(0)).expect("tambah");
        assert!(state.objects(2, Some(0)).is_empty());
        state.forget(1);
        assert!(state.objects(1, Some(0)).is_empty());
    }

    #[test]
    fn replacing_a_missing_object_is_refused_rather_than_inserted() {
        let state = AnnotState::new();
        let mut ghost = rect_obj(0);
        ghost.id = AnnotId(99);
        assert!(matches!(
            state.replace(1, vec![ghost]),
            Err(EditError::NotFound(_))
        ));
    }

    /// The cache is what keeps text editing from being a round trip per
    /// keystroke, so it has to actually answer once it has been told.
    #[test]
    fn the_font_cache_answers_for_what_it_has_been_told() {
        let mut cache = FontCache::default();
        let spec = FontSpec::default();
        assert_eq!(cache.unknown(&spec, "abc"), "abc");
        cache.store(
            &spec,
            FaceMetrics {
                ascent_milli: 700,
                descent_milli: -200,
            },
            vec![('a', 500), ('b', 520)],
            vec!['c'],
        );
        assert_eq!(cache.unknown(&spec, "abc"), "", "termasuk yang tidak ada");
        let font = cache.resolve(&spec).expect("sudah dikenal");
        assert_eq!(cache.glyph(font, 'a').map(|g| g.advance_milli), Some(500));
        assert_eq!(
            cache.glyph(font, 'c'),
            None,
            "glif yang tidak ada tetap tidak ada"
        );
        assert_eq!(cache.face(font).ascent_milli, 700);
    }

    #[test]
    fn a_face_is_one_entry_whatever_case_the_family_is_typed_in() {
        let mut cache = FontCache::default();
        cache.store(
            &FontSpec {
                family: "Arial".into(),
                ..Default::default()
            },
            FaceMetrics {
                ascent_milli: 700,
                descent_milli: -200,
            },
            vec![('a', 500)],
            Vec::new(),
        );
        let lower = FontSpec {
            family: "arial".into(),
            ..Default::default()
        };
        assert_eq!(cache.unknown(&lower, "a"), "");
        assert!(cache.resolve(&lower).is_some());
    }

    #[test]
    fn a_display_list_is_produced_for_a_shape_without_any_font_at_all() {
        let state = AnnotState::new();
        state.add(1, rect_obj(0)).expect("tambah");
        let lists = state.display_lists(1, 0);
        assert_eq!(lists.len(), 1);
        assert!(!lists[0].1.is_empty());
    }
}

/// The media type of an image, from its magic numbers.
///
/// Only the four formats a webview will certainly draw. Anything else is
/// refused at insert time with a message, rather than stored and found to be
/// undrawable later, when the user has already placed it on a page.
fn sniff_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() > 12 && bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Some("image/webp");
    }
    None
}

#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn formats_are_recognised_by_their_bytes_not_their_name() {
        assert_eq!(
            sniff_media_type(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]),
            Some("image/png")
        );
        assert_eq!(
            sniff_media_type(&[0xff, 0xd8, 0xff, 0xe0]),
            Some("image/jpeg")
        );
        assert_eq!(sniff_media_type(b"GIF89a....."), Some("image/gif"));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        webp.extend_from_slice(&[0; 8]);
        assert_eq!(sniff_media_type(&webp), Some("image/webp"));
    }

    /// A refusal at insert time is a message the user can act on; storing it and
    /// finding out later is an annotation that draws nothing.
    #[test]
    fn an_unknown_format_is_refused_rather_than_stored() {
        let state = AnnotState::new();
        assert!(state.add_image(1, b"not an image at all".to_vec()).is_err());
        assert!(state.image(1, 0).is_none());
    }

    #[test]
    fn images_are_numbered_per_document_and_dropped_with_it() {
        let state = AnnotState::new();
        let png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];
        assert_eq!(state.add_image(1, png.clone()), Ok(0));
        assert_eq!(state.add_image(1, png.clone()), Ok(1));
        assert_eq!(
            state.add_image(2, png),
            Ok(0),
            "dokumen lain mulai dari nol lagi"
        );
        assert!(state.image(1, 1).is_some());
        state.forget(1);
        assert!(state.image(1, 1).is_none());
        assert!(
            state.image(2, 0).is_some(),
            "dokumen lain tidak ikut terhapus"
        );
    }
}
