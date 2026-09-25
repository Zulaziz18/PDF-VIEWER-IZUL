//! `display_list(obj, font_ctx)` — the one place geometry is decided (SPEC 3.2).
//!
//! Pure and deterministic: same object and same font metrics in, byte-identical
//! list out. That property is what the parity guarantee rests on. The canvas
//! backend in the frontend and the appearance-stream backend in `ap` both
//! consume this list and neither invents geometry of its own, so the picture on
//! screen and the picture in the saved file cannot drift apart — not because we
//! test them against each other, but because there is only one description of
//! the shape.
//!
//! Where it would be tempting to let a backend decide something, this module
//! decides it instead:
//!
//! * **Curves.** Ink smoothing happens here, as explicit cubic Béziers. A
//!   canvas that smoothed with its own spline and an appearance stream that
//!   wrote the raw polyline would be two different drawings.
//! * **Arrow heads.** Built as a path, not as a `/LE` line-ending style that
//!   only the PDF side understands.
//! * **Text.** Wrapped, aligned and positioned here, glyph by glyph.
//! * **Rotation.** Emitted as a `PushTransform` about the object's centre, so
//!   both backends apply exactly the same matrix in the same place.

use crate::annot::{AnnotObject, AnnotPayload, NoteIcon, ShapeStyle, TextAlign};
use crate::display::{
    BlendMode, DisplayList, DisplayOp, FillRule, Path, PositionedGlyph, Rgba, StrokeStyle,
};
use crate::font::{FontCtx, TextError};
use crate::geom::{Matrix, PdfPointF, PdfRectF};

/// How thick an underline or strike-out is, as a fraction of the quad's height.
const MARKUP_LINE_RATIO: f32 = 0.06;
/// Where the strike-out sits within the quad, measured from its bottom.
const STRIKE_POSITION: f32 = 0.45;
/// Where the underline sits, just below the baseline.
const UNDERLINE_POSITION: f32 = 0.08;
/// Half-angle of an arrow head, in radians (about 22°, the usual drawn arrow).
const ARROW_SPREAD: f32 = 0.38;
/// Kappa: the circle-to-Bézier constant, `4/3·(√2 − 1)`.
const KAPPA: f32 = 0.552_284_8;

/// Builds the display list for one object.
///
/// Returns [`TextError`] only for the text-bearing kinds, and only when the font
/// context refuses — SPEC 11.2 asks for that refusal to be explicit rather than
/// drawn as empty boxes.
pub fn display_list(obj: &AnnotObject, fonts: &dyn FontCtx) -> Result<DisplayList, TextError> {
    let mut dl = DisplayList::new();
    let rotated = obj.rotation.abs() > f32::EPSILON;
    if rotated {
        // About the object's own centre, which is what a rotation handle turns
        // around. Written as translate-rotate-translate so both backends get one
        // matrix and neither has to work out the pivot.
        let cx = (obj.rect.left + obj.rect.right) / 2.0;
        let cy = (obj.rect.bottom + obj.rect.top) / 2.0;
        let m = Matrix::translate(-cx, -cy)
            .then(&Matrix::rotate(obj.rotation.to_radians()))
            .then(&Matrix::translate(cx, cy));
        dl.push(DisplayOp::PushTransform { matrix: m });
    }

    // The object's own opacity multiplies into every colour it paints. Doing it
    // here rather than in each backend is what keeps a 50 %-opacity highlight
    // the same colour on screen as in the file.
    let alpha = obj.opacity.clamp(0.0, 1.0);
    match &obj.payload {
        AnnotPayload::Markup { quads, color } => {
            markup(&mut dl, obj, quads, fade(*color, alpha));
        }
        AnnotPayload::FreeText {
            text,
            font,
            color,
            align,
            line_spacing,
            background,
            border,
        } => {
            if let Some(bg) = background {
                dl.push(DisplayOp::FillPath {
                    path: Path::rect(obj.rect),
                    color: fade(*bg, alpha),
                    rule: FillRule::NonZero,
                    blend: BlendMode::Normal,
                });
            }
            if let Some(stroke) = border {
                dl.push(DisplayOp::StrokePath {
                    path: Path::rect(obj.rect),
                    color: fade(*stroke, alpha),
                    style: StrokeStyle {
                        width: 1.0,
                        miter_limit: 10.0,
                        ..Default::default()
                    },
                    blend: BlendMode::Normal,
                });
            }
            text_block(
                &mut dl,
                fonts,
                text,
                font,
                obj.rect,
                *align,
                *line_spacing,
                fade(*color, alpha),
            )?;
        }
        AnnotPayload::Image {
            image,
            crop,
            opacity,
        } => {
            // The image's unit square is mapped onto the object's rect; the crop
            // shrinks the source within that square. One matrix, so the canvas
            // and the `/Do` invocation place it identically.
            let w = obj.rect.width() / crop.width().max(f32::EPSILON);
            let h = obj.rect.height() / crop.height().max(f32::EPSILON);
            let matrix = Matrix::scale(w, h).then(&Matrix::translate(
                obj.rect.left - crop.left * w,
                obj.rect.bottom - crop.bottom * h,
            ));
            dl.push(DisplayOp::PushClip {
                path: Path::rect(obj.rect),
                rule: FillRule::NonZero,
            });
            dl.push(DisplayOp::DrawImage {
                image: *image,
                matrix,
                opacity: opacity.clamp(0.0, 1.0) * alpha,
            });
            dl.push(DisplayOp::PopClip);
        }
        AnnotPayload::Ink {
            strokes,
            color,
            width,
            smooth,
        } => {
            for stroke in strokes {
                let path = if *smooth {
                    smooth_path(stroke)
                } else {
                    polyline(stroke, false)
                };
                if path.is_empty() {
                    continue;
                }
                dl.push(DisplayOp::StrokePath {
                    path,
                    color: fade(*color, alpha),
                    style: StrokeStyle {
                        width: *width,
                        cap: crate::display::LineCap::Round,
                        join: crate::display::LineJoin::Round,
                        miter_limit: 10.0,
                        dash: Vec::new(),
                        dash_phase: 0.0,
                    },
                    blend: BlendMode::Normal,
                });
            }
        }
        AnnotPayload::Line {
            from,
            to,
            color,
            width,
            dashed,
            arrow_head,
        } => {
            let style = StrokeStyle {
                width: *width,
                cap: crate::display::LineCap::Butt,
                join: crate::display::LineJoin::Miter,
                miter_limit: 10.0,
                dash: if *dashed {
                    vec![width * 3.0, width * 2.0]
                } else {
                    Vec::new()
                },
                dash_phase: 0.0,
            };
            dl.push(DisplayOp::StrokePath {
                path: Path::new().move_to(*from).line_to(*to),
                color: fade(*color, alpha),
                style: style.clone(),
                blend: BlendMode::Normal,
            });
            if *arrow_head > 0.0 {
                if let Some(head) = arrow_path(*from, *to, *arrow_head) {
                    dl.push(DisplayOp::FillPath {
                        path: head,
                        color: fade(*color, alpha),
                        rule: FillRule::NonZero,
                        blend: BlendMode::Normal,
                    });
                }
            }
        }
        AnnotPayload::Shape { style } => {
            let path = match obj.kind {
                crate::annot::AnnotKind::Ellipse => ellipse_path(obj.rect),
                _ => Path::rect(obj.rect),
            };
            paint(&mut dl, path, style, alpha, FillRule::NonZero);
        }
        AnnotPayload::Polygon {
            points,
            closed,
            style,
        } => {
            let path = polyline(points, *closed);
            if !path.is_empty() {
                paint(&mut dl, path, style, alpha, FillRule::NonZero);
            }
        }
        AnnotPayload::Note { icon, color, .. } => {
            note_icon(&mut dl, obj.rect, *icon, fade(*color, alpha));
        }
        AnnotPayload::Stamp { label, color, font } => {
            let c = fade(*color, alpha);
            dl.push(DisplayOp::StrokePath {
                path: rounded_rect(obj.rect, obj.rect.height().min(obj.rect.width()) * 0.18),
                color: c,
                style: StrokeStyle {
                    width: (obj.rect.height() * 0.06).max(1.0),
                    miter_limit: 10.0,
                    ..Default::default()
                },
                blend: BlendMode::Normal,
            });
            // Centred on the badge, one line, never wrapped: a stamp that
            // reflowed would stop being the same stamp at another size.
            let inset = obj.rect.inflate(-obj.rect.height() * 0.2);
            text_block(
                &mut dl,
                fonts,
                label,
                font,
                inset,
                TextAlign::Center,
                1.0,
                c,
            )?;
        }
    }

    if rotated {
        dl.push(DisplayOp::PopTransform);
    }
    Ok(dl)
}

