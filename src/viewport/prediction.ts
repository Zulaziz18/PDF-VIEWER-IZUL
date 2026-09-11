/**
 * Scroll prediction (SPEC 9).
 *
 * "Render the pages ahead before they are asked for" needs two numbers: which
 * way the user is going and how fast. Both come from the scroll offset over
 * time, smoothed, because a raw frame-to-frame delta on a trackpad is noise
 * with a direction attached.
 *
 * Pure and time-injected, so the behaviour is tested rather than watched.
 */

/** Tracks scroll velocity along the scrolling axis, in CSS pixels per second. */
export class ScrollTracker {
  #lastOffset: number | null = null;
  #lastTime = 0;
  #velocity = 0;

  /**
   * How much of the previous velocity survives one sample.
   *
   * High enough that a single stuttering frame does not reverse the prediction,
   * low enough that stopping is noticed within about a tenth of a second.
   */
  static readonly SMOOTHING = 0.6;

  /** A scroll offset seen at `timeMs`. */
  sample(offset: number, timeMs: number): void {
    if (this.#lastOffset === null) {
      this.#lastOffset = offset;
      this.#lastTime = timeMs;
      return;
    }
    const dt = timeMs - this.#lastTime;
    // Ignore samples closer than a millisecond: the division blows up and the
    // information content is nil.
    if (dt < 1) return;
    const instant = ((offset - this.#lastOffset) / dt) * 1000;
    this.#velocity = this.#velocity * ScrollTracker.SMOOTHING + instant * (1 - ScrollTracker.SMOOTHING);
    this.#lastOffset = offset;
    this.#lastTime = timeMs;
  }

  /** Signed velocity in CSS pixels per second. */
  get velocity(): number {
    return this.#velocity;
  }

  /** -1 backwards, 1 forwards, 0 when effectively still. */
  get direction(): -1 | 0 | 1 {
    if (this.#velocity > IDLE_SPEED) return 1;
    if (this.#velocity < -IDLE_SPEED) return -1;
    return 0;
  }

  /** Forgets the history, e.g. after a jump to a bookmark. */
  reset(): void {
    this.#lastOffset = null;
    this.#velocity = 0;
  }
}

/** Below this speed, in CSS px/s, the user counts as stationary. */
export const IDLE_SPEED = 40;

/** Most pages we will ever render ahead of the viewport. */
export const MAX_PREFETCH_PAGES = 8;

/**
 * The pages to render ahead, in the order they should be requested.
 *
 * While the user is moving, the prediction reaches further the faster they go.
 * When they stop, it becomes symmetric — one page either side — because the
 * next thing that happens after a pause is as likely to be a scroll back as a
 * scroll on.
 */
export function prefetchPages(
  visible: readonly number[],
  velocity: number,
  pageCount: number,
): number[] {
  if (pageCount <= 0 || visible.length === 0) return [];
  const first = Math.min(...visible);
  const last = Math.max(...visible);
  const speed = Math.abs(velocity);

  if (speed <= IDLE_SPEED) {
    return clampPages([last + 1, first - 1], pageCount, visible);
  }
  // One page per 400 px/s of travel: at a leisurely scroll that is a page or
  // two, and at a flung trackpad it saturates rather than queueing a hundred
  // renders nobody will see.
  const reach = Math.min(MAX_PREFETCH_PAGES, Math.max(1, Math.round(speed / 400)));
  const forwards = velocity > 0;
  const out: number[] = [];
  for (let i = 1; i <= reach; i++) {
    out.push(forwards ? last + i : first - i);
  }
  // One page behind, always: reversing a scroll is common and cheap to cover.
  out.push(forwards ? first - 1 : last + 1);
  return clampPages(out, pageCount, visible);
}

function clampPages(
  candidates: readonly number[],
  pageCount: number,
  exclude: readonly number[],
): number[] {
  const seen = new Set(exclude);
  const out: number[] = [];
  for (const p of candidates) {
    if (p < 0 || p >= pageCount || seen.has(p)) continue;
    seen.add(p);
    out.push(p);
  }
  return out;
}
