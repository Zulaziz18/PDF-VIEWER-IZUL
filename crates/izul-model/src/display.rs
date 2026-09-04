//! The display list (SPEC 3.2).
//!
//! This is the mechanism that makes editor/export parity structural rather than
//! something we chase with bug reports. An annotation does not produce pixels
//! and does not produce an appearance stream. It produces a [`DisplayOp`]
//! sequence in PDF user space, and two backends consume that same sequence:
//!
//!   * the canvas backend draws the live proxy while an object is being dragged;
//!   * the appearance-stream backend serialises it into PDF operators on save.
//!
//! Geometry cannot diverge between what the user sees and what gets written,
//! because there is only one description of it. Nothing in this module touches
//! PDFium or the filesystem, so it is testable on its own — which is the point
//! of keeping it in a crate that cannot even name those dependencies.
//!
//! Phase 0 defines the vocabulary. The `display_list(obj, font_ctx)` function
//! that produces it, and both backends, are Phase 3.

use serde::{Deserialize, Serialize};

use crate::geom::{Matrix, PdfPointF, PdfRectF};

/// Non-premultiplied sRGB with a separate alpha, matching how PDF expresses
/// colour (a colour operator plus an `ExtGState` alpha) rather than how a canvas
/// does. Keeping the two separate is what lets the appearance-stream backend
/// emit the alpha as `/CA`/`/ca` instead of baking it into the colour.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const BLACK: Rgba = Rgba::new(0.0, 0.0, 0.0, 1.0);

    pub fn from_rgb8(r: u8, g: u8, b: u8, alpha: f32) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: alpha,
        }
    }
}

/// The blend modes we support, which is exactly the set PDF names and PDFium
/// implements. `Multiply` is the one that matters for highlights: it is what
/// keeps the text underneath legible (SPEC 11.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Darken,
    Lighten,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Even-odd versus nonzero winding. PDF's `f` and `f*` operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    /// Dash pattern in points, with its phase. Empty means solid.
    pub dash: Vec<f32>,
    pub dash_phase: f32,
}

/// One segment of a path. Cubic Béziers only: PDF has no quadratic segment
/// operator, so admitting quadratics here would force a conversion that differs
/// between the two backends — precisely the class of divergence this design
/// exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PathSeg {
    MoveTo(PdfPointF),
    LineTo(PdfPointF),
    CurveTo {
        c1: PdfPointF,
        c2: PdfPointF,
        to: PdfPointF,
    },
    Close,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Path {
    pub segs: Vec<PathSeg>,
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_to(mut self, p: PdfPointF) -> Self {
        self.segs.push(PathSeg::MoveTo(p));
        self
    }

    pub fn line_to(mut self, p: PdfPointF) -> Self {
        self.segs.push(PathSeg::LineTo(p));
        self
    }

    pub fn curve_to(mut self, c1: PdfPointF, c2: PdfPointF, to: PdfPointF) -> Self {
        self.segs.push(PathSeg::CurveTo { c1, c2, to });
        self
    }

    pub fn close(mut self) -> Self {
        self.segs.push(PathSeg::Close);
        self
    }

    pub fn rect(r: PdfRectF) -> Self {
        Path::new()
            .move_to(PdfPointF::new(r.left, r.bottom))
            .line_to(PdfPointF::new(r.right, r.bottom))
            .line_to(PdfPointF::new(r.right, r.top))
            .line_to(PdfPointF::new(r.left, r.top))
            .close()
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    /// Bounding box over the path's control points.
    ///
    /// Deliberately the control hull, not the tight curve bounds: it is cheap,
    /// never too small, and both backends use it only for dirty rectangles and
    /// hit-test culling, where being conservative is correct.
    pub fn control_bounds(&self) -> Option<PdfRectF> {
        let mut out: Option<PdfRectF> = None;
        let mut add = |p: PdfPointF| {
            let r = PdfRectF::new(p.x, p.y, p.x, p.y);
            out = Some(match out {
                None => r,
                Some(cur) => PdfRectF {
                    left: cur.left.min(p.x),
                    bottom: cur.bottom.min(p.y),
                    right: cur.right.max(p.x),
                    top: cur.top.max(p.y),
                },
            });
        };
        for seg in &self.segs {
            match *seg {
                PathSeg::MoveTo(p) | PathSeg::LineTo(p) => add(p),
                PathSeg::CurveTo { c1, c2, to } => {
                    add(c1);
                    add(c2);
                    add(to);
                }
                PathSeg::Close => {}
            }
        }
        out
    }
}

/// Handle to a font resolved by the font context. The display list never carries
/// glyph outlines: the canvas backend needs a browser font, the appearance
/// backend needs an embedded font program, and resolving that is the font
/// context's job, not the geometry's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FontRef(pub u32);

/// Handle to an image in the document's resource table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ImageRef(pub u32);

/// One positioned glyph.
///
/// Carrying an explicit advance per glyph rather than letting each backend do
/// its own shaping is what keeps a run of text landing in the same place on the
/// canvas and in the exported file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PositionedGlyph {
    /// Glyph index within the font, not a Unicode code point.
    pub glyph_id: u16,
    /// The character this glyph came from, kept for the `/ToUnicode` map so the
    /// exported file stays searchable and copy-pasteable.
    pub unicode: char,
    /// Offset from the run's origin, in text space before the text matrix.
    pub offset: PdfPointF,
}

