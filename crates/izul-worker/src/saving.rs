//! Phase 4's requests: reading our annotations back, and the working copy a
//! save or an export is built on (SPEC 8, SPEC 11.3).
//!
//! The worker never writes a user's file. It produces bytes — a saved PDF, a
//! flattened copy, a page as PNG — and keeps them as a *blob* until the UI
//! process has fetched them piece by piece; the UI process, which owns the
//! user's files, writes them with an atomic replace. That keeps file writing
//! in the process that is not parsing untrusted input, and it is what will
//! keep saving working once workers run with a low-integrity token (SPEC 3.4).

use std::collections::HashMap;

use izul_ipc::message::{
    ArrangePage, DocId, RedactPageWire, RedactedPageWire, Request, Response, SavedAnnotWire,
    BLOB_CHUNK,
};
use izul_pdf::redaction::{check_left, redact_document, AreaRequest, PageRequest, RedactFailure};
use izul_pdf::{ArrangeSource, Arranged, Document, Engine, PdfError, Quality};

/// Working copies and blobs, per worker.
#[derive(Default)]
pub struct Workbench {
    work: HashMap<DocId, Document>,
    blobs: HashMap<u64, Vec<u8>>,
    next_blob: u64,
    /// The OCR engine, loaded on first use and kept: loading it takes a few
    /// milliseconds, but 12 MB of models read again for every page would not.
    ocr: Option<(String, izul_ocr::Ocr)>,
    /// The background model, loaded on first use: (runtime, model, session).
    background: Option<(String, String, izul_ocr::background::BackgroundRemover)>,
}

impl std::fmt::Debug for Workbench {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workbench")
            .field("work", &self.work.len())
            .field("blobs", &self.blobs.len())
            .finish()
    }
}

/// Why a request failed, for `fail`/`fail_kind` in `main.rs`.
#[derive(Debug)]
pub enum Failure {
    Pdf(Option<DocId>, PdfError),
    NoWorkingCopy(DocId),
    NoBlob,
    Encode(String),
    /// A redaction that could not be done, or did not verify. The detail is
    /// shown to the user as it is.
    Redact(Option<DocId>, String),
    /// OCR could not run (models missing or unreadable, or the engine failed).
    Ocr(Option<DocId>, String),
    /// Background removal could not run; the detail is for the user.
    Ai(String),
}

/// The largest picture accepted for background removal: 64 MB of encoded
/// image, far past any photo, well short of exhausting a worker.
const MAX_INPUT_BLOB: usize = 64 * 1024 * 1024;

impl Workbench {
    pub fn new() -> Self {
        Self::default()
    }

    fn put_blob(&mut self, bytes: Vec<u8>) -> Response {
        self.next_blob += 1;
        let blob = self.next_blob;
        let len = bytes.len() as u64;
        self.blobs.insert(blob, bytes);
        Response::BlobReady { blob, len }
    }

    /// One piece of a blob, never more than [`BLOB_CHUNK`] bytes.
    pub fn read_blob(&self, blob: u64, offset: u64, len: u32) -> Result<Response, Failure> {
        let bytes = self.blobs.get(&blob).ok_or(Failure::NoBlob)?;
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        let end = start
            .saturating_add(len.min(BLOB_CHUNK) as usize)
            .min(bytes.len());
        let data = bytes.get(start..end).unwrap_or_default().to_vec();
        Ok(Response::BlobBytes { blob, data })
    }

    /// Loads the OCR engine from `models` unless it is loaded from there.
    fn ocr(&mut self, models: &str) -> Result<(), Failure> {
        if self.ocr.as_ref().is_none_or(|(dir, _)| dir != models) {
            let dir = std::path::Path::new(models);
            let engine = izul_ocr::Ocr::load(
                &dir.join("text-detection.rten"),
                &dir.join("text-recognition.rten"),
            )
            .map_err(|e| Failure::Ocr(None, e.to_string()))?;
            self.ocr = Some((models.to_string(), engine));
        }
        Ok(())
    }

    fn work(&self, doc: DocId) -> Result<&Document, Failure> {
        self.work.get(&doc).ok_or(Failure::NoWorkingCopy(doc))
    }

    /// Forgets everything held for `doc`, for when its tab closes.
    pub fn forget(&mut self, doc: DocId) {
        self.work.remove(&doc);
    }

