/**
 * Grouping characters into lines decides where the selection highlight lands
 * and what a copy produces. Both are things users notice immediately and
 * neither is visible in a screenshot.
 */

import { describe, expect, it } from "vitest";
import type { TextChar } from "../textLayer";
import { groupIntoLines } from "../textLayer";

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
