//! PP-OCR (PaddleOCR) text detection and recognition, run by ONNX Runtime.
//!
//! The models are PP-OCRv6 small, as RapidOCR publishes them in ONNX
//! (Apache-2.0, see `vendor/ocr/fetch.sh`). Every number here — the 736 px
//! limit, 0.3 and 0.5 thresholds, unclip ratio 1.6, the 2×2 dilation, the
//! 48 px line height, the (x/255 − 0.5)/0.5 normalisation, CTC decoding with
//! the dictionary stored in the recognition model — is RapidOCR 3.9.2's, read
//! from its installed source (`ch_ppocr_det`, `ch_ppocr_rec`), because the
//! models were trained with exactly that and `tools/ocr-bakeoff` measured
//! them that way. What is not copied:
//!
//! - OpenCV. Contours become 8-connected components, `minAreaRect` rotating
//!   calipers over the components' hull, and the polygon offset of `unclip`
//!   the same offset applied to the rectangle (for a rectangle, a round-join
//!   offset's minimum-area rectangle is the rectangle grown by the distance on
//!   every side, which is all RapidOCR keeps of it). Resizing is plain
//!   bilinear with half-pixel centres, as `cv2.INTER_LINEAR`.
//! - The orientation classifier: the page is rendered upright, `/Rotate`
//!   applied, so a line upside down is the exception it is not worth a third
//!   model for.
//! - Word boxes are spread from the CTC columns the characters fired at, as
//!   RapidOCR's `return_word_box` does, but simpler: a word spans its first to
//!   last character's column, widened by half a character either side.

use std::path::Path;
use std::sync::Mutex;

use ort::session::Session;
use ort::value::Tensor;

use crate::{Line, OcrError, Word};

const DET_LIMIT: f32 = 736.0;
const DET_THRESH: f32 = 0.3;
const BOX_THRESH: f32 = 0.5;
const UNCLIP: f32 = 1.6;
const MIN_SIZE: f32 = 3.0;
const MAX_CANDIDATES: usize = 1000;
const LINE_Y_GAP: f32 = 10.0;
const REC_H: usize = 48;
const REC_MIN_W: usize = 320;
const REC_BATCH: usize = 6;
/// How much of a detected line's height its letters take, ascender to
/// descender: 12 pt type at 150 dpi (about 24 px of letters) is detected as
/// a box 36–37 px tall.
const LETTERS_IN_BOX: f32 = 0.65;
/// The gap left either side of a space's column, in characters.
const SPACE_KEPT: f32 = 0.25;

fn engine(e: impl std::fmt::Display) -> OcrError {
    OcrError::Engine(e.to_string())
}

/// A greyscale image as floats, rows top to bottom.
struct Gray<'a> {
    px: &'a [u8],
    w: usize,
    h: usize,
}

