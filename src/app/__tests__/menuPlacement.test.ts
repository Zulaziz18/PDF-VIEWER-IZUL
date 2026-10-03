import { describe, expect, it } from "vitest";
import { placeMenu } from "@/design/controls";

const view = { width: 1366, height: 768 };
const menu = { width: 200, height: 260 };

describe("menu placement", () => {
  it("opens under a button with room below", () => {
    const at = placeMenu({ left: 900, right: 960, top: 80, bottom: 140 }, menu, view, "left");
    expect(at.top).toBe(144);
    expect(at.left).toBe(900);
    expect(at.maxHeight).toBeGreaterThanOrEqual(menu.height);
  });

  it("opens above a button at the bottom of the window", () => {
    // The zoom menu in the bottom bar: no room below at all.
    const at = placeMenu({ left: 1200, right: 1260, top: 740, bottom: 764 }, menu, view, "left");
    expect(at.top + menu.height).toBeLessThanOrEqual(740);
    expect(at.top).toBeGreaterThanOrEqual(0);
  });

  it("never runs past the right edge", () => {
    const at = placeMenu({ left: 1320, right: 1360, top: 80, bottom: 110 }, menu, view, "left");
    expect(at.left + menu.width).toBeLessThanOrEqual(view.width);
  });

  it("aligns its right edge to the button when asked", () => {
    const at = placeMenu({ left: 600, right: 700, top: 80, bottom: 110 }, menu, view, "right");
    expect(at.left + menu.width).toBe(700);
  });

  it("scrolls instead of overflowing when neither side has room", () => {
    const tall = { width: 200, height: 2000 };
    const at = placeMenu({ left: 10, right: 50, top: 300, bottom: 330 }, tall, view, "left");
    expect(at.top).toBeGreaterThanOrEqual(0);
    expect(at.top + at.maxHeight).toBeLessThanOrEqual(view.height);
  });
});