/// A primitive drawing command in PDF user space.
///
/// Deliberately small. Every operation here maps onto both a canvas call and a
/// PDF content-stream operator without either backend having to invent
/// behaviour of its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DisplayOp {
    FillPath {
        path: Path,
        color: Rgba,
        rule: FillRule,
        blend: BlendMode,
    },
    StrokePath {
        path: Path,
        color: Rgba,
        style: StrokeStyle,
        blend: BlendMode,
    },
    DrawText {
        glyphs: Vec<PositionedGlyph>,
        font: FontRef,
        size: f32,
        /// Text matrix: places the run on the page.
        matrix: Matrix,
        color: Rgba,
        blend: BlendMode,
    },
    DrawImage {
        image: ImageRef,
        matrix: Matrix,
        opacity: f32,
    },
    PushClip {
        path: Path,
        rule: FillRule,
    },
    PopClip,
    PushTransform {
        matrix: Matrix,
    },
    PopTransform,
}

/// A complete display list for one object.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DisplayList {
    pub ops: Vec<DisplayOp>,
}

impl DisplayList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, op: DisplayOp) {
        self.ops.push(op);
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// True when every `Push*` has a matching `Pop*` and no `Pop*` appears
    /// before its `Push*`.
    ///
    /// An unbalanced list is a bug in whichever generator produced it, and it
    /// shows up very differently in the two backends — the canvas leaks a clip
    /// into the next object, the appearance stream produces a `q`/`Q` mismatch
    /// that some readers tolerate and others do not. Checking it here catches
    /// that at the source. Clip and transform stacks are checked independently
    /// because PDF keeps them in one graphics state but our canvas backend does
    /// not; a list that interleaves them wrongly would still be valid PDF and
    /// still render differently, so the ordering check below rejects it.
    pub fn is_balanced(&self) -> bool {
        let mut stack: Vec<u8> = Vec::new();
        for op in &self.ops {
            match op {
                DisplayOp::PushClip { .. } => stack.push(0),
                DisplayOp::PushTransform { .. } => stack.push(1),
                DisplayOp::PopClip => {
                    if stack.pop() != Some(0) {
                        return false;
                    }
                }
                DisplayOp::PopTransform => {
                    if stack.pop() != Some(1) {
                        return false;
                    }
                }
                _ => {}
            }
        }
        stack.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32) -> PdfPointF {
        PdfPointF::new(x, y)
    }

