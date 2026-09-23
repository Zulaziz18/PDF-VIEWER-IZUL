import { describe, expect, it } from "vitest";
import { formatPageRange, parsePageRange } from "../pageRange";

/** Pages as the user numbers them, for readable expectations. */
function pages(text: string, count = 10): number[] | string {
  const r = parsePageRange(text, count);
  return r.ok ? r.pages.map((p) => p + 1) : r.error.kind;
}

describe("parsePageRange", () => {
  it("reads single pages and inclusive ranges", () => {
    expect(pages("1-3, 5")).toEqual([1, 2, 3, 5]);
    expect(pages("7")).toEqual([7]);
    expect(pages("10")).toEqual([10]);
  });

  it("returns zero-based indices", () => {
    const r = parsePageRange("1", 3);
    expect(r).toEqual({ ok: true, pages: [0] });
  });

  it("accepts the separators people actually type", () => {
    expect(pages("1;3 5,7")).toEqual([1, 3, 5, 7]);
    expect(pages("  2 - 4 ,  6 ")).toEqual([2, 3, 4, 6]);
    expect(pages("1 -3")).toEqual([1, 2, 3]);
  });

  it("opens either end of a range", () => {
    expect(pages("8-")).toEqual([8, 9, 10]);
    expect(pages("-3")).toEqual([1, 2, 3]);
  });

  it("keeps the order asked for, and reverses a backwards range", () => {
    expect(pages("5, 1-2")).toEqual([5, 1, 2]);
    expect(pages("4-2")).toEqual([4, 3, 2]);
  });

  it("exports a page asked for twice once, where it first appeared", () => {
    expect(pages("3, 1-4")).toEqual([3, 1, 2, 4]);
  });

  it("refuses what it cannot read, naming the part", () => {
    expect(parsePageRange("1-3, x", 10)).toEqual({ ok: false, error: { kind: "syntax", part: "x" } });
    expect(pages("1--3")).toBe("syntax");
    expect(pages("-")).toBe("syntax");
    expect(pages("2.5")).toBe("syntax");
    expect(pages("1-3-5")).toBe("syntax");
  });

  it("refuses pages the document does not have", () => {
    expect(parsePageRange("9-12", 10)).toEqual({
      ok: false,
      error: { kind: "outOfRange", part: "9-12", pageCount: 10 },
    });
    expect(pages("0")).toBe("outOfRange");
    expect(pages("11")).toBe("outOfRange");
  });

  it("calls an empty box empty rather than 'no pages'", () => {
    expect(pages("")).toBe("empty");
    expect(pages(" ,; ")).toBe("empty");
  });
});

describe("formatPageRange", () => {
  it("writes runs back compactly", () => {
    expect(formatPageRange([0, 1, 2, 4])).toBe("1-3, 5");
    expect(formatPageRange([6])).toBe("7");
    expect(formatPageRange([])).toBe("");
  });

  it("round-trips through the parser", () => {
    for (const text of ["1-3, 5", "2", "1, 3, 5-7, 9-10"]) {
      const r = parsePageRange(text, 10);
      expect(r.ok && formatPageRange(r.pages)).toBe(text);
    }
  });
});
