/**
 * Where every page sits, and which of them are on screen (SPEC 11.1).
 *
 * This is what makes the scroll virtualized: the layout is computed once per
 * zoom or view-mode change for the whole document, and the per-frame question —
 * "which pages does this scroll position touch?" — is answered by binary search
 * over rows rather than by walking the document. A 500-page file and a 5-page
 * file cost the same per frame.
 *
 * Placeholders are sized from the real page dimensions, which the backend reads
 * out of the page tree at open time, so the scroll bar has its final size before
 * a single page has been rendered and never jumps (SPEC 11.1).
 *
 * Pure: no DOM, no canvas, no React.
 */

import type { PageMetrics, Rotation } from "./geometry";
import { displaySize } from "./geometry";

/** How pages are arranged (SPEC 11.1). */
export type ViewMode = "single" | "dual" | "dual_cover" | "horizontal";

export const VIEW_MODES: readonly ViewMode[] = ["single", "dual", "dual_cover", "horizontal"];

export function parseViewMode(value: string): ViewMode {
  return (VIEW_MODES as readonly string[]).includes(value) ? (value as ViewMode) : "single";
}

/** A page's box in content space: CSS pixels, origin at the content's top-left. */
export interface PageBox {
  readonly page: number;
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

/** One row of the layout: a page, or a facing pair. */
interface Row {
  /** Start of the row along the scroll axis. */
  readonly start: number;
  readonly end: number;
  readonly first: number;
  /** Exclusive. */
  readonly last: number;
}

export interface Layout {
  readonly pages: readonly PageBox[];
  readonly rows: readonly Row[];
  readonly width: number;
  readonly height: number;
  readonly mode: ViewMode;
  readonly zoom: number;
  /** True when scrolling runs left to right instead of top to bottom. */
  readonly horizontal: boolean;
}

export interface LayoutInput {
  readonly pageSizes: readonly PageMetrics[];
  readonly zoom: number;
  readonly mode: ViewMode;
  /** Effective rotation per page, already including the document's own. */
  readonly rotationOf: (page: number) => Rotation;
  /** Gap between pages, in CSS pixels. */
  readonly gap: number;
  /** Padding around the whole document, in CSS pixels. */
  readonly padding: number;
  /** Viewport width in CSS pixels, used to centre narrow documents. */
  readonly viewportWidth: number;
}

/** How pages group into rows for a view mode. */
function groupsFor(mode: ViewMode, count: number): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  if (mode === "single" || mode === "horizontal") {
    for (let i = 0; i < count; i++) out.push([i, i + 1]);
    return out;
  }
  // "Two pages with cover" puts page 1 alone, so that every later spread has
  // the odd page on the right, the way a printed book falls open.
  let i = 0;
  if (mode === "dual_cover" && count > 0) {
    out.push([0, 1]);
    i = 1;
  }
  for (; i < count; i += 2) {
    out.push([i, Math.min(i + 2, count)]);
  }
  return out;
}

/**
 * Lays out the whole document.
 *
 * O(pages), run on zoom, rotation, view-mode and window-size changes — not per
 * frame.
 */
