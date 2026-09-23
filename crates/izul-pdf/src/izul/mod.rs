//! This application's own annotations inside a PDF (SPEC 8, Phase 4).
//!
//! An annotation this application saved carries `/IzulObj` — the whole object
//! as JSON (see `izul-write`'s `metadata`). Three things happen to such
//! annotations here, and only here:
//!
//! * **In a document being viewed, they are taken out of the page the first
//!   time the page loads.** The editor draws them from their objects, live and
//!   editable; if PDFium also drew their appearance streams into the tiles,
//!   every one would be on screen twice, and moving one would leave its ghost
//!   behind. Their metadata — and, for pictures, the picture — is kept for the
//!   UI to take (`Document::izul_annots`). Nothing is written back: this is the
//!   in-memory copy, and the file on disk is untouched.
//! * **In a document being saved, they are replaced by placeholders** on the
//!   pages the editor owns — the pages it has read annotations from — and
//!   `izul-write` fills each placeholder in with its real dictionary.
//! * **In a saved file being verified, they are counted.**
//!
//! A picture is read back from the appearance stream PDFium would draw it
//! from, not from a second copy: `FPDFAnnot_GetObject` reaches the image
//! object of any annotation's appearance (measured: not only ink and stamp).
//! A JPEG comes back as the original file, byte for byte
//! (`FPDFImageObj_GetImageDataRaw` on a `/DCTDecode` stream). Anything else is
//! rendered at its own pixel size — the object's matrix is set to its width
//! and height first, which is what makes `FPDFImageObj_GetRenderedBitmap`
//! return one device pixel per image pixel with the soft mask applied, rather
//! than a resampled, opaque copy — and encoded as PNG.

use std::os::raw::{c_int, c_uint, c_ulong, c_void};

use pdfium_render::prelude::*;

use crate::engine::Document;
use crate::error::{PdfError, Result};

/// `/IzulObj`, without the slash. Mirrors `izul_write::metadata::KEY`; the
/// two crates do not depend on each other, and a test pins them equal.
pub const KEY: &str = "IzulObj";
/// Prefix of the `/NM` of every placeholder.
pub const NM_PREFIX: &str = "izul-";

const FPDF_ANNOT_SQUARE: FPDF_ANNOTATION_SUBTYPE = 5;
const FPDF_PAGEOBJ_IMAGE: c_int = 3;
/// `FLAT_NORMALDISPLAY` from `fpdf_flatten.h`.
const FLAT_NORMALDISPLAY: c_int = 0;
/// `FLATTEN_FAIL` from `fpdf_flatten.h`.
const FLATTEN_FAIL: c_int = 0;

/// A picture read back from an annotation's appearance.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedImage {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// One of our annotations, as found on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct IzulAnnot {
    /// The `/IzulObj` value.
    pub metadata: String,
    pub image: Option<ExtractedImage>,
}

fn utf16z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Reads a string-valued key of an annotation.
fn string_value(
    bindings: &dyn PdfiumLibraryBindings,
    annot: FPDF_ANNOTATION,
    key: &str,
) -> Option<String> {
    // SAFETY: `annot` is a live annotation handle. The first call only asks
    // for the length; the second writes at most `len` bytes into a buffer of
    // exactly that size.
    unsafe {
        let len = bindings.FPDFAnnot_GetStringValue(annot, key, std::ptr::null_mut(), 0);
        if len < 2 {
            return None;
        }
        let mut buf = vec![0u16; (len as usize).div_ceil(2)];
        let wrote = bindings.FPDFAnnot_GetStringValue(annot, key, buf.as_mut_ptr(), len);
        if wrote != len {
            return None;
        }
        while buf.last() == Some(&0) {
            buf.pop();
        }
        Some(String::from_utf16_lossy(&buf))
    }
}

