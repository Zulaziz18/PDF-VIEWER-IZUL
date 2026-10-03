/**
 * One quad per line of selected text.
 *
 * `Range.getClientRects()` returns a box per text-layer span, and for spans the
 * selection covers entirely it also returns the span's element box — the same
 * area twice. A highlight drawn over those multiplies with itself, so a line
 * came out in two shades, and the boxes are as tall as the text layer's line
 * boxes, which on tightly set headings reach into the lines above and below.
 * Both were visible in 7.0.0 (a title highlighted in overlapping bands).
 *
 * The rules, in PDF space (y up):
 *
 * 1. A box inside another box is dropped.
 * 2. Boxes that share most of their height and sit side by side, with a gap of
 *    no more than a line height, are one line. Two columns at the same height
 *    stay apart because their gap is wider.
 * 3. Neighbouring lines that still overlap vertically are split at the middle
 *    of the overlap, so no two quads cover the same strip.
 */

import type { PdfRect } from "./types";

const EPSILON = 0.01;

function height(r: PdfRect): number {
  return r.top - r.bottom;
}

function contains(outer: PdfRect, inner: PdfRect): boolean {
  return (
    outer.left <= inner.left + EPSILON &&
    outer.right >= inner.right - EPSILON &&
    outer.bottom <= inner.bottom + EPSILON &&
    outer.top >= inner.top - EPSILON
  );
}

function verticalOverlap(a: PdfRect, b: PdfRect): number {
  return Math.min(a.top, b.top) - Math.max(a.bottom, b.bottom);
}

function horizontalGap(a: PdfRect, b: PdfRect): number {
  return Math.max(a.left, b.left) - Math.min(a.right, b.right);
}

function union(a: PdfRect, b: PdfRect): PdfRect {
  return {
    left: Math.min(a.left, b.left),
    bottom: Math.min(a.bottom, b.bottom),
    right: Math.max(a.right, b.right),
    top: Math.max(a.top, b.top),
  };
}

/** Same line: most of the shorter box's height shared, and close enough sideways. */
function sameLine(a: PdfRect, b: PdfRect): boolean {
  const shorter = Math.min(height(a), height(b));
  if (shorter <= 0) return false;
  if (verticalOverlap(a, b) < shorter * 0.5) return false;
  return horizontalGap(a, b) <= Math.max(height(a), height(b));
}

export function lineQuads(quads: readonly PdfRect[]): PdfRect[] {
  const boxes = quads.filter((q) => q.right > q.left && q.top > q.bottom);
  const kept = boxes.filter(
    (q, i) => !boxes.some((o, j) => j !== i && contains(o, q) && (!contains(q, o) || j < i)),
  );

  // Merge until nothing changes: a merged line can reach a box that neither of
  // its parts reached on its own.
  let lines = [...kept];
  let merged = true;
  while (merged) {
    merged = false;
    outer: for (let i = 0; i < lines.length; i++) {
      for (let j = i + 1; j < lines.length; j++) {
        const a = lines[i] as PdfRect;
        const b = lines[j] as PdfRect;
        if (sameLine(a, b)) {
          lines[i] = union(a, b);
          lines.splice(j, 1);
          merged = true;
          break outer;
        }
      }
    }
  }

  // Top to bottom, then trim each pair of lines that still overlap.
  type Mutable = { -readonly [K in keyof PdfRect]: PdfRect[K] };
  const sorted: Mutable[] = lines
    .sort((a, b) => b.top - a.top || a.left - b.left)
    .map((l) => ({ ...l }));
  for (let i = 0; i < sorted.length; i++) {
    for (let j = i + 1; j < sorted.length; j++) {
      const upper = sorted[i] as Mutable;
      const lower = sorted[j] as Mutable;
      if (horizontalGap(upper, lower) >= 0) continue;
      if (verticalOverlap(upper, lower) <= 0) continue;
      const mid = (upper.bottom + lower.top) / 2;
      upper.bottom = Math.max(upper.bottom, mid);
      lower.top = Math.min(lower.top, mid);
    }
  }
  return sorted.filter((l) => l.top > l.bottom);
}
