import { describe, expect, it } from "vitest";
import { placeNotepad } from "../NotePopup";

const view = { width: 1000, height: 600 };
const size = { width: 240, height: 190 };

describe("notepad placement", () => {
  it("opens to the right of the badge", () => {
    const at = placeNotepad({ left: 100, top: 50, right: 122, bottom: 74 }, view, size);
    expect(at.left).toBe(130);
    expect(at.top).toBe(50);
  });

  it("opens to the left when the right side has no room", () => {
    const at = placeNotepad({ left: 900, top: 50, right: 922, bottom: 74 }, view, size);
    expect(at.left + size.width).toBeLessThanOrEqual(900);
  });

  it("never runs past the right edge, even beside a badge at the edge", () => {
    const at = placeNotepad({ left: 990, top: 50, right: 1012, bottom: 74 }, view, size);
    expect(at.left + size.width).toBeLessThanOrEqual(view.width);
  });

  it("stays inside the viewport at the bottom edge", () => {
    const at = placeNotepad({ left: 100, top: 580, right: 122, bottom: 604 }, view, size);
    expect(at.top + size.height).toBeLessThanOrEqual(view.height);
  });

  it("stays inside the viewport when the badge has scrolled above it", () => {
    const at = placeNotepad({ left: 100, top: -40, right: 122, bottom: -16 }, view, size);
    expect(at.top).toBeGreaterThanOrEqual(0);
  });
});