fn filter_is_dct(bindings: &dyn PdfiumLibraryBindings, object: FPDF_PAGEOBJECT) -> bool {
    // SAFETY: `object` is a live page object; each call writes at most
    // `buf.len()` bytes into `buf`.
    unsafe {
        let count = bindings.FPDFImageObj_GetImageFilterCount(object);
        (0..count).any(|i| {
            let mut buf = [0u8; 32];
            let n = bindings.FPDFImageObj_GetImageFilter(
                object,
                i,
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as c_ulong,
            ) as usize;
            buf.get(..n.saturating_sub(1)) == Some(b"DCTDecode".as_slice())
        })
    }
}

/// The picture of an annotation's appearance, if it draws one.
fn extract_image(
    doc: &Document,
    page: FPDF_PAGE,
    annot: FPDF_ANNOTATION,
) -> Option<ExtractedImage> {
    let bindings = doc.engine().bindings();
    // SAFETY: `annot` and `page` are live handles for the duration of this
    // call, and every object handle comes from `FPDFAnnot_GetObject` on that
    // annotation. Buffers are sized from the lengths PDFium reports.
    unsafe {
        let count = bindings.FPDFAnnot_GetObjectCount(annot);
        let object = (0..count)
            .map(|i| bindings.FPDFAnnot_GetObject(annot, i))
            .find(|o| !o.is_null() && bindings.FPDFPageObj_GetType(*o) == FPDF_PAGEOBJ_IMAGE)?;

        if filter_is_dct(bindings, object) {
            let len = bindings.FPDFImageObj_GetImageDataRaw(object, std::ptr::null_mut(), 0);
            if len == 0 {
                return None;
            }
            let mut bytes = vec![0u8; len as usize];
            let wrote = bindings.FPDFImageObj_GetImageDataRaw(
                object,
                bytes.as_mut_ptr() as *mut c_void,
                len,
            );
            return (wrote == len).then_some(ExtractedImage {
                mime: "image/jpeg",
                bytes,
            });
        }

        let (mut w, mut h): (c_uint, c_uint) = (0, 0);
        if bindings.FPDFImageObj_GetImagePixelSize(object, &mut w, &mut h) == 0 || w == 0 || h == 0
        {
            return None;
        }
        // One device pixel per image pixel; see the module note.
        let natural = FS_MATRIX {
            a: w as f32,
            b: 0.0,
            c: 0.0,
            d: h as f32,
            e: 0.0,
            f: 0.0,
        };
        bindings.FPDFPageObj_SetMatrix(object, &natural);
        let bitmap = bindings.FPDFImageObj_GetRenderedBitmap(doc.handle(), page, object);
        if bitmap.is_null() {
            return None;
        }
        let bw = bindings.FPDFBitmap_GetWidth(bitmap).max(0) as u32;
        let bh = bindings.FPDFBitmap_GetHeight(bitmap).max(0) as u32;
        let stride = bindings.FPDFBitmap_GetStride(bitmap).max(0) as usize;
        let format = bindings.FPDFBitmap_GetFormat(bitmap);
        let buffer = bindings.FPDFBitmap_GetBuffer(bitmap) as *const u8;
        let mut rgba = Vec::with_capacity((bw * bh * 4) as usize);
        if !buffer.is_null() && format == crate::sys::FPDF_BITMAP_BGRA {
            let raw = std::slice::from_raw_parts(buffer, stride * bh as usize);
            for row in raw.chunks_exact(stride).take(bh as usize) {
                for px in row.chunks_exact(4).take(bw as usize) {
                    if let [b, g, r, a] = *px {
                        rgba.extend_from_slice(&[r, g, b, a]);
                    }
                }
            }
        }
        bindings.FPDFBitmap_Destroy(bitmap);
        if rgba.len() != (bw * bh * 4) as usize {
            return None;
        }
        let image = image::RgbaImage::from_raw(bw, bh, rgba)?;
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).ok()?;
        Some(ExtractedImage {
            mime: "image/png",
            bytes: png.into_inner(),
        })
    }
}