impl Gray<'_> {
    fn at(&self, x: isize, y: isize) -> f32 {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        f32::from(self.px.get(y * self.w + x).copied().unwrap_or(255))
    }

    /// Bilinear sample at pixel coordinates, border replicated.
    fn sample(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (xi, yi) = (x0 as isize, y0 as isize);
        let top = self.at(xi, yi) * (1.0 - fx) + self.at(xi + 1, yi) * fx;
        let bottom = self.at(xi, yi + 1) * (1.0 - fx) + self.at(xi + 1, yi + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// Resized to `w`×`h` and normalised to the models' (v/255 − 0.5)/0.5, the
/// single grey channel written three times (the models take RGB).
fn planar(src: &Gray<'_>, w: usize, h: usize, out: &mut Vec<f32>) {
    let (sx, sy) = (src.w as f32 / w as f32, src.h as f32 / h as f32);
    let start = out.len();
    for y in 0..h {
        let fy = (y as f32 + 0.5) * sy - 0.5;
        for x in 0..w {
            let fx = (x as f32 + 0.5) * sx - 0.5;
            out.push(src.sample(fx, fy) / 127.5 - 1.0);
        }
    }
    let plane = out.len() - start;
    out.extend_from_within(start..start + plane);
    out.extend_from_within(start..start + plane);
}

type P = (f32, f32);

/// A rectangle, corners clockwise from the top left (image space, y down).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Quad([P; 4]);

fn dist(a: P, b: P) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn cross(o: P, a: P, b: P) -> f32 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// Convex hull (Andrew's monotone chain), counter-clockwise in math terms.
// Indices are taken only after checking the length (`>= 2`).
#[allow(clippy::indexing_slicing)]
fn hull(mut pts: Vec<P>) -> Vec<P> {
    pts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let mut lower: Vec<P> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<P> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The minimum-area rectangle around `hull`: centre, side along the chosen
/// edge, side across it, and the edge's unit direction.
#[derive(Debug, Clone, Copy)]
struct MinRect {
    c: P,
    along: f32,
    across: f32,
    dir: P,
}

impl MinRect {
    // `i` and `(i + 1) % n` are below `n`, checked non-zero above.
    #[allow(clippy::indexing_slicing)]
    fn of(hull: &[P]) -> Option<MinRect> {
        let n = hull.len();
        if n == 0 {
            return None;
        }
        if n == 1 {
            return Some(MinRect {
                c: hull[0],
                along: 0.0,
                across: 0.0,
                dir: (1.0, 0.0),
            });
        }
        let mut best: Option<(f32, MinRect)> = None;
        for i in 0..n {
            let (a, b) = (hull[i], hull[(i + 1) % n]);
            let len = dist(a, b);
            if len <= 0.0 {
                continue;
            }
            let d = ((b.0 - a.0) / len, (b.1 - a.1) / len);
            let nrm = (-d.1, d.0);
            let (mut lo_u, mut hi_u, mut lo_v, mut hi_v) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
            for p in hull {
                let u = p.0 * d.0 + p.1 * d.1;
                let v = p.0 * nrm.0 + p.1 * nrm.1;
                lo_u = lo_u.min(u);
                hi_u = hi_u.max(u);
                lo_v = lo_v.min(v);
                hi_v = hi_v.max(v);
            }
            let area = (hi_u - lo_u) * (hi_v - lo_v);
            if best.as_ref().is_none_or(|(a0, _)| area < *a0) {
                let (cu, cv) = ((lo_u + hi_u) / 2.0, (lo_v + hi_v) / 2.0);
                let c = (cu * d.0 + cv * nrm.0, cu * d.1 + cv * nrm.1);
                best = Some((
                    area,
                    MinRect {
                        c,
                        along: hi_u - lo_u,
                        across: hi_v - lo_v,
                        dir: d,
                    },
                ));
            }
        }
        best.map(|(_, r)| r)
    }

    fn short_side(&self) -> f32 {
        self.along.min(self.across)
    }

    /// Grown by `d` on every side.
    fn grown(self, d: f32) -> MinRect {
        MinRect {
            along: self.along + 2.0 * d,
            across: self.across + 2.0 * d,
            ..self
        }
    }

    fn corners(&self) -> [P; 4] {
        let (d, n) = (self.dir, (-self.dir.1, self.dir.0));
        let (hu, hv) = (self.along / 2.0, self.across / 2.0);
        let at = |su: f32, sv: f32| {
            (
                self.c.0 + su * hu * d.0 + sv * hv * n.0,
                self.c.1 + su * hu * d.1 + sv * hv * n.1,
            )
        };
        [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)]
    }
}

/// Top left, top right, bottom right, bottom left, as RapidOCR's
/// `order_points_clockwise`: the two leftmost by y, the two rightmost by y.
fn order(mut pts: [P; 4]) -> Quad {
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let [a, b, c, d] = pts;
    let (tl, bl) = if a.1 <= b.1 { (a, b) } else { (b, a) };
    let (tr, br) = if c.1 <= d.1 { (c, d) } else { (d, c) };
    Quad([tl, tr, br, bl])
}

// `i` and `(i + 1) % 4` index a four-element array.
#[allow(clippy::indexing_slicing)]
fn inside(q: &[P; 4], p: P) -> bool {
    let signs: Vec<f32> = (0..4).map(|i| cross(q[i], q[(i + 1) % 4], p)).collect();
    signs.iter().all(|s| *s >= -1e-3) || signs.iter().all(|s| *s <= 1e-3)
}

/// Text lines found in the probability map `prob` (`w`×`h`), as quads in the
/// source image's pixels (`src_w`×`src_h`), in reading order.
fn boxes(prob: &[f32], w: usize, h: usize, src_w: usize, src_h: usize) -> Vec<Quad> {
    let p = |x: usize, y: usize| prob.get(y * w + x).copied().unwrap_or(0.0);
    // Threshold, then a 2×2 dilation (OpenCV's anchor: the pixel and its
    // left, upper and upper-left neighbours).
    let on = |x: usize, y: usize| p(x, y) > DET_THRESH;
    let mut mask = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let hit = on(x, y)
                || (x > 0 && on(x - 1, y))
                || (y > 0 && on(x, y - 1))
                || (x > 0 && y > 0 && on(x - 1, y - 1));
            if let Some(m) = mask.get_mut(y * w + x) {
                *m = hit;
            }
        }
    }
    // 8-connected components; of each, the leftmost and rightmost pixel of
    // every row is all the hull needs.
    let mut label = vec![0u32; w * h];
    let mut out: Vec<Quad> = Vec::new();
    let mut next = 0u32;
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut candidates = 0usize;
    for sy in 0..h {
        for sx in 0..w {
            let i = sy * w + sx;
            if !mask.get(i).copied().unwrap_or(false) || label.get(i).copied().unwrap_or(1) != 0 {
                continue;
            }
            next += 1;
            candidates += 1;
            let mut rows: std::collections::BTreeMap<usize, (usize, usize)> = Default::default();
            stack.push((sx, sy));
            if let Some(l) = label.get_mut(i) {
                *l = next;
            }
            while let Some((x, y)) = stack.pop() {
                let e = rows.entry(y).or_insert((x, x));
                e.0 = e.0.min(x);
                e.1 = e.1.max(x);
                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        let (nx, ny) = (x as isize + dx, y as isize + dy);
                        if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                            continue;
                        }
                        let j = ny as usize * w + nx as usize;
                        if mask.get(j).copied().unwrap_or(false) && label.get(j).copied() == Some(0)
                        {
                            if let Some(l) = label.get_mut(j) {
                                *l = next;
                            }
                            stack.push((nx as usize, ny as usize));
                        }
                    }
                }
            }
            if candidates > MAX_CANDIDATES {
                continue;
            }
            let pts: Vec<P> = rows
                .iter()
                .flat_map(|(&y, &(a, b))| [(a as f32, y as f32), (b as f32, y as f32)])
                .collect();
            let Some(rect) = MinRect::of(&hull(pts)) else {
                continue;
            };
            if rect.short_side() < MIN_SIZE {
                continue;
            }
            // Mean probability inside the rectangle ("fast" score).
            let c = rect.corners();
            let (x0, x1) = c
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), q| (a.min(q.0), b.max(q.0)));
            let (y0, y1) = c
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), q| (a.min(q.1), b.max(q.1)));
            let clampx = |v: f32| (v.max(0.0) as usize).min(w - 1);
            let clampy = |v: f32| (v.max(0.0) as usize).min(h - 1);
            let (mut sum, mut n) = (0.0f32, 0u32);
            for y in clampy(y0.floor())..=clampy(y1.ceil()) {
                for x in clampx(x0.floor())..=clampx(x1.ceil()) {
                    if inside(&c, (x as f32, y as f32)) {
                        sum += p(x, y);
                        n += 1;
                    }
                }
            }
            if n == 0 || sum / (n as f32) < BOX_THRESH {
                continue;
            }
            let distance = rect.along * rect.across * UNCLIP / (2.0 * (rect.along + rect.across));
            let grown = rect.grown(distance);
            if grown.short_side() < MIN_SIZE + 2.0 {
                continue;
            }
            let (kx, ky) = (src_w as f32 / w as f32, src_h as f32 / h as f32);
            let scaled = grown.corners().map(|(x, y)| {
                (
                    (x * kx).round().clamp(0.0, src_w as f32 - 1.0),
                    (y * ky).round().clamp(0.0, src_h as f32 - 1.0),
                )
            });
            let q = order(scaled);
            if dist(q.0[0], q.0[1]) as i32 <= 3 || dist(q.0[0], q.0[3]) as i32 <= 3 {
                continue;
            }
            out.push(q);
        }
    }
    // Reading order: by top-left y; boxes within 10 px of the previous one
    // are the same line, ordered by x.
    out.sort_by(|a, b| a.0[0].1.total_cmp(&b.0[0].1));
    let mut line = 0u32;
    let mut keyed: Vec<(u32, f32, Quad)> = Vec::with_capacity(out.len());
    let mut prev: Option<f32> = None;
    for q in out {
        if prev.is_some_and(|py| q.0[0].1 - py >= LINE_Y_GAP) {
            line += 1;
        }
        prev = Some(q.0[0].1);
        keyed.push((line, q.0[0].0, q));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    keyed.into_iter().map(|(_, _, q)| q).collect()
}

