use std::os::raw::{c_int, c_void};

use pdfium_render::prelude::{FS_MATRIX, FS_RECTF};

use crate::engine::{Document, PageGeometry};
use crate::error::{PdfError, Result};
use crate::ffi_guard::guard;
use crate::geom::{PdfRectF, RotationQuarter};

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
/// `source` is the region of the page to draw, in **display space**: points,
/// origin bottom-left, with the page's own `/Rotate` and the user's extra
/// rotation already applied, so its extents are
/// [`PageGeometry::display_size`]. That is the space the viewport lays out in,
/// which means a rotated page needs no coordinate juggling on the frontend —
/// the flip and the swap happen once, here, inside the matrix.
///
/// `dest_w`/`dest_h` is the pixel box it lands in. Keeping the two independent
/// is what makes a tile a tile: at 400% zoom the viewport asks for a 512x512
/// pixel box covering a small `source` rect, and PDFium never touches the rest
/// of the page.
#[derive(Debug, Clone, Copy)]
pub struct TileRequest {
    pub page: u32,
    pub source: PdfRectF,
    pub dest_w: u32,
    pub dest_h: u32,
    /// Extra rotation the user asked for, on top of the page's own `/Rotate`.
    pub rotation: RotationQuarter,
    pub draw_annotations: bool,
    pub quality: Quality,
    /// Cap PDFium's internal image cache. Set for quarantined documents, where
    /// a bounded footprint matters more than speed.
    pub limit_image_cache: bool,
}

