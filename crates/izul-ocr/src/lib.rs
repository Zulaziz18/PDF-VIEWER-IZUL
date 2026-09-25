//! Local OCR for scanned pages (SPEC 17, Phase 7).
//!
//! [`Ocr`] is a thin layer over [`ocrs`]: an image in, lines of words out,
//! each word with its box in the image's own pixels (origin top-left, y down).
//! [`ocr_page`] is the one routine that reads a PDF page — render, recognise,
//! lay the words down as invisible text — run by the worker and by the proof
//! tool alike, so what is measured is what ships.
//!
//! `ocrs` recognises the Latin alphabet without accents, which is what printed
//! Indonesian and English need. It has no dictionary; what it reads is what is
//! on the page, letter by letter.

use std::path::Path;

use izul_pdf::ocr_layer::OcrWord;
use izul_pdf::{Document, PdfRectF, Quality};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("model OCR tidak bisa dimuat ({path}): {detail}")]
    Model { path: String, detail: String },
    #[error("gambar tidak valid: {0}")]
    Image(String),
    #[error("OCR gagal: {0}")]
    Engine(String),
    #[error(transparent)]
    Pdf(#[from] izul_pdf::PdfError),
}

/// Pixels per point the page is rendered at for recognition: 150 dpi.
/// Measured on `tools/ocr-proof` scans (10–12 pt text): character error rate
/// 0.67 % at 100 dpi, 0.50 % at 135, 0.88 % at 150, 1.88 % at 200, 2.59 % at
/// 300 — the models were trained on text about this size, and more pixels
/// make things worse, not better. 150 keeps small print legible without
/// giving that away.
pub const RENDER_SCALE: f32 = 150.0 / 72.0;

/// A page with at least this many non-blank characters already has text;
/// OCR leaves it alone unless asked to anyway.
pub const HAS_TEXT: usize = 20;

/// What happened to one page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageOutcome {
    /// The page already had a text layer and was left as it was.
    HadText,
    /// Words were recognised and written as invisible text.
    Read { words: u32 },
}

/// Reads one page of `doc` and writes what it read onto the page as invisible
/// text (see `izul_pdf::ocr_layer`). A page that already has text is skipped
/// unless `force`.
pub fn ocr_page(
    doc: &Document,
    page: u32,
    ocr: &Ocr,
    force: bool,
) -> Result<PageOutcome, OcrError> {
    if !force {
        let existing = doc.page_text(page)?;
        if existing.chars().filter(|c| !c.is_whitespace()).count() >= HAS_TEXT {
            return Ok(PageOutcome::HadText);
        }
    }
    let size = doc.page_size(page)?;
    let (geom, bgra) = doc.render_page(page, RENDER_SCALE, Quality::Sharp)?;
    let (w, h) = (geom.width, geom.height);
    let stride = geom.stride;
    let mut gray = Vec::with_capacity(w as usize * h as usize);
    for row in 0..h as usize {
        let line = bgra
            .get(row * stride..row * stride + w as usize * 4)
            .unwrap_or_default();
        gray.extend(line.chunks_exact(4).map(|p| match p {
            [b, g, r, _] => {
                ((u32::from(*r) * 299 + u32::from(*g) * 587 + u32::from(*b) * 114) / 1000) as u8
            }
            _ => 255,
        }));
    }
    let lines = ocr.read_gray(&gray, w, h)?;
    // Pixels (y down, from the rendered page's top-left) to display space
    // (points, y up, from its bottom-left).
    let sx = size.width / w as f32;
    let sy = size.height / h as f32;
    let words: Vec<OcrWord> = lines
        .iter()
        .flat_map(|l| {
            let last = l.words.len().saturating_sub(1);
            l.words
                .iter()
                .enumerate()
                .map(move |(i, wd)| (i < last, wd))
        })
        .map(|(space_after, wd)| OcrWord {
            text: wd.text.clone(),
            rect: PdfRectF::new(
                wd.rect[0] * sx,
                size.height - wd.rect[3] * sy,
                wd.rect[2] * sx,
                size.height - wd.rect[1] * sy,
            ),
            space_after,
        })
        .collect();
    let written = doc.add_invisible_words(page, &words)?;
    Ok(PageOutcome::Read { words: written })
}

/// One recognised word: its text and its box in image pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    /// `[left, top, right, bottom]`, pixels, y down.
    pub rect: [f32; 4],
}

/// Words in reading order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Line {
    pub words: Vec<Word>,
}

pub struct Ocr {
    engine: OcrEngine,
}

impl std::fmt::Debug for Ocr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ocr")
    }
}

fn model(path: &Path) -> Result<rten::Model, OcrError> {
    rten::Model::load_file(path).map_err(|e| OcrError::Model {
        path: path.display().to_string(),
        detail: e.to_string(),
    })
}

impl Ocr {
    /// Loads the detection and recognition models.
    pub fn load(detection: &Path, recognition: &Path) -> Result<Self, OcrError> {
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(model(detection)?),
            recognition_model: Some(model(recognition)?),
            ..OcrEngineParams::default()
        })
        .map_err(|e| OcrError::Engine(e.to_string()))?;
        Ok(Ocr { engine })
    }

    /// Reads a greyscale image (one byte per pixel, rows top to bottom).
    pub fn read_gray(&self, pixels: &[u8], width: u32, height: u32) -> Result<Vec<Line>, OcrError> {
        let source = ImageSource::from_bytes(pixels, (width, height))
            .map_err(|e| OcrError::Image(e.to_string()))?;
        let input = self
            .engine
            .prepare_input(source)
            .map_err(|e| OcrError::Engine(e.to_string()))?;
        let words = self
            .engine
            .detect_words(&input)
            .map_err(|e| OcrError::Engine(e.to_string()))?;
        let lines = self.engine.find_text_lines(&input, &words);
        let read = self
            .engine
            .recognize_text(&input, &lines)
            .map_err(|e| OcrError::Engine(e.to_string()))?;
        Ok(read
            .into_iter()
            .flatten()
            .map(|line| Line {
                words: line
                    .words()
                    .map(|w| {
                        let r = w.bounding_rect();
                        Word {
                            text: w.to_string(),
                            rect: [
                                r.left() as f32,
                                r.top() as f32,
                                r.right() as f32,
                                r.bottom() as f32,
                            ],
                        }
                    })
                    .collect(),
            })
            .filter(|l: &Line| !l.words.is_empty())
            .collect())
    }
}
