//! The page-space box a display list paints into.
//!
//! It becomes both the annotation's `/Rect` and its appearance form's `/BBox`.
//! The object's own `rect` is not enough: a rotated box's corners stick out of
//! its unrotated rect, a stroke is half its width wider than its path, and a
//! form XObject **clips to its `/BBox`** — so a box that is too small is not a
//! cosmetic error, it cuts pieces off the drawing in every reader.
//!
//! Conservative by construction: paths contribute their control hull, strokes
//! their half-width (and the mitre allowance), text a full em per glyph, and an
//! image its whole transformed unit square.

use izul_model::display::{DisplayList, DisplayOp, Path};
use izul_model::geom::{Matrix, PdfPointF, PdfRectF};

#[derive(Default)]
struct Acc {
    out: Option<PdfRectF>,
}

impl Acc {
    fn point(&mut self, p: PdfPointF) {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return;
        }
        let r = PdfRectF::new(p.x, p.y, p.x, p.y);
        self.out = Some(match self.out {
            None => r,
            Some(cur) => cur.union(&r),
        });
    }

    fn rect(&mut self, ctm: &Matrix, r: PdfRectF) {
        for (x, y) in [
            (r.left, r.bottom),
            (r.right, r.bottom),
            (r.left, r.top),
            (r.right, r.top),
        ] {
            self.point(ctm.apply(PdfPointF { x, y }));
        }
    }

    fn path(&mut self, ctm: &Matrix, path: &Path, pad: f32) {
        if let Some(b) = path.control_bounds() {
            self.rect(ctm, b.inflate(pad));
        }
    }
}

/// The painted box, padded by `margin` points, or `None` for a list that
/// paints nothing.
pub fn painted_bounds(list: &DisplayList, margin: f32) -> Option<PdfRectF> {
    let mut acc = Acc::default();
    let mut stack: Vec<Matrix> = Vec::new();
    let mut ctm = Matrix::IDENTITY;
    for op in &list.ops {
        match op {
            DisplayOp::FillPath { path, .. } => acc.path(&ctm, path, 0.0),
            DisplayOp::StrokePath { path, style, .. } => {
                // A mitred corner can reach `miter_limit * width / 2` past the
                // path; capped at 10 widths so a silly limit cannot make a
                // hairline's box enormous.
                let reach = style.width * 0.5 * style.miter_limit.clamp(1.0, 10.0);
                acc.path(&ctm, path, reach.max(style.width));
            }
            DisplayOp::DrawText {
                glyphs,
                size,
                matrix,
                ..
            } => {
                let text = matrix.then(&ctm);
                for g in glyphs {
                    acc.rect(
                        &text,
                        PdfRectF::new(
                            g.offset.x,
                            g.offset.y - 0.3 * size,
                            g.offset.x + *size,
                            g.offset.y + *size,
                        ),
                    );
                }
            }
            DisplayOp::DrawImage { matrix, .. } => {
                acc.rect(&matrix.then(&ctm), PdfRectF::new(0.0, 0.0, 1.0, 1.0));
            }
            DisplayOp::PushTransform { matrix } => {
                stack.push(ctm);
                ctm = matrix.then(&ctm);
            }
            DisplayOp::PopTransform => {
                ctm = stack.pop().unwrap_or(Matrix::IDENTITY);
            }
            // A clip only ever shrinks what is painted; ignoring it keeps the
            // box conservative.
            DisplayOp::PushClip { .. } | DisplayOp::PopClip => {}
        }
    }
    acc.out.map(|r| r.inflate(margin))
}

#[cfg(test)]
mod tests {
    use super::*;
    use izul_model::display::{BlendMode, FillRule, Rgba, StrokeStyle};

    fn square() -> Path {
        Path::rect(PdfRectF::new(100.0, 100.0, 200.0, 200.0))
    }

    #[test]
    fn a_fill_is_its_hull() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::FillPath {
            path: square(),
            color: Rgba::BLACK,
            rule: FillRule::NonZero,
            blend: BlendMode::Normal,
        });
        assert_eq!(
            painted_bounds(&dl, 0.0),
            Some(PdfRectF::new(100.0, 100.0, 200.0, 200.0))
        );
    }

    #[test]
    fn a_stroke_reaches_past_its_path() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::StrokePath {
            path: square(),
            color: Rgba::BLACK,
            style: StrokeStyle {
                width: 4.0,
                cap: izul_model::display::LineCap::Butt,
                join: izul_model::display::LineJoin::Miter,
                miter_limit: 1.0,
                dash: vec![],
                dash_phase: 0.0,
            },
            blend: BlendMode::Normal,
        });
        let b = painted_bounds(&dl, 0.0).unwrap();
        assert!(b.left <= 96.0 && b.top >= 204.0, "{b:?}");
    }

    #[test]
    fn a_rotation_is_covered_corner_to_corner() {
        // A 100-pt square turned 45 degrees about its centre is ~141 pt across;
        // its unrotated rect would clip all four corners.
        let centre = Matrix::translate(-150.0, -150.0)
            .then(&Matrix::rotate(std::f32::consts::FRAC_PI_4))
            .then(&Matrix::translate(150.0, 150.0));
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::PushTransform { matrix: centre });
        dl.push(DisplayOp::FillPath {
            path: square(),
            color: Rgba::BLACK,
            rule: FillRule::NonZero,
            blend: BlendMode::Normal,
        });
        dl.push(DisplayOp::PopTransform);
        let b = painted_bounds(&dl, 0.0).unwrap();
        assert!((b.width() - 141.42).abs() < 0.1, "{b:?}");
        assert!((b.left - 79.29).abs() < 0.1, "{b:?}");
    }

    #[test]
    fn nothing_painted_is_none() {
        assert_eq!(painted_bounds(&DisplayList::new(), 1.0), None);
    }
}
