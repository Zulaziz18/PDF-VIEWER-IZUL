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
    /// Dark mode's smart inversion (Phase 8): every pixel's lightness
    /// flipped, its hue kept, except where a picture is drawn (`invert.rs`).
    pub invert: bool,
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
            invert: false,
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
/// `dest_h` device bitmap.
///
/// ## What space PDFium wants this matrix in
///
/// `FPDF_RenderPageBitmapWithMatrix` does **not** take a user-space matrix.
/// PDFium composes the page's own display matrix first and applies this one on
/// top of it. That was established by experiment, not by reading: rendering
/// with an identity matrix produces a bitmap byte-for-byte identical to
/// `FPDF_RenderPageBitmap` (see `renders_identically_to_the_plain_api` below,
/// which fails the moment that stops being true).
///
/// So the space this matrix reads from — call it *page space* — is:
///
/// * measured in points, `page_w` by `page_h`;
/// * origin at the **top-left**, y growing **downwards**;
/// * the page's `/MediaBox` offset already subtracted;
/// * the page's own `/Rotate` already applied, so `page_w`/`page_h` are the
///   rotated dimensions.
///
/// Everything this function still has to do is therefore the *difference*
/// between page space and the tile: the user's extra rotation, the source
/// rect's offset, and the scale. Undoing the bounding box or the intrinsic
/// rotation here would apply them twice — which is exactly the bug this
/// replaced, and why anything above 100% zoom rendered blank.
///
/// `extra` is only the rotation the *user* asked for, never the page's own.
///
/// Pure and free of PDFium, so every rotation has its own unit test below
/// rather than being verified by eye against a rendered page.
fn tile_matrix(
    source: &PdfRectF,
    dest_w: u32,
    dest_h: u32,
    extra: RotationQuarter,
    page_w: f32,
    page_h: f32,
) -> FS_MATRIX {
    let sx = dest_w as f32 / source.width();
    let sy = dest_h as f32 / source.height();
    let (l, t) = (source.left, source.top);
    let (pw, ph) = (page_w, page_h);

    // Each arm is "page space -> display space -> tile pixels" multiplied out.
    // The display step turns page space (top-left origin, y down) into the
    // bottom-left, y-up space the viewport lays out in, turning it by `extra`;
    // the tile step subtracts `source`'s top-left corner and scales.
    match extra {
        RotationQuarter::None => FS_MATRIX {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: -l * sx,
            f: (t - ph) * sy,
        },
        RotationQuarter::Cw90 => FS_MATRIX {
            a: 0.0,
            b: sy,
            c: -sx,
            d: 0.0,
            e: (ph - l) * sx,
            f: (t - pw) * sy,
        },
        RotationQuarter::Cw180 => FS_MATRIX {
            a: -sx,
            b: 0.0,
            c: 0.0,
            d: -sy,
            e: (pw - l) * sx,
            f: t * sy,
        },
        RotationQuarter::Cw270 => FS_MATRIX {
            a: 0.0,
            b: -sy,
            c: sx,
            d: 0.0,
            e: -l * sx,
            f: t * sy,
        },
    }
}

/// Where `FPDF_FFLDraw` puts the whole page, in the tile's pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FflPlacement {
    start_x: c_int,
    start_y: c_int,
    size_x: c_int,
    size_y: c_int,
}

