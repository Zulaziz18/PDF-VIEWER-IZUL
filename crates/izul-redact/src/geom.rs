//! Geometry for deciding what lies inside a marked area.
//!
//! Everything is in the page's default user space (points, y up), which is the
//! space the areas arrive in. Content is mapped into it through the current
//! transformation matrix, so a glyph in a rotated form XObject is judged by
//! where it actually lands on the page.

/// A PDF matrix `[a b c d e f]`, applied to row vectors: `[x y 1] × M`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
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

    pub fn new(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Self {
        Matrix { a, b, c, d, e, f }
    }

    pub fn translate(x: f64, y: f64) -> Self {
        Matrix::new(1.0, 0.0, 0.0, 1.0, x, y)
    }

    /// `self × other`: first `self`, then `other`.
    pub fn then(&self, o: &Matrix) -> Matrix {
        Matrix {
            a: self.a * o.a + self.b * o.c,
            b: self.a * o.b + self.b * o.d,
            c: self.c * o.a + self.d * o.c,
            d: self.c * o.b + self.d * o.d,
            e: self.e * o.a + self.f * o.c + o.e,
            f: self.e * o.b + self.f * o.d + o.f,
        }
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            x * self.a + y * self.c + self.e,
            x * self.b + y * self.d + self.f,
        )
    }

    pub fn invert(&self) -> Option<Matrix> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let a = self.d / det;
        let b = -self.b / det;
        let c = -self.c / det;
        let d = self.a / det;
        Some(Matrix {
            a,
            b,
            c,
            d,
            e: -(self.e * a + self.f * c),
            f: -(self.e * b + self.f * d),
        })
    }

    /// The largest factor by which the matrix stretches a length; used to turn
    /// a line width into page units conservatively.
    pub fn max_scale(&self) -> f64 {
        let sx = (self.a * self.a + self.b * self.b).sqrt();
        let sy = (self.c * self.c + self.d * self.d).sqrt();
        sx.max(sy)
    }
}

/// An axis-aligned rectangle, `x0 < x1`, `y0 < y1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Rect {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        }
    }

    pub fn area(&self) -> f64 {
        (self.x1 - self.x0).max(0.0) * (self.y1 - self.y0).max(0.0)
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x > self.x0 && x < self.x1 && y > self.y0 && y < self.y1
    }

    pub fn contains_rect(&self, r: &Rect) -> bool {
        r.x0 >= self.x0 && r.x1 <= self.x1 && r.y0 >= self.y0 && r.y1 <= self.y1
    }

    /// Positive-area overlap; touching edges do not count.
    pub fn overlaps(&self, r: &Rect) -> bool {
        self.x0 < r.x1 && r.x0 < self.x1 && self.y0 < r.y1 && r.y0 < self.y1
    }

    pub fn inflate(&self, by: f64) -> Rect {
        Rect::new(self.x0 - by, self.y0 - by, self.x1 + by, self.y1 + by)
    }

    pub fn union(&self, r: &Rect) -> Rect {
        Rect {
            x0: self.x0.min(r.x0),
            y0: self.y0.min(r.y0),
            x1: self.x1.max(r.x1),
            y1: self.y1.max(r.y1),
        }
    }
}

/// A convex quadrilateral: a rectangle after a matrix.
pub type Quad = [(f64, f64); 4];

pub fn quad(r: &Rect, m: &Matrix) -> Quad {
    [
        m.apply(r.x0, r.y0),
        m.apply(r.x1, r.y0),
        m.apply(r.x1, r.y1),
        m.apply(r.x0, r.y1),
    ]
}

pub fn bounds(points: &[(f64, f64)]) -> Option<Rect> {
    let (&(x, y), rest) = points.split_first()?;
    let mut r = Rect {
        x0: x,
        y0: y,
        x1: x,
        y1: y,
    };
    for &(x, y) in rest {
        r.x0 = r.x0.min(x);
        r.y0 = r.y0.min(y);
        r.x1 = r.x1.max(x);
        r.y1 = r.y1.max(y);
    }
    Some(r)
}

/// Area of a polygon (shoelace), absolute.
pub fn polygon_area(p: &[(f64, f64)]) -> f64 {
    let n = p.len();
    if n < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    for i in 0..n {
        let (x0, y0) = p.get(i).copied().unwrap_or_default();
        let (x1, y1) = p.get((i + 1) % n).copied().unwrap_or_default();
        s += x0 * y1 - x1 * y0;
    }
    (s / 2.0).abs()
}