/// One detected line cut out straight: its pixels, and how a point of the
/// crop maps back to the page.
struct Crop {
    px: Vec<u8>,
    w: usize,
    h: usize,
    quad: Quad,
    /// The crop was turned a quarter (a line taller than wide).
    turned: bool,
}

impl Crop {
    fn cut(img: &Gray<'_>, quad: Quad) -> Option<Crop> {
        let [p0, p1, p2, p3] = quad.0;
        let w = dist(p0, p1).max(dist(p2, p3)) as usize;
        let h = dist(p0, p3).max(dist(p1, p2)) as usize;
        if w == 0 || h == 0 {
            return None;
        }
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            let t = y as f32 / h as f32;
            for x in 0..w {
                let s = x as f32 / w as f32;
                let (sx, sy) = bilerp(&quad.0, s, t);
                px.push(img.sample(sx, sy).round().clamp(0.0, 255.0) as u8);
            }
        }
        let mut crop = Crop {
            px,
            w,
            h,
            quad,
            turned: false,
        };
        if h as f32 / w as f32 >= 1.5 {
            // numpy's rot90: a quarter turn anticlockwise.
            let mut turned = Vec::with_capacity(w * h);
            for i in 0..w {
                for j in 0..h {
                    turned.push(crop.px.get(j * w + (w - 1 - i)).copied().unwrap_or(255));
                }
            }
            crop = Crop {
                px: turned,
                w: h,
                h: w,
                quad,
                turned: true,
            };
        }
        Some(crop)
    }

    /// The page box of the span `[a, b]` along the crop's reading direction,
    /// in crop pixels.
    ///
    /// For a horizontal line, an upright box as long as the word and as tall
    /// as the line's letters, centred where the word is. Two things it is
    /// not, both because the invisible font is sized from the box's height
    /// and extractors judge a gap between words against the font size — too
    /// tall a box and they stop seeing the spaces ("bulananinidisusun"):
    ///
    /// - the tilted span's bounding box: on a line tilted 6° (a phone photo)
    ///   that is several times taller than the text;
    /// - the detected box's full height: detection pads a line on purpose
    ///   (`UNCLIP`), so the letters are [`LETTERS_IN_BOX`] of it.
    ///
    /// Words are written upright even on a tilted line. Written along the
    /// slope instead, poppler found fewer whole words (77 % against 91 % on
    /// the phone-photo set) and MuPDF, pypdf and PDFium read the same.
    fn span(&self, a: f32, b: f32) -> [f32; 4] {
        let len = self.w as f32;
        let (s0, s1) = ((a / len).clamp(0.0, 1.0), (b / len).clamp(0.0, 1.0));
        let q = &self.quad.0;
        if self.turned {
            // Vertical text: the reading direction is the original crop's
            // top to bottom; an upright box around it is the best on offer.
            let pts = [(0.0, s0), (1.0, s0), (1.0, s1), (0.0, s1)].map(|(s, t)| bilerp(q, s, t));
            let (l, r) = pts
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
            let (t, bt) = pts
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
            return [l, t, r, bt];
        }
        let sm = (s0 + s1) / 2.0;
        let c = bilerp(q, sm, 0.5);
        let long = dist(bilerp(q, s0, 0.5), bilerp(q, s1, 0.5));
        let tall = dist(bilerp(q, sm, 0.0), bilerp(q, sm, 1.0)) * LETTERS_IN_BOX;
        [
            c.0 - long / 2.0,
            c.1 - tall / 2.0,
            c.0 + long / 2.0,
            c.1 + tall / 2.0,
        ]
    }
}

