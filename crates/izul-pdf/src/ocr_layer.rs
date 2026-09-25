//! The invisible text layer OCR leaves on a scanned page (Phase 7).
//!
//! Each recognised word becomes one text object in render mode 3 (neither
//! filled nor stroked — ISO 32000-1 §9.3.6), in Helvetica, sized and
//! stretched so that its box covers the word as it appears in the picture.
//! Nothing on the page changes to the eye; search, selection and copy now
//! find the words where they are seen.
//!
//! Words arrive in display space (see `izul_model::geom::PageFrame`), as the
//! page was rendered for recognition, and are carried into user space here,
//! so a scan stored with `/Rotate` gets its words the right way round.

use pdfium_render::prelude::*;

use crate::engine::Document;
use crate::error::{PdfError, Result};
use crate::ffi_guard::guard;
use crate::geom::PdfRectF;

/// `FPDF_TEXTRENDERMODE_INVISIBLE` from `fpdf_edit.h`: render mode 3.
const INVISIBLE: FPDF_TEXT_RENDERMODE = 3;

/// Helvetica's ascender and descender, in thousandths of an em (AFM).
const ASCENT: f32 = 718.0;
const DESCENT: f32 = -207.0;

/// A word to lay down: its text and its box in display space.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrWord {
    pub text: String,
    pub rect: PdfRectF,
    /// Write a space after the word. Extractors that do not guess spaces from
    /// gaps (pypdf) otherwise run a whole line into one word; the word is
    /// still fitted to its box without it.
    pub space_after: bool,
}

fn utf16z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Document {
    /// Writes `words` onto `page` as invisible text and regenerates its
    /// content. Returns how many words were written; a word PDFium will not
    /// take (empty, or a box with no size) is skipped and not counted.
    pub fn add_invisible_words(&self, page: u32, words: &[OcrWord]) -> Result<u32> {
        self.check_page(page)?;
        let to_user = self.page_geometry(page)?.frame().display_to_user();
        let bindings = self.engine().bindings();
        let doc = self.handle();
        let written = guard("add_invisible_words", || {
            self.with_page(page, |p| {
                // SAFETY: `doc` and `p` are live; the font is closed after the
                // content is generated, which is when PDFium last needs it by
                // handle (the objects hold their own reference); every text
                // object is either inserted (the page owns it) or destroyed.
                unsafe {
                    let font = bindings.FPDFText_LoadStandardFont(doc, "Helvetica");
                    if font.is_null() {
                        return Err(PdfError::Corrupt {
                            detail: "font Helvetica tidak dapat dimuat".into(),
                        });
                    }
                    let mut written = 0u32;
                    for w in words {
                        let text = w.text.trim();
                        let (bw, bh) = (w.rect.width(), w.rect.height());
                        if text.is_empty() || bw <= 0.0 || bh <= 0.0 {
                            continue;
                        }
                        let obj = bindings.FPDFPageObj_CreateTextObj(doc, font, 1.0);
                        if obj.is_null() {
                            continue;
                        }
                        let wide = utf16z(text);
                        let mut set =
                            bindings.FPDFText_SetText(obj, wide.as_ptr() as FPDF_WIDESTRING);
                        let mode = bindings.FPDFTextObj_SetTextRenderMode(obj, INVISIBLE);
                        let (mut l, mut b, mut r, mut t) = (0f32, 0f32, 0f32, 0f32);
                        let measured =
                            bindings.FPDFPageObj_GetBounds(obj, &mut l, &mut b, &mut r, &mut t);
                        let natural = r - l;
                        if set != 0 && w.space_after {
                            let spaced = utf16z(&format!("{text} "));
                            set =
                                bindings.FPDFText_SetText(obj, spaced.as_ptr() as FPDF_WIDESTRING);
                        }
                        if set == 0 || mode == 0 || measured == 0 || natural <= 0.0 {
                            bindings.FPDFPageObj_Destroy(obj);
                            continue;
                        }
                        // The em that makes ascender-to-descender the box's
                        // height, the stretch that makes the width match, and
                        // the baseline where the descender reaches the bottom.
                        let size = bh * 1000.0 / (ASCENT - DESCENT);
                        let stretch = bw / (natural * size);
                        let base = crate::geom::Matrix {
                            a: size * stretch,
                            b: 0.0,
                            c: 0.0,
                            d: size,
                            e: w.rect.left - l * size * stretch,
                            f: w.rect.bottom - DESCENT / 1000.0 * size,
                        };
                        let m = base.then(&to_user);
                        bindings.FPDFPageObj_Transform(
                            obj,
                            f64::from(m.a),
                            f64::from(m.b),
                            f64::from(m.c),
                            f64::from(m.d),
                            f64::from(m.e),
                            f64::from(m.f),
                        );
                        bindings.FPDFPage_InsertObject(p, obj);
                        written += 1;
                    }
                    let generated = bindings.FPDFPage_GenerateContent(p);
                    bindings.FPDFFont_Close(font);
                    if generated == 0 {
                        return Err(PdfError::Corrupt {
                            detail: format!("isi halaman {page} tidak dapat ditulis ulang"),
                        });
                    }
                    Ok(written)
                }
            })
        })?;
        // Text pages cached for this page no longer describe it.
        self.release_page(page);
        Ok(written)
    }
}