    /// Handles one Phase 4 request. `viewing` is the document the tab has open,
    /// for `PageAnnots`.
    pub fn handle(
        &mut self,
        engine: &'static Engine,
        viewing: impl Fn(DocId, u32) -> Option<Result<Vec<izul_pdf::izul::IzulAnnot>, PdfError>>,
        request: Request,
    ) -> Result<Response, Failure> {
        match request {
            Request::PageAnnots { doc, page } => {
                let found = viewing(doc, page)
                    .ok_or(Failure::NoWorkingCopy(doc))?
                    .map_err(|e| Failure::Pdf(Some(doc), e))?;
                let annots = found
                    .into_iter()
                    .map(|a| {
                        let image = a.image.map(|img| {
                            let mime = img.mime.to_string();
                            let Response::BlobReady { blob, len } = self.put_blob(img.bytes) else {
                                return (0, 0, mime);
                            };
                            (blob, len, mime)
                        });
                        SavedAnnotWire {
                            metadata: a.metadata,
                            image,
                        }
                    })
                    .collect();
                Ok(Response::PageAnnotsReady { doc, page, annots })
            }
            Request::WorkOpen { doc, path } => {
                // Read into memory, not mapped: the UI is about to replace this
                // very file, and Windows refuses to replace a mapped one.
                let copy = engine
                    .open_copied(&path, None)
                    .map_err(|e| Failure::Pdf(Some(doc), e))?;
                copy.set_strip_izul(false);
                let page_count = copy.page_count();
                self.work.insert(doc, copy);
                Ok(Response::WorkReady { doc, page_count })
            }
            Request::WorkPlaceholders {
                doc,
                pages,
                placeholders,
            } => {
                let work = self.work(doc)?;
                for page in pages {
                    let ids: Vec<u64> = placeholders
                        .iter()
                        .filter(|(p, _)| *p == page)
                        .map(|(_, id)| *id)
                        .collect();
                    work.put_placeholders(page, &ids)
                        .map_err(|e| Failure::Pdf(Some(doc), e))?;
                }
                Ok(Response::WorkReady {
                    doc,
                    page_count: work.page_count(),
                })
            }
            Request::WorkFlatten { doc, first, count } => {
                let work = self.work(doc)?;
                let end = first.saturating_add(count).min(work.page_count());
                for page in first..end {
                    work.flatten_page(page)
                        .map_err(|e| Failure::Pdf(Some(doc), e))?;
                }
                Ok(Response::WorkReady {
                    doc,
                    page_count: work.page_count(),
                })
            }
            Request::WorkArrange {
                doc,
                pages,
                sources,
            } => {
                let work = self.work(doc)?;
                let pdf = |e| Failure::Pdf(Some(doc), e);
                // Every other file the map draws on, opened once, read into
                // memory: these are copies the UI process made, and nothing
                // here may hold a user's file open.
                let mut opened: HashMap<u32, Document> = HashMap::new();
                let mut own_seen = std::collections::HashSet::new();
                let mut own_twice = false;
                for page in &pages {
                    match *page {
                        ArrangePage::Page {
                            source: 0, page, ..
                        } => {
                            own_twice |= !own_seen.insert(page);
                        }
                        ArrangePage::Page { source, .. } if !opened.contains_key(&source) => {
                            let path = sources.get(source as usize).ok_or_else(|| {
                                Failure::Encode(format!("sumber halaman {source} tidak dikenal"))
                            })?;
                            let doc = engine.open_copied(path, None).map_err(pdf)?;
                            doc.set_strip_izul(false);
                            opened.insert(source, doc);
                        }
                        _ => {}
                    }
                }
                // A second handle on the file itself, only for a duplicated
                // page (see `Document::arrange`).
                let again = if own_twice {
                    let path = sources
                        .first()
                        .ok_or_else(|| Failure::Encode("berkas asal tidak disebut".into()))?;
                    let again = engine.open_copied(path, None).map_err(pdf)?;
                    again.set_strip_izul(false);
                    Some(again)
                } else {
                    None
                };
                let mut plan = Vec::with_capacity(pages.len());
                for page in &pages {
                    plan.push(match *page {
                        ArrangePage::Page {
                            source: 0,
                            page,
                            rotation,
                        } => Arranged {
                            source: ArrangeSource::Own(page),
                            rotation,
                        },
                        ArrangePage::Page {
                            source,
                            page,
                            rotation,
                        } => Arranged {
                            source: ArrangeSource::From(
                                opened.get(&source).ok_or_else(|| {
                                    Failure::Encode(format!(
                                        "sumber halaman {source} tidak terbuka"
                                    ))
                                })?,
                                page,
                            ),
                            rotation,
                        },
                        ArrangePage::Blank {
                            width,
                            height,
                            rotation,
                        } => Arranged {
                            source: ArrangeSource::Blank { width, height },
                            rotation,
                        },
                    });
                }
                work.arrange(&plan, again.as_ref()).map_err(pdf)?;
                Ok(Response::WorkReady {
                    doc,
                    page_count: work.page_count(),
                })
            }
            Request::WorkExtract { doc, pages } => {
                let bytes = self
                    .work(doc)?
                    .extract_pages(&pages)
                    .map_err(|e| Failure::Pdf(Some(doc), e))?;
                let copy = engine
                    .open_bytes(bytes, None::<&str>, None)
                    .map_err(|e| Failure::Pdf(Some(doc), e))?;
                copy.set_strip_izul(false);
                let page_count = copy.page_count();
                self.work.insert(doc, copy);
                Ok(Response::WorkReady { doc, page_count })
            }
            Request::WorkRender {
                doc,
                page,
                scale,
                jpeg_quality,
            } => {
                let (geom, bgra) = self
                    .work(doc)?
                    .render_page(page, scale.clamp(0.1, 16.0), Quality::Sharp)
                    .map_err(|e| Failure::Pdf(Some(doc), e))?;
                let encoded = encode(geom.width, geom.height, geom.stride, &bgra, jpeg_quality)?;
                Ok(self.put_blob(encoded))
            }
            Request::WorkSave { doc } => {
                let bytes = self
                    .work(doc)?
                    .save_to_vec()
                    .map_err(|e| Failure::Pdf(Some(doc), e))?;
                Ok(self.put_blob(bytes))
            }
            Request::WorkClose { doc } => {
                self.work.remove(&doc);
                Ok(Response::Closed { doc })
            }
            Request::BlobRead { blob, offset, len } => self.read_blob(blob, offset, len),
            Request::BlobAppend { blob, data } => {
                let blob = if blob == 0 {
                    self.next_blob += 1;
                    self.blobs.insert(self.next_blob, Vec::new());
                    self.next_blob
                } else {
                    blob
                };
                let Some(bytes) = self.blobs.get_mut(&blob) else {
                    return Err(Failure::NoBlob);
                };
                // A picture, not a file system: refuse what no picture is.
                if bytes.len() + data.len() > MAX_INPUT_BLOB {
                    self.blobs.remove(&blob);
                    return Err(Failure::Encode("gambar terlalu besar".into()));
                }
                bytes.extend_from_slice(&data);
                Ok(Response::BlobAppended {
                    blob,
                    len: bytes.len() as u64,
                })
            }
            Request::RemoveBackground {
                blob,
                runtime,
                model,
                kind,
            } => {
                let bytes = self.blobs.remove(&blob).ok_or(Failure::NoBlob)?;
                let picture = image::load_from_memory(&bytes)
                    .map_err(|e| Failure::Ai(format!("gambar tidak terbaca: {e}")))?
                    .to_rgba8();
                let fresh = self
                    .background
                    .as_ref()
                    .is_none_or(|(r, m, _)| *r != runtime || *m != model);
                if fresh {
                    let remover = izul_ocr::background::BackgroundRemover::load(
                        std::path::Path::new(&runtime),
                        std::path::Path::new(&model),
                    )
                    .map_err(|e| Failure::Ai(e.to_string()))?;
                    self.background = Some((runtime, model, remover));
                }
                let Some((_, _, remover)) = self.background.as_mut() else {
                    return Err(Failure::Ai("model tidak dimuat".into()));
                };
                let device = match remover.device() {
                    izul_ocr::background::Device::DirectMl => "DirectML".to_string(),
                    izul_ocr::background::Device::Cpu { reason } if reason.is_empty() => {
                        "CPU".to_string()
                    }
                    izul_ocr::background::Device::Cpu { reason } => format!("CPU ({reason})"),
                };
                let kind = match kind {
                    izul_ipc::message::BackgroundKind::Photo => izul_ocr::background::Kind::Photo,
                    izul_ipc::message::BackgroundKind::OnPaper => {
                        izul_ocr::background::Kind::OnPaper
                    }
                };
                let (out, paper) = remover
                    .remove(&picture, kind)
                    .map_err(|e| Failure::Ai(e.to_string()))?;
                let mut png = std::io::Cursor::new(Vec::new());
                out.write_to(&mut png, image::ImageFormat::Png)
                    .map_err(|e| Failure::Encode(e.to_string()))?;
                let Response::BlobReady { blob, len } = self.put_blob(png.into_inner()) else {
                    return Err(Failure::NoBlob);
                };
                Ok(Response::BackgroundRemoved {
                    blob,
                    len,
                    device,
                    paper,
                })
            }
            Request::BlobDrop { blob } => {
                self.blobs.remove(&blob);
                Ok(Response::BlobReady { blob, len: 0 })
            }
            Request::VerifyFile { path, pages } => {
                let copy = engine
                    .open_copied(&path, None)
                    .map_err(|e| Failure::Pdf(None, e))?;
                copy.set_strip_izul(false);
                let mut izul_annots = 0;
                for page in pages {
                    izul_annots += copy.count_izul(page).map_err(|e| Failure::Pdf(None, e))?;
                }
                Ok(Response::Verified {
                    page_count: copy.page_count(),
                    izul_annots,
                })
            }
            Request::WorkRedact { doc, pages } => {
                let pages = self.redact(doc, &pages)?;
                Ok(Response::WorkRedacted { doc, pages })
            }
            Request::VerifyRedacted { path, pages } => {
                let copy = engine
                    .open_copied(&path, None)
                    .map_err(|e| Failure::Pdf(None, e))?;
                for (page, areas) in &pages {
                    let after = copy.char_layout(*page).map_err(|e| Failure::Pdf(None, e))?;
                    check_left(areas, &after)
                        .map_err(|e| Failure::Redact(None, format!("halaman {}: {e}", page + 1)))?;
                }
                Ok(Response::RedactionVerified {
                    pages: pages.len() as u32,
                })
            }
            Request::WorkOcr {
                doc,
                page,
                models,
                force,
            } => {
                if !self.work.contains_key(&doc) {
                    return Err(Failure::NoWorkingCopy(doc));
                }
                self.ocr(&models)?;
                let (Some(work), Some((_, ocr))) = (self.work.get(&doc), self.ocr.as_ref()) else {
                    return Err(Failure::NoWorkingCopy(doc));
                };
                let outcome = izul_ocr::ocr_page(work, page, ocr, force).map_err(|e| match e {
                    izul_ocr::OcrError::Pdf(p) => Failure::Pdf(Some(doc), p),
                    other => Failure::Ocr(Some(doc), other.to_string()),
                })?;
                Ok(Response::WorkOcrDone {
                    doc,
                    page,
                    words: match outcome {
                        izul_ocr::PageOutcome::HadText => None,
                        izul_ocr::PageOutcome::Read { words } => Some(words),
                    },
                })
            }
            Request::FillForm {
                doc,
                working: true,
                values,
            } => {
                let work = self.work(doc)?;
                // A field on a page the user deleted is nothing to fail a
                // save over; every other refusal is.
                let present = work
                    .form_widgets()
                    .map_err(|e| Failure::Pdf(Some(doc), e))?;
                let values: Vec<_> = values
                    .into_iter()
                    .filter(|(n, _)| present.iter().any(|w| &w.name == n))
                    .collect();
                let (changed, pages) = work
                    .fill_form(&values)
                    .map_err(|e| Failure::Pdf(Some(doc), e))?;
                Ok(Response::FormFilled {
                    doc,
                    changed,
                    pages,
                })
            }
            Request::WorkFrames { doc } => {
                let work = self.work(doc)?;
                let frames = (0..work.page_count())
                    .map(|p| work.page_geometry(p).map(|g| g.frame()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| Failure::Pdf(Some(doc), e))?;
                Ok(Response::WorkFramesReady { doc, frames })
            }
            other => Err(Failure::Encode(format!(
                "bukan permintaan Fase 4: {other:?}"
            ))),
        }
    }

    /// Redacts the working copy (`izul_pdf::redaction::redact_document`,
    /// the routine the proof tool runs too). On any failure the working copy
    /// is left as it was.
    fn redact(
        &mut self,
        doc: DocId,
        pages: &[RedactPageWire],
    ) -> Result<Vec<RedactedPageWire>, Failure> {
        let requests: Vec<PageRequest> = pages
            .iter()
            .map(|p| PageRequest {
                page: p.page,
                areas: p
                    .areas
                    .iter()
                    .map(|a| AreaRequest {
                        rect: a.rect,
                        fill: a.fill,
                    })
                    .collect(),
            })
            .collect();
        let (copy, results) = redact_document(self.work(doc)?, &requests).map_err(|e| match e {
            RedactFailure::Pdf(e) => Failure::Pdf(Some(doc), e),
            RedactFailure::Refused(detail) => Failure::Redact(Some(doc), detail),
        })?;
        copy.set_strip_izul(false);
        self.work.insert(doc, copy);
        Ok(results
            .into_iter()
            .map(|r| RedactedPageWire {
                page: r.page,
                areas: r.areas,
                glyphs: r.counts.glyphs,
                images_removed: r.counts.images_removed,
                images_cleared: r.counts.images_cleared,
                images_unsupported: r.counts.images_unsupported,
                paths: r.counts.paths,
                forms: r.counts.forms,
                annotations: r.annotations,
                marked_content: r.counts.marked_content,
            })
            .collect())
    }
}

/// PNG, or JPEG on white when a quality is given — a JPEG has no alpha, and a
/// transparent page corner turned black would be worse than white.
pub fn encode(
    width: u32,
    height: u32,
    stride: usize,
    bgra: &[u8],
    jpeg_quality: Option<u8>,
) -> Result<Vec<u8>, Failure> {
    let mut out = std::io::Cursor::new(Vec::new());
    match jpeg_quality {
        None => {
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for row in bgra.chunks(stride).take(height as usize) {
                for px in row.chunks_exact(4).take(width as usize) {
                    if let [b, g, r, a] = *px {
                        rgba.extend_from_slice(&[r, g, b, a]);
                    }
                }
            }
            let img = image::RgbaImage::from_raw(width, height, rgba)
                .ok_or_else(|| Failure::Encode("ukuran bitmap".into()))?;
            img.write_to(&mut out, image::ImageFormat::Png)
                .map_err(|e| Failure::Encode(e.to_string()))?;
        }
        Some(q) => {
            let mut rgb = Vec::with_capacity((width * height * 3) as usize);
            for row in bgra.chunks(stride).take(height as usize) {
                for px in row.chunks_exact(4).take(width as usize) {
                    if let [b, g, r, a] = *px {
                        let a = u16::from(a);
                        let over = |c: u8| ((u16::from(c) * a + 255 * (255 - a)) / 255) as u8;
                        rgb.extend_from_slice(&[over(r), over(g), over(b)]);
                    }
                }
            }
            let mut enc =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q.clamp(1, 100));
            enc.encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
                .map_err(|e| Failure::Encode(e.to_string()))?;
        }
    }
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn png_keeps_colour_and_alpha_and_jpeg_goes_over_white() {
        // One opaque blue pixel and one fully transparent one, BGRA.
        let bgra = [255, 0, 0, 255, 0, 0, 0, 0];
        let png = encode(2, 1, 8, &bgra, None).unwrap();
        let back = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(back.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(back.get_pixel(1, 0).0[3], 0);

        let jpg = encode(2, 1, 8, &bgra, Some(95)).unwrap();
        let back = image::load_from_memory(&jpg).unwrap().to_rgb8();
        let corner = back.get_pixel(1, 0).0;
        assert!(
            corner.iter().all(|c| *c > 240),
            "transparent became white: {corner:?}"
        );
    }

    #[test]
    fn a_blob_is_handed_out_in_pieces_and_then_forgotten() {
        let mut bench = Workbench::new();
        let Response::BlobReady { blob, len } = bench.put_blob((0..10u8).collect()) else {
            panic!("bukan BlobReady")
        };
        assert_eq!(len, 10);
        let piece = |off, n| match bench.read_blob(blob, off, n).unwrap() {
            Response::BlobBytes { data, .. } => data,
            _ => panic!("bukan BlobBytes"),
        };
        assert_eq!(piece(0, 4), [0, 1, 2, 3]);
        assert_eq!(piece(8, 4), [8, 9], "the last piece is short");
        assert!(
            piece(99, 4).is_empty(),
            "past the end is empty, not an error"
        );
        // Never more than one chunk, whatever is asked for.
        let Response::BlobReady { blob: big, .. } =
            bench.put_blob(vec![0; BLOB_CHUNK as usize + 5])
        else {
            panic!()
        };
        match bench.read_blob(big, 0, u32::MAX).unwrap() {
            Response::BlobBytes { data, .. } => assert_eq!(data.len(), BLOB_CHUNK as usize),
            _ => panic!(),
        }
        bench.blobs.remove(&blob);
        assert!(matches!(bench.read_blob(blob, 0, 1), Err(Failure::NoBlob)));
    }
}