fn bilerp(q: &[P; 4], s: f32, t: f32) -> P {
    let [p0, p1, p2, p3] = *q;
    let x =
        (1.0 - s) * (1.0 - t) * p0.0 + s * (1.0 - t) * p1.0 + s * t * p2.0 + (1.0 - s) * t * p3.0;
    let y =
        (1.0 - s) * (1.0 - t) * p0.1 + s * (1.0 - t) * p1.1 + s * t * p2.1 + (1.0 - s) * t * p3.1;
    (x, y)
}

/// Characters the invisible text layer can carry: it is written in the
/// standard Helvetica (WinAnsi), so anything outside Latin-1 and the few
/// typographic marks WinAnsi adds would come out as the wrong character.
/// The recognition model knows 18,000 characters, mostly CJK; in Latin text
/// one of those is a misreading anyway.
fn writable(c: char) -> bool {
    matches!(c, ' '..='~' | '\u{a0}'..='\u{ff}') || "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ".contains(c)
}

/// The decoded text of one line, and per character the CTC column it fired at.
fn ctc(row: &[f32], steps: usize, classes: usize, dict: &[String]) -> Vec<(char, usize)> {
    let mut out = Vec::new();
    let mut prev = usize::MAX;
    for t in 0..steps {
        let probs = row.get(t * classes..(t + 1) * classes).unwrap_or_default();
        let best = probs
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(i, _)| i);
        if best != prev && best != 0 {
            if let Some(s) = dict.get(best) {
                for c in s.chars() {
                    out.push((c, t));
                }
            }
        }
        prev = best;
    }
    out
}

