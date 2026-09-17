import { describe, expect, it } from "vitest";
import { highlightBox } from "../highlights";

const place = { x: 24, y: 24, zoom: 1, pageHeight: 792 };

describe("highlightBox", () => {
  /// PDF space counts up from the bottom of the page, CSS counts down from the
  /// top. Getting this backwards puts every highlight on the wrong half of the
  /// page, which looks exactly like a highlight that does not work at all.
  it("flips the y axis", () => {
    const box = highlightBox({ left: 72, bottom: 700, right: 172, top: 712 }, place);
    expect(box.top).toBe(792 - 712);
    expect(box.left).toBe(72);
    expect(box.width).toBe(100);
    expect(box.height).toBe(12);
  });

  it("scales with the zoom", () => {
    const box = highlightBox({ left: 72, bottom: 700, right: 172, top: 712 }, { ...place, zoom: 2 });
    expect(box.left).toBe(144);
    expect(box.top).toBe((792 - 712) * 2);
    expect(box.width).toBe(200);
    expect(box.height).toBe(24);
  });

  /// A match on a hairline-thin glyph box still has to be visible; a zero-height
  /// element is invisible whatever colour it is.
  it("never produces an invisible box", () => {
    const box = highlightBox({ left: 10, bottom: 100, right: 10, top: 100 }, place);
    expect(box.width).toBeGreaterThan(0);
    expect(box.height).toBeGreaterThan(0);
  });

  /// The placement's own offset belongs to the layer, not to each box: boxes are
  /// positioned inside a host that is already at the page's corner.
  it("is relative to the page, not the document", () => {
    const box = highlightBox({ left: 0, bottom: 780, right: 10, top: 792 }, place);
    expect(box.left).toBe(0);
    expect(box.top).toBe(0);
  });
});
