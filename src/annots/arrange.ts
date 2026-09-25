/**
 * Aligning and distributing several objects (SPEC 11.2: "Seleksi jamak,
 * ratakan, distribusikan, kunci"), found missing in the Phase 8 audit.
 *
 * As in an office suite, the reference is the selection itself: "align left"
 * brings every object to the leftmost left edge among them, "distribute"
 * keeps the two outermost where they are and makes the gaps between all of
 * them equal. A locked object is never moved — it still counts as part of the
 * selection's bounds, so locking one picture and aligning the rest to it is
 * how a user says "line these up with that".
 *
 * Objects on different pages are arranged page by page: aligning a box on
 * page 2 with one on page 5 means nothing.
 *
 * Pure: returns the moved copies of the objects that moved.
 */

import { translateObject } from "./interaction";
import type { AnnotObject } from "./types";

export type Alignment = "left" | "centerH" | "right" | "top" | "centerV" | "bottom";
export type Axis = "horizontal" | "vertical";

function moved(obj: AnnotObject, dx: number, dy: number): AnnotObject | null {
  if (obj.locked || (Math.abs(dx) < 1e-6 && Math.abs(dy) < 1e-6)) return null;
  const next = structuredClone(obj);
  translateObject(next, dx, dy);
  return next;
}

function byPage(objects: readonly AnnotObject[]): AnnotObject[][] {
  const pages = new Map<number, AnnotObject[]>();
  for (const o of objects) pages.set(o.page, [...(pages.get(o.page) ?? []), o]);
  return [...pages.values()];
}

/** The objects `align` moves, moved. */
export function align(objects: readonly AnnotObject[], how: Alignment): AnnotObject[] {
  const out: AnnotObject[] = [];
  for (const group of byPage(objects)) {
    if (group.length < 2) continue;
    const left = Math.min(...group.map((o) => o.rect.left));
    const right = Math.max(...group.map((o) => o.rect.right));
    const bottom = Math.min(...group.map((o) => o.rect.bottom));
    const top = Math.max(...group.map((o) => o.rect.top));
    for (const o of group) {
      const r = o.rect;
      const dx =
        how === "left"
          ? left - r.left
          : how === "right"
            ? right - r.right
            : how === "centerH"
              ? (left + right) / 2 - (r.left + r.right) / 2
              : 0;
      const dy =
        how === "bottom"
          ? bottom - r.bottom
          : how === "top"
            ? top - r.top
            : how === "centerV"
              ? (bottom + top) / 2 - (r.bottom + r.top) / 2
              : 0;
      const m = moved(o, dx, dy);
      if (m) out.push(m);
    }
  }
  return out;
}

/**
 * Equal gaps along `axis`, the outermost two left where they are. Needs three
 * objects on a page: with two there is one gap and nothing to even out.
 */
export function distribute(objects: readonly AnnotObject[], axis: Axis): AnnotObject[] {
  const out: AnnotObject[] = [];
  const lo = (o: AnnotObject): number => (axis === "horizontal" ? o.rect.left : o.rect.bottom);
  const hi = (o: AnnotObject): number => (axis === "horizontal" ? o.rect.right : o.rect.top);
  for (const group of byPage(objects)) {
    if (group.length < 3) continue;
    const sorted = [...group].sort((a, b) => lo(a) + hi(a) - (lo(b) + hi(b)));
    const first = sorted[0] as AnnotObject;
    const last = sorted[sorted.length - 1] as AnnotObject;
    const span = hi(last) - lo(first);
    const sizes = sorted.reduce((sum, o) => sum + (hi(o) - lo(o)), 0);
    const gap = (span - sizes) / (sorted.length - 1);
    let at = lo(first);
    for (const o of sorted) {
      const d = at - lo(o);
      const m = axis === "horizontal" ? moved(o, d, 0) : moved(o, 0, d);
      if (m) out.push(m);
      at += hi(o) - lo(o) + gap;
    }
  }
  return out;
}