/// A run of non-space characters with their CTC columns, and the columns of
/// the spaces before and after it.
type Run = (Vec<(char, usize)>, Option<usize>, Option<usize>);

/// Words of one decoded line, with their boxes on the page.
///
/// A word spans its first to last character's column, widened by half a
/// character (the median gap between characters) either side — but never
/// closer than [`SPACE_KEPT`] of a character to the column the space before
/// or after it fired at. Without that limit neighbouring boxes overlap or
/// touch, and text extractors, which find word breaks from the gaps between
/// glyphs, run the words together ("inidisusun").
fn words(chars: &[(char, usize)], col_px: f32, crop: &Crop) -> Vec<Word> {
    let mut gaps: Vec<usize> = chars
        .windows(2)
        .filter_map(|p| match p {
            [(a, x), (b, y)] if !a.is_whitespace() && !b.is_whitespace() => {
                Some(y.saturating_sub(*x))
            }
            _ => None,
        })
        .collect();
    gaps.sort_unstable();
    let char_w = gaps.get(gaps.len() / 2).copied().unwrap_or(2).max(1) as f32 * col_px;
    // Runs of non-space characters, with the columns of the spaces around.
    let mut runs: Vec<Run> = Vec::new();
    let mut cur: Vec<(char, usize)> = Vec::new();
    let mut before: Option<usize> = None;
    for &(c, col) in chars {
        if c.is_whitespace() {
            if !cur.is_empty() {
                runs.push((std::mem::take(&mut cur), before, Some(col)));
            }
            before = Some(col);
        } else {
            cur.push((c, col));
        }
    }
    if !cur.is_empty() {
        runs.push((cur, before, None));
    }
    runs.into_iter()
        .filter_map(|(run, before, after)| {
            let text: String = run.iter().map(|c| c.0).filter(|c| writable(*c)).collect();
            let (first, last) = (run.first()?, run.last()?);
            if text.trim().is_empty() {
                return None;
            }
            let centre = |col: usize| (col as f32 + 0.5) * col_px;
            let mut a = centre(first.1) - char_w / 2.0;
            let mut b = centre(last.1) + char_w / 2.0;
            if let Some(s) = before {
                a = a.max(centre(s) + char_w * SPACE_KEPT);
            }
            if let Some(s) = after {
                b = b.min(centre(s) - char_w * SPACE_KEPT);
            }
            (b > a).then(|| Word {
                text,
                rect: crop.span(a, b),
            })
        })
        .collect()
}

