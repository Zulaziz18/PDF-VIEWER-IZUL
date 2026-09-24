//! PDFium's side of redaction (Phase 6): where the characters are, before and
//! after.
//!
//! `izul-redact` takes content out of a file without PDFium; this module is
//! how the worker finds out, with PDFium, whether it worked. Two things are
//! checked on every redacted page, and a redaction that fails either is not
//! saved:
//!
//! * **Nothing is left in an area.** No character PDFium can extract has its
//!   centre inside a marked area.
//! * **Nothing outside moved.** Every character outside the areas is exactly
//!   where it was. A glyph taken out of a line is replaced by a gap computed
//!   from the font's widths; if those widths were wrong, the rest of the line
//!   would shift, and this is what would notice.
//!
//! Before redacting, each area is also widened to the whole of every
//! character whose centre it holds ([`plan_areas`]). A selection box drawn
//! from PDFium's character boxes can be a hair smaller than the glyph box the
//! redactor judges by; widening makes "the characters the user marked" and
//! "the characters that go" the same set, and the black bar cover them.

use std::collections::HashMap;
use std::os::raw::c_double;

use pdfium_render::prelude::FS_RECTF;

use crate::engine::{Document, PageGeometry};
use crate::error::Result;
use crate::ffi_guard::guard;
use crate::geom::{PdfRectF, RotationQuarter};
use crate::text::TextPage;

/// One character as PDFium extracts it, in the page's user space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharLayout {
    pub unicode: char,
    /// Inserted by PDFium's text extraction (a space between words, a line
    /// break), not drawn by the page.
    pub generated: bool,
    /// The glyph's actual extent.
    pub tight: PdfRectF,
    /// The font's full box for the glyph.
    pub loose: PdfRectF,
    pub origin: (f64, f64),
    /// Font size in points, as PDFium reports it.
    pub size: f64,
}

impl CharLayout {
    /// Where the character is, for "is it in the area": the middle of its
    /// glyph, or of its font box when the glyph has no extent (a space).
    pub fn centre(&self) -> (f32, f32) {
        let r = if self.tight.width() > 0.0 && self.tight.height() > 0.0 {
            self.tight
        } else {
            self.loose
        };
        ((r.left + r.right) / 2.0, (r.bottom + r.top) / 2.0)
    }

    fn counts(&self) -> bool {
        !self.generated && !self.unicode.is_whitespace() && self.unicode != '\u{0}'
    }
}

