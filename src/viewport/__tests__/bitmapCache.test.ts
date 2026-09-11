/**
 * `ImageBitmap`s are compositor memory, not garbage-collected memory: what is
 * not closed is leaked. These tests are about the closing as much as the
 * eviction.
 */

import { describe, expect, it } from "vitest";
import { BitmapCache } from "../bitmapCache";

class FakeBitmap {
  closed = false;
  close(): void {
    this.closed = true;
  }
}

describe("BitmapCache", () => {
  it("returns what it stored", () => {
    const cache = new BitmapCache<FakeBitmap>(1000);
    const a = new FakeBitmap();
    cache.set("a", a, 100);
    expect(cache.get("a")).toBe(a);
    expect(cache.bytes).toBe(100);
  });

  it("evicts the least recently used and closes it", () => {
    const cache = new BitmapCache<FakeBitmap>(250);
    const a = new FakeBitmap();
    const b = new FakeBitmap();
    const c = new FakeBitmap();
    cache.set("a", a, 100);
    cache.set("b", b, 100);
    cache.get("a"); // a is now the newer of the two
    cache.set("c", c, 100);
    expect(b.closed).toBe(true);
    expect(a.closed).toBe(false);
    expect(cache.has("b")).toBe(false);
    expect(cache.bytes).toBeLessThanOrEqual(250);
  });

  it("closes the bitmap it replaces", () => {
    const cache = new BitmapCache<FakeBitmap>(1000);
    const first = new FakeBitmap();
    const second = new FakeBitmap();
    cache.set("a", first, 100);
    cache.set("a", second, 100);
    expect(first.closed).toBe(true);
    expect(cache.bytes).toBe(100);
    expect(cache.size).toBe(1);
  });

  it("refuses a bitmap larger than the whole budget instead of emptying itself", () => {
    const cache = new BitmapCache<FakeBitmap>(150);
    const keep = new FakeBitmap();
    const huge = new FakeBitmap();
    cache.set("keep", keep, 100);
    cache.set("huge", huge, 500);
    expect(huge.closed).toBe(true);
    expect(cache.has("keep")).toBe(true);
  });

  it("drops what a zoom made unusable", () => {
    // The key carries the scale, so a zoom keeps the previews and drops the
    // tiles rendered for the scale nobody is looking at any more.
    const cache = new BitmapCache<FakeBitmap>(10_000);
    const stale = new FakeBitmap();
    const preview = new FakeBitmap();
    cache.set("1/0/0/1000/0/0/sharp", stale, 100);
    cache.set("1/0/0/256/0/0/preview", preview, 10);
    const dropped = cache.keepOnly((key) => key.endsWith("/preview"));
    expect(dropped).toBe(1);
    expect(stale.closed).toBe(true);
    expect(preview.closed).toBe(false);
  });

  it("closes everything when a document is closed", () => {
    const cache = new BitmapCache<FakeBitmap>(1000);
    const a = new FakeBitmap();
    cache.set("a", a, 100);
    cache.clear();
    expect(a.closed).toBe(true);
    expect(cache.bytes).toBe(0);
    expect(cache.size).toBe(0);
  });
});