pub struct Paddle {
    det: Mutex<Session>,
    rec: Mutex<Session>,
    /// Class index to text: 0 is the CTC blank, the last a space.
    dict: Vec<String>,
}

impl std::fmt::Debug for Paddle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Paddle")
            .field("classes", &self.dict.len())
            .finish()
    }
}

fn session(path: &Path) -> Result<Session, OcrError> {
    let missing = |detail: String| OcrError::Model {
        path: path.display().to_string(),
        detail,
    };
    if !path.is_file() {
        return Err(missing("model tidak ada".into()));
    }
    Session::builder()
        .map_err(|e| missing(e.to_string()))?
        .commit_from_file(path)
        .map_err(|e| missing(e.to_string()))
}

impl Paddle {
    /// Loads ONNX Runtime from `runtime` (once per process — see
    /// `background::BackgroundRemover::load`) and the two models.
    pub fn load(runtime: &Path, det: &Path, rec: &Path) -> Result<Self, OcrError> {
        if !runtime.is_file() {
            return Err(OcrError::Model {
                path: runtime.display().to_string(),
                detail: "ONNX Runtime tidak ada".into(),
            });
        }
        let _ = ort::init_from(runtime).map(|b| b.with_name("izul").commit());
        let det = session(det)?;
        let rec_session = session(rec)?;
        let chars = rec_session
            .metadata()
            .map_err(engine)?
            .custom("character")
            .ok_or_else(|| OcrError::Model {
                path: rec.display().to_string(),
                detail: "kamus karakter tidak ada di model".into(),
            })?;
        let mut dict = vec![String::new()];
        dict.extend(chars.split('\n').map(str::to_string));
        dict.push(" ".into());
        Ok(Paddle {
            det: Mutex::new(det),
            rec: Mutex::new(rec_session),
            dict,
        })
    }

    fn detect(&self, img: &Gray<'_>) -> Result<Vec<Quad>, OcrError> {
        let (w, h) = (img.w as f32, img.h as f32);
        let ratio = if w.min(h) < DET_LIMIT {
            DET_LIMIT / w.min(h)
        } else {
            1.0
        };
        let round32 =
            |v: f32| (((v * ratio).trunc() / 32.0).round_ties_even() * 32.0).max(32.0) as usize;
        let (rw, rh) = (round32(w), round32(h));
        let mut input = Vec::with_capacity(3 * rw * rh);
        planar(img, rw, rh, &mut input);
        let tensor = Tensor::from_array(([1usize, 3, rh, rw], input)).map_err(engine)?;
        let mut det = self
            .det
            .lock()
            .map_err(|_| OcrError::Engine("sesi deteksi rusak".into()))?;
        let outputs = det.run(ort::inputs![tensor]).map_err(engine)?;
        let first = outputs
            .values()
            .next()
            .ok_or_else(|| OcrError::Engine("model deteksi tidak mengeluarkan apa pun".into()))?;
        let (_, prob) = first.try_extract_tensor::<f32>().map_err(engine)?;
        if prob.len() != rw * rh {
            return Err(OcrError::Engine(
                "bentuk keluaran deteksi tak terduga".into(),
            ));
        }
        Ok(boxes(prob, rw, rh, img.w, img.h))
    }