    #[test]
    fn rect_path_closes_and_has_five_segments() {
        let p = Path::rect(PdfRectF::new(0.0, 0.0, 10.0, 20.0));
        assert_eq!(p.segs.len(), 5);
        assert_eq!(p.segs.last(), Some(&PathSeg::Close));
    }

    #[test]
    fn rect_path_bounds_match_the_rect() {
        let r = PdfRectF::new(3.0, -4.0, 10.0, 20.0);
        assert_eq!(Path::rect(r).control_bounds(), Some(r));
    }

    #[test]
    fn empty_path_has_no_bounds() {
        assert_eq!(Path::new().control_bounds(), None);
    }

    #[test]
    fn curve_bounds_use_the_control_hull() {
        // The curve itself never reaches y = 100, but the hull does; being
        // conservative here is the documented contract.
        let p = Path::new().move_to(pt(0.0, 0.0)).curve_to(
            pt(0.0, 100.0),
            pt(10.0, 100.0),
            pt(10.0, 0.0),
        );
        let b = p.control_bounds().expect("has points");
        assert_eq!(b.top, 100.0);
    }

    #[test]
    fn balanced_list_is_accepted() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::PushTransform {
            matrix: Matrix::IDENTITY,
        });
        dl.push(DisplayOp::PushClip {
            path: Path::new(),
            rule: FillRule::NonZero,
        });
        dl.push(DisplayOp::PopClip);
        dl.push(DisplayOp::PopTransform);
        assert!(dl.is_balanced());
    }

    #[test]
    fn unclosed_push_is_rejected() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::PushClip {
            path: Path::new(),
            rule: FillRule::NonZero,
        });
        assert!(!dl.is_balanced());
    }

    #[test]
    fn crossed_push_pop_is_rejected() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::PushTransform {
            matrix: Matrix::IDENTITY,
        });
        dl.push(DisplayOp::PushClip {
            path: Path::new(),
            rule: FillRule::NonZero,
        });
        dl.push(DisplayOp::PopTransform); // wrong order
        dl.push(DisplayOp::PopClip);
        assert!(!dl.is_balanced());
    }

    #[test]
    fn pop_without_push_is_rejected() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::PopClip);
        assert!(!dl.is_balanced());
    }

    #[test]
    fn display_list_round_trips_through_postcard() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::FillPath {
            path: Path::rect(PdfRectF::new(1.0, 2.0, 3.0, 4.0)),
            color: Rgba::from_rgb8(255, 235, 59, 0.4),
            rule: FillRule::NonZero,
            blend: BlendMode::Multiply,
        });
        dl.push(DisplayOp::DrawText {
            glyphs: vec![PositionedGlyph {
                glyph_id: 42,
                unicode: 'A',
                offset: pt(0.0, 0.0),
            }],
            font: FontRef(1),
            size: 12.0,
            matrix: Matrix::translate(72.0, 700.0),
            color: Rgba::BLACK,
            blend: BlendMode::Normal,
        });
        let bytes = postcard::to_allocvec(&dl).expect("serialises");
        let back: DisplayList = postcard::from_bytes(&bytes).expect("deserialises");
        assert_eq!(dl, back);
    }

    #[test]
    fn serialisation_is_deterministic() {
        // The AP-stream backend and the golden-image tests both depend on the
        // same list producing the same bytes every time.
        let dl = DisplayList {
            ops: vec![DisplayOp::StrokePath {
                path: Path::new().move_to(pt(0.0, 0.0)).line_to(pt(10.0, 10.0)),
                color: Rgba::BLACK,
                style: StrokeStyle {
                    width: 1.5,
                    dash: vec![3.0, 2.0],
                    ..Default::default()
                },
                blend: BlendMode::Normal,
            }],
        };
        let a = postcard::to_allocvec(&dl).expect("serialises");
        let b = postcard::to_allocvec(&dl).expect("serialises");
        assert_eq!(a, b);
    }
}
