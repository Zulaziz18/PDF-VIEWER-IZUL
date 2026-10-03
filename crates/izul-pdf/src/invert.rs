//! Dark mode's "invert halaman cerdas" (SPEC 11.1, Phase 8): text becomes
//! light, pictures and photos stay as they are.
//!
//! Every pixel outside the page's pictures has its HSL lightness flipped and
//! its hue and saturation kept: `c + (255 − max − min)` per channel, which is
//! exactly L' = 1 − L. White paper goes dark, black text goes light, a red
//! heading stays red, a yellow highlight becomes a dark olive with the same
//! hue — rather than PDFium's forced colour scheme, which paints every path
//! one colour and turns a coloured chart into a grey one. The result is then
//! tone-mapped between the dark canvas and a soft white, so the page is not
//! pure black with pure white text (harsh, and a hole in the canvas around it).
//!
//! Pictures are found as image objects — directly on the page, or inside a
//! form XObject, whose own bounds are then kept whole — and their boxes are
//! left untouched. A scan is a picture: it stays as scanned.

use pdfium_render::prelude::*;

use crate::engine::{Document, PageGeometry};
use crate::geom::{PdfRectF, RotationQuarter};

const PAGEOBJ_IMAGE: i32 = 3;
const PAGEOBJ_FORM: i32 = 5;

/// Darkest and lightest values after inversion: the dark canvas (#1c1c1e is
/// 28) and a soft white.
const FLOOR: i32 = 28;
const CEILING: i32 = 232;

/// One pixel, BGRA: lightness flipped, hue kept, tone-mapped.
#[inline]
fn invert_pixel(px: &mut [u8]) {
    let [b, g, r, ..] = px else { return };
    let (bi, gi, ri) = (i32::from(*b), i32::from(*g), i32::from(*r));
    let d = 255 - ri.max(gi).max(bi) - ri.min(gi).min(bi);
    let tone = |c: i32| -> u8 { (FLOOR + (c + d).clamp(0, 255) * (CEILING - FLOOR) / 255) as u8 };
    *b = tone(bi);
    *g = tone(gi);
    *r = tone(ri);
}

/// Inverts a BGRA bitmap, leaving the pixel rectangles in `keep` (x0, y0,
/// x1, y1; exclusive ends) as they are.
pub fn smart_invert(buf: &mut [u8], stride: usize, w: u32, h: u32, keep: &[(u32, u32, u32, u32)]) {
    for y in 0..h {
        let row_keep: Vec<(u32, u32)> = keep
            .iter()
            .filter(|k| y >= k.1 && y < k.3)
            .map(|k| (k.0, k.2))
            .collect();
        let Some(row) = buf.get_mut(y as usize * stride..y as usize * stride + w as usize * 4)
        else {
            return;
        };
        for (x, px) in row.chunks_exact_mut(4).enumerate() {
            let x = x as u32;
            if row_keep.iter().any(|&(a, b)| x >= a && x < b) {
                continue;
            }
            invert_pixel(px);
        }
    }
}

/// Where `rects` (display space, points, y up) fall in a tile showing
/// `source` at `w` × `h` pixels, rounded outwards and clipped.
pub fn to_tile_pixels(
    rects: &[PdfRectF],
    source: &PdfRectF,
    w: u32,
    h: u32,
) -> Vec<(u32, u32, u32, u32)> {
    let sx = w as f32 / source.width().max(1e-6);
    let sy = h as f32 / source.height().max(1e-6);
    rects
        .iter()
        .filter_map(|r| {
            let x0 = ((r.left - source.left) * sx).floor().max(0.0);
            let x1 = ((r.right - source.left) * sx).ceil().min(w as f32);
            let y0 = ((source.top - r.top) * sy).floor().max(0.0);
            let y1 = ((source.top - r.bottom) * sy).ceil().min(h as f32);
            (x1 > x0 && y1 > y0).then_some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
        })
        .collect()
}

/// Whether a form XObject draws a picture anywhere inside it.
fn form_has_image(b: &dyn PdfiumLibraryBindings, form: FPDF_PAGEOBJECT, depth: u32) -> bool {
    if depth > 12 {
        return false;
    }
    // SAFETY: `form` is a live form object of a loaded page.
    let n = unsafe { b.FPDFFormObj_CountObjects(form) };
    (0..n.max(0) as u64).any(|i| {
        // SAFETY: `i` is within the form's object count.
        let o = unsafe { b.FPDFFormObj_GetObject(form, i as _) };
        if o.is_null() {
            return false;
        }
        // SAFETY: `o` is a live object of the form.
        match unsafe { b.FPDFPageObj_GetType(o) } {
            PAGEOBJ_IMAGE => true,
            PAGEOBJ_FORM => form_has_image(b, o, depth + 1),
            _ => false,
        }
    })
}