impl PageGeometry {
    /// The inverse of [`PageGeometry::to_display`]: a rectangle in display
    /// space back into the page's own user space.
    pub fn from_display(&self, r: PdfRectF, extra: RotationQuarter) -> PdfRectF {
        let a = self.point_from_display(r.left, r.bottom, extra);
        let b = self.point_from_display(r.right, r.top, extra);
        PdfRectF::new(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
    }

    fn point_from_display(&self, dx: f32, dy: f32, extra: RotationQuarter) -> (f32, f32) {
        let (bw, bh) = (self.bbox.width(), self.bbox.height());
        let (u, v) = match self.intrinsic.plus(extra) {
            RotationQuarter::None => (dx, dy),
            RotationQuarter::Cw90 => (bw - dy, dx),
            RotationQuarter::Cw180 => (bw - dx, bh - dy),
            RotationQuarter::Cw270 => (dy, bh - dx),
        };
        (u + self.bbox.left, v + self.bbox.bottom)
    }
}

impl Document {
    /// Every character on a page, with both boxes and its origin.
    pub fn char_layout(&self, page: u32) -> Result<Vec<CharLayout>> {
        self.check_page(page)?;
        guard("char_layout", || {
            self.with_page(page, |p| {
                let tp = TextPage::load(self, p)?;
                let b = tp.bindings();
                let n = tp.count();
                let mut out = Vec::with_capacity(n.max(0) as usize);
                for i in 0..n {
                    // SAFETY: `i` is in `0..count` and the text page is live;
                    // every out-parameter is a live local of the right type.
                    let (code, generated, tight, loose, origin, size) = unsafe {
                        let code = b.FPDFText_GetUnicode(tp.raw(), i);
                        let generated = b.FPDFText_IsGenerated(tp.raw(), i) == 1;
                        let (mut l, mut r, mut bt, mut t): (
                            c_double,
                            c_double,
                            c_double,
                            c_double,
                        ) = (0.0, 0.0, 0.0, 0.0);
                        let ok =
                            b.FPDFText_GetCharBox(tp.raw(), i, &mut l, &mut r, &mut bt, &mut t);
                        let tight = (ok != 0)
                            .then(|| PdfRectF::new(l as f32, bt as f32, r as f32, t as f32));
                        let mut rect = FS_RECTF {
                            left: 0.0,
                            top: 0.0,
                            right: 0.0,
                            bottom: 0.0,
                        };
                        let ok = b.FPDFText_GetLooseCharBox(tp.raw(), i, &mut rect);
                        let loose = (ok != 0)
                            .then(|| PdfRectF::new(rect.left, rect.bottom, rect.right, rect.top));
                        let (mut x, mut y): (c_double, c_double) = (0.0, 0.0);
                        let ok = b.FPDFText_GetCharOrigin(tp.raw(), i, &mut x, &mut y);
                        let size = b.FPDFText_GetFontSize(tp.raw(), i);
                        (
                            code,
                            generated,
                            tight,
                            loose,
                            (ok != 0).then_some((x, y)),
                            size,
                        )
                    };
                    let (Some(unicode), Some(tight), Some(loose), Some(origin)) =
                        (char::from_u32(code), tight, loose, origin)
                    else {
                        continue;
                    };
                    out.push(CharLayout {
                        unicode,
                        generated,
                        tight,
                        loose,
                        origin,
                        size,
                    });
                }
                Ok(out)
            })
        })
    }
}

fn inside(r: &PdfRectF, (x, y): (f32, f32)) -> bool {
    x > r.left && x < r.right && y > r.bottom && y < r.top
}

fn union(a: PdfRectF, b: PdfRectF) -> PdfRectF {
    PdfRectF::new(
        a.left.min(b.left),
        a.bottom.min(b.bottom),
        a.right.max(b.right),
        a.top.max(b.top),
    )
}

fn shrink(r: &PdfRectF, by: f32) -> PdfRectF {
    PdfRectF::new(r.left + by, r.bottom + by, r.right - by, r.top - by)
}

fn overlaps(a: &PdfRectF, b: &PdfRectF) -> bool {
    a.left < b.right && b.left < a.right && a.bottom < b.top && b.bottom < a.top
}

/// Areas as the user drew them (display space) → areas to redact (user
/// space), each widened to the full box of every character it holds. One
/// out for each in, in order.
pub fn plan_areas(
    geometry: &PageGeometry,
    display: &[PdfRectF],
    chars: &[CharLayout],
) -> Vec<PdfRectF> {
    display
        .iter()
        .map(|d| {
            let user = geometry.from_display(*d, RotationQuarter::None);
            chars
                .iter()
                .filter(|c| c.counts() && inside(&user, c.centre()))
                .fold(user, |acc, c| union(acc, c.loose))
        })
        .collect()
}

/// Why a redaction did not verify.
#[derive(Debug, Clone, PartialEq)]
pub enum CheckFailure {
    /// A character is still inside an area.
    Left { unicode: char, x: f64, y: f64 },
    /// A character outside every area is no longer where it was.
    Moved { unicode: char, x: f64, y: f64 },
}

impl std::fmt::Display for CheckFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckFailure::Left { unicode, x, y } => write!(
                f,
                "karakter '{unicode}' masih ada di area redaksi (x {x:.1}, y {y:.1})"
            ),
            CheckFailure::Moved { unicode, x, y } => write!(
                f,
                "karakter '{unicode}' di luar area bergeser dari tempatnya (x {x:.1}, y {y:.1})"
            ),
        }
    }
}

/// How close to an area's edge a leftover character's centre may be before
/// it counts as inside: the width of a hair, for float noise.
const EDGE: f32 = 0.25;
/// How far a character outside the areas may move: well under a pixel at
/// any zoom, and well over the rounding of a six-decimal TJ number.
const MOVE: f64 = 0.05;