/// Multiplies an object-level opacity into a colour's alpha.
fn fade(mut color: Rgba, alpha: f32) -> Rgba {
    color.a *= alpha;
    color
}

fn paint(dl: &mut DisplayList, path: Path, style: &ShapeStyle, alpha: f32, rule: FillRule) {
    if let Some(fill) = style.fill {
        dl.push(DisplayOp::FillPath {
            path: path.clone(),
            color: fade(fill, alpha),
            rule,
            blend: BlendMode::Normal,
        });
    }
    if let Some(stroke) = style.stroke {
        dl.push(DisplayOp::StrokePath {
            path,
            color: fade(stroke, alpha),
            style: StrokeStyle {
                width: style.stroke_width,
                cap: crate::display::LineCap::Butt,
                join: crate::display::LineJoin::Miter,
                miter_limit: 10.0,
                dash: if style.dashed {
                    style.dash.to_vec()
                } else {
                    Vec::new()
                },
                dash_phase: 0.0,
            },
            blend: BlendMode::Normal,
        });
    }
}

/// Highlight, underline and strike-out, which differ only in what they draw per
/// quad.
fn markup(dl: &mut DisplayList, obj: &AnnotObject, quads: &[PdfRectF], color: Rgba) {
    use crate::annot::AnnotKind;
    for quad in quads {
        match obj.kind {
            AnnotKind::Highlight => dl.push(DisplayOp::FillPath {
                path: Path::rect(*quad),
                color,
                rule: FillRule::NonZero,
                // Multiply, so the text underneath stays legible instead of
                // being washed out by an opaque block (SPEC 11.2).
                blend: BlendMode::Multiply,
            }),
            AnnotKind::Underline | AnnotKind::StrikeOut => {
                let h = quad.height();
                let y = if obj.kind == AnnotKind::StrikeOut {
                    quad.bottom + h * STRIKE_POSITION
                } else {
                    quad.bottom + h * UNDERLINE_POSITION
                };
                dl.push(DisplayOp::StrokePath {
                    path: Path::new()
                        .move_to(PdfPointF::new(quad.left, y))
                        .line_to(PdfPointF::new(quad.right, y)),
                    color,
                    style: StrokeStyle {
                        width: (h * MARKUP_LINE_RATIO).max(0.5),
                        miter_limit: 10.0,
                        ..Default::default()
                    },
                    blend: BlendMode::Normal,
                });
            }
            AnnotKind::Redact => {
                // The mark, not the result: a red frame over a light red
                // wash, so what is marked stays readable until it is applied.
                dl.push(DisplayOp::FillPath {
                    path: Path::rect(*quad),
                    color: REDACT_WASH,
                    rule: FillRule::NonZero,
                    blend: BlendMode::Normal,
                });
                dl.push(DisplayOp::StrokePath {
                    path: Path::rect(*quad),
                    color: REDACT_FRAME,
                    style: StrokeStyle {
                        width: 1.0,
                        miter_limit: 10.0,
                        ..Default::default()
                    },
                    blend: BlendMode::Normal,
                });
            }
            _ => {}
        }
    }
}

