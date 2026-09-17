import { describe, expect, it } from "vitest";
import { tabsToTrim, WARM_TABS } from "../workspaceStore";

/**
 * The rule behind SPEC 17's "fifty documents open, memory under control".
 *
 * It is worth testing rather than eyeballing because the failure modes are both
 * invisible in a screenshot: trimming too eagerly re-renders the page every
 * time a reader flips between two documents, and trimming too little is the
 * memory the criterion is about.
 */
describe("tabsToTrim", () => {
  it("keeps the tabs the reader has actually been using", () => {
    const recent = [1, 2, 3, 4, 5];
    expect(tabsToTrim(recent)).toEqual([4, 5]);
    expect(tabsToTrim(recent)).not.toContain(1);
  });

  it("trims nothing while there are fewer tabs than the warm budget", () => {
    expect(tabsToTrim([1])).toEqual([]);
    expect(tabsToTrim([1, 2, 3])).toEqual([]);
  });

  /// Two documents being compared are flipped between every few seconds; a
  /// policy that trimmed on every flip would re-render both pages each time.
  it("leaves a two-document comparison alone", () => {
    let recent = [1, 2];
    for (let i = 0; i < 10; i++) {
      const front = recent[0] as number;
      recent = [recent[1] as number, front];
      expect(tabsToTrim(recent)).toEqual([]);
    }
  });

  it("trims everything past the budget when fifty are open", () => {
    const recent = Array.from({ length: 50 }, (_, i) => i + 1);
    const trimmed = tabsToTrim(recent);
    expect(trimmed).toHaveLength(50 - WARM_TABS);
    expect(trimmed).not.toContain(recent[0]);
    expect(trimmed[trimmed.length - 1]).toBe(50);
  });
});
