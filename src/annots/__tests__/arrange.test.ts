import { describe, expect, it } from "vitest";
import { align, distribute } from "../arrange";
import type { AnnotObject } from "../types";

function box(id: number, left: number, bottom: number, w: number, h: number, extra: Partial<AnnotObject> = {}): AnnotObject {
  return {
    id,
    page: 0,
    kind: "Rect",
    rect: { left, bottom, right: left + w, top: bottom + h },
    rotation: 0,
    opacity: 1,
    z: 0,
    locked: false,
    created_at: 0,
    modified_at: 0,
    author_note: "",
    payload: {
      Shape: {
        style: { fill: null, stroke: { r: 0, g: 0, b: 0, a: 1 }, stroke_width: 1, dash: [0, 0], dashed: false },
      },
    },
    ...extra,
  } as AnnotObject;
}

const byId = (list: AnnotObject[]) => new Map(list.map((o) => [o.id, o]));

describe("align", () => {
  it("brings every object to the selection's leftmost edge, and moves only those not there", () => {
    const out = byId(align([box(1, 10, 0, 20, 20), box(2, 50, 30, 40, 10)], "left"));
    expect(out.has(1)).toBe(false);
    expect(out.get(2)?.rect.left).toBe(10);
    expect(out.get(2)?.rect.bottom).toBe(30);
  });

  it("centres on the selection's middle", () => {
    const out = byId(align([box(1, 0, 0, 100, 10), box(2, 0, 20, 20, 10)], "centerH"));
    expect(out.get(2)?.rect.left).toBe(40);
  });

  it("moves the payload with the box", () => {
    const line = box(2, 50, 50, 10, 10, {
      kind: "Line",
      payload: { Line: { from: { x: 50, y: 50 }, to: { x: 60, y: 60 }, color: { r: 0, g: 0, b: 0, a: 1 }, width: 1, dashed: false, arrow_head: 0 } },
    } as Partial<AnnotObject>);
    const out = byId(align([box(1, 0, 0, 10, 10), line], "bottom"));
    const moved = out.get(2);
    if (!moved || !("Line" in moved.payload)) throw new Error("garis tidak bergerak");
    expect(moved.payload.Line.from.y).toBe(0);
    expect(moved.payload.Line.to.y).toBe(10);
  });

  it("never moves a locked object, and aligns the rest to it", () => {
    const out = byId(align([box(1, 80, 0, 20, 20, { locked: true }), box(2, 10, 0, 20, 20)], "right"));
    expect(out.has(1)).toBe(false);
    expect(out.get(2)?.rect.right).toBe(100);
  });

  it("does not align across pages", () => {
    expect(align([box(1, 0, 0, 10, 10), box(2, 50, 0, 10, 10, { page: 3 })], "left")).toEqual([]);
  });
});

describe("distribute", () => {
  it("makes the gaps equal and keeps the outermost two", () => {
    // Widths 10, 30, 10 across 0..100: 50 of space, 25 per gap.
    const out = byId(distribute([box(1, 0, 0, 10, 10), box(2, 20, 0, 30, 10), box(3, 90, 0, 10, 10)], "horizontal"));
    expect(out.has(1)).toBe(false);
    expect(out.has(3)).toBe(false);
    expect(out.get(2)?.rect.left).toBe(35);
  });

  it("orders by position, not by selection order", () => {
    const out = byId(distribute([box(3, 90, 0, 10, 10), box(2, 60, 0, 10, 10), box(1, 0, 0, 10, 10)], "horizontal"));
    expect(out.get(2)?.rect.left).toBe(45);
  });

  it("works vertically and needs three objects", () => {
    const out = byId(distribute([box(1, 0, 0, 10, 10), box(2, 0, 15, 10, 10), box(3, 0, 90, 10, 10)], "vertical"));
    expect(out.get(2)?.rect.bottom).toBe(45);
    expect(distribute([box(1, 0, 0, 10, 10), box(2, 50, 0, 10, 10)], "horizontal")).toEqual([]);
  });
});