export function layoutDocument(input: LayoutInput): Layout {
  const { pageSizes, zoom, mode, rotationOf, gap, padding, viewportWidth } = input;
  const horizontal = mode === "horizontal";
  const groups = groupsFor(mode, pageSizes.length);

  const sizes = pageSizes.map((size, i) => {
    const d = displaySize(size, rotationOf(i));
    return { w: Math.max(1, d.width * zoom), h: Math.max(1, d.height * zoom) };
  });

  const pages: PageBox[] = [];
  const rows: Row[] = [];

  if (horizontal) {
    // One row, pages side by side; the tallest page sets the strip's height.
    let x = padding;
    const tallest = sizes.reduce((m, s) => Math.max(m, s.h), 0);
    for (const [first, last] of groups) {
      const start = x;
      for (let p = first; p < last; p++) {
        const s = sizes[p];
        if (!s) continue;
        // Vertically centred, so pages of different heights sit on one line.
        pages.push({ page: p, x, y: padding + (tallest - s.h) / 2, w: s.w, h: s.h });
        x += s.w + gap;
      }
      rows.push({ start, end: x - gap, first, last });
    }
    const width = Math.max(x - gap + padding, viewportWidth);
    return {
      pages,
      rows,
      width,
      height: tallest + padding * 2,
      mode,
      zoom,
      horizontal: true,
    };
  }

  // Vertical modes. Rows are stacked; each row is centred in the content width,
  // which is itself the widest row (or the viewport, whichever is larger).
  const rowWidths = groups.map(([first, last]) => {
    let w = 0;
    for (let p = first; p < last; p++) w += (sizes[p]?.w ?? 0) + gap;
    return Math.max(0, w - gap);
  });
  const widest = rowWidths.reduce((m, w) => Math.max(m, w), 0);
  const contentWidth = Math.max(widest + padding * 2, viewportWidth);

  let y = padding;
  groups.forEach(([first, last], index) => {
    const rowWidth = rowWidths[index] ?? 0;
    let x = (contentWidth - rowWidth) / 2;
    let tallest = 0;
    for (let p = first; p < last; p++) {
      const s = sizes[p];
      if (!s) continue;
      pages.push({ page: p, x, y, w: s.w, h: s.h });
      x += s.w + gap;
      tallest = Math.max(tallest, s.h);
    }
    rows.push({ start: y, end: y + tallest, first, last });
    y += tallest + gap;
  });

  return {
    pages,
    rows,
    width: contentWidth,
    height: Math.max(y - gap + padding, 1),
    mode,
    zoom,
    horizontal: false,
  };
}

/** The visible window, in content-space CSS pixels. */
export interface ViewRect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

/**
 * Index of the first row whose end is at or past `offset`.
 *
 * Binary search: this is the call that keeps a frame's cost independent of the
 * document's length.
 */
function firstRowAt(rows: readonly Row[], offset: number): number {
  let lo = 0;
  let hi = rows.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    const row = rows[mid];
    if (row && row.end < offset) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/**
 * The pages intersecting a view rectangle, plus `margin` CSS pixels of slack on
 * each side along the scroll axis.
 *
 * The margin is what stops a page from appearing only after its top edge has
 * already crossed into view.
 */
export function visiblePages(layout: Layout, view: ViewRect, margin = 0): number[] {
  const axisStart = (layout.horizontal ? view.x : view.y) - margin;
  const axisEnd = (layout.horizontal ? view.x + view.w : view.y + view.h) + margin;
  const out: number[] = [];
  for (let i = firstRowAt(layout.rows, axisStart); i < layout.rows.length; i++) {
    const row = layout.rows[i];
    if (!row || row.start > axisEnd) break;
    for (let p = row.first; p < row.last; p++) out.push(p);
  }
  return out;
}

/** A page's box, or undefined if the index is out of range. */
export function boxOf(layout: Layout, page: number): PageBox | undefined {
  // `pages` is built in page order for every mode, so the index is the page.
  const box = layout.pages[page];
  return box && box.page === page ? box : layout.pages.find((b) => b.page === page);
}

/**
 * The page the user would say they are looking at.
 *
 * The one covering the most of the viewport, rather than the first one to
 * intersect it: with two-page spreads and tall pages, "first intersecting" puts
 * the page number one behind what the eye reports.
 */
export function dominantPage(layout: Layout, view: ViewRect): number {
  let best = 0;
  let bestArea = -1;
  for (const page of visiblePages(layout, view)) {
    const box = boxOf(layout, page);
    if (!box) continue;
    const w = Math.max(0, Math.min(box.x + box.w, view.x + view.w) - Math.max(box.x, view.x));
    const h = Math.max(0, Math.min(box.y + box.h, view.y + view.h) - Math.max(box.y, view.y));
    const area = w * h;
    if (area > bestArea) {
      bestArea = area;
      best = page;
    }
  }
  return best;
}

/** The scroll offset that puts a page's top-left corner at the view's origin. */
export function scrollToPage(
  layout: Layout,
  page: number,
  padding: number,
): { x: number; y: number } {
  const box = boxOf(layout, page);
  if (!box) return { x: 0, y: 0 };
  return layout.horizontal
    ? { x: Math.max(0, box.x - padding), y: 0 }
    : { x: 0, y: Math.max(0, box.y - padding) };
}
