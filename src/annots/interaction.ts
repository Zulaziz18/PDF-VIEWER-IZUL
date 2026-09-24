/**
 * Selecting, moving and resizing annotations (SPEC 11.2's "objek hidup").
 *
 * Everything here is pure arithmetic in **PDF user space**: points, origin at
 * the page's bottom-left, y upwards. Nothing takes a pixel. That is not a
 * stylistic choice — an annotation whose position were decided in pixels would
 * move the moment the zoom changed, and the bug would only show after a save
 * (SPEC 8). The single conversion to pixels happens in the canvas backend.
 *
 * Keeping the rules here rather than inside a pointer handler is what makes
 * them testable: "does a click 3 points outside a hairline still grab it" is a
 * question with one right answer, and it should not need a mouse to ask.
 */

import type { AnnotObject, PdfPoint, PdfRect } from "./types";

/** The eight resize handles, plus the one that rotates. */
export type HandleId =
  | "nw"
  | "n"
  | "ne"
  | "e"
  | "se"
  | "s"
  | "sw"
  | "w"
  | "rotate";

export const RESIZE_HANDLES: readonly HandleId[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

/** Handle size in points at 100 % zoom. Scaled by the viewport when drawn. */
export const HANDLE_SIZE = 8;

/** How far above the object's top edge the rotate handle floats, in points. */
export const ROTATE_OFFSET = 18;

/**
 * How close a click has to be to count as a hit on something thin.
 *
 * A hairline stroke is a fraction of a point wide and nobody can click it. The
 * tolerance is in points and therefore shrinks on screen as the user zooms out,
 * which is the right way round: at 25 % zoom a 3-point slack is under a pixel
 * of pointer movement, and at 400 % it is a comfortable target.
 */
export const HIT_SLACK = 3;

export function handlePoint(rect: PdfRect, handle: HandleId): PdfPoint {
  const midX = (rect.left + rect.right) / 2;
  const midY = (rect.bottom + rect.top) / 2;
  switch (handle) {
    case "nw":
      return { x: rect.left, y: rect.top };
    case "n":
      return { x: midX, y: rect.top };
    case "ne":
      return { x: rect.right, y: rect.top };
    case "e":
      return { x: rect.right, y: midY };
    case "se":
      return { x: rect.right, y: rect.bottom };
    case "s":
      return { x: midX, y: rect.bottom };
    case "sw":
      return { x: rect.left, y: rect.bottom };
    case "w":
      return { x: rect.left, y: midY };
    case "rotate":
      return { x: midX, y: rect.top + ROTATE_OFFSET };
  }
}

/** The handles an object offers. A redaction mark has no rotate handle: what
 * is applied is its upright quads, and a mark turned on screen would cover
 * something other than what goes. */
export function handlesFor(kind: AnnotObject["kind"]): HandleId[] {
  return kind === "Redact" ? [...RESIZE_HANDLES] : [...RESIZE_HANDLES, "rotate"];
}

/** The handle under `point`, if any. `scale` is pixels per point, so the grab
 * area stays the same size on screen at every zoom. */
export function handleAt(
  rect: PdfRect,
  point: PdfPoint,
  scale: number,
  handles: readonly HandleId[] = [...RESIZE_HANDLES, "rotate"],
): HandleId | null {
  const reach = (HANDLE_SIZE / Math.max(scale, 0.01)) * 0.75;
  for (const handle of handles) {
    const p = handlePoint(rect, handle);
    if (Math.abs(p.x - point.x) <= reach && Math.abs(p.y - point.y) <= reach) {
      return handle;
    }
  }
  return null;
}

/** Distance from a point to a segment, in points. */
function distanceToSegment(p: PdfPoint, a: PdfPoint, b: PdfPoint): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const lengthSq = dx * dx + dy * dy;
  if (lengthSq === 0) return Math.hypot(p.x - a.x, p.y - a.y);
  const t = Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / lengthSq));
  return Math.hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy));
}

