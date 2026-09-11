/**
 * The viewport's own bitmap store.
 *
 * The backend already caches tile *bytes* (SPEC 9). This caches the decoded
 * `ImageBitmap`s, which is a different resource: a bitmap lives in the
 * compositor, is what `drawImage` can blit without touching the CPU, and is not
 * released by the garbage collector at any predictable moment — it has to be
 * closed. An unbounded map of them is a GPU memory leak with a JavaScript
 * accent.
 *
 * So: a byte-budgeted LRU that closes what it evicts. Losing a bitmap is cheap —
 * the bytes are still in the backend's cache, so re-decoding costs a copy and a
 * `createImageBitmap`, not a render.
 *
 * Generic over the bitmap type so the eviction rule can be tested without a
 * browser.
 */

export interface Closable {
  close(): void;
}

interface Entry<T> {
  readonly value: T;
  readonly bytes: number;
  seq: number;
}

export class BitmapCache<T extends Closable> {
  #entries = new Map<string, Entry<T>>();
  #bytes = 0;
  #budget: number;
  #seq = 0;

  constructor(budgetBytes: number) {
    this.#budget = Math.max(1, budgetBytes);
  }

  get bytes(): number {
    return this.#bytes;
  }

  get size(): number {
    return this.#entries.size;
  }

  get(key: string): T | undefined {
    const entry = this.#entries.get(key);
    if (!entry) return undefined;
    entry.seq = this.#seq++;
    return entry.value;
  }

  has(key: string): boolean {
    return this.#entries.has(key);
  }

  set(key: string, value: T, bytes: number): void {
    if (bytes > this.#budget) {
      // Nothing this large can be kept without emptying the cache first, and an
      // empty cache is worse than a missing entry.
      value.close();
      return;
    }
    const existing = this.#entries.get(key);
    if (existing) {
      this.#bytes -= existing.bytes;
      existing.value.close();
    }
    this.#entries.set(key, { value, bytes, seq: this.#seq++ });
    this.#bytes += bytes;
    this.#evict();
  }

  /** Drops everything whose key the predicate rejects. */
  keepOnly(keep: (key: string) => boolean): number {
    let dropped = 0;
    for (const [key, entry] of this.#entries) {
      if (keep(key)) continue;
      entry.value.close();
      this.#bytes -= entry.bytes;
      this.#entries.delete(key);
      dropped++;
    }
    return dropped;
  }

  clear(): void {
    for (const entry of this.#entries.values()) entry.value.close();
    this.#entries.clear();
    this.#bytes = 0;
  }

  #evict(): void {
    while (this.#bytes > this.#budget && this.#entries.size > 1) {
      let oldestKey: string | undefined;
      let oldestSeq = Number.POSITIVE_INFINITY;
      for (const [key, entry] of this.#entries) {
        if (entry.seq < oldestSeq) {
          oldestSeq = entry.seq;
          oldestKey = key;
        }
      }
      if (oldestKey === undefined) return;
      const entry = this.#entries.get(oldestKey);
      if (!entry) return;
      entry.value.close();
      this.#bytes -= entry.bytes;
      this.#entries.delete(oldestKey);
    }
  }
}

/**
 * How much of the compositor we are willing to hold.
 *
 * 384 MiB is roughly six screens of tiles at 4K. Past that the win from keeping
 * a bitmap decoded is smaller than the cost of the memory, because the bytes are
 * still one `createImageBitmap` away in the backend's cache.
 */
export const BITMAP_BUDGET_BYTES = 384 * 1024 * 1024;
