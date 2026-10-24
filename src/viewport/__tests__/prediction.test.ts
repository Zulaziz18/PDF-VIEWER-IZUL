/**
 * Prefetch is a guess, and a guess that is wrong in the expensive direction —
 * queueing renders nobody looks at — is worse than not guessing.
 */

import { describe, expect, it } from "vitest";
import {
  FLING_SPEED,
  IDLE_SPEED,
  MAX_PREFETCH_PAGES,
  SETTLE_MS,
  ScrollTracker,
  prefetchPages,
} from "../prediction";

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

describe("ScrollTracker.isFlinging (7.1.1)", () => {
  /** A touchpad flick: 100 px a frame, about 6000 px/s. */
  function flung(): ScrollTracker {
    const t = new ScrollTracker();
    for (let i = 0; i <= 10; i++) t.sample(i * 100, i * 16);
    return t;
  }

  it("is flinging while a fast scroll is still moving", () => {
    expect(flung().isFlinging(160 + 10)).toBe(true);
  });

  it("stops flinging once no scroll event has come for a while", () => {
    // The velocity never decays without events: only the clock can say the
    // fling stopped dead. Without this the view would stay blurry for good.
    const t = flung();
    expect(t.velocity).toBeGreaterThan(FLING_SPEED);
    expect(t.isFlinging(160 + SETTLE_MS + 1)).toBe(false);
    expect(t.settlesAt()).toBe(160 + SETTLE_MS);
  });

  it("does not count reading speed as a fling", () => {
    const t = new ScrollTracker();
    for (let i = 0; i <= 10; i++) t.sample(i * 5, i * 16); // ~300 px/s
    expect(t.isFlinging(160)).toBe(false);
  });

  it("is not flinging before any scroll", () => {
    expect(new ScrollTracker().isFlinging(0)).toBe(false);
  });
});