/// PDFium and the specification disagree by up to this much per glyph, in
/// thousandths of an em — measured, not assumed. The gap left for a removed
/// glyph is its `/Widths` entry, as ISO 32000-1 §9.4.4 says and as MuPDF lays
/// text out (0.03 pt from the widths over a whole line of `viewer-10p.pdf`).
/// PDFium truncates each width to a whole unit first (443.8477 is drawn as
/// 443: 0.005 pt from the truncated widths over the same line, 0.22 pt from
/// the real ones). So after a gap, PDFium places the rest of the line up to
/// one unit per removed glyph away from where it placed it before; every
/// reader that follows the specification places it exactly. A width that is
/// actually wrong is off by tens of units, and still fails.
const PDFIUM_TRUNCATION: f64 = 1.0;

/// Nothing left in `areas`.
pub fn check_left(
    areas: &[PdfRectF],
    after: &[CharLayout],
) -> std::result::Result<(), CheckFailure> {
    let inner: Vec<PdfRectF> = areas.iter().map(|a| shrink(a, EDGE)).collect();
    for c in after.iter().filter(|c| c.counts()) {
        if inner.iter().any(|a| inside(a, c.centre())) {
            return Err(CheckFailure::Left {
                unicode: c.unicode,
                x: c.origin.0,
                y: c.origin.1,
            });
        }
    }
    Ok(())
}

/// Nothing left under the marks *as the user saw them*: the characters are
/// taken into display space the way the selection layer takes them there
/// (`to_display`), and compared with the areas as drawn — never through
/// [`PageGeometry::from_display`], the conversion the redaction itself used.
/// A mistake in that conversion would send the redaction and [`check`] to
/// the same wrong place, and both would agree; this check would not.
pub fn check_left_as_shown(
    geometry: &PageGeometry,
    display_areas: &[PdfRectF],
    after: &[CharLayout],
) -> std::result::Result<(), CheckFailure> {
    let shown: Vec<CharLayout> = after
        .iter()
        .map(|c| CharLayout {
            tight: geometry.to_display(c.tight, RotationQuarter::None),
            loose: geometry.to_display(c.loose, RotationQuarter::None),
            ..*c
        })
        .collect();
    check_left(display_areas, &shown)
}

