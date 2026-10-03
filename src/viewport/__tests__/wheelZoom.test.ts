import { describe, expect, it } from "vitest";
import { wheelZoomFactor } from "../renderer";

describe("Ctrl+wheel zoom step", () => {
  it("follows a pinch exactly", () => {
    // Chromium reports a pinch frame of scale s as deltaY = -100·ln(s).
    for (const scale of [1.02, 1.1, 0.95]) {
      expect(wheelZoomFactor(-100 * Math.log(scale))).toBeCloseTo(scale, 6);
    }
  });

  it("takes a gentle step per mouse notch", () => {
    expect(wheelZoomFactor(-100)).toBeCloseTo(Math.exp(0.25), 6);
    expect(wheelZoomFactor(100)).toBeCloseTo(Math.exp(-0.25), 6);
  });

  it("never jumps more than twofold in one event", () => {
    expect(wheelZoomFactor(-5000)).toBe(2);
    expect(wheelZoomFactor(5000)).toBe(0.5);
  });
});
