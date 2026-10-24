/**
 * Which tiles to ask for now, out of all the tiles the current frame wants
 * (7.1.1).
 *
 * Before this, every frame sent a request for every tile it was missing, at
 * once. A fling through a zoomed-out, two-page view passes dozens of pages a
 * second, and each one queued a preview and a sharp render. A document lives
 * on one worker, and on Windows every answer also crosses WebView2 one at a
 * time. So the queue filled with pages the user had already scrolled past, and
 * the pages on screen when the scroll stopped waited behind all of them. The
 * user saw a view that stayed blurry for seconds.
 *
 * Now the frame only says what it wants. At most `limit` requests are out at a
 * time, and the rest are chosen again on the next frame, from what is on
 * screen *then*. A page that has scrolled away is never asked for. A request
 * already sent is left to finish: aborting the fetch would not stop the
 * worker, it would only free a slot for yet more work behind it.
 */

/** One tile a frame would like to have. */
export interface Want<T> {
  readonly key: string;
  /** Higher first; equal priorities keep the frame's order (top of the view first). */
  readonly priority: number;
  readonly item: T;
}

/**
 * Most tile requests one viewport keeps in flight. Enough to keep a worker
 * busy while the previous answer travels back; few enough that what is queued
 * behind the worker is always what the user is looking at.
 */
export const MAX_IN_FLIGHT = 4;

/**
 * The wants to send now: the most urgent ones not already in flight, as many
 * as there are free slots. Duplicate keys count once.
 */
export function nextToSend<T>(
  wanted: readonly Want<T>[],
  inFlight: ReadonlySet<string>,
  limit: number = MAX_IN_FLIGHT,
): Want<T>[] {
  const free = limit - inFlight.size;
  if (free <= 0) return [];
  const seen = new Set<string>();
  const candidates: { want: Want<T>; order: number }[] = [];
  wanted.forEach((want, order) => {
    if (inFlight.has(want.key) || seen.has(want.key)) return;
    seen.add(want.key);
    candidates.push({ want, order });
  });
  candidates.sort((a, b) => b.want.priority - a.want.priority || a.order - b.order);
  return candidates.slice(0, free).map((c) => c.want);
}
