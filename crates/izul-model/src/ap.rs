//! The appearance-stream backend (SPEC 3.2).
//!
//! Turns a [`DisplayList`] into the PDF operators that go into an annotation's
//! `/AP` form XObject. It is the second of the two consumers of that list — the
//! canvas backend in the frontend is the first — and it is deliberately dumb:
//! it translates, it never decides. Every curve, every arrow head, every glyph
//! position was settled in `build`, so there is nothing here for the two
//! backends to disagree about.
//!
//! Two properties are load-bearing and both are tested:
//!
//! * **Deterministic bytes.** The same list produces the same stream, to the
//!   byte. Golden-image baselines and content hashes both depend on it, and
//!   float formatting is where determinism usually dies — so numbers go through
//!   one formatter with a fixed precision rather than `{}` at each call site.
//! * **Balanced state.** Every `q` has its `Q`. A stream that leaks a clip or a
//!   transform is still valid PDF, and it corrupts whatever the reader draws
//!   next — which, in a page full of annotations, is somebody else's.
//!
//! What this module does *not* do is write the PDF file. Building objects,
//! embedding fonts and streams is Phase 4's job in the crate that is allowed to
//! touch documents; here we produce the stream and say which resources it needs.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::display::{
    BlendMode, DisplayList, DisplayOp, FillRule, FontRef, ImageRef, LineCap, LineJoin, Path,
    PathSeg, Rgba, StrokeStyle,
};
use crate::geom::{Matrix, PdfRectF};

/// One `/ExtGState` the stream refers to.
///
/// Alpha and blend mode are graphics-state parameters in PDF, not part of a
/// colour, which is exactly why the display list keeps them separate: baking a
/// 40 % alpha into a yellow would give a *different* yellow over white and over
/// text, and the canvas would not make the same mistake.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GState {
    pub fill_alpha: f32,
    pub stroke_alpha: f32,
    pub blend: BlendMode,
}

/// What the stream needs the surrounding PDF to provide.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resources {
    /// `/Fn` name -> font handle.
    pub fonts: BTreeMap<String, FontRef>,
    /// `/Imn` name -> image handle.
    pub images: BTreeMap<String, ImageRef>,
    /// `/GSn` name -> graphics state.
    pub states: Vec<(String, GState)>,
}

/// A finished appearance stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    /// The content stream, ready to be wrapped in a form XObject.
    pub content: String,
    /// `/BBox` — the box the form declares. Callers pass the object's rect.
    pub bbox: PdfRectF,
    pub resources: Resources,
}

/// Decimal places every number in the stream is written with.
///
/// Three is about a thousandth of a point: far below anything a reader can see
/// at 1600 % zoom, and short enough that streams stay small. Fixing it is what
/// makes the output byte-stable across machines and Rust versions.
const PRECISION: usize = 3;

/// Formats a number the one way this module ever formats numbers.
///
/// Trailing zeros are trimmed so `1.0` is `1`, and negative zero is written as
/// `0`: some readers tolerate `-0` and some produce a different result with it,
/// and neither is worth finding out about in a file a user has saved.
fn num(v: f32) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let mut s = format!("{v:.PRECISION$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" || s.is_empty() {
        s = "0".into();
    }
    s
}