/** Rotates `p` about `origin` by `-degrees`, i.e. into the object's own frame. */
function unrotate(p: PdfPoint, origin: PdfPoint, degrees: number): PdfPoint {
  if (Math.abs(degrees) < 1e-6) return p;
  const rad = (-degrees * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  const dx = p.x - origin.x;
  const dy = p.y - origin.y;
  return { x: origin.x + dx * cos - dy * sin, y: origin.y + dx * sin + dy * cos };
}

function centre(rect: PdfRect): PdfPoint {
  return { x: (rect.left + rect.right) / 2, y: (rect.bottom + rect.top) / 2 };
}

function inflated(rect: PdfRect, d: number): PdfRect {
  return {
    left: rect.left - d,
    bottom: rect.bottom - d,
    right: rect.right + d,
    top: rect.top + d,
  };
}

function inside(rect: PdfRect, p: PdfPoint): boolean {
  return p.x >= rect.left && p.x <= rect.right && p.y >= rect.bottom && p.y <= rect.top;
}

/**
 * Whether `point` hits `obj`.
 *
 * Shapes are hit by their box; ink, lines and polygons by their actual geometry
 * plus a slack, because the box of a diagonal line is mostly empty space and
 * clicking that empty space to select the line is how you end up dragging the
 * wrong object.
 */
export function hitTest(obj: AnnotObject, point: PdfPoint, slack = HIT_SLACK): boolean {
  const local = unrotate(point, centre(obj.rect), obj.rotation);
  const payload = obj.payload;
  if ("Ink" in payload) {
    return payload.Ink.strokes.some((stroke) =>
      stroke.some((p, i) => i > 0 && distanceToSegment(local, stroke[i - 1] as PdfPoint, p) <= slack + payload.Ink.width / 2),
    );
  }
  if ("Line" in payload) {
    return (
      distanceToSegment(local, payload.Line.from, payload.Line.to) <=
      slack + payload.Line.width / 2
    );
  }
  if ("Polygon" in payload) {
    const pts = payload.Polygon.points;
    if (payload.Polygon.style.fill && inside(obj.rect, local)) return true;
    return pts.some((p, i) => {
      const prev = i === 0 ? (payload.Polygon.closed ? pts[pts.length - 1] : undefined) : pts[i - 1];
      return prev !== undefined && distanceToSegment(local, prev, p) <= slack;
    });
  }
  if ("Markup" in payload) {
    return payload.Markup.quads.some((q) => inside(inflated(q, slack), local));
  }
  return inside(inflated(obj.rect, slack), local);
}

/** The topmost object at `point`, or `null`. Locked objects are skipped: they
 * are locked so that clicking through them is possible (SPEC 11.2). */
export function pick(objects: readonly AnnotObject[], point: PdfPoint, slack = HIT_SLACK): AnnotObject | null {
  for (let i = objects.length - 1; i >= 0; i--) {
    const obj = objects[i] as AnnotObject;
    if (obj.locked) continue;
    if (hitTest(obj, point, slack)) return obj;
  }
  return null;
}

/** Every object whose box is inside a rubber-band rectangle. */
export function pickInside(objects: readonly AnnotObject[], band: PdfRect): AnnotObject[] {
  const norm = normalise(band);
  return objects.filter(
    (o) =>
      !o.locked &&
      o.rect.left >= norm.left &&
      o.rect.right <= norm.right &&
      o.rect.bottom >= norm.bottom &&
      o.rect.top <= norm.top,
  );
}

/** A rectangle dragged in any direction, put the right way up. */
export function normalise(rect: PdfRect): PdfRect {
  return {
    left: Math.min(rect.left, rect.right),
    right: Math.max(rect.left, rect.right),
    bottom: Math.min(rect.bottom, rect.top),
    top: Math.max(rect.bottom, rect.top),
  };
}

/** The union of several objects' boxes — the selection frame. */
export function boundsOf(objects: readonly AnnotObject[]): PdfRect | null {
  if (objects.length === 0) return null;
  let out: PdfRect | null = null;
  for (const o of objects) {
    out =
      out === null
        ? o.rect
        : {
            left: Math.min(out.left, o.rect.left),
            bottom: Math.min(out.bottom, o.rect.bottom),
            right: Math.max(out.right, o.rect.right),
            top: Math.max(out.top, o.rect.top),
          };
  }
  return out;
}

/**
 * The new box after dragging `handle` by `(dx, dy)`.
 *
 * The opposite corner stays put — that is what a resize handle means — and the
 * box is never allowed to collapse or turn inside out: a zero-width annotation
 * is unselectable, and a negative one draws inverted in one backend and not the
 * other.
 */
export function resized(rect: PdfRect, handle: HandleId, dx: number, dy: number, min = 4): PdfRect {
  let { left, bottom, right, top } = rect;
  if (handle.includes("w")) left += dx;
  if (handle.includes("e")) right += dx;
  if (handle.includes("s")) bottom += dy;
  if (handle.includes("n")) top += dy;
  if (right - left < min) {
    if (handle.includes("w")) left = right - min;
    else right = left + min;
  }
  if (top - bottom < min) {
    if (handle.includes("s")) bottom = top - min;
    else top = bottom + min;
  }
  return { left, bottom, right, top };
}

/** The scale factors and pivot that turn `before` into `after`. */
export function scaleBetween(
  before: PdfRect,
  after: PdfRect,
): { origin: PdfPoint; sx: number; sy: number } {
  const width = before.right - before.left;
  const height = before.top - before.bottom;
  const sx = width === 0 ? 1 : (after.right - after.left) / width;
  const sy = height === 0 ? 1 : (after.top - after.bottom) / height;
  // The pivot is the point that did not move: solving `after = origin + (before
  // - origin) * s` for the origin.
  const originX = sx === 1 ? before.left : (after.left - sx * before.left) / (1 - sx);
  const originY = sy === 1 ? before.bottom : (after.bottom - sy * before.bottom) / (1 - sy);
  return { origin: { x: originX, y: originY }, sx, sy };
}

/** The angle, in degrees, from an object's centre to a pointer — what the
 * rotate handle reads. Zero points straight up, matching the handle's rest
 * position. */
export function angleTo(rect: PdfRect, point: PdfPoint): number {
  const c = centre(rect);
  const degrees = (Math.atan2(point.y - c.y, point.x - c.x) * 180) / Math.PI;
  return degrees - 90;
}

/** Snaps to 15° while a modifier is held, the way every editor does. */
export function snapAngle(degrees: number, snap: boolean): number {
  if (!snap) return degrees;
  return Math.round(degrees / 15) * 15;
}
