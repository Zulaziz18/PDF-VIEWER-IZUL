//! The annotation object model (SPEC 8, SPEC 11.2).
//!
//! One `AnnotObject` type with a payload per kind, rather than a trait object
//! per kind. The reason is the display list: every object has to be turned into
//! the *same* primitive vocabulary by a pure function, and a closed enum is what
//! makes "every kind is handled" a compile error rather than a code review.
//!
//! **Coordinates are always PDF user space** — points, origin at the page's
//! bottom-left, y upwards (SPEC 8). Nothing here is ever pixels. Storing pixels
//! would put every annotation in the wrong place the moment the zoom changed,
//! and the mistake would only show up after a save.

use serde::{Deserialize, Serialize};

use crate::display::Rgba;
use crate::geom::{PdfPointF, PdfRectF};

/// Identity of one annotation within a document.
///
/// Monotonic per document and never reused, because the undo stack refers to
/// objects by id: reusing an id would let an undo resurrect the wrong object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AnnotId(pub u64);

/// Which of SPEC 11.2's kinds an object is.
///
/// Carried separately from the payload so a list panel, a filter or a saved row
/// can name the kind without matching on the whole payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnnotKind {
    Highlight,
    Underline,
    StrikeOut,
    FreeText,
    Image,
    Ink,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Polygon,
    Note,
    Stamp,
}

impl AnnotKind {
    /// Every kind, for the tests that must cover all of them and for the filter
    /// in the annotation list panel.
    pub const ALL: [AnnotKind; 13] = [
        AnnotKind::Highlight,
        AnnotKind::Underline,
        AnnotKind::StrikeOut,
        AnnotKind::FreeText,
        AnnotKind::Image,
        AnnotKind::Ink,
        AnnotKind::Line,
        AnnotKind::Arrow,
        AnnotKind::Rect,
        AnnotKind::Ellipse,
        AnnotKind::Polygon,
        AnnotKind::Note,
        AnnotKind::Stamp,
    ];

    /// The `/Subtype` this kind is written as when saved (SPEC 8's "objek hidup
    /// setelah simpan-buka": standard annotations, not flattened paint).
    ///
    /// Several of our kinds share a PDF subtype — an arrow *is* a `/Line` with
    /// a line ending, a rounded stamp is a `/Stamp` — and the difference is
    /// carried in our own metadata key. That is deliberate: another reader must
    /// show the annotation correctly, and we must be able to reopen it as the
    /// kind the user drew.
    pub fn pdf_subtype(self) -> &'static str {
        match self {
            AnnotKind::Highlight => "Highlight",
            AnnotKind::Underline => "Underline",
            AnnotKind::StrikeOut => "StrikeOut",
            AnnotKind::FreeText => "FreeText",
            AnnotKind::Image | AnnotKind::Stamp => "Stamp",
            AnnotKind::Ink => "Ink",
            AnnotKind::Line | AnnotKind::Arrow => "Line",
            AnnotKind::Rect => "Square",
            AnnotKind::Ellipse => "Circle",
            AnnotKind::Polygon => "Polygon",
            AnnotKind::Note => "Text",
        }
    }

    /// True when the object's geometry is a run of text quads rather than a
    /// shape the user dragged out — those follow the text when it reflows and
    /// cannot be resized freely.
    pub fn is_text_markup(self) -> bool {
        matches!(
            self,
            AnnotKind::Highlight | AnnotKind::Underline | AnnotKind::StrikeOut
        )
    }
}

/// How a shape is painted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShapeStyle {
    /// `None` means no fill at all, which is not the same as a transparent one:
    /// a transparent fill still covers what an even-odd hole would show.
    pub fill: Option<Rgba>,
    pub stroke: Option<Rgba>,
    pub stroke_width: f32,
    /// Dash pattern in points; empty is solid.
    pub dash: [f32; 2],
    pub dashed: bool,
}

impl Default for ShapeStyle {
    fn default() -> Self {
        ShapeStyle {
            fill: None,
            stroke: Some(Rgba::BLACK),
            stroke_width: 1.5,
            dash: [4.0, 3.0],
            dashed: false,
        }
    }
}