impl TileRequest {
    /// A whole page at a scale factor, the common case for thumbnails and for
    /// pages below the tiling zoom threshold.
    ///
    /// `page_w`/`page_h` are the *display* dimensions at `rotation`, i.e. what
    /// [`PageGeometry::display_size`] reports.
    pub fn whole_page(page: u32, page_w: f32, page_h: f32, scale: f32) -> Self {
        Self {
            page,
            source: PdfRectF::new(0.0, 0.0, page_w, page_h),
            dest_w: ((page_w * scale).round() as u32).max(1),
            dest_h: ((page_h * scale).round() as u32).max(1),
            rotation: RotationQuarter::None,
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

/// Maps `source` (display space, origin bottom-left, y up) onto a `dest_w` by
/// `dest_h` device bitmap (origin top-left, y down), applying `rotation`.
///
/// The matrix PDFium wants is expressed in the page's own user space, so this
/// composes three things at once: the rotation from display space back to user
/// space, the page bounding box's offset (a `/MediaBox` need not start at the
/// origin), and the y flip into device space.
///
/// Writing it as one matrix rather than a chain is deliberate — PDFium takes a
/// single `FS_MATRIX`, and a chain would only move the arithmetic somewhere
/// less testable. Pure and free of PDFium, so every rotation has its own unit
/// test below rather than being verified by eye against a rendered page.
///
/// `bbox` is the page bounding box in *unrotated* user space; `rotation` is the
/// total turn, the page's own `/Rotate` plus whatever the user added.
fn tile_matrix(
    source: &PdfRectF,
    dest_w: u32,
    dest_h: u32,
    rotation: RotationQuarter,
    bbox: &PdfRectF,
) -> FS_MATRIX {
    let sx = dest_w as f32 / source.width();
    let sy = dest_h as f32 / source.height();
    let (l, t) = (source.left, source.top);
    let (bw, bh) = (bbox.width(), bbox.height());
    let (bl, bb) = (bbox.left, bbox.bottom);

    // Each arm is the algebra of "undo the rotation, subtract the bbox origin,
    // scale, flip y" already multiplied out. The unit tests pin all four by
    // checking where the display rect's corners land in the bitmap.
    match rotation {
        RotationQuarter::None => FS_MATRIX {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: -sy,
            e: -(bl + l) * sx,
            f: (t + bb) * sy,
        },
        RotationQuarter::Cw90 => FS_MATRIX {
            a: 0.0,
            b: sy,
            c: sx,
            d: 0.0,
            e: -(bb + l) * sx,
            f: (t - bw - bl) * sy,
        },
        RotationQuarter::Cw180 => FS_MATRIX {
            a: -sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: (bw + bl - l) * sx,
            f: (t - bh - bb) * sy,
        },
        RotationQuarter::Cw270 => FS_MATRIX {
            a: 0.0,
            b: -sy,
            c: -sx,
            d: 0.0,
            e: (bh + bb - l) * sx,
            f: (t + bl) * sy,
        },
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
        let clip = FS_RECTF {
            left: 0.0,
            top: 0.0,
            right: req.dest_w as f32,
            bottom: req.dest_h as f32,
        };
        let flags = req.flags();

        guard("render_tile", || {
            self.with_page(req.page, |page| {
                // The bounding box and the page's own `/Rotate` are read from
                // the loaded page rather than assumed, because a `/MediaBox`
                // that does not start at the origin and a page that is already
                // rotated are both ordinary things to meet in the wild.
                let geometry_of_page = PageGeometry::of_page(self.engine(), page);
                let matrix = tile_matrix(
                    &req.source,
                    req.dest_w,
                    req.dest_h,
                    geometry_of_page.intrinsic.plus(req.rotation),
                    &geometry_of_page.bbox,
                );
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

    /// A4-ish page whose MediaBox starts at the origin.
    const A: PdfRectF = PdfRectF {
        left: 0.0,
        bottom: 0.0,
        right: 612.0,
        top: 792.0,
    };

    fn apply(m: &FS_MATRIX, x: f32, y: f32) -> (f32, f32) {
        (m.a * x + m.c * y + m.e, m.b * x + m.d * y + m.f)
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn whole_page_maps_top_left_of_page_to_origin_of_bitmap() {
        let src = PdfRectF::new(0.0, 0.0, 612.0, 792.0);
        let m = tile_matrix(&src, 612, 792, RotationQuarter::None, &A);
        assert!(close(apply(&m, 0.0, 792.0), (0.0, 0.0)));
    }

    #[test]
    fn whole_page_maps_bottom_right_of_page_to_far_corner() {
        let src = PdfRectF::new(0.0, 0.0, 612.0, 792.0);
        let m = tile_matrix(&src, 612, 792, RotationQuarter::None, &A);
        assert!(close(apply(&m, 612.0, 0.0), (612.0, 792.0)));
    }

    #[test]
    fn tile_maps_its_own_source_rect_onto_the_full_bitmap() {
        // A 512x512 tile covering the page region x 100..200, y 300..400.
        let src = PdfRectF::new(100.0, 300.0, 200.0, 400.0);
        let m = tile_matrix(&src, 512, 512, RotationQuarter::None, &A);
        assert!(close(apply(&m, 100.0, 400.0), (0.0, 0.0)), "top-left");
        assert!(
            close(apply(&m, 200.0, 300.0), (512.0, 512.0)),
            "bottom-right"
        );
    }

    #[test]
    fn non_square_tile_keeps_each_axis_independent() {
        let src = PdfRectF::new(0.0, 0.0, 100.0, 50.0);
        let m = tile_matrix(&src, 400, 100, RotationQuarter::None, &A);
        assert!((m.a - 4.0).abs() < 1e-6, "sx = {}", m.a);
        assert!((m.d + 2.0).abs() < 1e-6, "sy = {}", m.d);
    }

    #[test]
    fn y_axis_is_flipped() {
        let src = PdfRectF::new(0.0, 0.0, 10.0, 10.0);
        let m = tile_matrix(&src, 10, 10, RotationQuarter::None, &A);
        let (_, top) = apply(&m, 0.0, 10.0);
        let (_, bottom) = apply(&m, 0.0, 0.0);
        assert!(
            top < bottom,
            "PDF top must map above PDF bottom in device space"
        );
    }

    /// The corner test that pins each rotation.
    ///
    /// A quarter turn clockwise moves the page's bottom-left corner to the
    /// bitmap's top-left, the next one to the top-right, and so on. Getting a
    /// sign wrong here shows up as a page that is rotated the wrong way or
    /// mirrored, which is exactly the class of bug a rendered-and-eyeballed
    /// check misses at 3 a.m.
    #[test]
    fn each_quarter_turn_lands_the_page_corners_where_the_screen_expects_them() {
        // Bottom-left of the page in user space, and where it must appear in
        // the bitmap for each rotation. Device origin is top-left.
        let cases = [
            (RotationQuarter::None, 612.0, 792.0, (0.0, 792.0)),
            (RotationQuarter::Cw90, 792.0, 612.0, (0.0, 0.0)),
            (RotationQuarter::Cw180, 612.0, 792.0, (612.0, 0.0)),
            (RotationQuarter::Cw270, 792.0, 612.0, (792.0, 612.0)),
        ];
        for (rot, dw, dh, expect) in cases {
            let src = PdfRectF::new(0.0, 0.0, dw, dh);
            let m = tile_matrix(&src, dw as u32, dh as u32, rot, &A);
            let got = apply(&m, 0.0, 0.0);
            assert!(
                close(got, expect),
                "{rot:?}: page bottom-left landed at {got:?}, expected {expect:?}"
            );
        }
    }

    #[test]
    fn a_rotated_page_still_fills_the_bitmap_exactly() {
        // Every rotation must map the whole display rect onto the whole bitmap:
        // no letterboxing, no content pushed outside the tile.
        for (rot, dw, dh) in [
            (RotationQuarter::None, 612.0, 792.0),
            (RotationQuarter::Cw90, 792.0, 612.0),
            (RotationQuarter::Cw180, 612.0, 792.0),
            (RotationQuarter::Cw270, 792.0, 612.0),
        ] {
            let src = PdfRectF::new(0.0, 0.0, dw, dh);
            let m = tile_matrix(&src, dw as u32, dh as u32, rot, &A);
            let corners = [
                apply(&m, 0.0, 0.0),
                apply(&m, 612.0, 0.0),
                apply(&m, 0.0, 792.0),
                apply(&m, 612.0, 792.0),
            ];
            let xs: Vec<f32> = corners.iter().map(|c| c.0).collect();
            let ys: Vec<f32> = corners.iter().map(|c| c.1).collect();
            let (x0, x1) = (
                xs.iter().cloned().fold(f32::MAX, f32::min),
                xs.iter().cloned().fold(f32::MIN, f32::max),
            );
            let (y0, y1) = (
                ys.iter().cloned().fold(f32::MAX, f32::min),
                ys.iter().cloned().fold(f32::MIN, f32::max),
            );
            assert!(x0.abs() < 1e-3 && y0.abs() < 1e-3, "{rot:?}: origin");
            assert!(
                (x1 - dw).abs() < 1e-3 && (y1 - dh).abs() < 1e-3,
                "{rot:?}: extent {x1}x{y1} != {dw}x{dh}"
            );
        }
    }

    #[test]
    fn a_tile_of_a_rotated_page_covers_the_right_strip() {
        // Top-left 512x512 of a 90-degree-rotated page: in display space that
        // is x 0..512, y (612-512)..612 — and in user space it is the strip
        // along the page's left edge, read bottom-to-top.
        let display_h = 612.0;
        let src = PdfRectF::new(0.0, display_h - 512.0, 512.0, display_h);
        let m = tile_matrix(&src, 512, 512, RotationQuarter::Cw90, &A);
        // Page bottom-left is the display top-left, so it lands at (0,0).
        assert!(close(apply(&m, 0.0, 0.0), (0.0, 0.0)));
        // 512 points up the page's left edge is 512 px right in the bitmap.
        assert!(close(apply(&m, 0.0, 512.0), (512.0, 0.0)));
    }

    #[test]
    fn a_media_box_away_from_the_origin_does_not_shift_the_page() {
        // Some documents place the MediaBox at a non-zero origin. The rendered
        // page must still start at the bitmap's corner, not be offset by it.
        let shifted = PdfRectF::new(-50.0, 20.0, 562.0, 812.0); // 612 x 792
        for rot in [
            RotationQuarter::None,
            RotationQuarter::Cw90,
            RotationQuarter::Cw180,
            RotationQuarter::Cw270,
        ] {
            let (dw, dh) = if rot.swaps_axes() {
                (792.0, 612.0)
            } else {
                (612.0, 792.0)
            };
            let src = PdfRectF::new(0.0, 0.0, dw, dh);
            let m = tile_matrix(&src, dw as u32, dh as u32, rot, &shifted);
            // The bbox corner that maps to the display origin depends on the
            // rotation, so assert the weaker but sufficient property: all four
            // bbox corners land exactly on the bitmap's four corners.
            let mut got: Vec<(f32, f32)> = [
                (shifted.left, shifted.bottom),
                (shifted.right, shifted.bottom),
                (shifted.left, shifted.top),
                (shifted.right, shifted.top),
            ]
            .iter()
            .map(|(x, y)| apply(&m, *x, *y))
            .collect();
            got.sort_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).unwrap());
            let want = [(0.0, 0.0), (0.0, dh), (dw, 0.0), (dw, dh)];
            for (g, w) in got.iter().zip(want.iter()) {
                assert!(close(*g, *w), "{rot:?}: {g:?} != {w:?}");
            }
        }
    }
}