    fn recognise(&self, crops: &[Crop]) -> Result<Vec<Vec<Word>>, OcrError> {
        let ratio = |c: &Crop| c.w as f32 / c.h as f32;
        let mut order: Vec<usize> = (0..crops.len()).collect();
        order.sort_by(|&a, &b| {
            let r = |i: usize| crops.get(i).map_or(0.0, ratio);
            r(a).total_cmp(&r(b))
        });
        let mut out: Vec<Vec<Word>> = vec![Vec::new(); crops.len()];
        let mut rec = self
            .rec
            .lock()
            .map_err(|_| OcrError::Engine("sesi pengenalan rusak".into()))?;
        for batch in order.chunks(REC_BATCH) {
            let max_ratio = batch
                .iter()
                .filter_map(|&i| crops.get(i))
                .map(ratio)
                .fold(REC_MIN_W as f32 / REC_H as f32, f32::max);
            let width = (REC_H as f32 * max_ratio) as usize;
            let mut input = Vec::with_capacity(batch.len() * 3 * REC_H * width);
            let mut resized = Vec::with_capacity(batch.len());
            for &i in batch {
                let Some(c) = crops.get(i) else { continue };
                let rw = ((REC_H as f32 * ratio(c)).ceil() as usize).clamp(1, width);
                let mut one = Vec::with_capacity(3 * REC_H * rw);
                planar(
                    &Gray {
                        px: &c.px,
                        w: c.w,
                        h: c.h,
                    },
                    rw,
                    REC_H,
                    &mut one,
                );
                for ch in 0..3 {
                    for y in 0..REC_H {
                        let row = one
                            .get((ch * REC_H + y) * rw..(ch * REC_H + y + 1) * rw)
                            .unwrap_or_default();
                        input.extend_from_slice(row);
                        input.extend(std::iter::repeat_n(0.0, width - rw));
                    }
                }
                resized.push(rw);
            }
            let tensor =
                Tensor::from_array(([batch.len(), 3, REC_H, width], input)).map_err(engine)?;
            let outputs = rec.run(ort::inputs![tensor]).map_err(engine)?;
            let first = outputs.values().next().ok_or_else(|| {
                OcrError::Engine("model pengenalan tidak mengeluarkan apa pun".into())
            })?;
            let (shape, data) = first.try_extract_tensor::<f32>().map_err(engine)?;
            let (steps, classes) = match shape.as_ref() {
                [n, t, c] if *n as usize == batch.len() => (*t as usize, *c as usize),
                _ => {
                    return Err(OcrError::Engine(format!(
                        "bentuk keluaran pengenalan tak terduga: {shape:?}"
                    )))
                }
            };
            for (k, (&i, &rw)) in batch.iter().zip(&resized).enumerate() {
                let Some(c) = crops.get(i) else { continue };
                let row = data
                    .get(k * steps * classes..(k + 1) * steps * classes)
                    .unwrap_or_default();
                let chars = ctc(row, steps, classes, &self.dict);
                // One CTC column in crop pixels.
                let col_px = width as f32 / steps as f32 * c.w as f32 / rw as f32;
                if let Some(slot) = out.get_mut(i) {
                    *slot = words(&chars, col_px, c);
                }
            }
        }
        Ok(out)
    }