/// How a redaction mark looks before it is applied.
pub const REDACT_FRAME: Rgba = Rgba {
    r: 0.898,
    g: 0.282,
    b: 0.302,
    a: 1.0,
};
pub const REDACT_WASH: Rgba = Rgba {
    r: 0.898,
    g: 0.282,
    b: 0.302,
    a: 0.15,
};

fn polyline(points: &[PdfPointF], closed: bool) -> Path {
    let mut iter = points.iter().copied();
    let Some(first) = iter.next() else {
        return Path::new();
    };
    let mut path = Path::new().move_to(first);
    for p in iter {
        path = path.line_to(p);
    }
    if closed {
        path = path.close();
    }
    path
}

/// Fits a smooth curve through pen samples.
///
/// Catmull-Rom converted to cubic Bézier, which passes *through* every sample
/// rather than near it — a pen stroke that drifted off the points the user drew
/// would feel like lag. The conversion is exact and has no tuning constant, so
/// it gives the same curve on every machine and in both backends.
fn smooth_path(points: &[PdfPointF]) -> Path {
    if points.len() < 3 {
        return polyline(points, false);
    }
    let at = |i: isize| -> PdfPointF {
        let last = points.len() as isize - 1;
        let idx = i.clamp(0, last) as usize;
        points.get(idx).copied().unwrap_or_default()
    };
    let mut path = Path::new().move_to(at(0));
    for i in 0..points.len() as isize - 1 {
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let c1 = PdfPointF::new(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0);
        let c2 = PdfPointF::new(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0);
        path = path.curve_to(c1, c2, p2);
    }
    path
}

/// The triangle at the end of an arrow, as a filled path.
fn arrow_path(from: PdfPointF, to: PdfPointF, size: f32) -> Option<Path> {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < f32::EPSILON {
        return None;
    }
    let angle = dy.atan2(dx);
    let wing = |offset: f32| -> PdfPointF {
        let a = angle + std::f32::consts::PI + offset;
        PdfPointF::new(to.x + size * a.cos(), to.y + size * a.sin())
    };
    Some(
        Path::new()
            .move_to(to)
            .line_to(wing(ARROW_SPREAD))
            .line_to(wing(-ARROW_SPREAD))
            .close(),
    )
}

/// An ellipse inscribed in `r`, as four cubic Béziers.
///
/// PDF has no ellipse operator and canvas's `ellipse()` is a different
/// approximation, so drawing it here is the only way the two can agree.
fn ellipse_path(r: PdfRectF) -> Path {
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.bottom + r.top) / 2.0);
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    let (ox, oy) = (rx * KAPPA, ry * KAPPA);
    Path::new()
        .move_to(PdfPointF::new(cx - rx, cy))
        .curve_to(
            PdfPointF::new(cx - rx, cy + oy),
            PdfPointF::new(cx - ox, cy + ry),
            PdfPointF::new(cx, cy + ry),
        )
        .curve_to(
            PdfPointF::new(cx + ox, cy + ry),
            PdfPointF::new(cx + rx, cy + oy),
            PdfPointF::new(cx + rx, cy),
        )
        .curve_to(
            PdfPointF::new(cx + rx, cy - oy),
            PdfPointF::new(cx + ox, cy - ry),
            PdfPointF::new(cx, cy - ry),
        )
        .curve_to(
            PdfPointF::new(cx - ox, cy - ry),
            PdfPointF::new(cx - rx, cy - oy),
            PdfPointF::new(cx - rx, cy),
        )
        .close()
}

fn rounded_rect(r: PdfRectF, radius: f32) -> Path {
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let k = rad * KAPPA;
    Path::new()
        .move_to(PdfPointF::new(r.left + rad, r.bottom))
        .line_to(PdfPointF::new(r.right - rad, r.bottom))
        .curve_to(
            PdfPointF::new(r.right - rad + k, r.bottom),
            PdfPointF::new(r.right, r.bottom + rad - k),
            PdfPointF::new(r.right, r.bottom + rad),
        )
        .line_to(PdfPointF::new(r.right, r.top - rad))
        .curve_to(
            PdfPointF::new(r.right, r.top - rad + k),
            PdfPointF::new(r.right - rad + k, r.top),
            PdfPointF::new(r.right - rad, r.top),
        )
        .line_to(PdfPointF::new(r.left + rad, r.top))
        .curve_to(
            PdfPointF::new(r.left + rad - k, r.top),
            PdfPointF::new(r.left, r.top - rad + k),
            PdfPointF::new(r.left, r.top - rad),
        )
        .line_to(PdfPointF::new(r.left, r.bottom + rad))
        .curve_to(
            PdfPointF::new(r.left, r.bottom + rad - k),
            PdfPointF::new(r.left + rad - k, r.bottom),
            PdfPointF::new(r.left + rad, r.bottom),
        )
        .close()
}

