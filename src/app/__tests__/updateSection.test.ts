import { describe, expect, it } from "vitest";
import { percent } from "../UpdateSection";

describe("update download progress", () => {
  it("is unknown until the size is", () => {
    expect(percent(1000, null)).toBeNull();
    expect(percent(1000, 0)).toBeNull();
  });

  it("rounds and never passes 100", () => {
    expect(percent(1, 3)).toBe(33);
    expect(percent(5, 4)).toBe(100);
  });
});
