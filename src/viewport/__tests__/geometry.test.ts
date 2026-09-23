/**
 * The conversions the whole viewport rests on.
 *
 * The tile grid is asserted against the same numbers the backend's
 * `tile_source` test uses, because the two must agree to the pixel: a
 * disagreement shows on screen as a hairline seam at every tile boundary, which
 * is the kind of bug that survives a code review and is found by a user.
 */

import { describe, expect, it } from "vitest";
import {
  clampZoom,
  displaySize,
  fitPage,
  fitWidth,
  MAX_ZOOM,
  MIN_ZOOM,
  pageSizePx,
  pdfRectToPx,
  pixelsPerPoint,
  pxRectToPdf,
  sliderToZoom,
  zoomToSlider,
  zoomHoldingView,
  scaleKey,
  swapsAxes,
  TILE_EDGE,
  tilesCovering,
  zoomAbout,
  zoomIn,
  zoomOut,
} from "../geometry";

const A4ish = { width: 612, height: 792 };

describe("scale", () => {
  it("keeps zoom and device pixel ratio separate until they are used", () => {
    expect(pixelsPerPoint({ zoom: 1.25, dpr: 2 })).toBeCloseTo(2.5);
  });

  it("quantises the cache key so indistinguishable scales share a bitmap", () => {
    expect(scaleKey({ zoom: 1, dpr: 1 })).toBe(1000);
    expect(scaleKey({ zoom: 1.0000001, dpr: 1 })).toBe(1000);
    expect(scaleKey({ zoom: 1.5, dpr: 2 })).toBe(3000);
  });

  it("never produces a scale the backend would refuse", () => {
    expect(scaleKey({ zoom: 0.0001, dpr: 1 })).toBeGreaterThanOrEqual(1);
    expect(scaleKey({ zoom: 16, dpr: 8 })).toBeLessThanOrEqual(64_000);
  });
});

describe("rotation", () => {
  it("swaps the axes on a quarter and three-quarter turn only", () => {
    expect([0, 1, 2, 3].map((r) => swapsAxes(r as 0 | 1 | 2 | 3))).toEqual([
      false,
      true,
      false,
      true,
    ]);
  });

  it("reports the size the page appears at", () => {
    expect(displaySize(A4ish, 1)).toEqual({ width: 792, height: 612 });
    expect(displaySize(A4ish, 2)).toEqual(A4ish);
  });
});

describe("page pixels", () => {
  it("rounds once, so a page is never a fraction of a pixel", () => {
    expect(pageSizePx(A4ish, { zoom: 1.5, dpr: 1 })).toEqual({ w: 918, h: 1188 });
  });

  it("fits a page to a viewport in CSS pixels, not device ones", () => {
    // 1224 device px at dpr 2 is 612 CSS px, which is exactly the page width.
    expect(fitWidth(A4ish, 1224, 2)).toBeCloseTo(1);
    expect(fitPage(A4ish, 1224, 1584, 2)).toBeCloseTo(1);
  });
});

describe("pdf <-> pixel", () => {
  it("flips the y axis", () => {
    const r = pxRectToPdf({ x: 0, y: 0, w: 612, h: 792 }, A4ish, { zoom: 1, dpr: 1 });
    expect(r).toEqual({ left: 0, bottom: 0, right: 612, top: 792 });
  });

  it("round trips", () => {
    const scale = { zoom: 1.3, dpr: 2 };
    const px = { x: 40, y: 90, w: 100, h: 200 };
    const back = pdfRectToPx(pxRectToPdf(px, A4ish, scale), A4ish, scale);
    expect(back.x).toBeCloseTo(px.x, 5);
    expect(back.y).toBeCloseTo(px.y, 5);
    expect(back.w).toBeCloseTo(px.w, 5);
    expect(back.h).toBeCloseTo(px.h, 5);
  });
});