/// Takes this application's annotations out of a freshly loaded page.
pub(crate) fn take(doc: &Document, page: FPDF_PAGE) -> Vec<IzulAnnot> {
    let bindings = doc.engine().bindings();
    let mut found = Vec::new();
    // SAFETY: `page` is a live page handle. Annotations are visited from the
    // last index down, so removing one never shifts an index still to come.
    unsafe {
        let count = bindings.FPDFPage_GetAnnotCount(page);
        for i in (0..count).rev() {
            let annot = bindings.FPDFPage_GetAnnot(page, i);
            if annot.is_null() {
                continue;
            }
            let ours = bindings.FPDFAnnot_HasKey(annot, KEY) != 0;
            if ours {
                if let Some(metadata) = string_value(bindings, annot, KEY) {
                    let image = extract_image(doc, page, annot);
                    found.push(IzulAnnot { metadata, image });
                }
            }
            bindings.FPDFPage_CloseAnnot(annot);
            if ours {
                bindings.FPDFPage_RemoveAnnot(page, i);
            }
        }
    }
    // Back into page order: they were collected last to first.
    found.reverse();
    found
}

impl Document {
    /// Whether loading a page takes this application's annotations out of it.
    /// On for documents being viewed (the default), off for the copies the
    /// worker saves, exports and verifies.
    pub fn set_strip_izul(&self, on: bool) {
        self.strip_izul.set(on);
    }

    /// Our annotations on `page`, as they were when the page first loaded.
    pub fn izul_annots(&self, page: u32) -> Result<Vec<IzulAnnot>> {
        self.with_page(page, |_| Ok(()))?;
        Ok(self.izul.borrow().get(&page).cloned().unwrap_or_default())
    }

    /// How many of our annotations `page` carries. For verification, on a
    /// document opened with stripping off.
    pub fn count_izul(&self, page: u32) -> Result<u32> {
        let bindings = self.engine().bindings();
        self.with_page(page, |p| {
            // SAFETY: `p` is a live page handle and every annotation handle is
            // closed after use.
            unsafe {
                let mut n = 0;
                for i in 0..bindings.FPDFPage_GetAnnotCount(p) {
                    let a = bindings.FPDFPage_GetAnnot(p, i);
                    if a.is_null() {
                        continue;
                    }
                    if bindings.FPDFAnnot_HasKey(a, KEY) != 0 {
                        n += 1;
                    }
                    bindings.FPDFPage_CloseAnnot(a);
                }
                Ok(n)
            }
        })
    }

    /// Replaces our annotations on `page` with one placeholder per id, each
    /// named `/NM (izul-<id>)` for `izul-write` to find.
    pub fn put_placeholders(&self, page: u32, ids: &[u64]) -> Result<()> {
        let bindings = self.engine().bindings();
        self.with_page(page, |p| {
            // SAFETY: `p` is a live page handle; removal walks indices from the
            // top so none is skipped; every created annotation handle is closed.
            unsafe {
                for i in (0..bindings.FPDFPage_GetAnnotCount(p)).rev() {
                    let a = bindings.FPDFPage_GetAnnot(p, i);
                    if a.is_null() {
                        continue;
                    }
                    let ours = bindings.FPDFAnnot_HasKey(a, KEY) != 0;
                    bindings.FPDFPage_CloseAnnot(a);
                    if ours {
                        bindings.FPDFPage_RemoveAnnot(p, i);
                    }
                }
                for id in ids {
                    let a = bindings.FPDFPage_CreateAnnot(p, FPDF_ANNOT_SQUARE);
                    if a.is_null() {
                        return Err(PdfError::Corrupt {
                            detail: format!("anotasi tidak dapat dibuat di halaman {page}"),
                        });
                    }
                    let nm = utf16z(&format!("{NM_PREFIX}{id}"));
                    let ok =
                        bindings.FPDFAnnot_SetStringValue(a, "NM", nm.as_ptr() as FPDF_WIDESTRING);
                    bindings.FPDFPage_CloseAnnot(a);
                    if ok == 0 {
                        return Err(PdfError::Corrupt {
                            detail: "nama anotasi tidak dapat ditulis".into(),
                        });
                    }
                }
                Ok(())
            }
        })
    }