/// The same mapping as [`tile_matrix`], in the only terms `FPDF_FFLDraw`
/// takes: the whole page, turned by `extra`, scaled, and shifted so that
/// `source`'s top-left corner lands on the tile's.
///
/// Form widgets are drawn by `FPDF_FFLDraw` alone — the page render leaves
/// them out, with or without the form environment (measured) — and it has no
/// matrix variant, so this is the one place the tile's geometry is said
/// twice. The pixel test `widgets_land_where_flattening_puts_them` holds the
/// two together, on every rotation and on tiles off the page's corner.
fn ffl_placement(
    source: &PdfRectF,
    dest_w: u32,
    dest_h: u32,
    extra: RotationQuarter,
    page_w: f32,
    page_h: f32,
) -> FflPlacement {
    let sx = dest_w as f32 / source.width();
    let sy = dest_h as f32 / source.height();
    let (dw, dh) = match extra {
        RotationQuarter::None | RotationQuarter::Cw180 => (page_w, page_h),
        RotationQuarter::Cw90 | RotationQuarter::Cw270 => (page_h, page_w),
    };
    FflPlacement {
        start_x: (-source.left * sx).round() as c_int,
        start_y: (-(dh - source.top) * sy).round() as c_int,
        size_x: (dw * sx).round() as c_int,
        size_y: (dh * sy).round() as c_int,
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
                // Page space is what PDFium hands the matrix, so the size
                // needed here is the page's own `/Rotate` applied to its
                // bounding box and nothing more — the extra rotation the user
                // asked for is the matrix's job, not the size's. Read from the
                // loaded page rather than assumed, because a `/MediaBox` that
                // does not start at the origin and a page that is already
                // rotated are both ordinary things to meet in the wild.
                let geometry_of_page = PageGeometry::of_page(self.engine(), page);
                let page_space = geometry_of_page.display_size(RotationQuarter::None);
                let matrix = tile_matrix(
                    &req.source,
                    req.dest_w,
                    req.dest_h,
                    req.rotation,
                    page_space.width,
                    page_space.height,
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
                    if req.draw_annotations {
                        let form = self.form_handle();
                        if !form.is_null() {
                            let d = ffl_placement(
                                &req.source,
                                req.dest_w,
                                req.dest_h,
                                req.rotation,
                                page_space.width,
                                page_space.height,
                            );
                            bindings.FPDF_FFLDraw(
                                form,
                                bitmap,
                                page,
                                d.start_x,
                                d.start_y,
                                d.size_x,
                                d.size_y,
                                req.rotation as c_int,
                                flags & !crate::sys::FPDF_ANNOT,
                            );
                        }
                    }
                    bindings.FPDFBitmap_Destroy(bitmap);
                }
                if req.invert {
                    let pictures = crate::invert::image_rects_on(
                        bindings,
                        page,
                        &geometry_of_page,
                        req.rotation,
                    );
                    let keep = crate::invert::to_tile_pixels(
                        &pictures,
                        &req.source,
                        req.dest_w,
                        req.dest_h,
                    );
                    // SAFETY: `dest` is exclusively borrowed for this call and
                    // holds `stride * dest_h` bytes (checked above); PDFium is
                    // done with it (the bitmap was destroyed).
                    let buf = unsafe { std::slice::from_raw_parts_mut(dest_ptr, needed) };
                    crate::invert::smart_invert(buf, stride, req.dest_w, req.dest_h, &keep);
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

    /// A4-ish page, in page space: `W` by `H` points, origin top-left.
    const W: f32 = 612.0;
    const H: f32 = 792.0;

    fn apply(m: &FS_MATRIX, x: f32, y: f32) -> (f32, f32) {
        (m.a * x + m.c * y + m.e, m.b * x + m.d * y + m.f)
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn an_unzoomed_whole_page_tile_is_the_identity() {
        // The strongest statement of what page space is: asking for the whole
        // page at its own size must leave PDFium's own display matrix alone.
        // `renders_identically_to_the_plain_api` is the same claim, measured in
        // pixels against `FPDF_RenderPageBitmap`.
        let src = PdfRectF::new(0.0, 0.0, W, H);
        let m = tile_matrix(&src, W as u32, H as u32, RotationQuarter::None, W, H);
        assert!(close(apply(&m, 0.0, 0.0), (0.0, 0.0)), "top-left");
        assert!(close(apply(&m, W, H), (W, H)), "bottom-right");
    }

    #[test]
    fn tile_maps_its_own_source_rect_onto_the_full_bitmap() {
        // A 512x512 tile covering the display region x 100..200, y 300..400.
        // Display y 400 is page-space y 792-400.
        let src = PdfRectF::new(100.0, 300.0, 200.0, 400.0);
        let m = tile_matrix(&src, 512, 512, RotationQuarter::None, W, H);
        assert!(close(apply(&m, 100.0, H - 400.0), (0.0, 0.0)), "top-left");
        assert!(
            close(apply(&m, 200.0, H - 300.0), (512.0, 512.0)),
            "bottom-right"
        );
    }

    #[test]
    fn non_square_tile_keeps_each_axis_independent() {
        let src = PdfRectF::new(0.0, 0.0, 100.0, 50.0);
        let m = tile_matrix(&src, 400, 100, RotationQuarter::None, W, H);
        assert!((m.a - 4.0).abs() < 1e-6, "sx = {}", m.a);
        assert!((m.d - 2.0).abs() < 1e-6, "sy = {}", m.d);
    }

    #[test]
    fn page_space_y_is_not_flipped_again() {
        // Page space already points y downwards, because PDFium's own display
        // matrix flipped it. Flipping a second time was the bug that rendered
        // every zoomed tile blank.
        let src = PdfRectF::new(0.0, 0.0, 10.0, 10.0);
        let m = tile_matrix(&src, 10, 10, RotationQuarter::None, W, H);
        let (_, upper) = apply(&m, 0.0, H - 10.0);
        let (_, lower) = apply(&m, 0.0, H);
        assert!(
            upper < lower,
            "higher up the page must map higher up the bitmap: {upper} !< {lower}"
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
            let m = tile_matrix(&src, dw as u32, dh as u32, rot, W, H);
            // The page's bottom-left corner is page space (0, H).
            let got = apply(&m, 0.0, H);
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
            let m = tile_matrix(&src, dw as u32, dh as u32, rot, W, H);
            let corners = [
                apply(&m, 0.0, 0.0),
                apply(&m, W, 0.0),
                apply(&m, 0.0, H),
                apply(&m, W, H),
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
        let m = tile_matrix(&src, 512, 512, RotationQuarter::Cw90, W, H);
        // Page bottom-left — page space (0, H) — is the display top-left, so it
        // lands at (0,0).
        assert!(close(apply(&m, 0.0, H), (0.0, 0.0)));
        // 512 points up the page's left edge is 512 px right in the bitmap.
        assert!(close(apply(&m, 0.0, H - 512.0), (512.0, 0.0)));
    }
}

#[cfg(test)]
mod rendered {
    //! Tests that put real pixels on the board.
    //!
    //! Every claim `tile_matrix` makes about what space PDFium reads its matrix
    //! in is unprovable from the header, which says only "the transform matrix,
    //! which must be invertible". Two of this project's worst bugs came from
    //! guessing at it, so the claims are measured here instead: against
    //! `FPDF_RenderPageBitmap` for the space itself, and against synthesised
    //! pages that put ink in one known corner for the rotation and bounding-box
    //! rules.
    use super::*;
    use crate::{engine_and_lock, viewer_fixture};

    /// The engine, plus the lock that makes using it safe on a test thread pool.
    ///
    /// The guard comes back to the caller to bind — `let (engine, _pdfium) =
    /// engine_or_skip!()` — because PDFium's global state tolerates exactly one
    /// user at a time and `cargo test` provides several. See
    /// [`crate::test_support`].
    macro_rules! engine_or_skip {
        () => {{
            engine_and_lock!()
        }};
    }

    /// The small document with an outline, mixed page sizes and a page that
    /// carries its own `/Rotate`; `bench/make_fixtures.py viewer` builds it.
    macro_rules! fixture_or_skip {
        () => {{
            viewer_fixture!()
        }};
    }

    /// Fraction of pixels that are not white, over a rectangle of the bitmap.
    fn ink_in(buf: &[u8], w: u32, h: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> f64 {
        let mut dark = 0usize;
        let mut total = 0usize;
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                let i = (y as usize * w as usize + x as usize) * BYTES_PER_PIXEL;
                total += 1;
                if buf[i] < 200 {
                    dark += 1;
                }
            }
        }
        if total == 0 {
            0.0
        } else {
            dark as f64 / total as f64
        }
    }

    fn ink(buf: &[u8], w: u32, h: u32) -> f64 {
        ink_in(buf, w, h, 0, 0, w, h)
    }

    /// A one-page PDF with a black rectangle in the bottom-left corner of user
    /// space, 100 by 200 points inside a 200 by 400 page.
    ///
    /// The MediaBox origin and the `/Rotate` are parameters because those are
    /// precisely the two things `tile_matrix` must *not* handle itself, PDFium
    /// having handled them already. A page whose box starts at the origin and a
    /// page rotated zero degrees would prove neither.
    fn corner_page(mx: f32, my: f32, rotate: i32) -> Vec<u8> {
        let content = format!("0 0 0 rg\n{mx} {my} 100 200 re f\n");
        let objects = [
            "<</Type/Catalog/Pages 2 0 R>>".to_string(),
            "<</Type/Pages/Kids[3 0 R]/Count 1>>".to_string(),
            format!(
                "<</Type/Page/Parent 2 0 R/MediaBox[{mx} {my} {} {}]/Rotate {rotate}/Contents 4 0 R>>",
                mx + 200.0,
                my + 400.0
            ),
            format!("<</Length {}>>\nstream\n{content}endstream", content.len()),
        ];

        let mut pdf = String::from("%PDF-1.7\n");
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
        }
        let xref = pdf.len();
        pdf.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objects.len() + 1
        ));
        for off in &offsets {
            pdf.push_str(&format!("{off:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        ));
        pdf.into_bytes()
    }

    /// Which quarter of the bitmap the ink landed in, as (left?, top?).
    fn ink_quarter(buf: &[u8], w: u32, h: u32) -> (bool, bool) {
        let (hw, hh) = (w / 2, h / 2);
        let quads = [
            (ink_in(buf, w, h, 0, 0, hw, hh), (true, true)),
            (ink_in(buf, w, h, hw, 0, w, hh), (false, true)),
            (ink_in(buf, w, h, 0, hh, hw, h), (true, false)),
            (ink_in(buf, w, h, hw, hh, w, h), (false, false)),
        ];
        quads
            .iter()
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|q| q.1)
            .unwrap_or((true, true))
    }

    /// The anchor for everything `tile_matrix` assumes.
    ///
    /// PDFium composes the page's own display matrix before applying ours, so a
    /// whole-page tile at the page's own size must come out byte-for-byte
    /// identical to the plain API. If this ever fails, `tile_matrix` is being
    /// asked for a different space and every arm of it needs rederiving.
    #[test]
    fn renders_identically_to_the_plain_api() {
        let (engine, _pdfium) = engine_or_skip!();
        let fixture = fixture_or_skip!();
        let doc = engine.open(&fixture, None).expect("buka");
        let size = doc.page_size(0).expect("ukuran");
        let (w, h) = (size.width.round() as u32, size.height.round() as u32);

        let stride = w as usize * BYTES_PER_PIXEL;
        let mut plain = vec![0u8; stride * h as usize];
        let bindings = engine.bindings();
        doc.with_page(0, |page| {
            // SAFETY: `page` is live for the closure, and the bitmap wraps
            // `plain`, which holds exactly `stride * h` bytes and is borrowed
            // mutably for the whole call.
            unsafe {
                let bm = bindings.FPDFBitmap_CreateEx(
                    w as c_int,
                    h as c_int,
                    crate::sys::FPDF_BITMAP_BGRA,
                    plain.as_mut_ptr() as *mut c_void,
                    stride as c_int,
                );
                assert!(!bm.is_null());
                bindings.FPDFBitmap_FillRect(bm, 0, 0, w as c_int, h as c_int, 0xFFFF_FFFF);
                bindings.FPDF_RenderPageBitmap(bm, page, 0, 0, w as c_int, h as c_int, 0, 0);
                bindings.FPDFBitmap_Destroy(bm);
            }
            Ok(())
        })
        .expect("render polos");

        let mut ours = vec![0u8; stride * h as usize];
        let req = TileRequest {
            page: 0,
            source: PdfRectF::new(0.0, 0.0, size.width, size.height),
            dest_w: w,
            dest_h: h,
            rotation: RotationQuarter::None,
            draw_annotations: false,
            quality: Quality::Sharp,
            limit_image_cache: false,
            invert: false,
        };
        doc.render_tile_into(&req, &mut ours).expect("render ubin");

        assert!(ink(&plain, w, h) > 0.01, "halaman uji harus punya tinta");
        assert!(
            plain == ours,
            "ubin seluruh halaman harus identik dengan FPDF_RenderPageBitmap"
        );
    }

    /// The bug that made every page white above 100% zoom.
    ///
    /// A tile is defined by a source rect *smaller* than its pixel box, so the
    /// matrix scales up — and the old matrix, expressed in the wrong space,
    /// pushed all the content outside the clip as soon as that scale left 1.0.
    /// At 100% it happened to still land something on the bitmap, which is why
    /// nothing caught it until a zoomed tile was rendered.
    #[test]
    fn zooming_past_one_to_one_still_puts_ink_on_the_tile() {
        let (engine, _pdfium) = engine_or_skip!();
        let fixture = fixture_or_skip!();
        let doc = engine.open(&fixture, None).expect("buka");
        let size = doc.page_size(0).expect("ukuran");

        // Top-left 512x512 pixels of the page at each zoom: the region a
        // reader sees first, and where a text document keeps its ink.
        for ppp in [1.0f32, 1.5, 2.0, 3.0, 4.0] {
            let span = 512.0 / ppp;
            let source = PdfRectF::new(0.0, size.height - span, span.min(size.width), size.height);
            let req = TileRequest {
                page: 0,
                source,
                dest_w: (source.width() * ppp).round() as u32,
                dest_h: 512,
                rotation: RotationQuarter::None,
                draw_annotations: false,
                quality: Quality::Sharp,
                limit_image_cache: false,
                invert: false,
            };
            let mut buf = vec![0u8; req.dest_w as usize * req.dest_h as usize * BYTES_PER_PIXEL];
            doc.render_tile_into(&req, &mut buf).expect("render");
            let got = ink(&buf, req.dest_w, req.dest_h);
            assert!(
                got > 0.001,
                "zoom {ppp}x: ubin kosong (tinta {got:.4}), source {source:?}"
            );
        }
    }

    /// PDFium's display matrix already subtracts the `/MediaBox` origin, so
    /// `tile_matrix` must not subtract it again.
    #[test]
    fn a_media_box_away_from_the_origin_does_not_shift_the_page() {
        let (engine, _pdfium) = engine_or_skip!();
        for (mx, my) in [(0.0f32, 0.0f32), (-50.0, 20.0), (120.0, -300.0)] {
            let doc = engine
                .open_bytes(corner_page(mx, my, 0), None, None)
                .expect("buka sintetis");
            let (w, h) = (200u32, 400u32);
            let req = TileRequest {
                page: 0,
                source: PdfRectF::new(0.0, 0.0, 200.0, 400.0),
                dest_w: w,
                dest_h: h,
                rotation: RotationQuarter::None,
                draw_annotations: false,
                quality: Quality::Sharp,
                limit_image_cache: false,
                invert: false,
            };
            let mut buf = vec![0u8; w as usize * h as usize * BYTES_PER_PIXEL];
            doc.render_tile_into(&req, &mut buf).expect("render");
            assert!(ink(&buf, w, h) > 0.1, "MediaBox {mx},{my}: tidak ada tinta");
            assert_eq!(
                ink_quarter(&buf, w, h),
                (true, false),
                "MediaBox {mx},{my}: persegi kiri-bawah harus tetap di kiri-bawah"
            );
        }
    }

    /// PDFium's display matrix already applies the page's own `/Rotate`, so
    /// `tile_matrix` must be given only the rotation the user added.
    #[test]
    fn the_pages_own_rotate_is_applied_exactly_once() {
        let (engine, _pdfium) = engine_or_skip!();
        // The ink square sits at the page's bottom-left in user space. Turning
        // the page clockwise walks that corner round the bitmap.
        for (rotate, want) in [
            (0, (true, false)),
            (90, (true, true)),
            (180, (false, true)),
            (270, (false, false)),
        ] {
            let doc = engine
                .open_bytes(corner_page(0.0, 0.0, rotate), None, None)
                .expect("buka sintetis");
            let size = doc.page_size(0).expect("ukuran");
            let (w, h) = (size.width.round() as u32, size.height.round() as u32);
            let swaps = rotate % 180 != 0;
            assert_eq!(
                (w, h),
                if swaps { (400, 200) } else { (200, 400) },
                "/Rotate {rotate}: ukuran halaman"
            );
            let req = TileRequest {
                page: 0,
                source: PdfRectF::new(0.0, 0.0, size.width, size.height),
                dest_w: w,
                dest_h: h,
                rotation: RotationQuarter::None,
                draw_annotations: false,
                quality: Quality::Sharp,
                limit_image_cache: false,
                invert: false,
            };
            let mut buf = vec![0u8; w as usize * h as usize * BYTES_PER_PIXEL];
            doc.render_tile_into(&req, &mut buf).expect("render");
            assert!(ink(&buf, w, h) > 0.1, "/Rotate {rotate}: tidak ada tinta");
            assert_eq!(
                ink_quarter(&buf, w, h),
                want,
                "/Rotate {rotate}: sudut bertinta salah"
            );
        }
    }

    /// The rotation the *user* asks for stacks on top of the page's own.
    #[test]
    fn the_users_extra_rotation_turns_the_page_further() {
        let (engine, _pdfium) = engine_or_skip!();
        for (extra, want) in [
            (RotationQuarter::None, (true, false)),
            (RotationQuarter::Cw90, (true, true)),
            (RotationQuarter::Cw180, (false, true)),
            (RotationQuarter::Cw270, (false, false)),
        ] {
            let doc = engine
                .open_bytes(corner_page(0.0, 0.0, 0), None, None)
                .expect("buka sintetis");
            let geometry = doc.page_geometry(0).expect("geometri");
            let display = geometry.display_size(extra);
            let (w, h) = (display.width.round() as u32, display.height.round() as u32);
            let req = TileRequest {
                page: 0,
                source: PdfRectF::new(0.0, 0.0, display.width, display.height),
                dest_w: w,
                dest_h: h,
                rotation: extra,
                draw_annotations: false,
                quality: Quality::Sharp,
                limit_image_cache: false,
                invert: false,
            };
            let mut buf = vec![0u8; w as usize * h as usize * BYTES_PER_PIXEL];
            doc.render_tile_into(&req, &mut buf).expect("render");
            assert!(ink(&buf, w, h) > 0.1, "{extra:?}: tidak ada tinta");
            assert_eq!(
                ink_quarter(&buf, w, h),
                want,
                "{extra:?}: sudut bertinta salah"
            );
        }
    }

    /// A tile asks for one rectangle of the page and must get exactly that one.
    #[test]
    fn a_tile_shows_only_its_own_rectangle() {
        let (engine, _pdfium) = engine_or_skip!();
        let doc = engine
            .open_bytes(corner_page(0.0, 0.0, 0), None, None)
            .expect("buka sintetis");
        // The ink square covers display x 0..100, y 0..200 of a 200x400 page.
        let inside = PdfRectF::new(0.0, 0.0, 100.0, 200.0);
        let outside = PdfRectF::new(100.0, 200.0, 200.0, 400.0);
        for (source, expect_ink) in [(inside, true), (outside, false)] {
            let req = TileRequest {
                page: 0,
                source,
                dest_w: 256,
                dest_h: 512,
                rotation: RotationQuarter::None,
                draw_annotations: false,
                quality: Quality::Sharp,
                limit_image_cache: false,
                invert: false,
            };
            let mut buf = vec![0u8; 256 * 512 * BYTES_PER_PIXEL];
            doc.render_tile_into(&req, &mut buf).expect("render");
            let got = ink(&buf, 256, 512);
            if expect_ink {
                assert!(
                    got > 0.9,
                    "ubin di atas persegi harus penuh tinta: {got:.4}"
                );
            } else {
                assert!(got < 0.01, "ubin di luar persegi harus putih: {got:.4}");
            }
        }
    }

    /// A one-page AcroForm: a text field whose appearance is a solid block,
    /// 60 by 90 points, near the top-left of an unrotated 200 by 400 page
    /// whose box starts at (`mx`, `my`) and which carries `/Rotate rotate`.
    fn form_page(mx: f32, my: f32, rotate: i32) -> Vec<u8> {
        form_page_drawing(mx, my, rotate, "0 0 0 rg 0 0 60 90 re f\n")
    }

    /// [`form_page`], its field's appearance being `ap`.
    fn form_page_drawing(mx: f32, my: f32, rotate: i32, ap: &str) -> Vec<u8> {
        let objects = [
            "<</Type/Catalog/Pages 2 0 R/AcroForm<</Fields[4 0 R]/DA(/Helv 0 Tf 0 g)/DR<</Font<</Helv 6 0 R>>>>>>>>"
                .to_string(),
            "<</Type/Pages/Kids[3 0 R]/Count 1>>".to_string(),
            format!(
                "<</Type/Page/Parent 2 0 R/MediaBox[{mx} {my} {} {}]/Rotate {rotate}/Annots[4 0 R]>>",
                mx + 200.0,
                my + 400.0
            ),
            format!(
                "<</Type/Annot/Subtype/Widget/FT/Tx/T(nama)/F 4/P 3 0 R/Rect[{} {} {} {}]/AP<</N 5 0 R>>>>",
                mx + 20.0,
                my + 290.0,
                mx + 80.0,
                my + 380.0
            ),
            format!(
                "<</Type/XObject/Subtype/Form/BBox[0 0 60 90]/Length {}>>\nstream\n{ap}endstream",
                ap.len()
            ),
            "<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>".to_string(),
        ];
        let mut pdf = String::from("%PDF-1.7\n");
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
        }
        let xref = pdf.len();
        pdf.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objects.len() + 1
        ));
        for off in &offsets {
            pdf.push_str(&format!("{off:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        ));
        pdf.into_bytes()
    }

    /// Pixels dark in one bitmap and light in the other.
    fn mismatched(a: &[u8], b: &[u8]) -> usize {
        a.chunks_exact(BYTES_PER_PIXEL)
            .zip(b.chunks_exact(BYTES_PER_PIXEL))
            .filter(|(p, q)| (p[0] < 128) != (q[0] < 128))
            .count()
    }

    /// Form widgets are drawn by `FPDF_FFLDraw` — the page render leaves them
    /// out — and it takes a start, a size and a rotation rather than the tile
    /// matrix. So the same page flattened (the widget's appearance made part
    /// of the page's content, drawn by the matrix path) must come out the
    /// same: on every extra rotation, on pages with their own `/Rotate` and a
    /// box off the origin, and on a tile that is not the page's corner.
    #[test]
    fn widgets_land_where_flattening_puts_them() {
        let (engine, _pdfium) = engine_or_skip!();
        let scale = 1.5f32;
        for (mx, my, rotate) in [
            (0.0, 0.0, 0),
            (30.0, 40.0, 0),
            (30.0, 40.0, 90),
            (0.0, 0.0, 270),
        ] {
            let bytes = form_page(mx, my, rotate);
            let live = engine.open_bytes(bytes.clone(), None, None).expect("buka");
            let flat = {
                let d = engine.open_bytes(bytes, None, None).expect("buka");
                d.flatten_page(0).expect("ratakan");
                engine
                    .open_bytes(d.save_to_vec().expect("simpan"), None, None)
                    .expect("buka lagi")
            };
            assert!(
                !live.form_handle().is_null(),
                "dokumen berformulir tanpa lingkungan"
            );
            for extra in [
                RotationQuarter::None,
                RotationQuarter::Cw90,
                RotationQuarter::Cw180,
                RotationQuarter::Cw270,
            ] {
                let size = live.page_display_size(0, extra).expect("ukuran");
                let whole = PdfRectF::new(0.0, 0.0, size.width, size.height);
                // The four quarters too, cut on the pixel grid as the
                // viewport's tiles are: tiles that do not start at the page's
                // corner, one of which holds the widget whatever the turn.
                let (hw, hh) = (
                    (size.width * scale / 2.0).floor() / scale,
                    (size.height * scale / 2.0).floor() / scale,
                );
                let quarters = [
                    PdfRectF::new(0.0, size.height - hh, hw, size.height),
                    PdfRectF::new(hw, size.height - hh, size.width, size.height),
                    PdfRectF::new(0.0, 0.0, hw, size.height - hh),
                    PdfRectF::new(hw, 0.0, size.width, size.height - hh),
                ];
                let mut quarter_ink = 0.0;
                for (i, source) in std::iter::once(whole).chain(quarters).enumerate() {
                    let req = TileRequest {
                        page: 0,
                        source,
                        dest_w: (source.width() * scale).round() as u32,
                        dest_h: (source.height() * scale).round() as u32,
                        rotation: extra,
                        draw_annotations: true,
                        quality: Quality::Sharp,
                        limit_image_cache: false,
                        invert: false,
                    };
                    let n = req.dest_w as usize * req.dest_h as usize * BYTES_PER_PIXEL;
                    let (mut a, mut b) = (vec![0u8; n], vec![0u8; n]);
                    live.render_tile_into(&req, &mut a).expect("render");
                    flat.render_tile_into(&req, &mut b).expect("render datar");
                    let ink = ink(&b, req.dest_w, req.dest_h);
                    if i == 0 {
                        // The sanity half: the reference has the widget in
                        // it, so a render without it could not pass.
                        assert!(
                            ink > 0.05,
                            "acuan tanpa tinta: {ink:.3} ({mx},{my},/Rotate {rotate}, {extra:?})"
                        );
                    } else {
                        quarter_ink += ink;
                    }
                    let off = mismatched(&a, &b);
                    assert!(
                        off <= 2,
                        "{off} piksel beda ({mx},{my},/Rotate {rotate}, {extra:?}, {source:?})"
                    );
                }
                assert!(quarter_ink > 0.05, "tidak ada kuadran bertinta ({extra:?})");
            }
        }
    }

    /// What is typed into a field shows on the page before anything is
    /// saved: filling rebuilds the widget's appearance, and the render draws
    /// the widget through the environment that did it.
    #[test]
    fn a_filled_field_shows_its_value() {
        let (engine, _pdfium) = engine_or_skip!();
        let doc = engine
            .open_bytes(form_page_drawing(0.0, 0.0, 0, ""), None, None)
            .expect("buka");
        // The field's rectangle in the tile, points to pixels at 1x: x 20..80,
        // y from the top 20..110.
        let field_ink = |doc: &Document| {
            let (g, px) = doc.render_page(0, 1.0, Quality::Sharp).expect("render");
            ink_in(&px, g.width, g.height, 20, 20, 80, 110)
        };
        let before = field_ink(&doc);
        assert!(before < 0.001, "isian kosong bertinta: {before:.4}");
        doc.fill_form(&[("nama".into(), crate::forms::FieldValue::Text("MMMM".into()))])
            .expect("isi");
        let after = field_ink(&doc);
        assert!(after > 0.01, "nilai tidak tampil di halaman: {after:.4}");
    }

    /// A 200 x 200 page: a 2 x 2 picture drawn at (20, 20)–(100, 100), and
    /// a black square drawn as a path at (120, 120)–(180, 180).
    fn picture_and_ink_page() -> Vec<u8> {
        // Four mid-grey-blue pixels: inverted, they would change; kept, not.
        let mut content = b"q 80 0 0 80 20 20 cm BI /W 2 /H 2 /CS /RGB /BPC 8 ID ".to_vec();
        content.extend_from_slice(&[60, 90, 160].repeat(4));
        content.extend_from_slice(b" EI Q\n0 0 0 rg 120 120 60 60 re f\n");
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        let objs: Vec<Vec<u8>> = vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R>>".to_vec(),
            [
                format!("<</Length {}>>\nstream\n", content.len()).into_bytes(),
                content.clone(),
                b"\nendstream".to_vec(),
            ]
            .concat(),
        ];
        for (i, body) in objs.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes(),
        );
        for off in &offsets {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
                objs.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    /// Dark mode's smart inversion: the paper and the ink flip, the picture
    /// stays exactly as drawn — on the whole page, on a rotated one, and on a
    /// tile that holds only part of each.
    #[test]
    fn inversion_flips_the_page_and_leaves_its_picture() {
        let (engine, _pdfium) = engine_or_skip!();
        let doc = engine
            .open_bytes(picture_and_ink_page(), None, None)
            .expect("buka");
        let pics = doc.image_rects(0, RotationQuarter::None).expect("gambar");
        assert_eq!(pics.len(), 1, "satu gambar di halaman");
        let render = |invert: bool, rotation: RotationQuarter, source: PdfRectF| {
            let req = TileRequest {
                page: 0,
                source,
                dest_w: source.width().round() as u32,
                dest_h: source.height().round() as u32,
                rotation,
                draw_annotations: true,
                quality: Quality::Sharp,
                limit_image_cache: false,
                invert,
            };
            let mut buf = vec![0u8; (req.dest_w * req.dest_h * 4) as usize];
            doc.render_tile_into(&req, &mut buf).expect("render");
            (buf, req.dest_w)
        };
        let at = |buf: &[u8], w: u32, x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [buf[i], buf[i + 1], buf[i + 2]]
        };
        for rotation in [RotationQuarter::None, RotationQuarter::Cw90] {
            let whole = PdfRectF::new(0.0, 0.0, 200.0, 200.0);
            let (plain, w) = render(false, rotation, whole);
            let (dark, _) = render(true, rotation, whole);
            // Picture centre, ink centre and bare paper, in tile pixels (y
            // down) for this rotation.
            // A quarter turn clockwise takes the bottom-left picture to the
            // top-left and the top-right square to the bottom-right.
            let (pic, ink, paper) = match rotation {
                RotationQuarter::Cw90 => ((60, 60), (150, 150), (110, 110)),
                _ => ((60, 140), (150, 50), (110, 110)),
            };
            assert_eq!(
                at(&dark, w, pic.0, pic.1),
                at(&plain, w, pic.0, pic.1),
                "gambar tetap ({rotation:?})"
            );
            assert!(
                at(&plain, w, ink.0, ink.1)[0] < 30 && at(&dark, w, ink.0, ink.1)[0] > 200,
                "tinta jadi terang ({rotation:?})"
            );
            assert!(
                at(&plain, w, paper.0, paper.1)[0] > 240 && at(&dark, w, paper.0, paper.1)[0] < 40,
                "kertas jadi gelap ({rotation:?})"
            );
        }
        // A tile over the picture's right half and the square's left half.
        let part = PdfRectF::new(60.0, 60.0, 140.0, 140.0);
        let (plain, w) = render(false, RotationQuarter::None, part);
        let (dark, _) = render(true, RotationQuarter::None, part);
        // (70, 70) in points is inside the picture: tile (10, 70).
        assert_eq!(
            at(&dark, w, 10, 70),
            at(&plain, w, 10, 70),
            "separuh gambar di ubin tetap"
        );
        // (130, 130) is inside the square: tile (70, 10).
        assert!(
            at(&dark, w, 70, 10)[0] > 200,
            "separuh kotak di ubin terang"
        );
    }
}
