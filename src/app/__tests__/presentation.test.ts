import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setFullscreen: async () => undefined }) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => null }));

describe("presentation keys", () => {
  it("turns pages the way a presenter's clicker does, and stops at the ends", async () => {
    const { step } = await import("../presentation");
    expect(step("PageDown", false, 0, 5)).toBe(1);
    expect(step("ArrowRight", false, 3, 5)).toBe(4);
    expect(step(" ", false, 4, 5)).toBe(4);
    expect(step(" ", true, 2, 5)).toBe(1);
    expect(step("PageUp", false, 0, 5)).toBe(0);
    expect(step("Backspace", false, 3, 5)).toBe(2);
    expect(step("Home", false, 3, 5)).toBe(0);
    expect(step("End", false, 0, 5)).toBe(4);
    expect(step("a", false, 0, 5)).toBeNull();
    expect(step("Escape", false, 0, 5)).toBeNull();
  });
});

describe("print layout", () => {
  it("fits to the paper, or prints at the page's real size in points", async () => {
    const { printLayout, printUrl } = await import("../print");
    expect(printLayout("fit", 2480, 3508, 300)).toEqual({ width: "100%", height: "100%" });
    // A4 rendered at 300 dpi is 2480 x 3508 px: 595.20 x 841.92 pt.
    expect(printLayout("actual", 2480, 3508, 300)).toEqual({ width: "595.20pt", height: "841.92pt" });
    expect(printUrl(3, 7)).toBe("http://izul.localhost/print/3/7");
  });
});