    /// Burns every annotation of `page` into its content (SPEC 11.3's
    /// "ekspor rata"). The page handle is released afterwards, as
    /// `fpdf_flatten.h` requires before the page is used again.
    pub fn flatten_page(&self, page: u32) -> Result<()> {
        let bindings = self.engine().bindings();
        let result = self.with_page(page, |p| {
            // SAFETY: `p` is a live page handle.
            Ok(unsafe { bindings.FPDFPage_Flatten(p, FLAT_NORMALDISPLAY) })
        })?;
        self.release_page(page);
        if result == FLATTEN_FAIL {
            return Err(PdfError::Corrupt {
                detail: format!("halaman {page} tidak dapat diratakan"),
            });
        }
        Ok(())
    }

    /// The whole document, as PDFium writes it (`FPDF_NO_INCREMENTAL`).
    pub fn save_to_vec(&self) -> Result<Vec<u8>> {
        #[repr(C)]
        struct Sink {
            base: FPDF_FILEWRITE,
            out: Vec<u8>,
        }
        unsafe extern "C" fn write_block(
            this: *mut FPDF_FILEWRITE,
            data: *const c_void,
            size: c_ulong,
        ) -> c_int {
            // SAFETY: `this` is the first field of the `Sink` below (`repr(C)`),
            // so the cast recovers the whole struct; PDFium guarantees `data`
            // holds `size` readable bytes for the duration of the call.
            unsafe {
                let sink = this as *mut Sink;
                (*sink).out.extend_from_slice(std::slice::from_raw_parts(
                    data as *const u8,
                    size as usize,
                ));
            }
            1
        }
        // Pages hold parsed state that the writer does not need, and closing
        // them first is what `FPDF_SaveAsCopy`'s own documentation asks.
        self.release_all_pages();
        let mut sink = Sink {
            base: FPDF_FILEWRITE {
                version: 1,
                WriteBlock: Some(write_block),
            },
            out: Vec::new(),
        };
        const FPDF_NO_INCREMENTAL: FPDF_DWORD = 2;
        let bindings = self.engine().bindings();
        // SAFETY: `sink` outlives the call and its first field is the
        // `FPDF_FILEWRITE` PDFium writes through.
        let ok =
            unsafe { bindings.FPDF_SaveAsCopy(self.handle(), &mut sink.base, FPDF_NO_INCREMENTAL) };
        if ok == 0 {
            return Err(PdfError::Corrupt {
                detail: "PDFium gagal menulis dokumen".into(),
            });
        }
        Ok(sink.out)
    }

    /// A new document holding `pages` of this one, in that order.
    pub fn extract_pages(&self, pages: &[u32]) -> Result<Vec<u8>> {
        let bindings = self.engine().bindings();
        for p in pages {
            self.check_page(*p)?;
        }
        // SAFETY: both documents are live for the duration of the import; the
        // index buffer outlives the call; the new document is closed exactly
        // once, by the `Document` it is wrapped in below.
        let fresh = unsafe { bindings.FPDF_CreateNewDocument() };
        if fresh.is_null() {
            return Err(PdfError::Corrupt {
                detail: "dokumen baru tidak dapat dibuat".into(),
            });
        }
        let out = crate::engine::Document::adopt(self.engine(), fresh);
        let indices: Vec<c_int> = pages.iter().map(|p| *p as c_int).collect();
        // SAFETY: as above.
        let ok = unsafe {
            bindings.FPDF_ImportPagesByIndex(
                out.handle(),
                self.handle(),
                indices.as_ptr(),
                indices.len() as c_ulong,
                0,
            )
        };
        if ok == 0 {
            return Err(PdfError::Corrupt {
                detail: "halaman tidak dapat disalin".into(),
            });
        }
        out.save_to_vec()
    }
}

#[cfg(test)]
mod tests;