/// Clips a convex polygon to a rectangle (Sutherland–Hodgman).
pub fn clip_to_rect(poly: &[(f64, f64)], r: &Rect) -> Vec<(f64, f64)> {
    #[derive(Clone, Copy)]
    enum Edge {
        Left(f64),
        Right(f64),
        Bottom(f64),
        Top(f64),
    }
    fn inside(p: (f64, f64), e: Edge) -> bool {
        match e {
            Edge::Left(x) => p.0 >= x,
            Edge::Right(x) => p.0 <= x,
            Edge::Bottom(y) => p.1 >= y,
            Edge::Top(y) => p.1 <= y,
        }
    }
    fn cross(a: (f64, f64), b: (f64, f64), e: Edge) -> (f64, f64) {
        match e {
            Edge::Left(x) | Edge::Right(x) => {
                let t = if (b.0 - a.0).abs() < 1e-12 {
                    0.0
                } else {
                    (x - a.0) / (b.0 - a.0)
                };
                (x, a.1 + t * (b.1 - a.1))
            }
            Edge::Bottom(y) | Edge::Top(y) => {
                let t = if (b.1 - a.1).abs() < 1e-12 {
                    0.0
                } else {
                    (y - a.1) / (b.1 - a.1)
                };
                (a.0 + t * (b.0 - a.0), y)
            }
        }
    }
    let mut out = poly.to_vec();
    for e in [
        Edge::Left(r.x0),
        Edge::Right(r.x1),
        Edge::Bottom(r.y0),
        Edge::Top(r.y1),
    ] {
        let input = std::mem::take(&mut out);
        let Some(&last) = input.last() else { break };
        let mut s = last;
        for &p in &input {
            if inside(p, e) {
                if !inside(s, e) {
                    out.push(cross(s, p, e));
                }
                out.push(p);
            } else if inside(s, e) {
                out.push(cross(s, p, e));
            }
            s = p;
        }
    }
    out
}

/// How much of `q` lies inside `r`, as a fraction of `q`'s own area. A
/// degenerate quad (zero area) gives 0.
pub fn covered_fraction(q: &Quad, r: &Rect) -> f64 {
    let whole = polygon_area(q);
    if whole <= 1e-9 {
        return 0.0;
    }
    polygon_area(&clip_to_rect(q, r)) / whole
}

pub fn centre(q: &Quad) -> (f64, f64) {
    let (sx, sy) = q
        .iter()
        .fold((0.0, 0.0), |(ax, ay), &(x, y)| (ax + x, ay + y));
    (sx / 4.0, sy / 4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_compose_in_pdf_order() {
        // Scale by 2, then move right by 10.
        let m = Matrix::new(2.0, 0.0, 0.0, 2.0, 0.0, 0.0).then(&Matrix::translate(10.0, 0.0));
        assert_eq!(m.apply(1.0, 1.0), (12.0, 2.0));
        let inv = m.invert().unwrap();
        let (x, y) = inv.apply(12.0, 2.0);
        assert!((x - 1.0).abs() < 1e-9 && (y - 1.0).abs() < 1e-9);
    }

    #[test]
    fn coverage_of_a_rotated_square() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        // The square itself: fully covered.
        assert!((covered_fraction(&quad(&r, &Matrix::IDENTITY), &r) - 1.0).abs() < 1e-9);
        // Half of it moved out to the right.
        let moved = quad(&r, &Matrix::translate(5.0, 0.0));
        assert!((covered_fraction(&moved, &r) - 0.5).abs() < 1e-9);
        // Rotated 45° about its centre: the corners stick out.
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let rot = Matrix::translate(-5.0, -5.0)
            .then(&Matrix::new(s, s, -s, s, 0.0, 0.0))
            .then(&Matrix::translate(5.0, 5.0));
        let f = covered_fraction(&quad(&r, &rot), &r);
        assert!(f > 0.7 && f < 0.9, "{f}");
        // Far away: nothing.
        assert_eq!(
            covered_fraction(&quad(&r, &Matrix::translate(100.0, 0.0)), &r),
            0.0
        );
    }

    #[test]
    fn touching_is_not_overlapping() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(!a.overlaps(&Rect::new(10.0, 0.0, 20.0, 10.0)));
        assert!(a.overlaps(&Rect::new(9.9, 0.0, 20.0, 10.0)));
    }
}
