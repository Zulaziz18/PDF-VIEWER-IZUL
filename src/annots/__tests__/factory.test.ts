import { describe, expect, it } from "vitest";
import { DEFAULT_STYLE, markupFromQuads, objectFromDrawn, type Drawn } from "../factory";
import type { AnnotKind } from "../types";
import { ANNOT_KINDS } from "../types";

const drag: Drawn = {
  page: 2,
  rect: { left: 10, bottom: 10, right: 110, top: 60 },
  from: { x: 10, y: 10 },
  to: { x: 110, y: 60 },
  points: [
    { x: 10, y: 10 },
    { x: 60, y: 40 },
    { x: 110, y: 60 },
  ],
};

const click: Drawn = {
  page: 0,
  rect: { left: 50, bottom: 50, right: 50, top: 50 },
  from: { x: 50, y: 50 },
  to: { x: 50, y: 50 },
  points: [{ x: 50, y: 50 }],
};

describe("objectFromDrawn", () => {
  it("produces something for every kind a drag can make", () => {
    const draggable = ANNOT_KINDS.filter(
      (k) => !["Highlight", "Underline", "StrikeOut", "Image"].includes(k),
    );
    for (const kind of draggable) {
      const obj = objectFromDrawn(kind, drag, DEFAULT_STYLE);
      expect(obj, kind).not.toBeNull();
      expect(obj?.kind).toBe(kind);
      expect(obj?.page).toBe(2);
    }
  });

  /// Markup follows a text selection and an image needs a file; a rectangle
  /// dragged in empty space cannot stand in for either.
  it("refuses the kinds a drag cannot express", () => {
    for (const kind of ["Highlight", "Underline", "StrikeOut", "Image"] as AnnotKind[]) {
      expect(objectFromDrawn(kind, drag, DEFAULT_STYLE), kind).toBeNull();
    }
  });

  /// A click with no drag must still leave something selectable behind, or the
  /// object exists and cannot be reached again.
  it("never produces a zero-size object", () => {
    for (const kind of ["Rect", "Ellipse", "FreeText", "Stamp", "Note"] as AnnotKind[]) {
      const obj = objectFromDrawn(kind, click, DEFAULT_STYLE);
      expect(obj, kind).not.toBeNull();
      if (!obj) continue;
      expect(obj.rect.right - obj.rect.left, kind).toBeGreaterThan(0);
      expect(obj.rect.top - obj.rect.bottom, kind).toBeGreaterThan(0);
    }
  });

  it("gives an arrow a head and a line none", () => {
    const arrow = objectFromDrawn("Arrow", drag, DEFAULT_STYLE);
    const line = objectFromDrawn("Line", drag, DEFAULT_STYLE);
    if (!arrow || !line) throw new Error("no object");
    if (!("Line" in arrow.payload) || !("Line" in line.payload)) throw new Error("payload");
    expect(arrow.payload.Line.arrow_head).toBeGreaterThan(0);
    expect(line.payload.Line.arrow_head).toBe(0);
  });

  it("scales the arrow head with the pen so it stays in proportion", () => {
    const thin = objectFromDrawn("Arrow", drag, { ...DEFAULT_STYLE, strokeWidth: 1 });
    const fat = objectFromDrawn("Arrow", drag, { ...DEFAULT_STYLE, strokeWidth: 8 });
    if (!thin || !fat) throw new Error("no object");
    if (!("Line" in thin.payload) || !("Line" in fat.payload)) throw new Error("payload");
    expect(fat.payload.Line.arrow_head).toBeGreaterThan(thin.payload.Line.arrow_head);
  });

  it("keeps every sample of an ink stroke", () => {
    const ink = objectFromDrawn("Ink", drag, DEFAULT_STYLE);
    if (!ink || !("Ink" in ink.payload)) throw new Error("no ink");
    expect(ink.payload.Ink.strokes[0]).toHaveLength(3);
    expect(ink.payload.Ink.smooth).toBe(true);
  });

  it("gives a new text box room for a line of its own font size", () => {
    const style = { ...DEFAULT_STYLE, font: { ...DEFAULT_STYLE.font, size: 20 } };
    const text = objectFromDrawn("FreeText", click, style);
    if (!text) throw new Error("no text");
    expect(text.rect.right - text.rect.left).toBeGreaterThanOrEqual(20 * 6);
  });

  it("carries the current style onto the object", () => {
    const style = { ...DEFAULT_STYLE, opacity: 0.5, strokeWidth: 6, dashed: true };
    const rect = objectFromDrawn("Rect", drag, style);
    if (!rect || !("Shape" in rect.payload)) throw new Error("no shape");
    expect(rect.opacity).toBe(0.5);
    expect(rect.payload.Shape.style.stroke_width).toBe(6);
    expect(rect.payload.Shape.style.dashed).toBe(true);
  });
});

describe("markupFromQuads", () => {
  it("covers every quad and boxes them all", () => {
    const quads = [
      { left: 400, bottom: 700, right: 500, top: 712 },
      { left: 72, bottom: 688, right: 200, top: 700 },
    ];
    const obj = markupFromQuads("Highlight", 1, quads, DEFAULT_STYLE);
    if (!obj || !("Markup" in obj.payload)) throw new Error("no markup");
    expect(obj.payload.Markup.quads).toHaveLength(2);
    expect(obj.rect).toEqual({ left: 72, bottom: 688, right: 500, top: 712 });
  });

  /// A half-transparent underline reads as a mistake; a solid highlight hides
  /// the text. The two want different colours from the same style.
  it("uses the wash colour for a highlight and the pen colour for a line", () => {
    const quads = [{ left: 0, bottom: 0, right: 10, top: 10 }];
    const high = markupFromQuads("Highlight", 0, quads, DEFAULT_STYLE);
    const under = markupFromQuads("Underline", 0, quads, DEFAULT_STYLE);
    if (!high || !under) throw new Error("no object");
    if (!("Markup" in high.payload) || !("Markup" in under.payload)) throw new Error("payload");
    expect(high.payload.Markup.color).toEqual(DEFAULT_STYLE.highlightColor);
    expect(under.payload.Markup.color).toEqual(DEFAULT_STYLE.color);
  });

  it("refuses an empty selection", () => {
    expect(markupFromQuads("Highlight", 0, [], DEFAULT_STYLE)).toBeNull();
  });
});
