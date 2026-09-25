import { describe, expect, it } from "vitest";
import { objectFromDrawn, DEFAULT_STYLE, type Drawn } from "../factory";
import { cropOf, MIN_KEPT, textStyleOf, withCrop, withTextStyle } from "../style";
import type { AnnotObject } from "../types";

const drag: Drawn = {
  page: 0,
  rect: { left: 100, bottom: 100, right: 300, top: 200 },
  from: { x: 100, y: 100 },
  to: { x: 300, y: 200 },
  points: [],
};

function textBox(): AnnotObject {
  const o = objectFromDrawn("FreeText", drag, DEFAULT_STYLE, 1);
  if (!o) throw new Error("tidak ada kotak teks");
  return o;
}

function picture(): AnnotObject {
  return {
    ...textBox(),
    kind: "Image",
    rect: { left: 100, bottom: 100, right: 300, top: 200 },
    payload: { Image: { image: 1, crop: { left: 0, bottom: 0, right: 1, top: 1 }, opacity: 1 } },
  };
}

describe("text style", () => {
  it("reads and changes bold, italic, alignment and spacing, leaving the rest", () => {
    const o = textBox();
    const next = withTextStyle(o, { bold: true, align: "Justify", lineSpacing: 1.5 });
    expect(textStyleOf(next)).toEqual({ bold: true, italic: false, align: "Justify", lineSpacing: 1.5 });
    expect(textStyleOf(o)?.bold).toBe(false);
    if (!("FreeText" in next.payload) || !("FreeText" in o.payload)) throw new Error("jenis");
    expect(next.payload.FreeText.font.family).toBe(o.payload.FreeText.font.family);
  });

  it("keeps the spacing within what reads as text", () => {
    expect(textStyleOf(withTextStyle(textBox(), { lineSpacing: 40 }))?.lineSpacing).toBe(3);
    expect(textStyleOf(withTextStyle(textBox(), { lineSpacing: 0 }))?.lineSpacing).toBe(0.8);
  });

  it("has nothing to say about a picture", () => {
    expect(textStyleOf(picture())).toBeNull();
  });
});

describe("crop", () => {
  it("cuts an edge and shrinks the box with it, so the rest stays put", () => {
    const next = withCrop(picture(), "left", 0.25);
    expect(cropOf(next)).toEqual({ left: 0.25, right: 0, top: 0, bottom: 0 });
    // A quarter of 200 pt is 50 pt; the right edge does not move.
    expect(next.rect).toEqual({ left: 150, bottom: 100, right: 300, top: 200 });
  });

  it("works from the crop already there, not from the whole picture", () => {
    const half = withCrop(picture(), "top", 0.5);
    expect(half.rect.top).toBe(150);
    // The box now shows half the picture's height in 50 pt: 100 pt per unit.
    const more = withCrop(half, "bottom", 0.25);
    expect(more.rect).toEqual({ left: 100, bottom: 125, right: 300, top: 150 });
  });

  it("never crops a picture to nothing", () => {
    const next = withCrop(withCrop(picture(), "left", 0.7), "right", 0.9);
    const c = cropOf(next);
    expect(c && 1 - c.right - c.left).toBeCloseTo(MIN_KEPT);
    expect(next.rect.right).toBeGreaterThan(next.rect.left);
  });
});