describe("tile grid", () => {
  const scale = { zoom: 1, dpr: 1 };

  it("covers a page exactly, with the edge tiles clipped", () => {
    const tiles = tilesCovering({ x: 0, y: 0, w: 10_000, h: 10_000 }, A4ish, scale);
    // 612x792 px is 2 columns and 2 rows of 512.
    expect(tiles).toHaveLength(4);
    const last = tiles[3];
    expect(last).toBeDefined();
    // The same numbers the backend's `the_last_tile_is_clipped_to_the_page`
    // test asserts.
    expect(last?.px).toEqual({ x: 512, y: 512, w: 100, h: 280 });
  });

  it("asks only for the tiles a viewport touches", () => {
    const tiles = tilesCovering({ x: 0, y: 0, w: 100, h: 100 }, A4ish, scale);
    expect(tiles).toHaveLength(1);
    expect(tiles[0]?.col).toBe(0);
    expect(tiles[0]?.row).toBe(0);
  });

  it("aligns to the page grid so panning reuses tiles", () => {
    const a = tilesCovering({ x: 500, y: 0, w: 20, h: 20 }, A4ish, scale);
    const b = tilesCovering({ x: 505, y: 0, w: 20, h: 20 }, A4ish, scale);
    expect(a.map((t) => t.col)).toEqual(b.map((t) => t.col));
  });

  it("leaves no gap between neighbouring tiles", () => {
    const tiles = tilesCovering({ x: 0, y: 0, w: 10_000, h: 10_000 }, A4ish, {
      zoom: 2.3,
      dpr: 1,
    });
    const { w, h } = pageSizePx(A4ish, { zoom: 2.3, dpr: 1 });
    const area = tiles.reduce((sum, t) => sum + t.px.w * t.px.h, 0);
    expect(area).toBe(w * h);
    expect(TILE_EDGE).toBe(512);
  });
});

describe("zoom", () => {
  it("keeps the anchor point still", () => {
    // A point 100 px into the viewport, 300 px into the content, doubling.
    const next = zoomAbout({ x: 100, y: 100 }, { x: 200, y: 200 }, 1, 2);
    // Content coordinate 300 becomes 600; to keep it 100 px in, scroll to 500.
    expect(next).toEqual({ x: 500, y: 500 });
  });

  it("stays inside the range SPEC 11.1 names", () => {
    expect(clampZoom(0)).toBe(MIN_ZOOM);
    expect(clampZoom(100)).toBe(MAX_ZOOM);
    expect(clampZoom(Number.NaN)).toBe(1);
  });

  it("steps through a fixed ladder that always hits 100 %", () => {
    expect(zoomIn(0.9)).toBe(1);
    expect(zoomOut(1.1)).toBe(1);
    // The ends are stable rather than wrapping or overshooting.
    expect(zoomIn(MAX_ZOOM)).toBe(MAX_ZOOM);
    expect(zoomOut(MIN_ZOOM)).toBe(MIN_ZOOM);
  });
});

describe("zoom slider", () => {
  it("maps the ends of the track to the ends of the zoom range", () => {
    expect(sliderToZoom(0)).toBeCloseTo(MIN_ZOOM, 6);
    expect(sliderToZoom(1)).toBeCloseTo(MAX_ZOOM, 6);
    expect(sliderToZoom(-3)).toBeCloseTo(MIN_ZOOM, 6);
  });

  it("round-trips, and puts 100 % well inside the track rather than near an end", () => {
    for (const z of [0.1, 0.5, 1, 2.5, 16]) expect(sliderToZoom(zoomToSlider(z))).toBeCloseTo(z, 6);
    const p = zoomToSlider(1);
    expect(p).toBeGreaterThan(0.4);
    expect(p).toBeLessThan(0.5);
  });
});

describe("zoomHoldingView", () => {
  it("keeps a reader who has not scrolled at the top of the document", () => {
    const next = zoomHoldingView({ x: 0, y: 0 }, { w: 1000, h: 600 }, 1, 1.25);
    expect(next).toEqual({ x: 0, y: 0 });
  });

  it("holds the centre once the reader has scrolled", () => {
    const next = zoomHoldingView({ x: 0, y: 1000 }, { w: 1000, h: 600 }, 1, 2);
    // The content point at the centre was y = 1000 + 300 = 1300; at twice the
    // zoom it is 2600, and it must still sit 300 px down the view.
    expect(next.y).toBeCloseTo(2300, 6);
    expect(next.x).toBe(0);
  });
});
