import { describe, expect, it } from "vitest";
import { lineQuads } from "../lines";
import type { PdfRect } from "../types";

const r = (left: number, bottom: number, right: number, top: number): PdfRect => ({
  left,
  bottom,
  right,
  top,
});

describe("one quad per line of a selection", () => {
  it("drops the element box that repeats its text boxes", () => {
    const text = r(10, 100, 60, 112);
    expect(lineQuads([r(10, 100, 60, 112), text])).toEqual([text]);
    expect(lineQuads([r(8, 98, 62, 114), text])).toEqual([r(8, 98, 62, 114)]);
  });

  it("joins the spans of one line", () => {
    expect(lineQuads([r(10, 100, 60, 112), r(61, 100, 120, 112), r(122, 101, 180, 111)])).toEqual([
      r(10, 100, 180, 112),
    ]);
  });

  it("keeps two columns at the same height apart", () => {
    const out = lineQuads([r(10, 100, 200, 112), r(300, 100, 480, 112)]);
    expect(out).toHaveLength(2);
  });

  it("splits lines whose boxes overlap so nothing is covered twice", () => {
    // A tightly set heading: 60 pt line boxes, 50 pt apart.
    const out = lineQuads([r(10, 640, 500, 700), r(10, 590, 400, 650)]);
    expect(out).toHaveLength(2);
    const [upper, lower] = out as [PdfRect, PdfRect];
    expect(upper.bottom).toBeCloseTo(645);
    expect(lower.top).toBeCloseTo(645);
    expect(upper.top).toBe(700);
    expect(lower.bottom).toBe(590);
  });

  it("orders lines top to bottom", () => {
    const out = lineQuads([r(10, 80, 100, 92), r(10, 100, 100, 112)]);
    expect(out.map((q) => q.top)).toEqual([112, 92]);
  });

  it("ignores empty boxes", () => {
    expect(lineQuads([r(10, 10, 10, 20), r(5, 5, 6, 5)])).toEqual([]);
  });
});
