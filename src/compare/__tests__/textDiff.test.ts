import { describe, expect, it } from "vitest";
import type { TextChar } from "@/viewport/textLayer";
import { changedWords, mergeOnLines, textDiff, words } from "../textDiff";

/** Lays text out as a line of 10-point-wide characters; "\n" starts a new
 * line 20 points lower. */
function chars(text: string): TextChar[] {
  const out: TextChar[] = [];
  let x = 0;
  let y = 700;
  for (const c of text) {
    if (c === "\n") {
      x = 0;
      y -= 20;
      continue;
    }
    out.push({ c, left: x, bottom: y, right: x + 10, top: y + 12 });
    x += 10;
  }
  return out;
}

describe("words", () => {
  it("splits on spaces and on line breaks, with boxes around each word", () => {
    const w = words(chars("ab cd\nef"));
    expect(w.map((x) => x.text)).toEqual(["ab", "cd", "ef"]);
    expect(w[1]).toMatchObject({ left: 30, right: 50, bottom: 700 });
    expect(w[2]).toMatchObject({ left: 0, bottom: 680 });
  });
});

describe("changedWords", () => {
  it("marks nothing on identical text", () => {
    expect(changedWords(["a", "b"], ["a", "b"])).toEqual({ a: [], b: [] });
  });

  it("marks only a replaced word", () => {
    expect(changedWords(["the", "fee", "is", "5"], ["the", "fee", "is", "7"])).toEqual({ a: [3], b: [3] });
  });

  /// The reason for an alignment rather than a position-by-position check:
  /// one inserted word must not mark everything after it.
  it("marks an inserted phrase and nothing after it", () => {
    const a = ["pay", "within", "30", "days", "of", "invoice"];
    const b = ["pay", "in", "full", "within", "30", "days", "of", "invoice"];
    expect(changedWords(a, b)).toEqual({ a: [], b: [1, 2] });
  });

  it("marks a deletion on the side that still has it", () => {
    expect(changedWords(["a", "b", "c"], ["a", "c"])).toEqual({ a: [1], b: [] });
  });

  it("falls back to the differing middle when the pages are huge", () => {
    const a = Array.from({ length: 50 }, (_, i) => `w${i}`);
    const b = [...a];
    b[10] = "x";
    b[40] = "y";
    const r = changedWords(a, b, 5);
    expect(r.a).toEqual(Array.from({ length: 31 }, (_, i) => i + 10));
  });
});

describe("mergeOnLines", () => {
  it("joins neighbours on a line and keeps lines apart", () => {
    const merged = mergeOnLines([
      { left: 0, bottom: 700, right: 20, top: 712 },
      { left: 30, bottom: 700, right: 50, top: 712 },
      { left: 0, bottom: 680, right: 20, top: 692 },
    ]);
    expect(merged).toEqual([
      { left: 0, bottom: 700, right: 50, top: 712 },
      { left: 0, bottom: 680, right: 20, top: 692 },
    ]);
  });
});

describe("textDiff", () => {
  it("returns boxes over the changed words on each page", () => {
    const r = textDiff(chars("Biaya 500 ribu"), chars("Biaya 750 ribu"));
    expect(r.changed).toBe(2);
    expect(r.a).toEqual([{ left: 60, bottom: 700, right: 90, top: 712 }]);
    expect(r.b).toEqual([{ left: 60, bottom: 700, right: 90, top: 712 }]);
  });
});
