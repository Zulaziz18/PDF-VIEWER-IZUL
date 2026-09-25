/**
 * Grouping characters into lines decides where the selection highlight lands
 * and what a copy produces. Both are things users notice immediately and
 * neither is visible in a screenshot.
 */

import { describe, expect, it } from "vitest";
import type { TextChar } from "../textLayer";
import { groupIntoLines, layerSlot, textInRect } from "../textLayer";

/** A character box on a line whose baseline sits at `bottom`. */
function ch(c: string, left: number, bottom: number, w = 6, h = 10): TextChar {
  return { c, left, bottom, right: left + w, top: bottom + h };
}

describe("groupIntoLines", () => {
  it("keeps a run of characters on one line", () => {
    const line = groupIntoLines([ch("H", 0, 100), ch("a", 6, 100), ch("i", 12, 100)]);
    expect(line).toHaveLength(1);
    expect(line[0]?.text).toBe("Hai");
    expect(line[0]?.left).toBe(0);
    expect(line[0]?.right).toBe(18);
  });

  it("starts a new line when the text drops down the page", () => {
    const lines = groupIntoLines([ch("a", 0, 100), ch("b", 6, 100), ch("c", 0, 80)]);
    expect(lines.map((l) => l.text)).toEqual(["ab", "c"]);
  });

  it("separates columns even when they share a baseline", () => {
    // Two columns: the second column's first character sits to the left of
    // where the first column ended, on the same line.
    const lines = groupIntoLines([ch("a", 300, 100), ch("b", 306, 100), ch("c", 20, 100)]);
    expect(lines.map((l) => l.text)).toEqual(["ab", "c"]);
  });

  it("tolerates the small vertical wobble of a real line of type", () => {
    // Ascenders and descenders move the box without breaking the line.
    const lines = groupIntoLines([
      ch("T", 0, 100, 6, 12),
      ch("y", 6, 97, 6, 10),
      ch("p", 12, 97, 6, 10),
      ch("e", 18, 100, 6, 8),
    ]);
    expect(lines).toHaveLength(1);
    expect(lines[0]?.text).toBe("Type");
  });

  it("drops explicit line breaks rather than treating them as glyphs", () => {
    const lines = groupIntoLines([ch("a", 0, 100), ch("\n", 6, 100), ch("b", 0, 80)]);
    expect(lines.map((l) => l.text)).toEqual(["a", "b"]);
  });

  it("ignores a line of nothing but spaces", () => {
    expect(groupIntoLines([ch(" ", 0, 100), ch(" ", 6, 100)])).toEqual([]);
  });

  it("has nothing to say about an empty page", () => {
    expect(groupIntoLines([])).toEqual([]);
  });

  it("keeps the line's box tight around its characters", () => {
    const [line] = groupIntoLines([ch("a", 10, 100, 6, 10), ch("g", 16, 96, 6, 14)]);
    expect(line?.left).toBe(10);
    expect(line?.bottom).toBe(96);
    expect(line?.right).toBe(22);
    expect(line?.top).toBe(110);
  });
});

describe("layerSlot", () => {
  // Scrolling up brings page 2 in after page 3 is already there; appended,
  // a screen reader would read page 3 and then page 2.
  it("puts a page before the later pages already there", () => {
    expect(layerSlot([3, 4], 2)).toBe(0);
    expect(layerSlot([1, 4], 2)).toBe(1);
  });

  it("puts the last page last, and the first into an empty layer", () => {
    expect(layerSlot([1, 2], 3)).toBe(2);
    expect(layerSlot([], 7)).toBe(0);
  });
});

describe("textInRect", () => {
  // Two columns on two lines: "Nama   Nilai" / "Ayu    90".
  const row = (y: number, left: string, right: string) => [
    ...[...left].map((c, i) => ch(c, 10 + i * 6, y)),
    ...[...right].map((c, i) => ch(c, 100 + i * 6, y)),
  ];
  const page = [...row(700, "Nama", "Nilai"), ...row(680, "Ayu", "90")];

  it("takes one column of a table, line by line", () => {
    expect(textInRect(page, { left: 95, bottom: 670, right: 150, top: 715 })).toBe("Nilai\n90");
  });

  it("leaves out a glyph the edge only grazes, and works dragged either way", () => {
    // The edge at x = 14 cuts the first letters, whose centres are at x = 13.
    const taken = textInRect(page, { left: 150, bottom: 715, right: 14, top: 670 });
    expect(taken.startsWith("ama")).toBe(true);
    expect(taken.split("\n")[1]?.startsWith("yu")).toBe(true);
  });

  it("is empty over empty space", () => {
    expect(textInRect(page, { left: 300, bottom: 0, right: 400, top: 50 })).toBe("");
  });
});