/// Which font a text object asks for. Resolving it to something drawable is the
/// font context's job (see [`crate::font`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSpec {
    /// Family name as the user picked it. Times New Roman is the default
    /// (SPEC 11.2).
    pub family: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        FontSpec {
            family: "Times New Roman".into(),
            size: 12.0,
            bold: false,
            italic: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// Which icon a sticky note shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NoteIcon {
    #[default]
    Comment,
    Note,
    Help,
}

/// Everything that varies between kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnnotPayload {
    /// Text markup: one quad per line of covered text, from PDFium's character
    /// boxes. A list, not a rectangle, because a selection that wraps covers
    /// several lines and one box around them would cover the whole paragraph.
    Markup {
        quads: Vec<PdfRectF>,
        color: Rgba,
    },
    FreeText {
        text: String,
        font: FontSpec,
        color: Rgba,
        align: TextAlign,
        /// Multiple of the font size; 1.2 is the usual single spacing.
        line_spacing: f32,
        background: Option<Rgba>,
        border: Option<Rgba>,
    },
    Image {
        image: crate::display::ImageRef,
        /// Sub-rectangle of the source image to show, in 0..1 of its own size.
        crop: PdfRectF,
        opacity: f32,
    },
    /// One entry per pen-down..pen-up stroke, sampled in page space.
    Ink {
        strokes: Vec<Vec<PdfPointF>>,
        color: Rgba,
        width: f32,
        /// Whether to fit smooth curves through the samples. Off gives exactly
        /// the polyline the pointer produced.
        smooth: bool,
    },
    Line {
        from: PdfPointF,
        to: PdfPointF,
        color: Rgba,
        width: f32,
        dashed: bool,
        /// Length of the arrow head in points. Zero draws a plain line, which
        /// is what makes `Line` and `Arrow` one payload instead of two.
        arrow_head: f32,
    },
    Shape {
        style: ShapeStyle,
    },
    Polygon {
        points: Vec<PdfPointF>,
        closed: bool,
        style: ShapeStyle,
    },
    Note {
        icon: NoteIcon,
        color: Rgba,
        text: String,
    },
    Stamp {
        label: String,
        color: Rgba,
        font: FontSpec,
    },
}

/// One annotation (SPEC 8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotObject {
    pub id: AnnotId,
    pub page: u32,
    pub kind: AnnotKind,
    /// Bounding box in page space. For shapes it *is* the geometry; for markup
    /// and ink it is the box around it, kept in step by [`Self::recompute_rect`].
    pub rect: PdfRectF,
    /// Rotation about the rect's centre, in degrees counter-clockwise.
    pub rotation: f32,
    /// 0..1, applied to the whole object on top of any colour alpha.
    pub opacity: f32,
    /// Paint order within a page. Higher is nearer the reader.
    pub z: i32,
    pub locked: bool,
    pub created_at: i64,
    pub modified_at: i64,
    /// Free-text note shown in the annotation list and in other readers'
    /// popups. Not the same as `FreeText`'s own text, which is drawn.
    pub author_note: String,
    pub payload: AnnotPayload,
}

impl AnnotObject {
    /// A new object with sensible defaults; callers set what they care about.
    pub fn new(
        id: AnnotId,
        page: u32,
        kind: AnnotKind,
        rect: PdfRectF,
        payload: AnnotPayload,
    ) -> Self {
        AnnotObject {
            id,
            page,
            kind,
            rect,
            rotation: 0.0,
            opacity: 1.0,
            z: 0,
            locked: false,
            created_at: 0,
            modified_at: 0,
            author_note: String::new(),
            payload,
        }
    }

    /// The box the object actually covers, derived from its payload.
    ///
    /// Shapes are their rect; markup, ink and polygons are the union of what
    /// they contain. Keeping this derivable is what stops a resize handle from
    /// drifting away from the ink it is supposed to be holding.
    pub fn derived_rect(&self) -> Option<PdfRectF> {
        fn around(points: impl Iterator<Item = PdfPointF>) -> Option<PdfRectF> {
            let mut out: Option<PdfRectF> = None;
            for p in points {
                out = Some(match out {
                    None => PdfRectF::new(p.x, p.y, p.x, p.y),
                    Some(r) => PdfRectF {
                        left: r.left.min(p.x),
                        bottom: r.bottom.min(p.y),
                        right: r.right.max(p.x),
                        top: r.top.max(p.y),
                    },
                });
            }
            out
        }
        match &self.payload {
            AnnotPayload::Markup { quads, .. } => quads.iter().copied().reduce(|a, b| a.union(&b)),
            AnnotPayload::Ink { strokes, width, .. } => {
                around(strokes.iter().flatten().copied()).map(|r| r.inflate(*width / 2.0))
            }
            AnnotPayload::Polygon { points, style, .. } => {
                around(points.iter().copied()).map(|r| r.inflate(style.stroke_width / 2.0))
            }
            AnnotPayload::Line {
                from, to, width, ..
            } => around([*from, *to].into_iter()).map(|r| r.inflate(*width / 2.0)),
            _ => Some(self.rect),
        }
    }

