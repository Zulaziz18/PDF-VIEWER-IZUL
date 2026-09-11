use std::os::raw::{c_int, c_ushort};

use pdfium_render::prelude::{FPDF_PAGE, FPDF_TEXTPAGE, FS_RECTF};

use crate::engine::{Document, PageGeometry};
use crate::error::{PdfError, Result};
use crate::ffi_guard::guard;
use crate::geom::{PdfRectF, RotationQuarter};

/// One character with its box in display space.
///
/// SPEC 11.1 asks for selection based on PDFium's character boxes rather than a
/// layout we re-derive ourselves. Carrying the box per character is what makes
/// the selection land where the glyph actually is, including in justified and
/// rotated text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharBox {
    pub unicode: char,
    pub rect: PdfRectF,
}

/// Extracted text for one page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageText {
    pub page: u32,
    pub text: String,
    pub chars: Vec<CharBox>,
}

/// Owns one `FPDF_TEXTPAGE`, closing it on drop.
struct TextPage<'d> {
    raw: FPDF_TEXTPAGE,
    doc: &'d Document,
}

impl Drop for TextPage<'_> {
    fn drop(&mut self) {
        // SAFETY: `raw` came from a non-null `FPDFText_LoadPage` and is closed
        // exactly once, here.
        unsafe { self.doc.engine().bindings().FPDFText_ClosePage(self.raw) }
    }
}

impl<'d> TextPage<'d> {
    fn load(doc: &'d Document, page: FPDF_PAGE) -> Result<Self> {
        // SAFETY: `page` is a live page handle from the document's page cache.
        let raw = unsafe { doc.engine().bindings().FPDFText_LoadPage(page) };
        if raw.is_null() {
            return Err(PdfError::Corrupt {
                detail: "lapisan teks tidak dapat dimuat".into(),
            });
        }
        Ok(Self { raw, doc })
    }

    fn count(&self) -> i32 {
        // SAFETY: `raw` is a live text page handle owned by `self`.
        unsafe { self.doc.engine().bindings().FPDFText_CountChars(self.raw) }
    }

    /// All text on the page as a `String`.
    ///
    /// `FPDFText_GetText` writes UTF-16LE and always appends a NUL terminator,
    /// which we drop before decoding.
    fn all_text(&self, count: i32) -> String {
        if count <= 0 {
            return String::new();
        }
        let bindings = self.doc.engine().bindings();
        let mut buf = vec![0u16; count as usize + 1];
        // SAFETY: `buf` has room for `count` UTF-16 units plus the terminator,
        // which is exactly the contract of `FPDFText_GetText`.
        let written = unsafe {
            bindings.FPDFText_GetText(self.raw, 0, count, buf.as_mut_ptr() as *mut c_ushort)
        };
        let len = (written.max(0) as usize).saturating_sub(1).min(buf.len());
        String::from_utf16_lossy(buf.get(..len).unwrap_or(&[]))
    }

    fn char_at(&self, index: c_int) -> Option<CharBox> {
        let bindings = self.doc.engine().bindings();
        // SAFETY: `index` is in `0..count` from the caller's loop, and `raw` is
        // a live text page handle.
        let code = unsafe { bindings.FPDFText_GetUnicode(self.raw, index) };
        let unicode = char::from_u32(code)?;
        let mut rect = FS_RECTF {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        };
        // SAFETY: as above; `rect` is a live, correctly typed out-parameter.
        let ok = unsafe { bindings.FPDFText_GetLooseCharBox(self.raw, index, &mut rect) };
        if ok == 0 {
            return None;
        }
        Some(CharBox {
            unicode,
            // FS_RECTF from PDFium's text API is in PDF space, so `top` is the
            // larger y. Our PdfRectF keeps the same convention.
            rect: PdfRectF::new(rect.left, rect.bottom, rect.right, rect.top),
        })
    }
}

impl Document {
    /// Plain text of a page, for the FTS5 index (SPEC 7).
    ///
    /// Cheaper than [`Document::page_text_boxed`] because it never asks PDFium
    /// for per-character geometry, which is the expensive half.
    pub fn page_text(&self, page: u32) -> Result<String> {
        self.check_page(page)?;
        guard("page_text", || {
            self.with_page(page, |p| {
                let tp = TextPage::load(self, p)?;
                let n = tp.count();
                Ok(tp.all_text(n))
            })
        })
    }

    /// Text plus per-character boxes, for the selection layer.
    ///
    /// Boxes come back in **display space** at `rotation`, the same space the
    /// tiles are drawn in. PDFium reports them in the page's own user space, so
    /// without the mapping a selection on a rotated page would sit where the
    /// glyph would have been had the page not been turned.
    pub fn page_text_boxed(&self, page: u32, rotation: RotationQuarter) -> Result<PageText> {
        self.check_page(page)?;
        let engine = self.engine();
        guard("page_text_boxed", || {
            self.with_page(page, |p| {
                let geometry = PageGeometry::of_page(engine, p);
                let tp = TextPage::load(self, p)?;
                let n = tp.count();
                let text = tp.all_text(n);
                let mut chars = Vec::with_capacity(n.max(0) as usize);
                for i in 0..n {
                    if let Some(mut cb) = tp.char_at(i) {
                        cb.rect = geometry.to_display(cb.rect, rotation);
                        chars.push(cb);
                    }
                }
                Ok(PageText { page, text, chars })
            })
        })
    }
}
