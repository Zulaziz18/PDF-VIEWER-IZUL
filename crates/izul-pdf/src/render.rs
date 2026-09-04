use std::os::raw::{c_int, c_void};

use pdfium_render::prelude::{FS_MATRIX, FS_RECTF};

use crate::engine::Document;
use crate::error::{PdfError, Result};
use crate::ffi_guard::guard;
use crate::geom::PdfRectF;

/// Bytes per pixel in every buffer we hand to PDFium: BGRA, 8 bits each.
pub const BYTES_PER_PIXEL: usize = 4;

/// Largest tile we will allocate, as a guard against a corrupt request asking
/// for a gigapixel bitmap. 8192x8192 BGRA is already 256 MB, far past anything
/// the viewport needs.
pub const MAX_TILE_DIM: u32 = 8192;

/// Pixel format PDFium writes.
///
/// Always BGRA: it is PDFium's native output order, so asking for anything else
/// costs a full-buffer swizzle. The frontend reinterprets it when constructing
/// the `ImageBitmap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bgra8,
}

/// Quality tier for a render. SPEC 9 wants a low-resolution answer instantly
/// during scroll, replaced by the sharp one when it is ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    /// No image smoothing, no LCD text. For the first tier during scroll.
    Fast,
    /// Everything on. For the settled view and for export.
    #[default]
    Sharp,
}

/// What to draw and where.
///
/// `source` is the region of the page to draw, in PDF points. `dest_w`/`dest_h`
/// is the pixel box it lands in. Keeping the two independent is what makes a
/// tile a tile: at 400% zoom the viewport asks for a 512x512 pixel box covering
/// a small `source` rect, and PDFium never touches the rest of the page.
#[derive(Debug, Clone, Copy)]
pub struct TileRequest {
    pub page: u32,
    pub source: PdfRectF,
    pub dest_w: u32,
    pub dest_h: u32,
    pub draw_annotations: bool,
    pub quality: Quality,
    /// Cap PDFium's internal image cache. Set for quarantined documents, where
    /// a bounded footprint matters more than speed.
    pub limit_image_cache: bool,
}

impl TileRequest {
    /// A whole page at a scale factor, the common case for thumbnails and for
    /// pages below the tiling zoom threshold.
    pub fn whole_page(page: u32, page_w: f32, page_h: f32, scale: f32) -> Self {
        Self {
            page,
            source: PdfRectF::new(0.0, 0.0, page_w, page_h),
            dest_w: ((page_w * scale).round() as u32).max(1),
            dest_h: ((page_h * scale).round() as u32).max(1),
            draw_annotations: true,
            quality: Quality::Sharp,
            limit_image_cache: false,
        }
    }

    fn flags(&self) -> c_int {
        let mut flags = 0;
        if self.draw_annotations {
            flags |= crate::sys::FPDF_ANNOT;
        }
        match self.quality {
            Quality::Sharp => flags |= crate::sys::FPDF_LCD_TEXT,
            Quality::Fast => flags |= crate::sys::FPDF_RENDER_NO_SMOOTHIMAGE,
        }
        if self.limit_image_cache {
            flags |= crate::sys::FPDF_RENDER_LIMITEDIMAGECACHE;
        }
        flags
    }
}

/// Description of a completed render: enough for the frontend to wrap the bytes
/// in an `ImageBitmap` without copying them.
#[derive(Debug, Clone, Copy)]
pub struct TileGeometry {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
}

impl TileGeometry {
    pub fn required_bytes(&self) -> usize {
        self.stride.saturating_mul(self.height as usize)
    }
}

/// Maps `source` (PDF user space, origin bottom-left, y up) onto a `dest_w` by
/// `dest_h` device bitmap (origin top-left, y down).
///
/// Pure and free of PDFium, so the flip has its own unit tests below rather than
/// being verified only by eye against a rendered page.
fn tile_matrix(source: &PdfRectF, dest_w: u32, dest_h: u32) -> FS_MATRIX {
    let sx = dest_w as f32 / source.width();
    let sy = dest_h as f32 / source.height();
    FS_MATRIX {
        a: sx,
        b: 0.0,
        c: 0.0,
        d: -sy,
        e: -source.left * sx,
        f: source.top * sy,
    }
}