/// Nothing left in `areas`, and nothing outside them moved.
pub fn check(
    areas: &[PdfRectF],
    before: &[CharLayout],
    after: &[CharLayout],
) -> std::result::Result<(), CheckFailure> {
    check_left(areas, after)?;
    let mut found: HashMap<char, Vec<(f64, f64)>> = HashMap::new();
    for c in after.iter().filter(|c| c.counts()) {
        found.entry(c.unicode).or_default().push(c.origin);
    }
    // Characters near an area are left out of the comparison: whether a
    // glyph a quarter inside went or stayed is the redactor's call, not a
    // movement.
    let near: Vec<PdfRectF> = areas.iter().map(|a| shrink(a, -1.0)).collect();
    let removed: Vec<&CharLayout> = before
        .iter()
        .filter(|c| c.counts() && areas.iter().any(|a| inside(a, c.centre())))
        .collect();
    for c in before.iter().filter(|c| c.counts()) {
        if near.iter().any(|a| overlaps(a, &c.loose)) {
            continue;
        }
        // Removed glyphs on the same line — horizontal or vertical — are
        // the ones whose gaps can have moved this character.
        let gaps = removed
            .iter()
            .filter(|r| {
                (r.origin.1 - c.origin.1).abs() < 0.5 || (r.origin.0 - c.origin.0).abs() < 0.5
            })
            .count() as f64;
        let tolerance = MOVE + gaps * PDFIUM_TRUNCATION * c.size / 1000.0;
        let here = found.get(&c.unicode).is_some_and(|v| {
            v.iter().any(|o| {
                let (dx, dy) = (o.0 - c.origin.0, o.1 - c.origin.1);
                (dx * dx + dy * dy).sqrt() < tolerance
            })
        });
        if !here {
            return Err(CheckFailure::Moved {
                unicode: c.unicode,
                x: c.origin.0,
                y: c.origin.1,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(u: char, x: f32, y: f32) -> CharLayout {
        CharLayout {
            unicode: u,
            generated: false,
            tight: PdfRectF::new(x, y, x + 5.0, y + 7.0),
            loose: PdfRectF::new(x, y - 2.0, x + 6.0, y + 9.0),
            origin: (f64::from(x), f64::from(y)),
            size: 10.0,
        }
    }

    #[test]
    fn display_and_user_space_round_trip_at_every_rotation() {
        let g = |rot| PageGeometry {
            bbox: PdfRectF::new(10.0, 20.0, 622.0, 812.0),
            intrinsic: rot,
        };
        let r = PdfRectF::new(30.0, 40.0, 90.0, 55.0);
        for rot in [
            RotationQuarter::None,
            RotationQuarter::Cw90,
            RotationQuarter::Cw180,
            RotationQuarter::Cw270,
        ] {
            let d = g(rot).to_display(r, RotationQuarter::None);
            let back = g(rot).from_display(d, RotationQuarter::None);
            for (a, b) in [
                (back.left, r.left),
                (back.bottom, r.bottom),
                (back.right, r.right),
                (back.top, r.top),
            ] {
                assert!((a - b).abs() < 1e-3, "{rot:?}: {back:?} != {r:?}");
            }
        }
    }

    #[test]
    fn areas_grow_to_the_characters_they_hold() {
        let g = PageGeometry {
            bbox: PdfRectF::new(0.0, 0.0, 600.0, 800.0),
            intrinsic: RotationQuarter::None,
        };
        let chars = [
            ch('a', 100.0, 700.0),
            ch('b', 106.0, 700.0),
            ch('c', 200.0, 700.0),
        ];
        // A box over the middle of 'a' and 'b' only.
        let areas = plan_areas(&g, &[PdfRectF::new(101.0, 702.0, 110.0, 705.0)], &chars);
        assert_eq!(areas, vec![PdfRectF::new(100.0, 698.0, 112.0, 709.0)]);
    }

    #[test]
    fn a_character_left_in_an_area_fails() {
        let area = [PdfRectF::new(90.0, 690.0, 150.0, 720.0)];
        assert!(check_left(&area, &[ch('x', 300.0, 700.0)]).is_ok());
        assert!(matches!(
            check_left(&area, &[ch('x', 100.0, 700.0)]),
            Err(CheckFailure::Left { unicode: 'x', .. })
        ));
    }

    #[test]
    fn a_character_outside_that_moved_fails() {
        let area = [PdfRectF::new(90.0, 690.0, 150.0, 720.0)];
        let before = [ch('s', 100.0, 700.0), ch('k', 300.0, 700.0)];
        assert!(check(&area, &before, &[ch('k', 300.0, 700.0)]).is_ok());
        assert!(matches!(
            check(&area, &before, &[ch('k', 300.2, 700.0)]),
            Err(CheckFailure::Moved { unicode: 'k', .. })
        ));
        // Gone altogether is also "not where it was".
        assert!(check(&area, &before, &[]).is_err());
    }

    /// One removed glyph on the line allows PDFium's one unit of truncation
    /// (0.01 pt at 10 pt), and no more.
    #[test]
    fn the_allowance_is_one_unit_per_removed_glyph_on_the_line() {
        let area = [PdfRectF::new(90.0, 690.0, 150.0, 720.0)];
        let before = [
            ch('s', 100.0, 700.0),
            ch('k', 300.0, 700.0),
            ch('o', 300.0, 500.0),
        ];
        assert!(check(
            &area,
            &before,
            &[ch('k', 300.055, 700.0), ch('o', 300.0, 500.0)]
        )
        .is_ok());
        assert!(check(
            &area,
            &before,
            &[ch('k', 300.07, 700.0), ch('o', 300.0, 500.0)]
        )
        .is_err());
        // Another line gets no allowance from this one's gap.
        assert!(check(
            &area,
            &before,
            &[ch('k', 300.0, 700.0), ch('o', 300.055, 500.0)]
        )
        .is_err());
    }
}

/// Redaction of real documents, checked by PDFium. This is where the
/// redactor's font arithmetic meets the renderer's: if a single width were
/// wrong, a character after a gap would have moved and `check` would say so.
#[cfg(test)]
mod real {
    use super::*;
    use crate::test_support::root;

    fn to_rect(r: &PdfRectF) -> izul_redact::Rect {
        izul_redact::Rect::new(
            f64::from(r.left),
            f64::from(r.bottom),
            f64::from(r.right),
            f64::from(r.top),
        )
    }

    /// Picks a run of characters in the middle of a page and returns an area
    /// over their centres only — as a user's selection would be.
    fn middle_run(chars: &[CharLayout], len: usize) -> Option<PdfRectF> {
        let counted: Vec<&CharLayout> = chars.iter().filter(|c| c.counts()).collect();
        let start = counted.len() / 2;
        let run = counted.get(start..start + len)?;
        let first = run.first()?;
        // Same line only.
        let run: Vec<&&CharLayout> = run
            .iter()
            .take_while(|c| (c.origin.1 - first.origin.1).abs() < 0.5)
            .collect();
        let (x0, x1) = (
            run.iter().map(|c| c.centre().0).fold(f32::MAX, f32::min),
            run.iter().map(|c| c.centre().0).fold(f32::MIN, f32::max),
        );
        let y = first.centre().1;
        Some(PdfRectF::new(x0 - 0.5, y - 1.0, x1 + 0.5, y + 1.0))
    }

    fn redact_and_check(engine: &'static crate::Engine, path: &std::path::Path, pages: &[u32]) {
        let doc = engine.open_copied(path, None).expect("open");
        let bytes = doc.save_to_vec().expect("save");
        let mut plans = Vec::new();
        let mut before = Vec::new();
        for &page in pages {
            let geometry = doc.page_geometry(page).expect("geometry");
            let chars = doc.char_layout(page).expect("chars");
            let Some(user) = middle_run(&chars, 12) else {
                continue;
            };
            let display = geometry.to_display(user, RotationQuarter::None);
            let areas = plan_areas(&geometry, &[display], &chars);
            let inside: String = chars
                .iter()
                .filter(|c| c.counts() && areas.iter().any(|a| inside(a, c.centre())))
                .map(|c| c.unicode)
                .collect();
            assert!(
                !inside.is_empty(),
                "{}: halaman {page} tanpa teks di area",
                path.display()
            );
            plans.push(izul_redact::PageAreas {
                page,
                areas: areas
                    .iter()
                    .map(|a| izul_redact::Area {
                        rect: to_rect(a),
                        fill: Some([0.0, 0.0, 0.0]),
                    })
                    .collect(),
            });
            before.push((page, areas, chars, inside));
        }
        let (out, reports) = izul_redact::redact(&bytes, &plans).expect("redact");
        let after_doc = engine.open_bytes(out, None::<&str>, None).expect("reopen");
        assert_eq!(after_doc.page_count(), doc.page_count());
        for ((page, areas, chars, inside), report) in before.iter().zip(&reports) {
            let after = after_doc.char_layout(*page).expect("chars after");
            if let Err(e) = check(areas, chars, &after) {
                panic!("{} halaman {page}: {e}", path.display());
            }
            assert!(
                report.counts.glyphs as usize >= inside.chars().count(),
                "{report:?} vs {inside}"
            );
            eprintln!(
                "{} hlm {page}: '{inside}' dibuang ({} glyph), {} karakter lain tetap di tempatnya",
                path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                report.counts.glyphs,
                after.iter().filter(|c| c.counts()).count()
            );
        }
    }

    #[test]
    fn redacting_real_documents_moves_nothing_else() {
        let (engine, _pdfium) = crate::engine_and_lock!();
        let fixtures = root().join("test-fixtures");
        let viewer = fixtures.join("viewer-10p.pdf");
        let text = fixtures.join("text-500p.pdf");
        crate::require_fixture!("test-fixtures/viewer-10p.pdf", !viewer.exists());
        crate::require_fixture!("test-fixtures/text-500p.pdf", !text.exists());
        // viewer-10p has pages of different sizes and one with its own /Rotate.
        redact_and_check(engine, &viewer, &(0..10).collect::<Vec<_>>());
        redact_and_check(engine, &text, &[0, 1, 250, 499]);
        // Larger fixtures, when this machine has them.
        for name in ["mixed-500p.pdf", "mixed-raw-500p.pdf"] {
            let p = fixtures.join(name);
            if p.exists() {
                redact_and_check(engine, &p, &[0, 3]);
            }
        }
    }
}