/// The boxes of a loaded page's pictures, in display space at `rotation`.
pub(crate) fn image_rects_on(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    geometry: &PageGeometry,
    rotation: RotationQuarter,
) -> Vec<PdfRectF> {
    // SAFETY: `page` is a live page handle; every object handle comes from it.
    let n = unsafe { b.FPDFPage_CountObjects(page) };
    let mut out = Vec::new();
    for i in 0..n.max(0) {
        // SAFETY: as above.
        let o = unsafe { b.FPDFPage_GetObject(page, i) };
        if o.is_null() {
            continue;
        }
        // SAFETY: `o` is a live page object.
        let kind = unsafe { b.FPDFPageObj_GetType(o) };
        let picture = kind == PAGEOBJ_IMAGE || (kind == PAGEOBJ_FORM && form_has_image(b, o, 0));
        if !picture {
            continue;
        }
        let (mut l, mut bo, mut r, mut t) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        // SAFETY: the four pointers are to live locals.
        if unsafe { b.FPDFPageObj_GetBounds(o, &mut l, &mut bo, &mut r, &mut t) } == 0 {
            continue;
        }
        let user = PdfRectF::new(l.min(r), bo.min(t), l.max(r), bo.max(t));
        out.push(geometry.to_display(user, rotation));
    }
    out
}

impl Document {
    /// The boxes of `page`'s pictures, in display space at `rotation`.
    pub fn image_rects(
        &self,
        page: u32,
        rotation: RotationQuarter,
    ) -> crate::error::Result<Vec<PdfRectF>> {
        let b = self.engine().bindings();
        self.with_page(page, |p| {
            let geometry = PageGeometry::of_page(self.engine(), p);
            Ok(image_rects_on(b, p, &geometry, rotation))
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing)]
    use super::*;

    fn px(b: u8, g: u8, r: u8) -> [u8; 4] {
        let mut p = [b, g, r, 255];
        invert_pixel(&mut p);
        p
    }

    #[test]
    fn paper_goes_dark_ink_goes_light_and_colour_keeps_its_hue() {
        assert_eq!(
            px(255, 255, 255),
            [28, 28, 28, 255],
            "kertas putih jadi kanvas gelap"
        );
        assert_eq!(
            px(0, 0, 0),
            [232, 232, 232, 255],
            "tinta hitam jadi putih lembut"
        );
        // Pure red has lightness one half: it stays red, only tone-mapped.
        let red = px(0, 0, 255);
        assert!(red[2] > 200 && red[0] < 40 && red[1] < 40, "{red:?}");
        // A pale yellow highlight goes dark, and stays yellow: red and green
        // above blue.
        let y = px(200, 255, 255);
        assert!(y[2] > y[0] + 20 && y[1] > y[0] + 20 && y[2] < 100, "{y:?}");
    }

    #[test]
    fn what_is_kept_is_left_exactly_as_it_was() {
        let (w, h) = (8u32, 4u32);
        let mut buf = vec![255u8; (w * h * 4) as usize];
        smart_invert(&mut buf, (w * 4) as usize, w, h, &[(2, 1, 5, 3)]);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let kept = (2..5).contains(&x) && (1..3).contains(&y);
                assert_eq!(buf[i], if kept { 255 } else { 28 }, "({x},{y})");
            }
        }
    }

    #[test]
    fn a_picture_maps_onto_the_tile_that_shows_it() {
        // A 100 x 100 pt tile at 2 px/pt, its top-left at (50, 750).
        let source = PdfRectF::new(50.0, 650.0, 150.0, 750.0);
        let pic = PdfRectF::new(60.0, 700.0, 80.0, 740.0);
        assert_eq!(
            to_tile_pixels(&[pic], &source, 200, 200),
            vec![(20, 20, 60, 100)]
        );
        // Wholly elsewhere: nothing to keep.
        let away = PdfRectF::new(300.0, 100.0, 320.0, 120.0);
        assert!(to_tile_pixels(&[away], &source, 200, 200).is_empty());
    }
}