    /// Pulls `rect` back into agreement with the payload.
    pub fn recompute_rect(&mut self) {
        if let Some(r) = self.derived_rect() {
            self.rect = r;
        }
    }

    /// Moves the object by `(dx, dy)` in page space, payload and all.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        let shift_rect = |r: &mut PdfRectF| {
            r.left += dx;
            r.right += dx;
            r.bottom += dy;
            r.top += dy;
        };
        let shift_pt = |p: &mut PdfPointF| {
            p.x += dx;
            p.y += dy;
        };
        shift_rect(&mut self.rect);
        match &mut self.payload {
            AnnotPayload::Markup { quads, .. } => quads.iter_mut().for_each(shift_rect),
            AnnotPayload::Ink { strokes, .. } => {
                strokes.iter_mut().flatten().for_each(shift_pt);
            }
            AnnotPayload::Polygon { points, .. } => points.iter_mut().for_each(shift_pt),
            AnnotPayload::Line { from, to, .. } => {
                shift_pt(from);
                shift_pt(to);
            }
            _ => {}
        }
    }

    /// Scales the object about `origin`.
    ///
    /// Stroke widths and font sizes scale with it: a rectangle dragged to twice
    /// the size with a hairline border that stayed hairline is not what anyone
    /// means by resizing, and the exported file would disagree with the screen
    /// if the two layers decided that differently.
    pub fn scale_about(&mut self, origin: PdfPointF, sx: f32, sy: f32) {
        let s = (sx.abs() + sy.abs()) / 2.0;
        let map_pt = |p: &mut PdfPointF| {
            p.x = origin.x + (p.x - origin.x) * sx;
            p.y = origin.y + (p.y - origin.y) * sy;
        };
        let map_rect = |r: &mut PdfRectF| {
            let mut a = PdfPointF::new(r.left, r.bottom);
            let mut b = PdfPointF::new(r.right, r.top);
            map_pt(&mut a);
            map_pt(&mut b);
            *r = PdfRectF::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y));
        };
        map_rect(&mut self.rect);
        match &mut self.payload {
            AnnotPayload::Markup { quads, .. } => quads.iter_mut().for_each(map_rect),
            AnnotPayload::Ink { strokes, width, .. } => {
                strokes.iter_mut().flatten().for_each(map_pt);
                *width *= s;
            }
            AnnotPayload::Polygon { points, style, .. } => {
                points.iter_mut().for_each(map_pt);
                style.stroke_width *= s;
            }
            AnnotPayload::Line {
                from,
                to,
                width,
                arrow_head,
                ..
            } => {
                map_pt(from);
                map_pt(to);
                *width *= s;
                *arrow_head *= s;
            }
            AnnotPayload::Shape { style } => style.stroke_width *= s,
            AnnotPayload::FreeText { font, .. } | AnnotPayload::Stamp { font, .. } => {
                font.size *= s;
            }
            AnnotPayload::Image { .. } | AnnotPayload::Note { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(l: f32, b: f32, r: f32, t: f32) -> PdfRectF {
        PdfRectF::new(l, b, r, t)
    }

    fn ink() -> AnnotObject {
        AnnotObject::new(
            AnnotId(1),
            0,
            AnnotKind::Ink,
            rect(0.0, 0.0, 0.0, 0.0),
            AnnotPayload::Ink {
                strokes: vec![vec![PdfPointF::new(10.0, 10.0), PdfPointF::new(30.0, 40.0)]],
                color: Rgba::BLACK,
                width: 2.0,
                smooth: true,
            },
        )
    }

    /// Every kind must map to a subtype another reader understands, or "objek
    /// hidup setelah simpan-buka" (SPEC 8) is only true in our own viewer.
    #[test]
    fn every_kind_has_a_pdf_subtype() {
        for kind in AnnotKind::ALL {
            let sub = kind.pdf_subtype();
            assert!(!sub.is_empty(), "{kind:?} tanpa subtype");
            assert!(
                sub.chars().next().is_some_and(char::is_uppercase),
                "{kind:?}: subtype PDF selalu diawali huruf besar"
            );
        }
    }

    #[test]
    fn the_kind_list_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for kind in AnnotKind::ALL {
            assert!(seen.insert(kind), "{kind:?} tercantum dua kali");
        }
        assert_eq!(
            AnnotKind::ALL.len(),
            13,
            "SPEC 11.2 menyebut tiga belas jenis"
        );
    }

    #[test]
    fn ink_derives_its_box_from_the_strokes_plus_half_the_pen() {
        let mut obj = ink();
        obj.recompute_rect();
        assert_eq!(obj.rect, rect(9.0, 9.0, 31.0, 41.0));
    }

    /// The handle the user drags must stay on the ink; a rect that did not move
    /// with its payload is a selection box floating next to the object.
    #[test]
    fn translating_moves_the_payload_with_the_rect() {
        let mut obj = ink();
        obj.recompute_rect();
        obj.translate(5.0, -2.0);
        let AnnotPayload::Ink { strokes, .. } = &obj.payload else {
            panic!("payload berubah jenis");
        };
        assert_eq!(strokes[0][0], PdfPointF::new(15.0, 8.0));
        assert_eq!(obj.rect.left, 14.0);
        let before = obj.rect;
        obj.recompute_rect();
        assert_eq!(obj.rect, before, "kotak sudah konsisten dengan isinya");
    }

    #[test]
    fn scaling_takes_the_pen_width_with_it() {
        let mut obj = ink();
        obj.recompute_rect();
        obj.scale_about(PdfPointF::new(0.0, 0.0), 2.0, 2.0);
        let AnnotPayload::Ink { width, strokes, .. } = &obj.payload else {
            panic!("payload berubah jenis");
        };
        assert_eq!(*width, 4.0, "garis ikut menebal");
        assert_eq!(strokes[0][1], PdfPointF::new(60.0, 80.0));
    }

    #[test]
    fn scaling_text_takes_the_font_size_with_it() {
        let mut obj = AnnotObject::new(
            AnnotId(2),
            0,
            AnnotKind::FreeText,
            rect(0.0, 0.0, 100.0, 50.0),
            AnnotPayload::FreeText {
                text: "halo".into(),
                font: FontSpec::default(),
                color: Rgba::BLACK,
                align: TextAlign::Left,
                line_spacing: 1.2,
                background: None,
                border: None,
            },
        );
        obj.scale_about(PdfPointF::new(0.0, 0.0), 2.0, 2.0);
        let AnnotPayload::FreeText { font, .. } = &obj.payload else {
            panic!("payload berubah jenis");
        };
        assert_eq!(font.size, 24.0);
        assert_eq!(obj.rect, rect(0.0, 0.0, 200.0, 100.0));
    }

    /// A selection that wraps covers several lines; one box around them would
    /// highlight the whole paragraph between the first and last word.
    #[test]
    fn markup_keeps_one_quad_per_line() {
        let mut obj = AnnotObject::new(
            AnnotId(3),
            0,
            AnnotKind::Highlight,
            rect(0.0, 0.0, 0.0, 0.0),
            AnnotPayload::Markup {
                quads: vec![
                    rect(400.0, 700.0, 500.0, 712.0),
                    rect(72.0, 688.0, 200.0, 700.0),
                ],
                color: Rgba::from_rgb8(255, 235, 59, 0.4),
            },
        );
        obj.recompute_rect();
        assert_eq!(obj.rect, rect(72.0, 688.0, 500.0, 712.0));
        let AnnotPayload::Markup { quads, .. } = &obj.payload else {
            panic!("payload berubah jenis");
        };
        assert_eq!(quads.len(), 2, "dua baris tetap dua kotak");
    }

    #[test]
    fn text_markup_kinds_are_the_three_that_follow_text() {
        let markup: Vec<_> = AnnotKind::ALL
            .into_iter()
            .filter(|k| k.is_text_markup())
            .collect();
        assert_eq!(
            markup,
            vec![
                AnnotKind::Highlight,
                AnnotKind::Underline,
                AnnotKind::StrikeOut
            ]
        );
    }
}
