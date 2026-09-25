/**
 * The tile URI is a contract with the backend's parser (see
 * `src-tauri/src/protocol.rs`). These assertions are the frontend half of it.
 */

import { describe, expect, it } from "vitest";
import { PRIORITY, tileKey, tileUri, type TileRef } from "../tileSource";

const ref: TileRef = {
  doc: 7,
  page: 3,
  rotation: 1,
  scale: 1500,
  col: 2,
  row: 1,
  tier: "sharp",
};

describe("tileUri", () => {
  it("writes the path the backend parses", () => {
    expect(tileUri(ref, 9, PRIORITY.visible)).toBe(
      "http://izul.localhost/tile/7/3/1/1500/2/1/sharp?g=9&p=2",
    );
  });

  it("carries the priority the scheduler reads", () => {
    expect(tileUri(ref, 1, PRIORITY.prefetch)).toContain("p=0");
    expect(tileUri(ref, 1, PRIORITY.preview)).toContain("p=1");
  });

  it("names a preview by its edge length rather than a scale", () => {
    const preview: TileRef = { ...ref, tier: "preview", scale: 256, col: 0, row: 0 };
    expect(tileUri(preview, 4, PRIORITY.preview)).toBe(
      "http://izul.localhost/tile/7/3/1/256/0/0/preview?g=4&p=1",
    );
  });
});

describe("tileKey", () => {
  it("identifies a tile by content, not by when it was asked for", () => {
    expect(tileKey(ref)).toBe("7/3/1/1500/2/1/sharp");
    // Generation and priority are scheduling, not identity: a tile does not
    // become a different tile because the user zoomed and came back.
    expect(tileUri(ref, 1, PRIORITY.visible)).not.toBe(tileUri(ref, 2, PRIORITY.visible));
    expect(tileKey({ ...ref })).toBe(tileKey(ref));
  });

  it("separates every field that changes the pixels", () => {
    const keys = new Set([
      tileKey(ref),
      tileKey({ ...ref, page: 4 }),
      tileKey({ ...ref, rotation: 2 }),
      tileKey({ ...ref, scale: 3000 }),
      tileKey({ ...ref, col: 3 }),
      tileKey({ ...ref, row: 2 }),
      tileKey({ ...ref, tier: "preview" }),
    ]);
    expect(keys.size).toBe(7);
  });
});

describe("dark mode's inversion", () => {
  it("is part of the tile's URI and of its cache key, and only when on", async () => {
    const { setPageInversion, tileKey, tileUri, PRIORITY } = await import("../tileSource");
    const ref = { doc: 1, page: 2, rotation: 0, scale: 1500, col: 0, row: 0, tier: "sharp" as const };
    const light = [tileUri(ref, 3, PRIORITY.visible), tileKey(ref)];
    expect(setPageInversion(true)).toBe(true);
    expect(setPageInversion(true)).toBe(false);
    const dark = [tileUri(ref, 3, PRIORITY.visible), tileKey(ref)];
    setPageInversion(false);
    expect(light[0]).not.toContain("inv=");
    expect(dark[0]).toContain("&inv=1");
    expect(dark[1]).not.toBe(light[1]);
    // Still under the page's prefix, so a page's bitmaps are dropped together.
    expect(dark[1]?.startsWith("1/2/")).toBe(true);
    expect([tileUri(ref, 3, PRIORITY.visible), tileKey(ref)]).toEqual(light);
  });
});
