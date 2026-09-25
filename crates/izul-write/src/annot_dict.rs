//! The standard annotation dictionary for each kind (ISO 32000-1 §12.5.6).
//!
//! SPEC 8 asks for annotations "ditulis sebagai anotasi PDF standar (bukan
//! diratakan)": a highlight must be a `/Highlight` with `/QuadPoints`, so that
//! another reader lists it as a highlight, lets its user reply to it, and — if
//! it ever regenerates an appearance — regenerates the right one. The
//! appearance stream is still what every reader draws; these keys are what the
//! annotation *is*.
//!
//! Mapping from the thirteen kinds of SPEC 11.2:
//!
//! | kind | `/Subtype` | geometry key |
//! |---|---|---|
//! | Highlight, Underline, StrikeOut | same | `/QuadPoints` |
//! | FreeText | `/FreeText` | `/DA`, `/Q` |
//! | Ink | `/Ink` | `/InkList` |
//! | Line, Arrow | `/Line` | `/L`, `/LE` |
//! | Rect | `/Square` | — |
//! | Ellipse | `/Circle` | — |
//! | Polygon | `/Polygon` or `/PolyLine` | `/Vertices` |
//! | Note | `/Text` | `/Name` |
//! | Stamp, Image | `/Stamp` | `/Name` |
//!
//! There is no standard image annotation; a `/Stamp` whose appearance is the
//! picture is what every editor writes for one.

use izul_model::annot::{AnnotKind, AnnotObject, AnnotPayload, NoteIcon, ShapeStyle, TextAlign};
use izul_model::display::Rgba;
use izul_model::geom::{Matrix, PdfPointF, PdfRectF};

use crate::syntax::{date, name, num, rect, rgb, text_string};

/// Annotation flags: `Print` (bit 3). Without it, a printed copy silently
/// loses every annotation — the opposite of what someone who marked up a
/// document expects.
const FLAG_PRINT: u32 = 4;

fn border(width: f32, dashed: bool, dash: [f32; 2]) -> String {
    if dashed {
        format!(
            "/BS<</Type/Border/W {}/S/D/D[{} {}]>>",
            num(width),
            num(dash[0]),
            num(dash[1])
        )
    } else {
        format!("/BS<</Type/Border/W {}/S/S>>", num(width))
    }
}

fn shape_keys(style: &ShapeStyle) -> String {
    let mut out = String::new();
    if let Some(c) = style.stroke {
        out.push_str(&format!("/C{}", rgb(c)));
    }
    if let Some(f) = style.fill {
        out.push_str(&format!("/IC{}", rgb(f)));
    }
    let width = if style.stroke.is_some() {
        style.stroke_width
    } else {
        0.0
    };
    out.push_str(&border(width, style.dashed, style.dash));
    out
}

