/**
 * The queue between a frame's wants and the worker (7.1.1). What it must
 * guarantee is that the worker only ever has a handful of this view's tiles
 * ahead of it, and that those are the most urgent ones on screen now.
 */

import { describe, expect, it } from "vitest";
import { MAX_IN_FLIGHT, nextToSend, type Want } from "../requestQueue";

const want = (key: string, priority: number): Want<string> => ({ key, priority, item: key });

describe("nextToSend", () => {
  it("never has more than the limit in flight", () => {
    const wanted = Array.from({ length: 50 }, (_, i) => want(`t${i}`, 2));
    expect(nextToSend(wanted, new Set())).toHaveLength(MAX_IN_FLIGHT);
    expect(nextToSend(wanted, new Set(["a", "b", "c"]), 4)).toHaveLength(1);
    expect(nextToSend(wanted, new Set(["a", "b", "c", "d"]), 4)).toEqual([]);
  });

  it("sends the most urgent first, in the frame's order within a priority", () => {
    const wanted = [want("prefetch", 0), want("sharp-top", 2), want("preview", 1), want("sharp-bottom", 2)];
    expect(nextToSend(wanted, new Set(), 3).map((w) => w.key)).toEqual(["sharp-top", "sharp-bottom", "preview"]);
  });

  it("does not send what is already in flight, or the same tile twice", () => {
    const wanted = [want("a", 2), want("a", 2), want("b", 2), want("c", 1)];
    expect(nextToSend(wanted, new Set(["b"]), 4).map((w) => w.key)).toEqual(["a", "c"]);
  });

  it("forgets what the frame no longer wants: a page scrolled away is never asked for", () => {
    // Frame 1 wants pages 1..20; four go out.
    const first = Array.from({ length: 20 }, (_, i) => want(`p${i + 1}`, 2));
    const inFlight = new Set(nextToSend(first, new Set()).map((w) => w.key));
    // The user flung on: frame 2 sees pages 60..63 only. Nothing of 5..20 is sent.
    const second = [60, 61, 62, 63].map((p) => want(`p${p}`, 2));
    inFlight.clear(); // the first four answered
    expect(nextToSend(second, inFlight).map((w) => w.key)).toEqual(["p60", "p61", "p62", "p63"]);
  });
});
