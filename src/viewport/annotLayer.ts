/**
 * Where an annotation sits on screen, and which page a click landed on.
 *
 * The conversion between PDF user space and the viewport's content space is the
 * one place pixels and points meet (SPEC 8), so it lives in one small module
 * with tests rather than inline in a pointer handler. Two mistakes are easy to
 * make here and both are invisible until something is saved: forgetting the y
 * flip, and forgetting that "content space" already includes the scroll offset.
 */

import type { Layout } from "./layout";
import { boxOf, visiblePages } from "./layout";
import type { PageMetrics, Rotation } from "./geometry";
import { displaySize } from "./geometry";
import type { PdfPoint } from "@/annots/types";

/** A point in the scrolling element's content space, in CSS pixels. */
export interface ContentPoint {
  readonly x: number;
  readonly y: number;
}

/** Where a page's top-left corner is in content space, and how big it is. */
export interface PageBox {
  readonly page: number;
  readonly x: number;
  readonly y: number;
  /** CSS pixels per PDF point. */
  readonly zoom: number;
  /** Page size in points, after rotation. */
  readonly widthPt: number;
  readonly heightPt: number;
}

export function pageBox(
  layout: Layout,
  page: number,
  sizes: readonly PageMetrics[],
  rotationOf: (page: number) => Rotation,
): PageBox | null {
  const box = boxOf(layout, page);
  const metrics = sizes[page];
  if (!box || !metrics) return null;
  const size = displaySize(metrics, rotationOf(page));
  return {
    page,
    x: box.x,
    y: box.y,
    zoom: layout.zoom,
    widthPt: size.width,
    heightPt: size.height,
  };
}

/**
 * Content-space pixels to PDF points on that page.
 *
 * The y flip is the whole of it: content space counts down from the page's top
 * edge, PDF counts up from its bottom.
 */
export function toPagePoint(box: PageBox, point: ContentPoint): PdfPoint {
  return {
    x: (point.x - box.x) / box.zoom,
    y: box.heightPt - (point.y - box.y) / box.zoom,
  };
}

/** PDF points back to content-space pixels. */
export function toContentPoint(box: PageBox, point: PdfPoint): ContentPoint {
  return {
    x: box.x + point.x * box.zoom,
    y: box.y + (box.heightPt - point.y) * box.zoom,
  };
}

/** Whether a content-space point is inside the page's own area. */
export function within(box: PageBox, point: ContentPoint): boolean {
  return (
    point.x >= box.x &&
    point.x <= box.x + box.widthPt * box.zoom &&
    point.y >= box.y &&
    point.y <= box.y + box.heightPt * box.zoom
  );
}

/**
 * The page under a content-space point.
 *
 * Returns the nearest visible page when the click landed in the gap between
 * two: a drag that starts on a page and wanders into the margin has to keep
 * working, and "no page" in the middle of a gesture would end it.
 */
export function pageAt(
  layout: Layout,
  view: { x: number; y: number; w: number; h: number },
  sizes: readonly PageMetrics[],
  rotationOf: (page: number) => Rotation,
  point: ContentPoint,
): PageBox | null {
  const pages = visiblePages(layout, view, 0);
  let nearest: PageBox | null = null;
  let best = Infinity;
  for (const page of pages) {
    const box = pageBox(layout, page, sizes, rotationOf);
    if (!box) continue;
    if (within(box, point)) return box;
    const cy = box.y + (box.heightPt * box.zoom) / 2;
    const distance = Math.abs(point.y - cy);
    if (distance < best) {
      best = distance;
      nearest = box;
    }
  }
  return nearest;
}
