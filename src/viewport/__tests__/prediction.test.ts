/**
 * Prefetch is a guess, and a guess that is wrong in the expensive direction —
 * queueing renders nobody looks at — is worse than not guessing.
 */

import { describe, expect, it } from "vitest";
import { IDLE_SPEED, MAX_PREFETCH_PAGES, ScrollTracker, prefetchPages } from "../prediction";

describe("ScrollTracker", () => {
  it("reports no direction from a single sample", () => {
    const t = new ScrollTracker();
    t.sample(0, 0);
    expect(t.direction).toBe(0);
  });

  it("follows a steady scroll", () => {
    const t = new ScrollTracker();
    for (let i = 0; i <= 10; i++) t.sample(i * 100, i * 16);
    expect(t.direction).toBe(1);
    expect(t.velocity).toBeGreaterThan(1000);
  });

  it("follows a scroll backwards", () => {
    const t = new ScrollTracker();
    for (let i = 0; i <= 10; i++) t.sample(5000 - i * 100, i * 16);
    expect(t.direction).toBe(-1);
  });

  it("settles to still when the scroll stops", () => {
    const t = new ScrollTracker();
    for (let i = 0; i <= 10; i++) t.sample(i * 100, i * 16);
    for (let i = 11; i <= 40; i++) t.sample(1000, i * 16);
    expect(t.direction).toBe(0);
  });

  it("ignores samples too close together to divide by", () => {
    const t = new ScrollTracker();
    t.sample(0, 0);
    t.sample(1000, 0.1);
    expect(Number.isFinite(t.velocity)).toBe(true);
    expect(t.velocity).toBe(0);
  });

  it("forgets its history on a jump", () => {
    const t = new ScrollTracker();
    for (let i = 0; i <= 10; i++) t.sample(i * 100, i * 16);
    t.reset();
    expect(t.velocity).toBe(0);
  });
});

describe("prefetchPages", () => {
  it("reaches both ways when the user is still", () => {
    expect(prefetchPages([4], 0, 10)).toEqual([5, 3]);
  });

  it("reaches ahead in the direction of travel", () => {
    const out = prefetchPages([4], 1200, 100);
    expect(out[0]).toBe(5);
    expect(out).toContain(3);
    expect(out.length).toBeLessThanOrEqual(MAX_PREFETCH_PAGES + 1);
  });

  it("reaches further the faster the scroll", () => {
    const slow = prefetchPages([4], IDLE_SPEED + 100, 100).length;
    const fast = prefetchPages([4], 4000, 100).length;
    expect(fast).toBeGreaterThan(slow);
  });

  it("never runs past the ends of the document", () => {
    expect(prefetchPages([0], -2000, 5).every((p) => p >= 0)).toBe(true);
    expect(prefetchPages([4], 2000, 5)).toEqual([3]);
  });

  it("never re-requests a page that is already on screen", () => {
    const visible = [3, 4, 5];
    expect(prefetchPages(visible, 800, 20).some((p) => visible.includes(p))).toBe(false);
  });

  it("has nothing to do without a document", () => {
    expect(prefetchPages([], 1000, 0)).toEqual([]);
  });
});
