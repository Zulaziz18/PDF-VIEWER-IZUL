/**
 * Turning a drawn gesture into an annotation object.
 *
 * The geometry comes from the pointer, the *look* comes from the current style
 * — colour, width, font — which is remembered between objects the way every
 * drawing tool does it. Pure, so "what does the arrow tool produce" is a
 * question with a testable answer rather than something you find out by drawing
 * one.
 */

import type {
  AnnotKind,
  AnnotObject,
  AnnotPayload,
  FontSpec,
  PdfPoint,
  PdfRect,
  Rgba,
  ShapeStyle,
} from "./types";
import { rgba, NEW_OBJECT_ID } from "./types";

/** What the tools draw with until the user changes it. */
export interface AnnotStyle {
  color: Rgba;
  fill: Rgba | null;
  strokeWidth: number;
  opacity: number;
  dashed: boolean;
  font: FontSpec;
  highlightColor: Rgba;
}

export const DEFAULT_STYLE: AnnotStyle = {
  color: rgba(220, 38, 38),
  fill: null,
  strokeWidth: 2,
  opacity: 1,
  dashed: false,
  // Times New Roman is SPEC 11.2's default.
  font: { family: "Times New Roman", size: 14, bold: false, italic: false },
  highlightColor: rgba(255, 214, 0, 0.45),
};

/** The gesture, in page space. */
export interface Drawn {
  readonly page: number;
  readonly rect: PdfRect;
  readonly from: PdfPoint;
  readonly to: PdfPoint;
  readonly points: readonly PdfPoint[];
}

/** Smallest object a click-without-drag produces, in points. */
const MIN_SIZE = 12;

/** A shape dragged out too small to see becomes a default-sized one at the
 * click, rather than a zero-size object that cannot be selected again. */
function atLeast(rect: PdfRect, size = MIN_SIZE): PdfRect {
  const w = rect.right - rect.left;
  const h = rect.top - rect.bottom;
  return {
    left: rect.left,
    bottom: rect.bottom,
    right: rect.left + Math.max(w, size),
    top: rect.bottom + Math.max(h, size),
  };
}

function shapeStyle(style: AnnotStyle): ShapeStyle {
  return {
    fill: style.fill,
    stroke: style.color,
    stroke_width: style.strokeWidth,
    dash: [style.strokeWidth * 3, style.strokeWidth * 2],
    dashed: style.dashed,
  };
}

/**
 * Builds the object a tool produces from a gesture.
 *
 * `null` for kinds a drag cannot produce on its own: a highlight needs the text
 * selection's quads, and an image needs a file. Both are created through their
 * own paths and would be meaningless as an empty rectangle.
 */
export function objectFromDrawn(
  kind: AnnotKind,
  drawn: Drawn,
  style: AnnotStyle,
  now = 0,
): AnnotObject | null {
  let payload: AnnotPayload;
  let rect = atLeast(drawn.rect);

  switch (kind) {
    case "Rect":
    case "Ellipse":
      payload = { Shape: { style: shapeStyle(style) } };
      break;
    case "Line":
    case "Arrow":
      payload = {
        Line: {
          from: drawn.from,
          to: drawn.to,
          color: style.color,
          width: style.strokeWidth,
          dashed: style.dashed,
          // Scaled to the pen: a hairline arrow with a 14-point head looks
          // like a dart, and a fat one with a tiny head looks unfinished.
          arrow_head: kind === "Arrow" ? Math.max(6, style.strokeWidth * 4) : 0,
        },
      };
      rect = drawn.rect;
      break;
    case "Ink": {
      const points = drawn.points.length > 1 ? [...drawn.points] : [drawn.from, drawn.to];
      payload = {
        Ink: {
          strokes: [points],
          color: style.color,
          width: style.strokeWidth,
          smooth: true,
        },
      };
      rect = drawn.rect;
      break;
    }
    case "Polygon":
      payload = {
        Polygon: {
          points: drawn.points.length > 2 ? [...drawn.points] : rectCorners(rect),
          closed: true,
          style: shapeStyle(style),
        },
      };
      break;
    case "FreeText":
      payload = {
        FreeText: {
          text: "",
          font: { ...style.font },
          color: style.color,
          align: "Left",
          line_spacing: 1.3,
          background: null,
          border: null,
        },
      };
      // Big enough to hold a line of the chosen size, so the caret has
      // somewhere to be before anything is typed.
      rect = atLeast(drawn.rect, style.font.size * 6);
      break;
    case "Note":
      payload = { Note: { icon: "Comment", color: style.color, text: "" } };
      // Sticky notes are a fixed badge, not a shape you size: every reader
      // draws them at one size and a stretched one looks like a bug.
      rect = {
        left: drawn.from.x,
        bottom: drawn.from.y - 22,
        right: drawn.from.x + 18,
        top: drawn.from.y,
      };
      break;
    case "Stamp":
      payload = { Stamp: { label: "DISETUJUI", color: style.color, font: { ...style.font } } };
      rect = atLeast(drawn.rect, 90);
      break;
    case "Highlight":
    case "Underline":
    case "StrikeOut":
    case "Image":
      return null;
  }

  return {
    id: NEW_OBJECT_ID,
    page: drawn.page,
    kind,
    rect,
    rotation: 0,
    opacity: style.opacity,
    z: 0,
    locked: false,
    created_at: now,
    modified_at: now,
    author_note: "",
    payload,
  };
}

function rectCorners(rect: PdfRect): PdfPoint[] {
  return [
    { x: rect.left, y: rect.bottom },
    { x: rect.right, y: rect.bottom },
    { x: rect.right, y: rect.top },
    { x: rect.left, y: rect.top },
  ];
}

/** A text-markup object over the quads of a selection (SPEC 11.2). */
export function markupFromQuads(
  kind: "Highlight" | "Underline" | "StrikeOut",
  page: number,
  quads: readonly PdfRect[],
  style: AnnotStyle,
  now = 0,
): AnnotObject | null {
  if (quads.length === 0) return null;
  const rect = quads.reduce((a, b) => ({
    left: Math.min(a.left, b.left),
    bottom: Math.min(a.bottom, b.bottom),
    right: Math.max(a.right, b.right),
    top: Math.max(a.top, b.top),
  }));
  return {
    id: NEW_OBJECT_ID,
    page,
    kind,
    rect,
    rotation: 0,
    opacity: 1,
    z: 0,
    locked: false,
    created_at: now,
    modified_at: now,
    author_note: "",
    payload: {
      Markup: {
        quads: [...quads],
        // A highlight is a wash of colour; an underline is a line, and a
        // half-transparent line reads as a mistake.
        color: kind === "Highlight" ? style.highlightColor : style.color,
      },
    },
  };
}
