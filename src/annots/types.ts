/**
 * The annotation model, as it arrives from Rust.
 *
 * These are mirrors of `izul-model`'s types under serde's default encoding, and
 * they are deliberately *only* mirrors: nothing here computes geometry. The
 * display list is built by one pure function in Rust and consumed by two
 * backends — the appearance-stream writer and the canvas renderer next door —
 * and the moment this file starts deciding where a corner goes, the parity
 * guarantee SPEC 3.2 rests on is gone.
 *
 * Enums arrive externally tagged, which is what serde does by default and what
 * `postcard` (used on the IPC channel) can encode: `FillPath { .. }` becomes
 * `{ FillPath: { .. } }`, and a unit variant becomes the bare string
 * `"NonZero"`. That shape is a little awkward to read in TypeScript and is kept
 * anyway, because the alternative — an internally tagged representation — is
 * one `postcard` cannot serialise at all.
 */

export interface PdfPoint {
  readonly x: number;
  readonly y: number;
}

export interface PdfRect {
  readonly left: number;
  readonly bottom: number;
  readonly right: number;
  readonly top: number;
}

export interface Rgba {
  readonly r: number;
  readonly g: number;
  readonly b: number;
  readonly a: number;
}

/** PDF's `[a b c d e f]`, mapping `(x,y)` to `(ax+cy+e, bx+dy+f)`. */
export interface Matrix {
  readonly a: number;
  readonly b: number;
  readonly c: number;
  readonly d: number;
  readonly e: number;
  readonly f: number;
}

export type FillRule = "NonZero" | "EvenOdd";
export type BlendMode = "Normal" | "Multiply" | "Screen" | "Darken" | "Lighten";
export type LineCap = "Butt" | "Round" | "Square";
export type LineJoin = "Miter" | "Round" | "Bevel";

export interface StrokeStyle {
  readonly width: number;
  readonly cap: LineCap;
  readonly join: LineJoin;
  readonly miter_limit: number;
  readonly dash: number[];
  readonly dash_phase: number;
}

export type PathSeg =
  | { readonly MoveTo: PdfPoint }
  | { readonly LineTo: PdfPoint }
  | { readonly CurveTo: { readonly c1: PdfPoint; readonly c2: PdfPoint; readonly to: PdfPoint } }
  | "Close";

export interface Path {
  readonly segs: readonly PathSeg[];
}

export interface PositionedGlyph {
  readonly glyph_id: number;
  readonly unicode: string;
  readonly offset: PdfPoint;
}

export type DisplayOp =
  | {
      readonly FillPath: {
        readonly path: Path;
        readonly color: Rgba;
        readonly rule: FillRule;
        readonly blend: BlendMode;
      };
    }
  | {
      readonly StrokePath: {
        readonly path: Path;
        readonly color: Rgba;
        readonly style: StrokeStyle;
        readonly blend: BlendMode;
      };
    }
  | {
      readonly DrawText: {
        readonly glyphs: readonly PositionedGlyph[];
        readonly font: number;
        readonly size: number;
        readonly matrix: Matrix;
        readonly color: Rgba;
        readonly blend: BlendMode;
      };
    }
  | {
      readonly DrawImage: {
        readonly image: number;
        readonly matrix: Matrix;
        readonly opacity: number;
      };
    }
  | { readonly PushClip: { readonly path: Path; readonly rule: FillRule } }
  | "PopClip"
  | { readonly PushTransform: { readonly matrix: Matrix } }
  | "PopTransform";

export interface DisplayList {
  readonly ops: readonly DisplayOp[];
}

/** One object's list, as `annot_display_lists` returns it. */
export interface DisplayListOut {
  readonly id: number;
  readonly ops: DisplayList;
}

/** The thirteen kinds of SPEC 11.2. */
export type AnnotKind =
  | "Highlight"
  | "Underline"
  | "StrikeOut"
  | "FreeText"
  | "Image"
  | "Ink"
  | "Line"
  | "Arrow"
  | "Rect"
  | "Ellipse"
  | "Polygon"
  | "Note"
  | "Stamp";

export const ANNOT_KINDS: readonly AnnotKind[] = [
  "Highlight",
  "Underline",
  "StrikeOut",
  "FreeText",
  "Image",
  "Ink",
  "Line",
  "Arrow",
  "Rect",
  "Ellipse",
  "Polygon",
  "Note",
  "Stamp",
];

export type TextAlign = "Left" | "Center" | "Right" | "Justify";
export type NoteIcon = "Comment" | "Note" | "Help";

export interface FontSpec {
  family: string;
  size: number;
  bold: boolean;
  italic: boolean;
}

export interface ShapeStyle {
  fill: Rgba | null;
  stroke: Rgba | null;
  stroke_width: number;
  dash: [number, number];
  dashed: boolean;
}

export type AnnotPayload =
  | { Markup: { quads: PdfRect[]; color: Rgba } }
  | {
      FreeText: {
        text: string;
        font: FontSpec;
        color: Rgba;
        align: TextAlign;
        line_spacing: number;
        background: Rgba | null;
        border: Rgba | null;
      };
    }
  | { Image: { image: number; crop: PdfRect; opacity: number } }
  | { Ink: { strokes: PdfPoint[][]; color: Rgba; width: number; smooth: boolean } }
  | {
      Line: {
        from: PdfPoint;
        to: PdfPoint;
        color: Rgba;
        width: number;
        dashed: boolean;
        arrow_head: number;
      };
    }
  | { Shape: { style: ShapeStyle } }
  | { Polygon: { points: PdfPoint[]; closed: boolean; style: ShapeStyle } }
  | { Note: { icon: NoteIcon; color: Rgba; text: string } }
  | { Stamp: { label: string; color: Rgba; font: FontSpec } };

export interface AnnotObject {
  id: number;
  page: number;
  kind: AnnotKind;
  rect: PdfRect;
  rotation: number;
  opacity: number;
  z: number;
  locked: boolean;
  created_at: number;
  modified_at: number;
  author_note: string;
  payload: AnnotPayload;
}

/** What every editing command returns. */
export interface EditResult {
  readonly objects: AnnotObject[];
  readonly can_undo: boolean;
  readonly can_redo: boolean;
  /** The document has changes its file does not (Phase 4). */
  readonly dirty: boolean;
}

/** `id` is a `u64` in Rust but a plain number here; annotation ids never get
 * near 2^53, and the alternative — `bigint` over the invoke bridge — would
 * infect every call site for a limit no document will reach. */
export const NEW_OBJECT_ID = 0;

export function rgba(r: number, g: number, b: number, a = 1): Rgba {
  return { r: r / 255, g: g / 255, b: b / 255, a };
}

/** CSS colour for a model colour, alpha included. */
export function cssColor(c: Rgba): string {
  const to255 = (v: number): number => Math.round(Math.min(1, Math.max(0, v)) * 255);
  return `rgba(${to255(c.r)}, ${to255(c.g)}, ${to255(c.b)}, ${c.a})`;
}
