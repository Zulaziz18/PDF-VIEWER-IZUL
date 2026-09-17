import { describe, expect, it } from "vitest";
import {
  angleTo,
  boundsOf,
  handleAt,
  handlePoint,
  hitTest,
  normalise,
  pick,
  pickInside,
  resized,
  scaleBetween,
  snapAngle,
} from "../interaction";
import type { AnnotObject, PdfRect } from "../types";
import { rgba } from "../types";

function base(overrides: Partial<AnnotObject> = {}): AnnotObject {
  return {
    id: 1,
    page: 0,
    kind: "Rect",
    rect: { left: 10, bottom: 10, right: 110, top: 60 },
    rotation: 0,
    opacity: 1,
    z: 0,
    locked: false,
    created_at: 0,
    modified_at: 0,
    author_note: "",
    payload: {
      Shape: {
        style: { fill: null, stroke: rgba(0, 0, 0), stroke_width: 1, dash: [4, 3], dashed: false },
      },
    },
    ...overrides,
  };
}

function line(): AnnotObject {
  return base({
    id: 2,
    kind: "Line",
    payload: {
      Line: {
        from: { x: 10, y: 10 },
        to: { x: 110, y: 60 },
        color: rgba(0, 0, 0),
        width: 2,
        dashed: false,
        arrow_head: 0,
      },
    },
  });
}

describe("hitTest", () => {
  it("takes a click inside a shape", () => {
    expect(hitTest(base(), { x: 50, y: 30 })).toBe(true);
    expect(hitTest(base(), { x: 200, y: 30 })).toBe(false);
  });

  /// The box of a diagonal line is mostly empty; clicking that emptiness and
  /// getting the line is how a user ends up dragging the wrong object.
  it("follows a line's geometry, not its bounding box", () => {
    const l = line();
    expect(hitTest(l, { x: 60, y: 35 })).toBe(true);
    expect(hitTest(l, { x: 20, y: 55 })).toBe(false);
  });

  it("gives thin objects a tolerance so they can be clicked at all", () => {
    const l = line();
    expect(hitTest(l, { x: 60, y: 37 })).toBe(true);
    expect(hitTest(l, { x: 60, y: 50 })).toBe(false);
  });

  /// A rotated object is hit where it *is*, not where its unrotated box was.
  it("accounts for rotation", () => {
    const upright = base();
    const turned = base({ rotation: 90 });
    const pointAboveCentre = { x: 60, y: 80 };
    expect(hitTest(upright, pointAboveCentre)).toBe(false);
    expect(hitTest(turned, pointAboveCentre)).toBe(true);
  });

  it("hits a highlight only on the lines it covers", () => {
    const markup = base({
      kind: "Highlight",
      rect: { left: 10, bottom: 10, right: 110, top: 60 },
      payload: {
        Markup: {
          quads: [{ left: 10, bottom: 48, right: 110, top: 60 }],
          color: rgba(255, 235, 59, 0.4),
        },
      },
    });
    expect(hitTest(markup, { x: 50, y: 54 })).toBe(true);
    expect(hitTest(markup, { x: 50, y: 20 })).toBe(false);
  });
});

describe("pick", () => {
  it("returns the topmost object", () => {
    const under = base({ id: 1 });
    const over = base({ id: 2 });
    expect(pick([under, over], { x: 50, y: 30 })?.id).toBe(2);
  });

  /// Locking exists so that a background stamp can be clicked *through*.
  it("skips locked objects", () => {
    const locked = base({ id: 9, locked: true });
    expect(pick([locked], { x: 50, y: 30 })).toBeNull();
  });

  it("returns null on empty space", () => {
    expect(pick([base()], { x: 500, y: 500 })).toBeNull();
  });
});

describe("rubber band", () => {
  it("takes only objects fully inside, in any drag direction", () => {
    const a = base({ id: 1 });
    const b = base({ id: 2, rect: { left: 200, bottom: 200, right: 260, top: 240 } });
    const dragged: PdfRect = { left: 300, bottom: 300, right: 0, top: 0 };
    expect(pickInside([a, b], dragged).map((o) => o.id)).toEqual([1, 2]);
    const narrow: PdfRect = { left: 0, bottom: 0, right: 120, top: 120 };
    expect(pickInside([a, b], narrow).map((o) => o.id)).toEqual([1]);
  });

  it("normalises a rectangle dragged backwards", () => {
    expect(normalise({ left: 10, bottom: 20, right: 0, top: 0 })).toEqual({
      left: 0,
      bottom: 0,
      right: 10,
      top: 20,
    });
  });
});

