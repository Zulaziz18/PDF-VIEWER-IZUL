//! Local OCR for scanned pages (SPEC 17, Phase 7).
//!
//! [`Ocr`] is PP-OCR (PaddleOCR) run by ONNX Runtime, see [`paddle`]: an
//! image in, lines of words out, each word with its box in the image's own
//! pixels (origin top-left, y down).
//! [`ocr_page`] is the one routine that reads a PDF page — render, recognise,
//! lay the words down as invisible text — run by the worker and by the proof
//! tool alike, so what is measured is what ships.
//!
//! Until 7.0.0 the engine was `ocrs`. It was replaced after
//! `tools/ocr-bakeoff` measured both on the same pages: character error rate
//! 11.0 % against 1.9 % over five kinds of scan, 42 % against 2 % on phone
//! photos (`bench/results/ocr-bakeoff.txt`) — and the `ocrs` weights had no
//! stated licence, while PP-OCR's are Apache-2.0.

pub mod background;
mod paddle;

use std::path::Path;

use izul_pdf::ocr_layer::OcrWord;
use izul_pdf::{Document, PdfRectF, Quality};

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
/// Chosen for `ocrs` (character error rate 0.50 % at 135 dpi, 0.88 % at 150,
/// 2.59 % at 300 on `tools/ocr-proof` scans) and still right for PP-OCRv6:
/// on `tools/ocr-bakeoff` its error rate is 1.85 % at 150 dpi against 2.95 %
/// at 300, and 300 is four times slower. The models were trained on text
/// about this size; more pixels make things worse, not better.
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

/// The OCR engine: PP-OCR detection and recognition (see [`paddle`]).
#[derive(Debug)]
pub struct Ocr {
    engine: paddle::Paddle,
}

impl Ocr {
    /// Loads ONNX Runtime from `runtime` and the detection and recognition
    /// models.
    pub fn load(runtime: &Path, detection: &Path, recognition: &Path) -> Result<Self, OcrError> {
        Ok(Ocr {
            engine: paddle::Paddle::load(runtime, detection, recognition)?,
        })
    }

    /// Reads a greyscale image (one byte per pixel, rows top to bottom).
    pub fn read_gray(&self, pixels: &[u8], width: u32, height: u32) -> Result<Vec<Line>, OcrError> {
        self.engine.read_gray(pixels, width, height)
    }
}
