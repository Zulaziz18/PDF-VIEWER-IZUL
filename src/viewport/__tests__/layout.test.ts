/**
 * The layout is what makes the scroll virtualized and the scroll bar honest.
 *
 * Two properties matter beyond "the numbers are right": the visible query must
 * not walk the document, and the placeholder geometry must be final before
 * anything is rendered, or the scroll bar jumps under the user's hand.
 */

import { describe, expect, it } from "vitest";
import type { PageMetrics, Rotation } from "../geometry";
import {
  boxOf,
  dominantPage,
  layoutDocument,
  parseViewMode,
  scrollToPage,
  visiblePages,
} from "../layout";
import type { ViewMode } from "../layout";

const A4 = { width: 612, height: 792 };

function pages(n: number, size: PageMetrics = A4): PageMetrics[] {
  return Array.from({ length: n }, () => size);
}

function build(
  n: number,
  mode: ViewMode = "single",
  zoom = 1,
  rotationOf: (page: number) => Rotation = () => 0,
) {
  return layoutDocument({
    pageSizes: pages(n),
    zoom,
    mode,
    rotationOf,
    gap: 16,
    padding: 24,
    viewportWidth: 1000,
  });
}

describe("view modes", () => {
  it("stacks single pages vertically", () => {
    const layout = build(3);
    expect(layout.pages).toHaveLength(3);
    const [first, second] = layout.pages;
    expect(first?.y).toBe(24);
    expect(second?.y).toBe(24 + 792 + 16);
    expect(layout.height).toBe(24 + 792 * 3 + 16 * 2 + 24);
  });

  it("pairs pages side by side in dual mode", () => {
    const layout = build(4, "dual");
    const [a, b, c] = layout.pages;
    expect(a?.y).toBe(b?.y);
    expect(b?.x).toBeGreaterThan(a?.x ?? 0);
    expect(c?.y).toBeGreaterThan(a?.y ?? 0);
  });

  it("puts the cover alone so later spreads fall like a book", () => {
    const layout = build(5, "dual_cover");
    const rows = layout.rows;
    expect(rows[0]).toMatchObject({ first: 0, last: 1 });
    expect(rows[1]).toMatchObject({ first: 1, last: 3 });
    expect(rows[2]).toMatchObject({ first: 3, last: 5 });
  });

  it("lays a horizontal document out in one row", () => {
    const layout = build(4, "horizontal");
    expect(layout.horizontal).toBe(true);
    expect(layout.rows).toHaveLength(4);
    const ys = new Set(layout.pages.map((p) => p.y));
    expect(ys.size).toBe(1);
  });

  it("accepts only the modes it knows", () => {
    expect(parseViewMode("dual_cover")).toBe("dual_cover");
    expect(parseViewMode("kaleidoscope")).toBe("single");
  });
});

describe("geometry", () => {
  it("scales with zoom", () => {
    const layout = build(2, "single", 2);
    expect(layout.pages[0]?.w).toBe(1224);
    expect(layout.pages[0]?.h).toBe(1584);
  });

  it("uses the rotated size for a rotated page", () => {
    const layout = build(2, "single", 1, (p) => (p === 0 ? 1 : 0));
    expect(layout.pages[0]?.w).toBe(792);
    expect(layout.pages[0]?.h).toBe(612);
    expect(layout.pages[1]?.w).toBe(612);
  });

  it("centres a document narrower than the window", () => {
    const layout = build(1);
    const box = layout.pages[0];
    expect(box).toBeDefined();
    expect(box?.x).toBeCloseTo((1000 - 612) / 2);
  });

  it("sizes the content before anything is rendered, so the bar never jumps", () => {
    // The layout is built from page sizes alone: no bitmap, no render, no
    // measurement of anything on screen.
    const layout = build(500);
    expect(layout.height).toBe(24 + 792 * 500 + 16 * 499 + 24);
  });
});

describe("visibility", () => {
  it("returns only the pages the window touches", () => {
    const layout = build(100);
    const visible = visiblePages(layout, { x: 0, y: 0, w: 1000, h: 800 });
    expect(visible).toEqual([0]);
  });

  it("includes a page that is only just off screen when a margin is given", () => {
    const layout = build(100);
    expect(visiblePages(layout, { x: 0, y: 0, w: 1000, h: 800 }, 200)).toEqual([0, 1]);
  });

  it("costs the same on a long document as on a short one", () => {
    // The guarantee is structural — a binary search over rows — so the assertion
    // is on the shape of the answer, not on a stopwatch: a window in the middle
    // of a 5000-page document must return two pages, not five thousand.
    const layout = build(5000);
    const middle = layout.height / 2;
    const visible = visiblePages(layout, { x: 0, y: middle, w: 1000, h: 900 });
    expect(visible.length).toBeLessThanOrEqual(3);
    expect(visible[0]).toBeGreaterThan(2000);
  });

  it("names the page covering most of the window, not merely the first", () => {
    const layout = build(10);
    const secondTop = layout.pages[1]?.y ?? 0;
    // A window showing the last sliver of page 1 and most of page 2.
    const page = dominantPage(layout, { x: 0, y: secondTop - 50, w: 1000, h: 800 });
    expect(page).toBe(1);
  });
});

describe("navigation", () => {
  it("scrolls a page to the top of the window", () => {
    const layout = build(10);
    const target = scrollToPage(layout, 3, 12);
    expect(target.y).toBeCloseTo((layout.pages[3]?.y ?? 0) - 12);
    expect(target.x).toBe(0);
  });

  it("scrolls horizontally in a horizontal layout", () => {
    const layout = build(10, "horizontal");
    const target = scrollToPage(layout, 3, 12);
    expect(target.x).toBeGreaterThan(0);
    expect(target.y).toBe(0);
  });

  it("finds a page's box by index", () => {
    const layout = build(5, "dual");
    expect(boxOf(layout, 3)?.page).toBe(3);
    expect(boxOf(layout, 99)).toBeUndefined();
  });
});
