//! Geometry in PDF user space.
//!
//! SPEC 8 is categorical: coordinates are points with the origin at the
//! bottom-left of the page and y growing upward, and the conversion to pixels
//! happens only in the viewport layer. Storing pixels would break every
//! annotation the moment the zoom level changed.
//!
//! Everything here is pure arithmetic with no dependencies, so it is shared by
//! the model, the PDFium layer, and the IPC layer without dragging anything
//! along with it.

use serde::{Deserialize, Serialize};

/// A point in PDF user space.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PdfPointF {
    pub x: f32,
    pub y: f32,
}

impl PdfPointF {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle in PDF user space.
///
/// `top` is always the larger y, matching PDFium's `FS_RECTF` and the PDF
/// specification itself. A rectangle whose `top < bottom` is invalid rather than
/// merely inverted, and [`PdfRectF::is_valid`] is checked before any such rect
/// crosses into PDFium.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PdfRectF {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

impl PdfRectF {
    pub const fn new(left: f32, bottom: f32, right: f32, top: f32) -> Self {
        Self {
            left,
            bottom,
            right,
            top,
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.top - self.bottom
    }

    pub fn is_valid(&self) -> bool {
        self.right > self.left
            && self.top > self.bottom
            && [self.left, self.bottom, self.right, self.top]
                .iter()
                .all(|v| v.is_finite())
    }

    pub fn contains(&self, p: PdfPointF) -> bool {
        p.x >= self.left && p.x <= self.right && p.y >= self.bottom && p.y <= self.top
    }

    pub fn intersects(&self, other: &Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.bottom < other.top
            && other.bottom < self.top
    }

    /// Smallest rectangle containing both.
    pub fn union(&self, other: &Self) -> Self {
        Self {
            left: self.left.min(other.left),
            bottom: self.bottom.min(other.bottom),
            right: self.right.max(other.right),
            top: self.top.max(other.top),
        }
    }

    /// Grows the rectangle by `d` on every side. Used for hit-testing tolerance
    /// and for the dirty rect around a stroked path.
    pub fn inflate(&self, d: f32) -> Self {
        Self {
            left: self.left - d,
            bottom: self.bottom - d,
            right: self.right + d,
            top: self.top + d,
        }
    }
}

/// A 2D affine transform, in the order PDF writes them: `[a b c d e f]`.
///
/// Maps `(x, y)` to `(a·x + c·y + e, b·x + d·y + f)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Matrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Default for Matrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub const fn translate(tx: f32, ty: f32) -> Self {
        Matrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    pub const fn scale(sx: f32, sy: f32) -> Self {
        Matrix {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn rotate(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Matrix {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn apply(&self, p: PdfPointF) -> PdfPointF {
        PdfPointF {
            x: self.a * p.x + self.c * p.y + self.e,
            y: self.b * p.x + self.d * p.y + self.f,
        }
    }

    /// `self` then `next`. Reads left to right in application order, which is
    /// the opposite of the usual matrix-product notation and is chosen because
    /// every call site here builds transforms by describing what happens first.
    pub fn then(&self, next: &Matrix) -> Matrix {
        Matrix {
            a: self.a * next.a + self.b * next.c,
            b: self.a * next.b + self.b * next.d,
            c: self.c * next.a + self.d * next.c,
            d: self.c * next.b + self.d * next.d,
            e: self.e * next.a + self.f * next.c + next.e,
            f: self.e * next.b + self.f * next.d + next.f,
        }
    }

    pub fn determinant(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    pub fn invert(&self) -> Option<Matrix> {
        let det = self.determinant();
        if det.abs() < f32::EPSILON {
            return None;
        }
        let inv = 1.0 / det;
        Some(Matrix {
            a: self.d * inv,
            b: -self.b * inv,
            c: -self.c * inv,
            d: self.a * inv,
            e: (self.c * self.f - self.d * self.e) * inv,
            f: (self.b * self.e - self.a * self.f) * inv,
        })
    }
}

/// Quarter-turn rotation, as PDF and PDFium both count it: clockwise, in units
/// of 90 degrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum RotationQuarter {
    #[default]
    None = 0,
    Cw90 = 1,
    Cw180 = 2,
    Cw270 = 3,
}

impl RotationQuarter {
    pub fn from_degrees(deg: i32) -> Self {
        match deg.rem_euclid(360) / 90 {
            1 => Self::Cw90,
            2 => Self::Cw180,
            3 => Self::Cw270,
            _ => Self::None,
        }
    }

    pub fn degrees(self) -> i32 {
        self as i32 * 90
    }

    /// True when the rotation swaps the page's width and height.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Self::Cw90 | Self::Cw270)
    }

    pub fn plus(self, other: Self) -> Self {
        Self::from_degrees(self.degrees() + other.degrees())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_leaves_points_alone() {
        let p = PdfPointF::new(3.5, -7.25);
        assert_eq!(Matrix::IDENTITY.apply(p), p);
    }

    #[test]
    fn then_applies_left_to_right() {
        // Scale by 2, then move right by 10: (1,1) -> (2,2) -> (12,2).
        let m = Matrix::scale(2.0, 2.0).then(&Matrix::translate(10.0, 0.0));
        let out = m.apply(PdfPointF::new(1.0, 1.0));
        assert!((out.x - 12.0).abs() < 1e-6, "x = {}", out.x);
        assert!((out.y - 2.0).abs() < 1e-6, "y = {}", out.y);
    }

    #[test]
    fn then_is_not_commutative() {
        let a = Matrix::scale(2.0, 2.0).then(&Matrix::translate(10.0, 0.0));
        let b = Matrix::translate(10.0, 0.0).then(&Matrix::scale(2.0, 2.0));
        assert_ne!(
            a.apply(PdfPointF::new(1.0, 1.0)),
            b.apply(PdfPointF::new(1.0, 1.0))
        );
    }

    #[test]
    fn inverse_round_trips() {
        let m = Matrix::scale(3.0, -2.0)
            .then(&Matrix::rotate(0.7))
            .then(&Matrix::translate(5.0, 9.0));
        let inv = m.invert().expect("non-degenerate");
        let p = PdfPointF::new(11.0, -4.0);
        let back = inv.apply(m.apply(p));
        assert!((back.x - p.x).abs() < 1e-3, "x {} vs {}", back.x, p.x);
        assert!((back.y - p.y).abs() < 1e-3, "y {} vs {}", back.y, p.y);
    }

    #[test]
    fn degenerate_matrix_has_no_inverse() {
        assert!(Matrix::scale(0.0, 1.0).invert().is_none());
    }

    #[test]
    fn rect_validity_rejects_inverted_and_non_finite() {
        assert!(PdfRectF::new(0.0, 0.0, 10.0, 10.0).is_valid());
        assert!(!PdfRectF::new(10.0, 0.0, 0.0, 10.0).is_valid());
        assert!(!PdfRectF::new(0.0, 10.0, 10.0, 0.0).is_valid());
        assert!(!PdfRectF::new(0.0, 0.0, f32::NAN, 10.0).is_valid());
        assert!(!PdfRectF::new(0.0, 0.0, f32::INFINITY, 10.0).is_valid());
    }

    #[test]
    fn union_covers_both_inputs() {
        let a = PdfRectF::new(0.0, 0.0, 10.0, 10.0);
        let b = PdfRectF::new(5.0, -5.0, 20.0, 8.0);
        let u = a.union(&b);
        assert_eq!(u, PdfRectF::new(0.0, -5.0, 20.0, 10.0));
    }

    #[test]
    fn touching_rects_do_not_intersect() {
        let a = PdfRectF::new(0.0, 0.0, 10.0, 10.0);
        let b = PdfRectF::new(10.0, 0.0, 20.0, 10.0);
        assert!(!a.intersects(&b));
    }

    #[test]
    fn rotation_wraps_and_reports_axis_swap() {
        assert_eq!(RotationQuarter::from_degrees(450), RotationQuarter::Cw90);
        assert_eq!(RotationQuarter::from_degrees(-90), RotationQuarter::Cw270);
        assert!(RotationQuarter::Cw90.swaps_axes());
        assert!(!RotationQuarter::Cw180.swaps_axes());
        assert_eq!(
            RotationQuarter::Cw270.plus(RotationQuarter::Cw180),
            RotationQuarter::Cw90
        );
    }
}
