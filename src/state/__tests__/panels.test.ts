import { describe, expect, it } from "vitest";
import {
  SINGLE,
  assign,
  clampRatio,
  closeDoc,
  decodePanels,
  encodePanels,
  setLayout,
  showDoc,
  type PanelState,
} from "../panels";

const tabs = [10, 20, 30, 40, 50];

describe("setLayout", () => {
  it("fills new panels with open tabs, the document in front first", () => {
    const s = setLayout({ ...SINGLE, panels: [30] }, "grid", tabs, 30);
    expect(s.panels).toEqual([30, 10, 20, 40]);
    expect(s.focused).toBe(0);
  });

  it("keeps what was visible when going down, the front one first", () => {
    const grid: PanelState = { ...SINGLE, layout: "grid", panels: [10, 20, 30, 40], focused: 2 };
    const s = setLayout(grid, "columns", tabs, 30);
    expect(s.panels).toEqual([30, 10]);
    expect(s.focused).toBe(0);
  });

  it("leaves panels empty when there are fewer tabs than panels", () => {
    const s = setLayout(SINGLE, "grid", [10], 10);
    expect(s.panels).toEqual([10, null, null, null]);
  });

  it("drops documents that are no longer open", () => {
    const s = setLayout({ ...SINGLE, layout: "columns", panels: [99, 10] }, "columns", [10], 10);
    expect(s.panels).toEqual([10, null]);
  });
});

describe("one document, one panel", () => {
  const two: PanelState = { ...SINGLE, layout: "columns", panels: [10, 20], focused: 0 };

  it("showing a visible document focuses its panel instead of copying it", () => {
    expect(showDoc(two, 20)).toEqual({ ...two, focused: 1 });
  });

  it("showing a hidden document puts it in the focused panel", () => {
    expect(showDoc(two, 30).panels).toEqual([30, 20]);
  });

  it("dropping a tab onto another panel swaps the two", () => {
    const s = assign(two, 1, 10);
    expect(s.panels).toEqual([20, 10]);
    expect(s.focused).toBe(1);
    expect(assign(two, 1, 30).panels).toEqual([10, 30]);
    expect(assign(two, 5, 30)).toBe(two);
  });
});

describe("closeDoc", () => {
  it("empties the panel in a split, and hands a single panel to the successor", () => {
    const two: PanelState = { ...SINGLE, layout: "columns", panels: [10, 20], focused: 0 };
    expect(closeDoc(two, 20, 10).panels).toEqual([10, null]);
    expect(closeDoc({ ...SINGLE, panels: [10] }, 10, 20).panels).toEqual([20]);
    expect(closeDoc({ ...SINGLE, panels: [10] }, 10, null).panels).toEqual([null]);
  });
});

describe("persistence", () => {
  it("remembers layout and dividers, never document ids", () => {
    const s: PanelState = { layout: "grid", panels: [1, 2, 3, 4], focused: 2, ratio: { x: 0.3, y: 0.6 } };
    const text = encodePanels(s);
    expect(text).not.toContain("panels");
    expect(decodePanels(text)).toEqual({ layout: "grid", ratio: { x: 0.3, y: 0.6 } });
  });

  it("reads anything unexpected as the default", () => {
    expect(decodePanels(null).layout).toBe("single");
    expect(decodePanels('{"layout":"hexagon"}').layout).toBe("single");
    expect(decodePanels('{"layout":"rows","ratio":{"x":2}}').ratio.x).toBe(0.85);
    expect(clampRatio(Number.NaN)).toBe(0.5);
  });
});
