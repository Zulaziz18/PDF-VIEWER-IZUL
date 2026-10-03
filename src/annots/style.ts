/**
 * Text style and image crop, as edits of an object (Phase 8, the SPEC 11.2
 * audit). The model always had bold, italic, alignment, line spacing and a
 * crop rectangle; nothing in the interface reached them.
 *
 * Pure, for the tests: each function returns a new object and leaves the one
 * it was given alone.
 */

import type { AnnotObject, FontSpec, PdfRect, TextAlign } from "./types";

/** What the text controls show for an object, or `null` for one without text
 * style (only a text box has alignment and spacing). */
export interface TextStyle {
  readonly bold: boolean;
  readonly italic: boolean;
  readonly align: TextAlign;
  readonly lineSpacing: number;
}

export function textStyleOf(obj: AnnotObject): TextStyle | null {
  const p = obj.payload;
  if (!("FreeText" in p)) return null;
  const f = p.FreeText;
  return { bold: f.font.bold, italic: f.font.italic, align: f.align, lineSpacing: f.line_spacing };
}

/** The same object with `patch` applied to its text style. */
export function withTextStyle(obj: AnnotObject, patch: Partial<TextStyle>): AnnotObject {
  const next = structuredClone(obj);
  const p = next.payload;
  if (!("FreeText" in p)) return next;
  const f = p.FreeText;
  const font: FontSpec = { ...f.font };
  if (patch.bold !== undefined) font.bold = patch.bold;
  if (patch.italic !== undefined) font.italic = patch.italic;
  f.font = font;
  if (patch.align !== undefined) f.align = patch.align;
  if (patch.lineSpacing !== undefined) f.line_spacing = Math.min(Math.max(patch.lineSpacing, 0.8), 3);
  return next;
}

export type CropEdge = "left" | "right" | "top" | "bottom";

/** How much of each edge of the picture is cut away, 0..1 of its own size. */
export interface CropAmounts {
  readonly left: number;
  readonly right: number;
  readonly top: number;
  readonly bottom: number;
}

/** The least of a picture that stays: cropping it to nothing is deleting it. */
export const MIN_KEPT = 0.05;

export function cropOf(obj: AnnotObject): CropAmounts | null {
  const p = obj.payload;
  if (!("Image" in p)) return null;
  const c = p.Image.crop;
  return { left: c.left, right: 1 - c.right, top: 1 - c.top, bottom: c.bottom };
}

/**
 * The picture with `edge` cut by `amount`, and its box resized with it, so
 * what stays is drawn at the same size and in the same place — a crop that
 * stretched the rest over the old box would be a resize.
 *
 * The box is the picture's unrotated frame: on a rotated picture the kept
 * part stays the same size, but the frame's centre moves with the cut.
 */
export function withCrop(obj: AnnotObject, edge: CropEdge, amount: number): AnnotObject {
  const next = structuredClone(obj);
  const p = next.payload;
  if (!("Image" in p)) return next;
  const old = p.Image.crop;
  const c: { -readonly [K in keyof PdfRect]: PdfRect[K] } = { ...old };
  const a = Math.min(Math.max(amount, 0), 1);
  if (edge === "left") c.left = Math.min(a, c.right - MIN_KEPT);
  if (edge === "right") c.right = Math.max(1 - a, c.left + MIN_KEPT);
  if (edge === "bottom") c.bottom = Math.min(a, c.top - MIN_KEPT);
  if (edge === "top") c.top = Math.max(1 - a, c.bottom + MIN_KEPT);
  // Points per unit of the picture, from the box as it is.
  const sx = (obj.rect.right - obj.rect.left) / Math.max(old.right - old.left, 1e-6);
  const sy = (obj.rect.top - obj.rect.bottom) / Math.max(old.top - old.bottom, 1e-6);
  next.rect = {
    left: obj.rect.left + (c.left - old.left) * sx,
    right: obj.rect.right + (c.right - old.right) * sx,
    bottom: obj.rect.bottom + (c.bottom - old.bottom) * sy,
    top: obj.rect.top + (c.top - old.top) * sy,
  };
  p.Image.crop = c;
  return next;
}
