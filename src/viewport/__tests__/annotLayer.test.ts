import { describe, expect, it } from "vitest";
import { pageAt, pageBox, toContentPoint, toPagePoint, within } from "../annotLayer";
import { layoutDocument } from "../layout";

const sizes = [
  { width: 200, height: 400 },
  { width: 200, height: 400 },
];

function layout() {
  return layoutDocument({
    pageSizes: sizes,
    zoom: 2,
    mode: "single",
    rotationOf: () => 0,
    gap: 16,
    padding: 24,
    viewportWidth: 800,
  });
}

const rotationOf = (): 0 => 0;

describe("page coordinates", () => {
  it("flips y between content pixels and PDF points", () => {
    const box = pageBox(layout(), 0, sizes, rotationOf);
    expect(box).not.toBeNull();
    if (!box) return;
    // The page's top-left corner in content space is (0, height) in points.
    expect(toPagePoint(box, { x: box.x, y: box.y })).toEqual({ x: 0, y: 400 });
    // And its bottom-left corner is the origin.
    expect(toPagePoint(box, { x: box.x, y: box.y + 400 * 2 })).toEqual({ x: 0, y: 0 });
  });

  it("round trips", () => {
    const box = pageBox(layout(), 0, sizes, rotationOf);
    if (!box) throw new Error("no box");
    const point = { x: 37.5, y: 219.25 };
    const back = toPagePoint(box, toContentPoint(box, point));
    expect(back.x).toBeCloseTo(point.x);
    expect(back.y).toBeCloseTo(point.y);
  });

  it("scales with the zoom", () => {
    const box = pageBox(layout(), 0, sizes, rotationOf);
    if (!box) throw new Error("no box");
    // Zoom is 2, so ten points along the page is twenty pixels across.
    const a = toContentPoint(box, { x: 0, y: 0 });
    const b = toContentPoint(box, { x: 10, y: 0 });
    expect(b.x - a.x).toBe(20);
  });
});

describe("pageAt", () => {
  const view = { x: 0, y: 0, w: 800, h: 2000 };

  it("finds the page a point is on", () => {
    const l = layout();
    const first = pageBox(l, 0, sizes, rotationOf);
    if (!first) throw new Error("no box");
    const hit = pageAt(l, view, sizes, rotationOf, { x: first.x + 10, y: first.y + 10 });
    expect(hit?.page).toBe(0);
  });

  /// A drag that starts on a page and wanders into the gap must keep working;
  /// returning "no page" mid-gesture would drop the object being dragged.
  it("falls back to the nearest page in the gap between two", () => {
    const l = layout();
    const first = pageBox(l, 0, sizes, rotationOf);
    if (!first) throw new Error("no box");
    const inGap = { x: first.x + 10, y: first.y + 400 * 2 + 8 };
    expect(within(first, inGap)).toBe(false);
    expect(pageAt(l, view, sizes, rotationOf, inGap)).not.toBeNull();
  });
});