/// Four corners per quad, in the order Acrobat and PDFium read them: upper
/// left, upper right, lower left, lower right. (The specification's prose
/// describes counter-clockwise order; the readers that matter do not follow it,
/// and a highlight whose quads are "correct" but drawn crossed helps nobody.)
///
/// The corners are named in display space and each is carried into user
/// space by `m`, so on a turned page "upper left" stays the corner where the
/// text starts.
fn quad_points(quads: &[PdfRectF], m: &Matrix) -> String {
    let mut out = String::from("[");
    for q in quads {
        for (x, y) in [
            (q.left, q.top),
            (q.right, q.top),
            (q.left, q.bottom),
            (q.right, q.bottom),
        ] {
            let PdfPointF { x, y } = m.apply(PdfPointF::new(x, y));
            out.push_str(&num(x));
            out.push(' ');
            out.push_str(&num(y));
            out.push(' ');
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out.push(']');
    out
}

fn points(pts: impl Iterator<Item = (f32, f32)>, m: &Matrix) -> String {
    let parts: Vec<String> = pts
        .map(|(x, y)| m.apply(PdfPointF::new(x, y)))
        .map(|p| format!("{} {}", num(p.x), num(p.y)))
        .collect();
    format!("[{}]", parts.join(" "))
}

fn da(font: &izul_model::annot::FontSpec, color: Rgba) -> String {
    // The resource names Acrobat's own default resources carry for the
    // standard faces. `/DA` is only consulted by a reader that regenerates the
    // appearance; ours is always present.
    let base = izul_model::font::standard_base_font(font);
    let short = if base.starts_with("Times") {
        "TiRo"
    } else if base.starts_with("Courier") {
        "Cour"
    } else {
        "Helv"
    };
    text_string(&format!(
        "/{short} {} Tf {} {} {} rg",
        num(font.size),
        num(color.r),
        num(color.g),
        num(color.b)
    ))
}

/// The dictionary body for one annotation.
///
/// `obj` is in display space (see `izul_model::geom::PageFrame`) and `m`
/// carries display space into the page's user space, where every geometry key
/// is written; `rect_` is the painted box, already in user space (see
/// [`crate::bounds`]); `ap` the appearance stream's object number; `metadata`
/// the `/IzulObj` value, which keeps display space.
pub fn annotation_dict(
    obj: &AnnotObject,
    rect_: PdfRectF,
    ap: u32,
    metadata: &str,
    m: &Matrix,
) -> String {
    let subtype = match (&obj.kind, &obj.payload) {
        (AnnotKind::Polygon, AnnotPayload::Polygon { closed: false, .. }) => "PolyLine",
        (AnnotKind::Image, _) => "Stamp",
        _ => obj.kind.pdf_subtype(),
    };
    let mut d = format!(
        "<</Type/Annot/Subtype/{subtype}/Rect{}/NM{}/M{}/F {FLAG_PRINT}",
        rect(rect_),
        text_string(&format!("{}{}", crate::metadata::NM_PREFIX, obj.id.0)),
        date(obj.modified_at.max(obj.created_at)),
    );
    // No `/CA`. The object's opacity is already multiplied into every colour
    // of the appearance stream (`izul_model::build`), and ISO 32000 has a
    // reader apply `/CA` *to* the appearance when painting it — Acrobat does —
    // so writing both would make a 50 % object 25 % there and 50 % in PDFium.

    let mut contents = obj.author_note.clone();
    match &obj.payload {
        AnnotPayload::Markup { quads, color } if obj.kind == AnnotKind::Redact => {
            // A mark not yet applied (§12.5.6.23): the areas, and in /IC the
            // colour they are filled with once applied. The red of the mark
            // itself is in the appearance stream.
            d.push_str(&format!(
                "/QuadPoints{}/IC{}",
                quad_points(quads, m),
                rgb(*color)
            ));
        }
        AnnotPayload::Markup { quads, color } => {
            d.push_str(&format!(
                "/C{}/QuadPoints{}",
                rgb(*color),
                quad_points(quads, m)
            ));
        }
        AnnotPayload::FreeText {
            text,
            font,
            color,
            align,
            background,
            border: edge,
            ..
        } => {
            contents = text.clone();
            let q = match align {
                TextAlign::Center => 1,
                TextAlign::Right => 2,
                TextAlign::Left | TextAlign::Justify => 0,
            };
            d.push_str(&format!("/DA{}/Q {q}", da(font, *color)));
            if let Some(bg) = background {
                d.push_str(&format!("/C{}", rgb(*bg)));
            }
            d.push_str(&border(
                if edge.is_some() { 1.0 } else { 0.0 },
                false,
                [0.0, 0.0],
            ));
        }
        AnnotPayload::Image { .. } => {
            d.push_str(&format!("/Name{}", name("Image")));
        }
        AnnotPayload::Ink {
            strokes,
            color,
            width,
            ..
        } => {
            let lists: Vec<String> = strokes
                .iter()
                .map(|s| points(s.iter().map(|p| (p.x, p.y)), m))
                .collect();
            d.push_str(&format!(
                "/C{}/InkList[{}]{}",
                rgb(*color),
                lists.join(" "),
                border(*width, false, [0.0, 0.0])
            ));
        }
        AnnotPayload::Line {
            from,
            to,
            color,
            width,
            dashed,
            arrow_head,
        } => {
            let end = if *arrow_head > 0.0 {
                "/OpenArrow"
            } else {
                "/None"
            };
            let (from, to) = (m.apply(*from), m.apply(*to));
            d.push_str(&format!(
                "/C{}/L[{} {} {} {}]/LE[/None{end}]{}",
                rgb(*color),
                num(from.x),
                num(from.y),
                num(to.x),
                num(to.y),
                border(*width, *dashed, [4.0, 3.0])
            ));
        }
        AnnotPayload::Shape { style } => d.push_str(&shape_keys(style)),
        AnnotPayload::Polygon {
            points: pts, style, ..
        } => {
            d.push_str(&format!(
                "/Vertices{}{}",
                points(pts.iter().map(|p| (p.x, p.y)), m),
                shape_keys(style)
            ));
        }
        AnnotPayload::Note { icon, color, text } => {
            contents = text.clone();
            let icon = match icon {
                NoteIcon::Comment => "Comment",
                NoteIcon::Note => "Note",
                NoteIcon::Help => "Help",
            };
            d.push_str(&format!("/C{}/Name/{icon}/Open false", rgb(*color)));
        }
        AnnotPayload::Stamp { label, color, .. } => {
            if contents.is_empty() {
                contents = label.clone();
            }
            d.push_str(&format!("/C{}/Name{}", rgb(*color), name("Izul")));
        }
    }
    if !contents.is_empty() {
        d.push_str(&format!("/Contents{}", text_string(&contents)));
    }
    d.push_str(&format!(
        "/AP<</N {ap} 0 R>>/{}{}>>",
        crate::metadata::KEY,
        text_string(metadata)
    ));
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use izul_model::annot::{AnnotId, FontSpec};
    use izul_model::geom::PdfPointF;

    fn obj(kind: AnnotKind, payload: AnnotPayload) -> AnnotObject {
        let mut o = AnnotObject::new(
            AnnotId(3),
            0,
            kind,
            PdfRectF::new(10.0, 10.0, 50.0, 30.0),
            payload,
        );
        o.created_at = 1_790_132_645_000;
        o
    }

    fn dict(o: &AnnotObject) -> String {
        annotation_dict(o, o.rect, 12, "{}", &Matrix::IDENTITY)
    }

    #[test]
    fn a_highlight_is_a_standard_highlight() {
        let o = obj(
            AnnotKind::Highlight,
            AnnotPayload::Markup {
                quads: vec![PdfRectF::new(10.0, 10.0, 50.0, 20.0)],
                color: Rgba::new(1.0, 0.8, 0.0, 1.0),
            },
        );
        let d = dict(&o);
        assert!(
            d.starts_with("<</Type/Annot/Subtype/Highlight/Rect[10 10 50 30]/NM(izul-3)"),
            "{d}"
        );
        assert!(d.contains("/QuadPoints[10 20 50 20 10 10 50 10]"), "{d}");
        assert!(d.contains("/M(D:20260923030405Z)"));
        assert!(d.contains("/F 4"));
        assert!(d.contains("/AP<</N 12 0 R>>/IzulObj({})>>"));
    }

    /// A redaction mark is the standard annotation Acrobat applies: its areas
    /// in /QuadPoints and the colour they become in /IC — not /C, which would
    /// tint the mark itself in other readers.
    #[test]
    fn a_redaction_mark_is_a_standard_redact_annotation() {
        let o = obj(
            AnnotKind::Redact,
            AnnotPayload::Markup {
                quads: vec![PdfRectF::new(10.0, 10.0, 50.0, 20.0)],
                color: Rgba::BLACK,
            },
        );
        let d = dict(&o);
        assert!(d.starts_with("<</Type/Annot/Subtype/Redact/"), "{d}");
        assert!(
            d.contains("/QuadPoints[10 20 50 20 10 10 50 10]/IC[0 0 0]"),
            "{d}"
        );
        assert!(!d.contains("/C["), "{d}");
    }

    #[test]
    fn an_arrow_is_a_line_with_an_arrow_ending() {
        let o = obj(
            AnnotKind::Arrow,
            AnnotPayload::Line {
                from: PdfPointF::new(0.0, 0.0),
                to: PdfPointF::new(10.0, 5.0),
                color: Rgba::BLACK,
                width: 2.0,
                dashed: true,
                arrow_head: 8.0,
            },
        );
        let d = dict(&o);
        assert!(d.contains("/Subtype/Line"), "{d}");
        assert!(d.contains("/L[0 0 10 5]/LE[/None/OpenArrow]"), "{d}");
        assert!(d.contains("/BS<</Type/Border/W 2/S/D/D[4 3]>>"), "{d}");
    }

    #[test]
    fn free_text_carries_its_text_and_a_default_appearance() {
        let o = obj(
            AnnotKind::FreeText,
            AnnotPayload::FreeText {
                text: "Catatan é".into(),
                font: FontSpec::default(),
                color: Rgba::new(1.0, 0.0, 0.0, 1.0),
                align: TextAlign::Center,
                line_spacing: 1.2,
                background: None,
                border: None,
            },
        );
        let d = dict(&o);
        assert!(d.contains("/Subtype/FreeText"));
        assert!(d.contains("/DA(/TiRo 12 Tf 1 0 0 rg)/Q 1"), "{d}");
        assert!(d.contains(&format!("/Contents{}", text_string("Catatan é"))));
    }

    #[test]
    fn an_open_polygon_is_a_polyline_and_opacity_is_not_applied_twice() {
        let mut o = obj(
            AnnotKind::Polygon,
            AnnotPayload::Polygon {
                points: vec![PdfPointF::new(0.0, 0.0), PdfPointF::new(5.0, 5.0)],
                closed: false,
                style: ShapeStyle::default(),
            },
        );
        o.opacity = 0.5;
        let d = dict(&o);
        assert!(d.contains("/Subtype/PolyLine/"), "{d}");
        assert!(d.contains("/Vertices[0 0 5 5]"));
        assert!(!d.contains("/CA"), "{d}");
    }

    /// On a page turned a quarter clockwise with its box at (100, 200) and
    /// 595 wide, display (x, y) is user (695 - y, 200 + x): every geometry key
    /// is carried over, not only /Rect.
    #[test]
    fn geometry_keys_are_written_in_user_space() {
        use izul_model::geom::{PageFrame, RotationQuarter};
        let m = PageFrame {
            bbox: PdfRectF::new(100.0, 200.0, 695.0, 1042.0),
            rotation: RotationQuarter::Cw90,
        }
        .display_to_user();
        let ink = AnnotObject::new(
            izul_model::annot::AnnotId(1),
            0,
            AnnotKind::Ink,
            PdfRectF::new(0.0, 0.0, 1.0, 1.0),
            AnnotPayload::Ink {
                strokes: vec![vec![PdfPointF::new(10.0, 20.0), PdfPointF::new(30.0, 40.0)]],
                color: Rgba::BLACK,
                width: 1.0,
                smooth: false,
            },
        );
        let d = annotation_dict(&ink, ink.rect, 1, "{}", &m);
        assert!(d.contains("/InkList[[675 210 655 230]]"), "{d}");
        let line = AnnotObject::new(
            izul_model::annot::AnnotId(2),
            0,
            AnnotKind::Line,
            PdfRectF::new(0.0, 0.0, 1.0, 1.0),
            AnnotPayload::Line {
                from: PdfPointF::new(0.0, 0.0),
                to: PdfPointF::new(100.0, 50.0),
                color: Rgba::BLACK,
                width: 1.0,
                dashed: false,
                arrow_head: 0.0,
            },
        );
        let d = annotation_dict(&line, line.rect, 1, "{}", &m);
        assert!(d.contains("/L[695 200 645 300]"), "{d}");
    }
}
