import { describe, expect, it } from "vitest";
import {
  clickSelect,
  decodePageDrag,
  dropBefore,
  encodePageDrag,
  moveChangesNothing,
  targetPages,
} from "../pageSelection";

const none = { toggle: false, range: false };

describe("clickSelect", () => {
  it("a plain click selects one page and anchors there", () => {
    expect(clickSelect({ pages: [1, 2], anchor: 1 }, 4, none)).toEqual({ pages: [4], anchor: 4 });
  });

  it("Ctrl adds and removes", () => {
    let s = clickSelect({ pages: [], anchor: null }, 3, none);
    s = clickSelect(s, 1, { toggle: true, range: false });
    expect(s.pages).toEqual([1, 3]);
    s = clickSelect(s, 3, { toggle: true, range: false });
    expect(s.pages).toEqual([1]);
  });

  it("Shift selects the run from the anchor, in either direction", () => {
    const s = clickSelect({ pages: [5], anchor: 5 }, 2, { toggle: false, range: true });
    expect(s).toEqual({ pages: [2, 3, 4, 5], anchor: 5 });
    const t = clickSelect(s, 7, { toggle: false, range: true });
    expect(t.pages).toEqual([5, 6, 7]);
  });

  it("Ctrl+Shift adds the run to what was selected", () => {
    const s = clickSelect({ pages: [0], anchor: 3 }, 4, { toggle: true, range: true });
    expect(s.pages).toEqual([0, 3, 4]);
  });
});

describe("dropBefore", () => {
  it("top half of a slot is before it, bottom half after it", () => {
    expect(dropBefore(10, 100, 5)).toBe(0);
    expect(dropBefore(60, 100, 5)).toBe(1);
    expect(dropBefore(249, 100, 5)).toBe(2);
    expect(dropBefore(251, 100, 5)).toBe(3);
  });

  it("past the last page is the end, and nothing is before the start", () => {
    expect(dropBefore(9999, 100, 5)).toBe(5);
    expect(dropBefore(-20, 100, 5)).toBe(0);
    expect(dropBefore(10, 100, 0)).toBe(0);
  });
});

describe("moveChangesNothing", () => {
  it("dropping a page onto its own place is not a move", () => {
    expect(moveChangesNothing([2], 2)).toBe(true);
    expect(moveChangesNothing([2], 3)).toBe(true);
    expect(moveChangesNothing([2, 3], 3)).toBe(true);
    expect(moveChangesNothing([2], 0)).toBe(false);
    expect(moveChangesNothing([2], 5)).toBe(false);
  });

  it("gathering scattered pages is a move even next to them", () => {
    expect(moveChangesNothing([1, 3], 2)).toBe(false);
  });
});

describe("targetPages", () => {
  it("acts on the selection, or on the page being read", () => {
    expect(targetPages([4, 6], 1)).toEqual([4, 6]);
    expect(targetPages([], 1)).toEqual([1]);
  });
});

describe("page drag payload", () => {
  it("round-trips and refuses anything else", () => {
    const text = encodePageDrag({ doc: 3, pages: [0, 2] });
    expect(decodePageDrag(text)).toEqual({ doc: 3, pages: [0, 2] });
    expect(decodePageDrag("not json")).toBeNull();
    expect(decodePageDrag('{"doc":"3","pages":[0]}')).toBeNull();
    expect(decodePageDrag('{"doc":3,"pages":[-1]}')).toBeNull();
  });
});