/// Escapes a string for a PDF literal string.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for ch in text.chars() {
        match ch {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 32 || (c as u32) > 126 => {
                // Octal, which is the only escape a PDF literal string has for
                // arbitrary bytes. Characters outside Latin-1 cannot be written
                // in a simple font at all; they are dropped here and refused
                // much earlier, in `build`, where the message can name them.
                let code = c as u32;
                if code < 256 {
                    let _ = write!(out, "\\{code:03o}");
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// Builds the appearance stream for one display list.
pub fn appearance(list: &DisplayList, bbox: PdfRectF) -> Appearance {
    let mut w = Writer::default();
    // One outer q/Q: the form's own state must not leak into the page, and a
    // reader that draws this next to another annotation is entitled to assume
    // it did not.
    w.line("q");
    for op in &list.ops {
        w.op(op);
    }
    // Anything the list left open is closed here rather than written out
    // unbalanced. `build` guarantees balance and `DisplayList::is_balanced`
    // checks it, but a stream that silently loses a `Q` corrupts the page it is
    // drawn on, so this is the one place that does not trust its input.
    while w.depth > 0 {
        w.line("Q");
        w.depth -= 1;
    }
    w.line("Q");
    Appearance {
        content: w.out,
        bbox,
        resources: w.resources,
    }
}

#[derive(Default)]
struct Writer {
    out: String,
    resources: Resources,
    depth: u32,
}

impl Writer {
    fn line(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }

    /// Names the `/ExtGState` for these parameters, reusing one if an identical
    /// state is already in the resource table.
    fn gstate(&mut self, fill_alpha: f32, stroke_alpha: f32, blend: BlendMode) -> Option<String> {
        if (fill_alpha - 1.0).abs() < 1e-6
            && (stroke_alpha - 1.0).abs() < 1e-6
            && blend == BlendMode::Normal
        {
            return None;
        }
        let state = GState {
            fill_alpha,
            stroke_alpha,
            blend,
        };
        if let Some((name, _)) = self.resources.states.iter().find(|(_, s)| *s == state) {
            return Some(name.clone());
        }
        let name = format!("GS{}", self.resources.states.len());
        self.resources.states.push((name.clone(), state));
        Some(name)
    }

    fn font_name(&mut self, font: FontRef) -> String {
        if let Some((name, _)) = self.resources.fonts.iter().find(|(_, f)| **f == font) {
            return name.clone();
        }
        let name = format!("F{}", self.resources.fonts.len());
        self.resources.fonts.insert(name.clone(), font);
        name
    }

    fn image_name(&mut self, image: ImageRef) -> String {
        if let Some((name, _)) = self.resources.images.iter().find(|(_, i)| **i == image) {
            return name.clone();
        }
        let name = format!("Im{}", self.resources.images.len());
        self.resources.images.insert(name.clone(), image);
        name
    }

    fn path(&mut self, path: &Path) {
        for seg in &path.segs {
            match seg {
                PathSeg::MoveTo(p) => {
                    let s = format!("{} {} m", num(p.x), num(p.y));
                    self.line(&s);
                }
                PathSeg::LineTo(p) => {
                    let s = format!("{} {} l", num(p.x), num(p.y));
                    self.line(&s);
                }
                PathSeg::CurveTo { c1, c2, to } => {
                    let s = format!(
                        "{} {} {} {} {} {} c",
                        num(c1.x),
                        num(c1.y),
                        num(c2.x),
                        num(c2.y),
                        num(to.x),
                        num(to.y)
                    );
                    self.line(&s);
                }
                PathSeg::Close => self.line("h"),
            }
        }
    }

    fn matrix(&mut self, m: &Matrix, operator: &str) {
        let s = format!(
            "{} {} {} {} {} {} {operator}",
            num(m.a),
            num(m.b),
            num(m.c),
            num(m.d),
            num(m.e),
            num(m.f)
        );
        self.line(&s);
    }

    fn stroke_style(&mut self, style: &StrokeStyle) {
        let s = format!("{} w", num(style.width));
        self.line(&s);
        let cap = match style.cap {
            LineCap::Butt => 0,
            LineCap::Round => 1,
            LineCap::Square => 2,
        };
        let join = match style.join {
            LineJoin::Miter => 0,
            LineJoin::Round => 1,
            LineJoin::Bevel => 2,
        };
        self.line(&format!("{cap} J"));
        self.line(&format!("{join} j"));
        if style.miter_limit > 0.0 {
            let s = format!("{} M", num(style.miter_limit));
            self.line(&s);
        }
        let dashes: Vec<String> = style.dash.iter().map(|d| num(*d)).collect();
        let s = format!("[{}] {} d", dashes.join(" "), num(style.dash_phase));
        self.line(&s);
    }

    fn op(&mut self, op: &DisplayOp) {
        match op {
            DisplayOp::FillPath {
                path,
                color,
                rule,
                blend,
            } => {
                self.line("q");
                if let Some(gs) = self.gstate(color.a, 1.0, *blend) {
                    self.line(&format!("/{gs} gs"));
                }
                self.fill_color(*color);
                self.path(path);
                self.line(match rule {
                    FillRule::NonZero => "f",
                    FillRule::EvenOdd => "f*",
                });
                self.line("Q");
            }
            DisplayOp::StrokePath {
                path,
                color,
                style,
                blend,
            } => {
                self.line("q");
                if let Some(gs) = self.gstate(1.0, color.a, *blend) {
                    self.line(&format!("/{gs} gs"));
                }
                self.stroke_color(*color);
                self.stroke_style(style);
                self.path(path);
                self.line("S");
                self.line("Q");
            }
            DisplayOp::DrawText {
                glyphs,
                font,
                size,
                matrix,
                color,
                blend,
            } => {
                self.line("q");
                if let Some(gs) = self.gstate(color.a, color.a, *blend) {
                    self.line(&format!("/{gs} gs"));
                }
                self.fill_color(*color);
                self.line("BT");
                let name = self.font_name(*font);
                let s = format!("/{name} {} Tf", num(*size));
                self.line(&s);
                self.matrix(matrix, "Tm");
                // One `Tj` per glyph, each placed by its own offset.
                //
                // The alternative — one string plus `TJ` adjustments — is
                // shorter, but it makes the position of every glyph depend on
                // the widths the *reader* has for the font. When those differ
                // from the ones the layout used, the run drifts. Placing each
                // glyph explicitly is the only form that cannot drift, and text
                // annotations are short enough that the extra bytes do not
                // matter.
                for g in glyphs {
                    let placed = Matrix::translate(g.offset.x, g.offset.y).then(matrix);
                    self.matrix(&placed, "Tm");
                    let s = format!("({}) Tj", escape(&g.unicode.to_string()));
                    self.line(&s);
                }
                self.line("ET");
                self.line("Q");
            }
            DisplayOp::DrawImage {
                image,
                matrix,
                opacity,
            } => {
                self.line("q");
                if let Some(gs) = self.gstate(*opacity, *opacity, BlendMode::Normal) {
                    self.line(&format!("/{gs} gs"));
                }
                self.matrix(matrix, "cm");
                let name = self.image_name(*image);
                self.line(&format!("/{name} Do"));
                self.line("Q");
            }
            DisplayOp::PushClip { path, rule } => {
                self.line("q");
                self.depth += 1;
                self.path(path);
                self.line(match rule {
                    FillRule::NonZero => "W n",
                    FillRule::EvenOdd => "W* n",
                });
            }
            DisplayOp::PopClip | DisplayOp::PopTransform => {
                if self.depth > 0 {
                    self.depth -= 1;
                    self.line("Q");
                }
            }
            DisplayOp::PushTransform { matrix } => {
                self.line("q");
                self.depth += 1;
                self.matrix(matrix, "cm");
            }
        }
    }

    fn fill_color(&mut self, c: Rgba) {
        let s = format!("{} {} {} rg", num(c.r), num(c.g), num(c.b));
        self.line(&s);
    }

    fn stroke_color(&mut self, c: Rgba) {
        let s = format!("{} {} {} RG", num(c.r), num(c.g), num(c.b));
        self.line(&s);
    }
}

/// How many `q` are unmatched by a `Q` in a stream. Zero is the only correct
/// answer; exported so the golden harness can assert it on real output.
pub fn unbalanced_depth(content: &str) -> i32 {
    let mut depth = 0;
    for token in content.split_whitespace() {
        match token {
            "q" => depth += 1,
            "Q" => depth -= 1,
            _ => {}
        }
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annot::{AnnotId, AnnotKind, AnnotObject, AnnotPayload, ShapeStyle};
    use crate::build::display_list;
    use crate::display::{FillRule, Path, PositionedGlyph};
    use crate::font::FixedFont;
    use crate::geom::PdfPointF;

    fn bbox() -> PdfRectF {
        PdfRectF::new(0.0, 0.0, 200.0, 100.0)
    }

    #[test]
    fn numbers_are_written_one_way_only() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(0.123456), "0.123");
        assert_eq!(num(-0.0), "0", "-0 membingungkan sebagian pembaca");
        assert_eq!(num(f32::NAN), "0");
        assert_eq!(num(f32::INFINITY), "0");
        assert_eq!(num(100.0), "100");
    }

    #[test]
    fn strings_are_escaped_so_the_stream_cannot_break() {
        assert_eq!(escape("halo"), "halo");
        assert_eq!(escape("(a)"), "\\(a\\)");
        assert_eq!(escape("c:\\x"), "c:\\\\x");
        assert_eq!(escape("é"), "\\351");
    }

    /// Golden baselines and content hashes both rest on this.
    #[test]
    fn the_same_list_produces_the_same_bytes() {
        let fonts = FixedFont::default();
        let obj = AnnotObject::new(
            AnnotId(1),
            0,
            AnnotKind::Ellipse,
            bbox(),
            AnnotPayload::Shape {
                style: ShapeStyle {
                    fill: Some(Rgba::from_rgb8(10, 20, 30, 0.5)),
                    ..Default::default()
                },
            },
        );
        let dl = display_list(&obj, &fonts).expect("list");
        let a = appearance(&dl, bbox());
        let b = appearance(&dl, bbox());
        assert_eq!(a.content, b.content);
        assert!(!a.content.is_empty());
    }

    /// A stream that leaks a `q` corrupts whatever the reader draws next, which
    /// in a page of annotations is somebody else's.
    #[test]
    fn every_kind_produces_a_balanced_stream() {
        let fonts = FixedFont::default();
        for kind in AnnotKind::ALL {
            let payload = match kind {
                AnnotKind::Highlight | AnnotKind::Underline | AnnotKind::StrikeOut => {
                    AnnotPayload::Markup {
                        quads: vec![PdfRectF::new(10.0, 10.0, 90.0, 24.0)],
                        color: Rgba::BLACK,
                    }
                }
                AnnotKind::FreeText => AnnotPayload::FreeText {
                    text: "halo (dunia)".into(),
                    font: Default::default(),
                    color: Rgba::BLACK,
                    align: Default::default(),
                    line_spacing: 1.2,
                    background: Some(Rgba::from_rgb8(255, 255, 200, 1.0)),
                    border: Some(Rgba::BLACK),
                },
                AnnotKind::Image => AnnotPayload::Image {
                    image: ImageRef(1),
                    crop: PdfRectF::new(0.0, 0.0, 1.0, 1.0),
                    opacity: 0.8,
                },
                AnnotKind::Ink => AnnotPayload::Ink {
                    strokes: vec![vec![
                        PdfPointF::new(10.0, 10.0),
                        PdfPointF::new(50.0, 60.0),
                        PdfPointF::new(90.0, 20.0),
                    ]],
                    color: Rgba::BLACK,
                    width: 2.0,
                    smooth: true,
                },
                AnnotKind::Line | AnnotKind::Arrow => AnnotPayload::Line {
                    from: PdfPointF::new(10.0, 10.0),
                    to: PdfPointF::new(180.0, 80.0),
                    color: Rgba::BLACK,
                    width: 2.0,
                    dashed: true,
                    arrow_head: if kind == AnnotKind::Arrow { 10.0 } else { 0.0 },
                },
                AnnotKind::Rect | AnnotKind::Ellipse => AnnotPayload::Shape {
                    style: ShapeStyle::default(),
                },
                AnnotKind::Polygon => AnnotPayload::Polygon {
                    points: vec![
                        PdfPointF::new(10.0, 10.0),
                        PdfPointF::new(100.0, 90.0),
                        PdfPointF::new(190.0, 20.0),
                    ],
                    closed: true,
                    style: ShapeStyle::default(),
                },
                AnnotKind::Note => AnnotPayload::Note {
                    icon: crate::annot::NoteIcon::Help,
                    color: Rgba::from_rgb8(255, 200, 0, 1.0),
                    text: String::new(),
                },
                AnnotKind::Stamp => AnnotPayload::Stamp {
                    label: "OK".into(),
                    color: Rgba::BLACK,
                    font: Default::default(),
                },
            };
            let mut obj = AnnotObject::new(AnnotId(1), 0, kind, bbox(), payload);
            obj.rotation = 15.0;
            obj.opacity = 0.75;
            let dl = display_list(&obj, &fonts).expect("list");
            let ap = appearance(&dl, bbox());
            assert_eq!(
                unbalanced_depth(&ap.content),
                0,
                "{kind:?} meninggalkan q tanpa Q:\n{}",
                ap.content
            );
        }
    }

    #[test]
    fn alpha_becomes_an_ext_gstate_not_a_different_colour() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::FillPath {
            path: Path::rect(bbox()),
            color: Rgba::from_rgb8(255, 0, 0, 0.4),
            rule: FillRule::NonZero,
            blend: BlendMode::Multiply,
        });
        let ap = appearance(&dl, bbox());
        assert!(ap.content.contains("/GS0 gs"), "{}", ap.content);
        assert!(
            ap.content.contains("1 0 0 rg"),
            "warnanya tetap merah penuh"
        );
        let (_, state) = ap.resources.states.first().expect("ada gstate");
        assert_eq!(state.fill_alpha, 0.4);
        assert_eq!(state.blend, BlendMode::Multiply);
    }

    #[test]
    fn identical_states_are_shared_rather_than_repeated() {
        let mut dl = DisplayList::new();
        for _ in 0..3 {
            dl.push(DisplayOp::FillPath {
                path: Path::rect(bbox()),
                color: Rgba::from_rgb8(0, 0, 0, 0.5),
                rule: FillRule::NonZero,
                blend: BlendMode::Normal,
            });
        }
        let ap = appearance(&dl, bbox());
        assert_eq!(ap.resources.states.len(), 1);
    }

    #[test]
    fn a_fully_opaque_normal_paint_needs_no_gstate_at_all() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::FillPath {
            path: Path::rect(bbox()),
            color: Rgba::BLACK,
            rule: FillRule::NonZero,
            blend: BlendMode::Normal,
        });
        let ap = appearance(&dl, bbox());
        assert!(ap.resources.states.is_empty());
        assert!(!ap.content.contains(" gs"), "{}", ap.content);
    }

    #[test]
    fn each_glyph_is_placed_by_its_own_matrix() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::DrawText {
            glyphs: vec![
                PositionedGlyph {
                    glyph_id: 1,
                    unicode: 'a',
                    offset: PdfPointF::new(0.0, 0.0),
                },
                PositionedGlyph {
                    glyph_id: 2,
                    unicode: 'b',
                    offset: PdfPointF::new(6.0, 0.0),
                },
            ],
            font: FontRef(1),
            size: 12.0,
            matrix: Matrix::translate(10.0, 20.0),
            color: Rgba::BLACK,
            blend: BlendMode::Normal,
        });
        let ap = appearance(&dl, bbox());
        assert!(ap.content.contains("1 0 0 1 10 20 Tm"), "{}", ap.content);
        assert!(ap.content.contains("1 0 0 1 16 20 Tm"), "{}", ap.content);
        assert!(ap.content.contains("(a) Tj"));
        assert_eq!(ap.resources.fonts.len(), 1);
    }

    #[test]
    fn a_clip_opens_and_closes_around_what_it_clips() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::PushClip {
            path: Path::rect(bbox()),
            rule: FillRule::EvenOdd,
        });
        dl.push(DisplayOp::DrawImage {
            image: ImageRef(3),
            matrix: Matrix::IDENTITY,
            opacity: 1.0,
        });
        dl.push(DisplayOp::PopClip);
        let ap = appearance(&dl, bbox());
        assert!(ap.content.contains("W* n"), "{}", ap.content);
        assert!(ap.content.contains("/Im0 Do"));
        assert_eq!(unbalanced_depth(&ap.content), 0);
    }

    /// The generator guarantees balance, but a stream is what reaches a user's
    /// file — so the writer closes what it was handed rather than trusting it.
    #[test]
    fn an_unbalanced_list_is_closed_rather_than_written_out_broken() {
        let mut dl = DisplayList::new();
        dl.push(DisplayOp::PushClip {
            path: Path::rect(bbox()),
            rule: FillRule::NonZero,
        });
        assert!(!dl.is_balanced());
        let ap = appearance(&dl, bbox());
        assert_eq!(unbalanced_depth(&ap.content), 0, "{}", ap.content);
    }
}