/// The sticky-note badge: a rounded speech bubble with a tail, plus a mark that
/// says which icon it is.
fn note_icon(dl: &mut DisplayList, rect: PdfRectF, icon: NoteIcon, color: Rgba) {
    let body = PdfRectF::new(
        rect.left,
        rect.bottom + rect.height() * 0.22,
        rect.right,
        rect.top,
    );
    dl.push(DisplayOp::FillPath {
        path: rounded_rect(body, body.height() * 0.25),
        color,
        rule: FillRule::NonZero,
        blend: BlendMode::Normal,
    });
    let tail = Path::new()
        .move_to(PdfPointF::new(rect.left + rect.width() * 0.25, body.bottom))
        .line_to(PdfPointF::new(rect.left + rect.width() * 0.25, rect.bottom))
        .line_to(PdfPointF::new(rect.left + rect.width() * 0.5, body.bottom))
        .close();
    dl.push(DisplayOp::FillPath {
        path: tail,
        color,
        rule: FillRule::NonZero,
        blend: BlendMode::Normal,
    });

    // The mark inside, drawn in the page's background colour by knocking it out
    // with even-odd is not possible in one path here, so it is stroked in white:
    // the icons have to be distinguishable at 16 points, which is the size they
    // are actually drawn at.
    let ink = Rgba::new(1.0, 1.0, 1.0, color.a);
    let width = (body.height() * 0.12).max(0.4);
    let line = |dl: &mut DisplayList, fy: f32| {
        let y = body.bottom + body.height() * fy;
        dl.push(DisplayOp::StrokePath {
            path: Path::new()
                .move_to(PdfPointF::new(body.left + body.width() * 0.2, y))
                .line_to(PdfPointF::new(body.right - body.width() * 0.2, y)),
            color: ink,
            style: StrokeStyle {
                width,
                miter_limit: 10.0,
                ..Default::default()
            },
            blend: BlendMode::Normal,
        });
    };
    match icon {
        NoteIcon::Comment => {
            line(dl, 0.62);
            line(dl, 0.38);
        }
        NoteIcon::Note => {
            line(dl, 0.72);
            line(dl, 0.5);
            line(dl, 0.28);
        }
        NoteIcon::Help => {
            // A question mark drawn as an arc plus a dot, which reads at small
            // sizes where a glyph would need a font we might not have.
            let (cx, cy) = (
                (body.left + body.right) / 2.0,
                body.bottom + body.height() * 0.62,
            );
            let r = body.height() * 0.2;
            dl.push(DisplayOp::StrokePath {
                path: Path::new()
                    .move_to(PdfPointF::new(cx - r, cy + r * 0.3))
                    .curve_to(
                        PdfPointF::new(cx - r, cy + r * 1.4),
                        PdfPointF::new(cx + r, cy + r * 1.4),
                        PdfPointF::new(cx + r, cy + r * 0.3),
                    )
                    .curve_to(
                        PdfPointF::new(cx + r, cy - r * 0.2),
                        PdfPointF::new(cx, cy - r * 0.2),
                        PdfPointF::new(cx, cy - r * 0.7),
                    ),
                color: ink,
                style: StrokeStyle {
                    width,
                    miter_limit: 10.0,
                    ..Default::default()
                },
                blend: BlendMode::Normal,
            });
            dl.push(DisplayOp::FillPath {
                path: Path::rect(PdfRectF::new(
                    cx - width,
                    cy - r * 1.4,
                    cx + width,
                    cy - r * 1.4 + width * 2.0,
                )),
                color: ink,
                rule: FillRule::NonZero,
                blend: BlendMode::Normal,
            });
        }
    }
}

/// One laid-out line of text.
struct Line {
    glyphs: Vec<PositionedGlyph>,
    width: f32,
    /// Ended by the box edge, not by the paragraph: the only kind of line
    /// justification stretches.
    wrapped: bool,
}

/// Stretches a wrapped line to `max_width` by widening its interior spaces
/// (Phase 8; `TextAlign::Justify` was drawn as left until then). The
/// trailing spaces a wrap leaves are not interior and get nothing, and a line
/// without an interior space — one long word — stays as it is.
fn justify(line: &Line, max_width: f32, space: f32) -> Vec<PositionedGlyph> {
    let end = line
        .glyphs
        .iter()
        .rposition(|g| g.unicode != ' ')
        .map_or(0, |i| i + 1);
    let trailing = (line.glyphs.len() - end) as f32 * space;
    let interior = line
        .glyphs
        .iter()
        .take(end)
        .filter(|g| g.unicode == ' ')
        .count();
    let slack = max_width - (line.width - trailing);
    if interior == 0 || slack <= 0.0 {
        return line.glyphs.clone();
    }
    let extra = slack / interior as f32;
    let mut seen = 0usize;
    line.glyphs
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let shifted = PositionedGlyph {
                offset: PdfPointF::new(g.offset.x + extra * seen as f32, g.offset.y),
                ..*g
            };
            if g.unicode == ' ' && i < end {
                seen += 1;
            }
            shifted
        })
        .collect()
}