impl Document {
    /// Renders a tile straight into `dest`.
    ///
    /// `dest` is normally a slice of the shared-memory ring the UI process reads
    /// from, so PDFium writes the final pixels exactly once: no intermediate
    /// bitmap, no copy on the way out (SPEC 6).
    pub fn render_tile_into(&self, req: &TileRequest, dest: &mut [u8]) -> Result<TileGeometry> {
        self.check_page(req.page)?;
        if req.dest_w == 0
            || req.dest_h == 0
            || req.dest_w > MAX_TILE_DIM
            || req.dest_h > MAX_TILE_DIM
        {
            return Err(PdfError::BadTileSize {
                w: req.dest_w,
                h: req.dest_h,
            });
        }
        if !req.source.is_valid() {
            return Err(PdfError::BadSourceRect { rect: req.source });
        }

        let stride = (req.dest_w as usize) * BYTES_PER_PIXEL;
        let geometry = TileGeometry {
            width: req.dest_w,
            height: req.dest_h,
            stride,
            format: PixelFormat::Bgra8,
        };
        let needed = geometry.required_bytes();
        if dest.len() < needed {
            return Err(PdfError::BufferTooSmall {
                needed,
                available: dest.len(),
            });
        }

        let bindings = self.engine().bindings();
        let dest_ptr = dest.as_mut_ptr();
        let matrix = tile_matrix(&req.source, req.dest_w, req.dest_h);
        let clip = FS_RECTF {
            left: 0.0,
            top: 0.0,
            right: req.dest_w as f32,
            bottom: req.dest_h as f32,
        };
        let flags = req.flags();

        guard("render_tile", || {
            self.with_page(req.page, |page| {
                // SAFETY: `dest_ptr` points into `dest`, which is exclusively
                // borrowed for this whole call and was verified above to hold at
                // least `stride * dest_h` bytes; the same width, height and
                // stride are passed to `FPDFBitmap_CreateEx`, so PDFium writes
                // strictly inside it. `page` is a live handle from the document's
                // page cache. The bitmap wraps our buffer without owning it and
                // is destroyed on every path out of this block.
                unsafe {
                    let bitmap = bindings.FPDFBitmap_CreateEx(
                        req.dest_w as c_int,
                        req.dest_h as c_int,
                        crate::sys::FPDF_BITMAP_BGRA,
                        dest_ptr as *mut c_void,
                        stride as c_int,
                    );
                    if bitmap.is_null() {
                        return Err(PdfError::BadTileSize {
                            w: req.dest_w,
                            h: req.dest_h,
                        });
                    }
                    // A PDF page is white by definition. Without this the tile
                    // shows whatever the shared-memory ring last held.
                    bindings.FPDFBitmap_FillRect(
                        bitmap,
                        0,
                        0,
                        req.dest_w as c_int,
                        req.dest_h as c_int,
                        0xFFFF_FFFF,
                    );
                    bindings.FPDF_RenderPageBitmapWithMatrix(bitmap, page, &matrix, &clip, flags);
                    bindings.FPDFBitmap_Destroy(bitmap);
                }
                Ok(geometry)
            })
        })
    }

    /// Renders a whole page at a scale factor into a freshly allocated buffer.
    ///
    /// Used by thumbnails and by the benchmark harness. The hot viewport path
    /// uses [`Document::render_tile_into`] against shared memory instead.
    pub fn render_page(
        &self,
        page: u32,
        scale: f32,
        quality: Quality,
    ) -> Result<(TileGeometry, Vec<u8>)> {
        let size = self.page_size(page)?;
        let mut req = TileRequest::whole_page(page, size.width, size.height, scale);
        req.quality = quality;
        let mut buf = vec![0u8; req.dest_w as usize * req.dest_h as usize * BYTES_PER_PIXEL];
        let geom = self.render_tile_into(&req, &mut buf)?;
        Ok((geom, buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &FS_MATRIX, x: f32, y: f32) -> (f32, f32) {
        (m.a * x + m.c * y + m.e, m.b * x + m.d * y + m.f)
    }

    #[test]
    fn whole_page_maps_top_left_of_page_to_origin_of_bitmap() {
        let src = PdfRectF::new(0.0, 0.0, 612.0, 792.0);
        let m = tile_matrix(&src, 612, 792);
        let (x, y) = apply(&m, 0.0, 792.0); // top-left corner in PDF space
        assert!(x.abs() < 1e-3, "x = {x}");
        assert!(y.abs() < 1e-3, "y = {y}");
    }

    #[test]
    fn whole_page_maps_bottom_right_of_page_to_far_corner() {
        let src = PdfRectF::new(0.0, 0.0, 612.0, 792.0);
        let m = tile_matrix(&src, 612, 792);
        let (x, y) = apply(&m, 612.0, 0.0);
        assert!((x - 612.0).abs() < 1e-3, "x = {x}");
        assert!((y - 792.0).abs() < 1e-3, "y = {y}");
    }

    #[test]
    fn tile_maps_its_own_source_rect_onto_the_full_bitmap() {
        // A 512x512 tile covering the page region x 100..200, y 300..400.
        let src = PdfRectF::new(100.0, 300.0, 200.0, 400.0);
        let m = tile_matrix(&src, 512, 512);
        let (x0, y0) = apply(&m, 100.0, 400.0); // tile's top-left
        let (x1, y1) = apply(&m, 200.0, 300.0); // tile's bottom-right
        assert!(x0.abs() < 1e-3 && y0.abs() < 1e-3, "top-left = {x0},{y0}");
        assert!(
            (x1 - 512.0).abs() < 1e-3 && (y1 - 512.0).abs() < 1e-3,
            "br = {x1},{y1}"
        );
    }

    #[test]
    fn non_square_tile_keeps_each_axis_independent() {
        let src = PdfRectF::new(0.0, 0.0, 100.0, 50.0);
        let m = tile_matrix(&src, 400, 100);
        assert!((m.a - 4.0).abs() < 1e-6, "sx = {}", m.a);
        assert!((m.d + 2.0).abs() < 1e-6, "sy = {}", m.d);
    }

    #[test]
    fn y_axis_is_flipped() {
        let src = PdfRectF::new(0.0, 0.0, 10.0, 10.0);
        let m = tile_matrix(&src, 10, 10);
        let (_, top) = apply(&m, 0.0, 10.0);
        let (_, bottom) = apply(&m, 0.0, 0.0);
        assert!(
            top < bottom,
            "PDF top must map above PDF bottom in device space"
        );
    }
}