describe("handles", () => {
  it("puts the eight handles on the box and the rotate handle above it", () => {
    const r: PdfRect = { left: 0, bottom: 0, right: 100, top: 50 };
    expect(handlePoint(r, "nw")).toEqual({ x: 0, y: 50 });
    expect(handlePoint(r, "se")).toEqual({ x: 100, y: 0 });
    expect(handlePoint(r, "n")).toEqual({ x: 50, y: 50 });
    expect(handlePoint(r, "rotate").y).toBeGreaterThan(r.top);
  });

  /// The grab area has to stay the same size on screen, so it shrinks in points
  /// as the zoom grows — otherwise handles are untouchable when zoomed out.
  it("grows its grab area as the zoom shrinks", () => {
    const r: PdfRect = { left: 0, bottom: 0, right: 100, top: 50 };
    const nearCorner = { x: 4, y: 46 };
    expect(handleAt(r, nearCorner, 0.25)).toBe("nw");
    expect(handleAt(r, nearCorner, 4)).toBeNull();
  });
});

describe("resized", () => {
  it("keeps the opposite corner still", () => {
    const r: PdfRect = { left: 0, bottom: 0, right: 100, top: 50 };
    expect(resized(r, "se", 20, -10)).toEqual({ left: 0, bottom: -10, right: 120, top: 50 });
    expect(resized(r, "nw", 20, 10)).toEqual({ left: 20, bottom: 0, right: 100, top: 60 });
  });

  /// A box dragged through itself must not come out inside out: one backend
  /// would draw it, the other would not.
  it("never collapses or inverts the box", () => {
    const r: PdfRect = { left: 0, bottom: 0, right: 100, top: 50 };
    const crushed = resized(r, "e", -500, 0);
    expect(crushed.right).toBeGreaterThan(crushed.left);
    const flipped = resized(r, "n", 0, -500);
    expect(flipped.top).toBeGreaterThan(flipped.bottom);
  });
});

describe("scaleBetween", () => {
  it("finds the pivot that did not move", () => {
    const before: PdfRect = { left: 0, bottom: 0, right: 100, top: 50 };
    const after: PdfRect = { left: 0, bottom: 0, right: 200, top: 100 };
    const { origin, sx, sy } = scaleBetween(before, after);
    expect(sx).toBe(2);
    expect(sy).toBe(2);
    expect(origin.x).toBeCloseTo(0);
    expect(origin.y).toBeCloseTo(0);
  });

  it("handles a drag that only moves one edge", () => {
    const before: PdfRect = { left: 0, bottom: 0, right: 100, top: 50 };
    const after: PdfRect = { left: 50, bottom: 0, right: 100, top: 50 };
    const { origin, sx, sy } = scaleBetween(before, after);
    expect(sx).toBeCloseTo(0.5);
    expect(sy).toBe(1);
    expect(origin.x).toBeCloseTo(100, 5);
  });
});

describe("rotation", () => {
  it("reads zero when the pointer is straight above the centre", () => {
    const r: PdfRect = { left: 0, bottom: 0, right: 100, top: 50 };
    expect(angleTo(r, { x: 50, y: 100 })).toBeCloseTo(0);
    expect(angleTo(r, { x: 100, y: 25 })).toBeCloseTo(-90);
  });

  it("snaps to fifteen degrees when asked", () => {
    expect(snapAngle(37, true)).toBe(30);
    expect(snapAngle(37, false)).toBe(37);
    expect(snapAngle(-7, true)).toBe(-0);
  });
});

describe("boundsOf", () => {
  it("frames a multi-selection", () => {
    const a = base({ id: 1, rect: { left: 0, bottom: 0, right: 10, top: 10 } });
    const b = base({ id: 2, rect: { left: 50, bottom: 20, right: 60, top: 80 } });
    expect(boundsOf([a, b])).toEqual({ left: 0, bottom: 0, right: 60, top: 80 });
    expect(boundsOf([])).toBeNull();
  });
});