/// Lays text out inside `rect` and emits one `DrawText` per line.
///
/// Wrapping, alignment and line positions are decided here rather than by either
/// backend — the whole point of positioned glyphs. Lines that fall below the
/// box are dropped rather than drawn outside it, which is what a PDF reader does
/// with an overfull `FreeText` and what keeps the object's rect honest.
#[allow(clippy::too_many_arguments)]
fn text_block(
    dl: &mut DisplayList,
    fonts: &dyn FontCtx,
    text: &str,
    spec: &crate::annot::FontSpec,
    rect: PdfRectF,
    align: TextAlign,
    line_spacing: f32,
    color: Rgba,
) -> Result<(), TextError> {
    if text.is_empty() {
        return Ok(());
    }
    let font = fonts
        .resolve(spec)
        .ok_or_else(|| TextError::FontUnavailable(spec.family.clone()))?;

    // Every character is checked before anything is drawn, so a missing glyph
    // is one clear message rather than a line of text with holes in it.
    let missing: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && fonts.glyph(font, *c).is_none())
        .take(8)
        .collect();
    if !missing.is_empty() {
        return Err(TextError::MissingGlyphs(missing));
    }

    let size = spec.size.max(0.1);
    let advance = |ch: char| -> f32 {
        fonts
            .glyph(font, ch)
            .map(|g| f32::from(g.advance_milli) / 1000.0 * size)
            .unwrap_or(0.0)
    };

    let max_width = rect.width();
    let mut lines: Vec<Line> = Vec::new();
    for paragraph in text.split('\n') {
        let mut current: Vec<PositionedGlyph> = Vec::new();
        let mut x = 0.0f32;
        let mut word: Vec<PositionedGlyph> = Vec::new();
        let mut word_width = 0.0f32;

        let flush_word = |current: &mut Vec<PositionedGlyph>,
                          x: &mut f32,
                          word: &mut Vec<PositionedGlyph>,
                          word_width: &mut f32| {
            for g in word.drain(..) {
                current.push(PositionedGlyph {
                    offset: PdfPointF::new(*x + g.offset.x, 0.0),
                    ..g
                });
            }
            *x += *word_width;
            *word_width = 0.0;
        };

        for ch in paragraph.chars() {
            let w = advance(ch);
            if ch == ' ' {
                flush_word(&mut current, &mut x, &mut word, &mut word_width);
                if let Some(g) = fonts.glyph(font, ch) {
                    current.push(PositionedGlyph {
                        glyph_id: g.glyph_id,
                        unicode: ch,
                        offset: PdfPointF::new(x, 0.0),
                    });
                }
                x += w;
                continue;
            }
            // Wrap before adding, so a word longer than the box still starts on
            // its own line instead of being silently cut in half.
            if x + word_width + w > max_width && (!current.is_empty() || !word.is_empty()) {
                if current.is_empty() {
                    flush_word(&mut current, &mut x, &mut word, &mut word_width);
                }
                lines.push(Line {
                    glyphs: std::mem::take(&mut current),
                    width: x,
                    wrapped: true,
                });
                x = 0.0;
            }
            if let Some(g) = fonts.glyph(font, ch) {
                word.push(PositionedGlyph {
                    glyph_id: g.glyph_id,
                    unicode: ch,
                    offset: PdfPointF::new(word_width, 0.0),
                });
            }
            word_width += w;
        }
        flush_word(&mut current, &mut x, &mut word, &mut word_width);
        lines.push(Line {
            glyphs: current,
            width: x,
            wrapped: false,
        });
    }

    let face = fonts.face(font);
    let ascent = f32::from(face.ascent_milli) / 1000.0 * size;
    let leading = size * line_spacing.max(0.1);
    for (i, line) in lines.iter().enumerate() {
        if line.glyphs.is_empty() {
            continue;
        }
        let baseline = rect.top - ascent - leading * i as f32;
        if baseline < rect.bottom {
            break;
        }
        let x = match align {
            TextAlign::Left | TextAlign::Justify => rect.left,
            TextAlign::Center => rect.left + (max_width - line.width) / 2.0,
            TextAlign::Right => rect.right - line.width,
        };
        // The last line of a paragraph is not stretched: a two-word closing
        // line spread across the box is what nobody's justification does.
        let glyphs = if align == TextAlign::Justify && line.wrapped {
            justify(line, max_width, advance(' '))
        } else {
            line.glyphs.clone()
        };
        dl.push(DisplayOp::DrawText {
            glyphs,
            font,
            size,
            matrix: Matrix::translate(x, baseline),
            color,
            blend: BlendMode::Normal,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annot::{AnnotId, AnnotKind, FontSpec, ShapeStyle};
    use crate::font::FixedFont;

    fn pt(x: f32, y: f32) -> PdfPointF {
        PdfPointF::new(x, y)
    }

    fn rect(l: f32, b: f32, r: f32, t: f32) -> PdfRectF {
        PdfRectF::new(l, b, r, t)
    }

    fn obj(kind: AnnotKind, payload: AnnotPayload) -> AnnotObject {
        let mut o = AnnotObject::new(AnnotId(1), 0, kind, rect(50.0, 50.0, 250.0, 150.0), payload);
        o.recompute_rect();
        o
    }

    fn markup_obj(kind: AnnotKind) -> AnnotObject {
        obj(
            kind,
            AnnotPayload::Markup {
                quads: vec![rect(72.0, 700.0, 200.0, 712.0)],
                color: Rgba::from_rgb8(255, 235, 59, 0.4),
            },
        )
    }

    fn shape(kind: AnnotKind) -> AnnotObject {
        obj(
            kind,
            AnnotPayload::Shape {
                style: ShapeStyle {
                    fill: Some(Rgba::from_rgb8(200, 220, 255, 1.0)),
                    ..Default::default()
                },
            },
        )
    }

    fn one_of_every_kind() -> Vec<AnnotObject> {
        use AnnotKind::*;
        vec![
            markup_obj(Highlight),
            markup_obj(Underline),
            markup_obj(StrikeOut),
            obj(
                FreeText,
                AnnotPayload::FreeText {
                    text: "Catatan singkat".into(),
                    font: FontSpec::default(),
                    color: Rgba::BLACK,
                    align: TextAlign::Left,
                    line_spacing: 1.2,
                    background: Some(Rgba::from_rgb8(255, 255, 210, 1.0)),
                    border: Some(Rgba::BLACK),
                },
            ),
            obj(
                Image,
                AnnotPayload::Image {
                    image: crate::display::ImageRef(1),
                    crop: rect(0.0, 0.0, 1.0, 1.0),
                    opacity: 1.0,
                },
            ),
            obj(
                Ink,
                AnnotPayload::Ink {
                    strokes: vec![vec![pt(60.0, 60.0), pt(90.0, 110.0), pt(140.0, 70.0)]],
                    color: Rgba::BLACK,
                    width: 2.0,
                    smooth: true,
                },
            ),
            obj(
                Line,
                AnnotPayload::Line {
                    from: pt(60.0, 60.0),
                    to: pt(200.0, 130.0),
                    color: Rgba::BLACK,
                    width: 2.0,
                    dashed: false,
                    arrow_head: 0.0,
                },
            ),
            obj(
                Arrow,
                AnnotPayload::Line {
                    from: pt(60.0, 60.0),
                    to: pt(200.0, 130.0),
                    color: Rgba::BLACK,
                    width: 2.0,
                    dashed: false,
                    arrow_head: 12.0,
                },
            ),
            shape(Rect),
            shape(Ellipse),
            obj(
                Polygon,
                AnnotPayload::Polygon {
                    points: vec![pt(60.0, 60.0), pt(150.0, 140.0), pt(230.0, 70.0)],
                    closed: true,
                    style: ShapeStyle::default(),
                },
            ),
            obj(
                Note,
                AnnotPayload::Note {
                    icon: NoteIcon::Comment,
                    color: Rgba::from_rgb8(255, 200, 0, 1.0),
                    text: "lihat ini".into(),
                },
            ),
            obj(
                Stamp,
                AnnotPayload::Stamp {
                    label: "DISETUJUI".into(),
                    color: Rgba::from_rgb8(0, 140, 60, 1.0),
                    font: FontSpec::default(),
                },
            ),
            markup_obj(Redact),
        ]
    }

    /// The pass criterion for Phase 3 is per annotation kind, so the first thing
    /// worth knowing is that no kind produces nothing at all.
    #[test]
    fn every_kind_draws_something() {
        let fonts = FixedFont::default();
        let objects = one_of_every_kind();
        assert_eq!(
            objects.len(),
            AnnotKind::ALL.len(),
            "satu contoh tiap jenis"
        );
        for o in objects {
            let dl = display_list(&o, &fonts).expect("display list");
            assert!(!dl.is_empty(), "{:?} tidak menggambar apa pun", o.kind);
            assert!(dl.is_balanced(), "{:?} meninggalkan push tanpa pop", o.kind);
        }
    }

    /// The parity guarantee is worth nothing if the same object can produce two
    /// different lists, so this is the property everything else rests on.
    #[test]
    fn the_same_object_always_produces_the_same_list() {
        let fonts = FixedFont::default();
        for o in one_of_every_kind() {
            let a = display_list(&o, &fonts).expect("list");
            let b = display_list(&o, &fonts).expect("list");
            assert_eq!(a, b, "{:?} tidak deterministik", o.kind);
        }
    }

    #[test]
    fn a_highlight_multiplies_so_the_text_stays_readable() {
        let fonts = FixedFont::default();
        let dl = display_list(&markup_obj(AnnotKind::Highlight), &fonts).expect("list");
        match dl.ops.first() {
            Some(DisplayOp::FillPath { blend, .. }) => {
                assert_eq!(
                    *blend,
                    BlendMode::Multiply,
                    "SPEC 11.2: selalu di belakang teks"
                );
            }
            other => panic!("stabilo harus mengisi, bukan {other:?}"),
        }
    }

    #[test]
    fn underline_and_strikeout_sit_at_different_heights() {
        let fonts = FixedFont::default();
        let y_of = |kind| {
            let dl = display_list(&markup_obj(kind), &fonts).expect("list");
            match dl.ops.first() {
                Some(DisplayOp::StrokePath { path, .. }) => match path.segs.first() {
                    Some(crate::display::PathSeg::MoveTo(p)) => p.y,
                    other => panic!("jalur tak terduga: {other:?}"),
                },
                other => panic!("harus digores: {other:?}"),
            }
        };
        let under = y_of(AnnotKind::Underline);
        let strike = y_of(AnnotKind::StrikeOut);
        assert!(strike > under, "coret di tengah, garis bawah di bawah");
        assert!(
            under >= 700.0 && strike <= 712.0,
            "keduanya di dalam kotak teks"
        );
    }

    /// An object-level opacity must reach the paint, or a 50 % shape looks solid
    /// on screen and solid in the file — consistently wrong is still wrong.
    #[test]
    fn object_opacity_multiplies_into_the_colours() {
        let fonts = FixedFont::default();
        let mut o = shape(AnnotKind::Rect);
        o.opacity = 0.5;
        let dl = display_list(&o, &fonts).expect("list");
        match dl.ops.first() {
            Some(DisplayOp::FillPath { color, .. }) => assert_eq!(color.a, 0.5),
            other => panic!("harus mengisi: {other:?}"),
        }
    }

    #[test]
    fn rotation_is_one_transform_about_the_centre() {
        let fonts = FixedFont::default();
        let mut o = shape(AnnotKind::Rect);
        o.rotation = 30.0;
        let dl = display_list(&o, &fonts).expect("list");
        let Some(DisplayOp::PushTransform { matrix }) = dl.ops.first() else {
            panic!("rotasi harus jadi satu transform di depan");
        };
        // The centre of the rect is the one point a rotation about the centre
        // leaves alone.
        let centre = pt(150.0, 100.0);
        let moved = matrix.apply(centre);
        assert!((moved.x - centre.x).abs() < 1e-3, "x bergeser: {moved:?}");
        assert!((moved.y - centre.y).abs() < 1e-3, "y bergeser: {moved:?}");
        assert!(matches!(dl.ops.last(), Some(DisplayOp::PopTransform)));
    }

    /// PDF has no ellipse operator and canvas's own `ellipse()` is a different
    /// approximation; drawing it as Béziers here is what lets the two agree.
    #[test]
    fn an_ellipse_is_four_beziers_touching_its_box() {
        let fonts = FixedFont::default();
        let dl = display_list(&shape(AnnotKind::Ellipse), &fonts).expect("list");
        let Some(DisplayOp::FillPath { path, .. }) = dl.ops.first() else {
            panic!("harus mengisi");
        };
        let curves = path
            .segs
            .iter()
            .filter(|s| matches!(s, crate::display::PathSeg::CurveTo { .. }))
            .count();
        assert_eq!(curves, 4);
        let b = path.control_bounds().expect("punya titik");
        assert!((b.left - 50.0).abs() < 1e-3 && (b.right - 250.0).abs() < 1e-3);
    }

    #[test]
    fn an_arrow_adds_a_filled_head_a_plain_line_does_not() {
        let fonts = FixedFont::default();
        let line = display_list(&one_of_every_kind()[6], &fonts).expect("list");
        let arrow = display_list(&one_of_every_kind()[7], &fonts).expect("list");
        assert_eq!(line.len(), 1, "garis biasa hanya digores");
        assert_eq!(arrow.len(), 2, "panah: garis + kepala");
        let Some(DisplayOp::FillPath { path, .. }) = arrow.ops.get(1) else {
            panic!("kepala panah harus terisi");
        };
        assert_eq!(path.segs.len(), 4, "segitiga tertutup");
    }

    /// Smoothing has to happen here: a canvas that smoothed with its own spline
    /// and an appearance stream that wrote the raw polyline are two drawings.
    #[test]
    fn smoothed_ink_passes_through_every_sample() {
        let samples = vec![pt(0.0, 0.0), pt(10.0, 20.0), pt(20.0, 0.0), pt(30.0, 15.0)];
        let path = smooth_path(&samples);
        let mut visited = vec![samples[0]];
        for seg in &path.segs {
            if let crate::display::PathSeg::CurveTo { to, .. } = seg {
                visited.push(*to);
            }
        }
        assert_eq!(
            visited, samples,
            "kurva harus melewati titik, bukan mendekatinya"
        );
    }

    #[test]
    fn two_samples_are_a_straight_line_not_a_curve() {
        let path = smooth_path(&[pt(0.0, 0.0), pt(10.0, 10.0)]);
        assert!(path
            .segs
            .iter()
            .all(|s| !matches!(s, crate::display::PathSeg::CurveTo { .. })));
    }

    #[test]
    fn text_wraps_inside_the_box_and_drops_what_does_not_fit() {
        let fonts = FixedFont::default();
        // Each glyph is 0.5 em wide, so at 12 pt a character is 6 pt: a 60 pt
        // box holds ten characters.
        let mut o = obj(
            AnnotKind::FreeText,
            AnnotPayload::FreeText {
                text: "satu dua tiga empat lima enam".into(),
                font: FontSpec::default(),
                color: Rgba::BLACK,
                align: TextAlign::Left,
                line_spacing: 1.2,
                background: None,
                border: None,
            },
        );
        o.rect = rect(0.0, 0.0, 60.0, 40.0);
        let dl = display_list(&o, &fonts).expect("list");
        let lines: Vec<_> = dl
            .ops
            .iter()
            .filter_map(|op| match op {
                DisplayOp::DrawText { glyphs, matrix, .. } => Some((glyphs.len(), matrix.f)),
                _ => None,
            })
            .collect();
        assert!(lines.len() > 1, "teks panjang harus membungkus");
        for (count, _) in &lines {
            assert!(
                *count <= 11,
                "satu baris memuat {count} glif, kotaknya 60 pt"
            );
        }
        for (_, baseline) in &lines {
            assert!(
                *baseline >= o.rect.bottom,
                "baris di luar kotak harus dibuang"
            );
        }
    }

    #[test]
    fn alignment_moves_the_line_not_the_glyph_offsets() {
        let fonts = FixedFont::default();
        let make = |align| {
            let mut o = obj(
                AnnotKind::FreeText,
                AnnotPayload::FreeText {
                    text: "abc".into(),
                    font: FontSpec::default(),
                    color: Rgba::BLACK,
                    align,
                    line_spacing: 1.2,
                    background: None,
                    border: None,
                },
            );
            o.rect = rect(0.0, 0.0, 100.0, 40.0);
            let dl = display_list(&o, &fonts).expect("list");
            match dl.ops.into_iter().find_map(|op| match op {
                DisplayOp::DrawText { matrix, glyphs, .. } => Some((matrix.e, glyphs)),
                _ => None,
            }) {
                Some(v) => v,
                None => panic!("tidak ada teks"),
            }
        };
        let (left_x, left_glyphs) = make(TextAlign::Left);
        let (centre_x, centre_glyphs) = make(TextAlign::Center);
        let (right_x, _) = make(TextAlign::Right);
        assert_eq!(left_x, 0.0);
        assert!(centre_x > left_x && right_x > centre_x);
        assert_eq!(
            left_glyphs.iter().map(|g| g.offset.x).collect::<Vec<_>>(),
            centre_glyphs.iter().map(|g| g.offset.x).collect::<Vec<_>>(),
            "perataan menggeser barisnya, bukan jarak antar huruf"
        );
    }

    #[test]
    fn justified_lines_reach_the_right_edge_and_the_last_line_does_not() {
        let fonts = FixedFont::default();
        let make = |align| {
            let mut o = obj(
                AnnotKind::FreeText,
                AnnotPayload::FreeText {
                    text: "aa bb cc dd ee ff gg hh ii jj".into(),
                    font: FontSpec::default(),
                    color: Rgba::BLACK,
                    align,
                    line_spacing: 1.2,
                    background: None,
                    border: None,
                },
            );
            o.rect = rect(0.0, 0.0, 70.0, 200.0);
            let dl = display_list(&o, &fonts).expect("list");
            dl.ops
                .into_iter()
                .filter_map(|op| match op {
                    DisplayOp::DrawText { glyphs, size, .. } => Some((glyphs, size)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let lines = make(TextAlign::Justify);
        assert!(
            lines.len() >= 2,
            "teksnya harus terbungkus: {}",
            lines.len()
        );
        let right_ink = |glyphs: &[PositionedGlyph], size: f32| {
            let last = glyphs
                .iter()
                .rev()
                .find(|g| g.unicode != ' ')
                .expect("huruf");
            let adv = fonts
                .glyph(fonts.font, last.unicode)
                .expect("glif")
                .advance_milli;
            last.offset.x + f32::from(adv) / 1000.0 * size
        };
        let (first, size) = &lines[0];
        assert!(
            (right_ink(first, *size) - 70.0).abs() < 0.01,
            "baris rata kanan-kiri berakhir di tepi kotak: {}",
            right_ink(first, *size)
        );
        let (last, size) = &lines[lines.len() - 1];
        assert!(
            right_ink(last, *size) < 69.0,
            "baris terakhir paragraf tidak diregangkan"
        );
        // Left alignment of the same text keeps the first line short of the edge.
        let left = make(TextAlign::Left);
        assert!(right_ink(&left[0].0, left[0].1) < 69.0);
    }

    /// SPEC 11.2: a missing font is refused with a clear message, never drawn as
    /// empty boxes.
    #[test]
    fn a_missing_glyph_refuses_rather_than_drawing_boxes() {
        let fonts = FixedFont {
            covers: Some("abcdefghijklmnopqrstuvwxyz ".chars().collect()),
            ..Default::default()
        };
        let o = obj(
            AnnotKind::FreeText,
            AnnotPayload::FreeText {
                text: "halo 漢字".into(),
                font: FontSpec::default(),
                color: Rgba::BLACK,
                align: TextAlign::Left,
                line_spacing: 1.2,
                background: None,
                border: None,
            },
        );
        match display_list(&o, &fonts) {
            Err(TextError::MissingGlyphs(chars)) => {
                assert!(chars.contains('漢'), "pesan menyebut karakternya: {chars}");
            }
            other => panic!("harus menolak, bukan {other:?}"),
        }
    }

    #[test]
    fn glyphs_carry_the_character_they_came_from() {
        let fonts = FixedFont::default();
        let o = obj(
            AnnotKind::FreeText,
            AnnotPayload::FreeText {
                text: "hi".into(),
                font: FontSpec::default(),
                color: Rgba::BLACK,
                align: TextAlign::Left,
                line_spacing: 1.2,
                background: None,
                border: None,
            },
        );
        let dl = display_list(&o, &fonts).expect("list");
        let Some(DisplayOp::DrawText { glyphs, .. }) = dl
            .ops
            .iter()
            .find(|op| matches!(op, DisplayOp::DrawText { .. }))
        else {
            panic!("tidak ada teks");
        };
        // Kept for /ToUnicode, so the exported file stays searchable (SPEC 8).
        assert_eq!(glyphs.iter().map(|g| g.unicode).collect::<String>(), "hi");
    }

    #[test]
    fn an_image_is_clipped_to_its_box() {
        let fonts = FixedFont::default();
        let o = obj(
            AnnotKind::Image,
            AnnotPayload::Image {
                image: crate::display::ImageRef(7),
                crop: rect(0.25, 0.25, 0.75, 0.75),
                opacity: 0.5,
            },
        );
        let dl = display_list(&o, &fonts).expect("list");
        assert!(matches!(dl.ops.first(), Some(DisplayOp::PushClip { .. })));
        assert!(matches!(dl.ops.last(), Some(DisplayOp::PopClip)));
        let Some(DisplayOp::DrawImage {
            matrix, opacity, ..
        }) = dl.ops.get(1)
        else {
            panic!("tidak ada gambar");
        };
        assert_eq!(*opacity, 0.5);
        // The cropped sub-rectangle must land exactly on the object's box.
        let corner = matrix.apply(pt(0.25, 0.25));
        assert!((corner.x - o.rect.left).abs() < 1e-3, "{corner:?}");
        assert!((corner.y - o.rect.bottom).abs() < 1e-3, "{corner:?}");
    }

    #[test]
    fn an_empty_ink_stroke_draws_nothing_rather_than_a_dot() {
        let fonts = FixedFont::default();
        let o = obj(
            AnnotKind::Ink,
            AnnotPayload::Ink {
                strokes: vec![vec![]],
                color: Rgba::BLACK,
                width: 2.0,
                smooth: true,
            },
        );
        assert!(display_list(&o, &fonts).expect("list").is_empty());
    }
}