    /// Reads a greyscale image (one byte per pixel, rows top to bottom).
    pub fn read_gray(&self, pixels: &[u8], width: u32, height: u32) -> Result<Vec<Line>, OcrError> {
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 || pixels.len() < w * h {
            return Err(OcrError::Image(format!(
                "{width}x{height} dengan {} byte",
                pixels.len()
            )));
        }
        let img = Gray { px: pixels, w, h };
        let crops: Vec<Crop> = self
            .detect(&img)?
            .into_iter()
            .filter_map(|q| Crop::cut(&img, q))
            .collect();
        Ok(self
            .recognise(&crops)?
            .into_iter()
            .filter(|ws| !ws.is_empty())
            .map(|words| Line { words })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_minimum_rectangle_of_a_tilted_bar_follows_the_tilt() {
        // A 100×10 bar turned 5°: an axis-aligned box would be 18 px tall.
        let (s, c) = 5f32.to_radians().sin_cos();
        let mut pts = Vec::new();
        for i in 0..=100 {
            for j in [0.0, 10.0] {
                let (x, y) = (i as f32, j);
                pts.push((x * c - y * s, x * s + y * c));
            }
        }
        let r = MinRect::of(&hull(pts)).unwrap_or(MinRect {
            c: (0.0, 0.0),
            along: 0.0,
            across: 0.0,
            dir: (1.0, 0.0),
        });
        let (long, short) = (r.along.max(r.across), r.short_side());
        assert!((long - 100.0).abs() < 0.5, "{long}");
        assert!((short - 10.0).abs() < 0.5, "{short}");
    }

    #[test]
    fn corners_are_ordered_from_the_top_left_clockwise() {
        let q = order([(10.0, 20.0), (0.0, 0.0), (10.0, 0.0), (0.0, 20.0)]);
        assert_eq!(q.0, [(0.0, 0.0), (10.0, 0.0), (10.0, 20.0), (0.0, 20.0)]);
    }

    #[test]
    fn ctc_drops_blanks_and_repeats_but_keeps_doubled_letters() {
        // Classes: blank, "a", "l", space. "a a _ l _ l" reads "all".
        let dict: Vec<String> = ["", "a", "l", " "].iter().map(|s| s.to_string()).collect();
        let one_hot = |k: usize| {
            (0..4)
                .map(|i| if i == k { 1.0 } else { 0.0 })
                .collect::<Vec<f32>>()
        };
        let row: Vec<f32> = [1, 1, 0, 2, 0, 2]
            .iter()
            .flat_map(|&k| one_hot(k))
            .collect();
        let text: String = ctc(&row, 6, 4, &dict).iter().map(|c| c.0).collect();
        assert_eq!(text, "all");
    }

    /// A straight 200×40 crop at the page's origin.
    fn straight_crop() -> Crop {
        Crop {
            px: Vec::new(),
            w: 200,
            h: 40,
            quad: Quad([(0.0, 0.0), (200.0, 0.0), (200.0, 40.0), (0.0, 40.0)]),
            turned: false,
        }
    }

    #[test]
    fn neighbouring_words_leave_a_gap_where_the_space_was() {
        // "ab cd": characters every 2 columns and the space firing right
        // after "b", as CTC does — so half a character either side reaches
        // the space from both words. Boxes that met there let poppler run
        // the words together.
        let chars = [('a', 0), ('b', 2), (' ', 3), ('c', 4), ('d', 6)];
        let ws = words(&chars, 4.0, &straight_crop());
        let [first, second] = ws.as_slice() else {
            return assert_eq!(ws.len(), 2, "{ws:?}");
        };
        assert_eq!((first.text.as_str(), second.text.as_str()), ("ab", "cd"));
        assert!(first.rect[2] < second.rect[0], "{first:?} {second:?}");
    }

    #[test]
    fn a_word_is_as_tall_as_the_letters_not_the_padded_line() {
        // Detection pads a line; a box that tall sizes the invisible font
        // too large, and extractors then miss the gaps between words.
        let chars = [('a', 0), ('b', 2)];
        let ws = words(&chars, 4.0, &straight_crop());
        let w = ws.first().map(|w| w.rect).unwrap_or_default();
        let tall = w[3] - w[1];
        assert!(tall > 20.0 && tall < 30.0, "{w:?}");
    }

    #[test]
    fn only_what_the_text_layer_can_write_is_kept() {
        assert!(writable('a') && writable('é') && writable('“') && writable('€'));
        assert!(!writable('中') && !writable('ı'));
    }
}
