/**
 * Fetches rendered tiles over the `izul` custom protocol and turns them into
 * `ImageBitmap`s.
 *
 * This is the only module permitted to call `fetch` (see eslint.config.js). It
 * reaches a custom scheme served by our own process out of shared memory, never
 * the network — SPEC 4's offline rule is intact.
 *
 * Why not `invoke`: Tauri's command bridge serialises payloads as JSON, so a
 * 1 MiB BGRA tile would arrive as several MiB of base64 and cost a parse on the
 * UI thread. The whole 16 ms frame budget would be gone before anything drew.
 *
 * Why `https://izul.localhost/...` and not `izul://...`: WebView2 (the engine
 * Tauri uses on Windows) refuses to `fetch()` a bare custom scheme — only
 * `http`/`https` are fetchable there, a restriction macOS and Linux's webviews
 * do not share. Tauri's registered-protocol handler answers both spellings, so
 * writing the `https://<scheme>.localhost/...` form is what makes the same
 * code work on every platform SPEC 4 targets, Windows included.
 *
 * A tile is addressed by what it *is* rather than by where it happens to live,
 * so the same URI is the cache key in the webview, in the backend's LRU, and in
 * the log. One fetch either hits a cached bitmap or schedules the render and
 * waits for it; there is no separate "please render this" call to get out of
 * step with.
 */

/** Which tier of SPEC 9's two-tier render a request belongs to. */
export type TileTier = "sharp" | "fast" | "preview";

/** How urgent a request is; mirrors `Priority` in the scheduler. */
export const PRIORITY = { prefetch: 0, preview: 1, visible: 2 } as const;
export type Priority = (typeof PRIORITY)[keyof typeof PRIORITY];

/** Identity of one tile. Every field is part of the cache key on both sides. */
export interface TileRef {
  readonly doc: number;
  readonly page: number;
  /** Total rotation in quarter turns, 0..3. */
  readonly rotation: number;
  /**
   * Pixels per PDF point in thousandths — or, for the preview tier, the
   * bitmap's maximum edge in pixels.
   */
  readonly scale: number;
  readonly col: number;
  readonly row: number;
  readonly tier: TileTier;
}

/** The URI that identifies a tile, and doubles as its cache key. */
export function tileUri(ref: TileRef, generation: number, priority: Priority): string {
  return (
    `https://izul.localhost/tile/${ref.doc}/${ref.page}/${ref.rotation}/${ref.scale}/${ref.col}/${ref.row}/${ref.tier}` +
    `?g=${generation}&p=${priority}`
  );
}

/** A stable key for a tile, without the scheduling parameters. */
export function tileKey(ref: TileRef): string {
  return `${ref.doc}/${ref.page}/${ref.rotation}/${ref.scale}/${ref.col}/${ref.row}/${ref.tier}`;
}

/** A tile whose pixels are ready to draw. */
export interface DecodedTile {
  readonly bitmap: ImageBitmap;
  readonly width: number;
  readonly height: number;
  /** Bytes the bitmap occupies, for the viewport's own memory budget. */
  readonly bytes: number;
}

/**
 * Raised when the request belonged to a layout epoch the user has already left,
 * or when the scheduler shed it under load.
 *
 * Normal during a fast zoom, and not an error the user should ever hear about:
 * the viewport simply asks again at the current generation.
 */
export class SupersededTileError extends Error {
  constructor(readonly ref: TileRef) {
    super(`ubin dari generasi lama: ${tileKey(ref)}`);
    this.name = "SupersededTileError";
  }
}

/** Raised when the worker recycled the shared-memory slot before we read it. */
export class StaleTileError extends Error {
  constructor(readonly ref: TileRef) {
    super(`ubin sudah kedaluwarsa: ${tileKey(ref)}`);
    this.name = "StaleTileError";
  }
}

/** Raised when the document or page is gone — a closed tab, usually. */
export class MissingTileError extends Error {
  constructor(readonly ref: TileRef) {
    super(`ubin tidak tersedia: ${tileKey(ref)}`);
    this.name = "MissingTileError";
  }
}

/**
 * Converts BGRA (PDFium's native order) to the RGBA that `ImageData` wants.
 *
 * Done in place over the transferred buffer, so no second allocation. This is
 * the one per-pixel pass on the UI side; PDFium's own RGBA output path costs the
 * same swizzle inside the worker, where it would compete with rendering instead.
 */
function bgraToRgbaInPlace(buf: Uint8ClampedArray<ArrayBuffer>): void {
  for (let i = 0; i + 3 < buf.length; i += 4) {
    const b = buf[i] as number;
    buf[i] = buf[i + 2] as number;
    buf[i + 2] = b;
  }
}

/**
 * Fetches one tile and decodes it.
 *
 * `signal` lets the viewport abandon tiles for a generation the user has already
 * zoomed past, which is the frontend half of SPEC 6's cancellation rule.
 */
export async function loadTile(
  ref: TileRef,
  generation: number,
  priority: Priority,
  signal?: AbortSignal,
): Promise<DecodedTile> {
  const response = await fetch(tileUri(ref, generation, priority), signal ? { signal } : {});
  if (response.status === 409) {
    throw new SupersededTileError(ref);
  }
  if (response.status === 410) {
    throw new StaleTileError(ref);
  }
  if (response.status === 404) {
    throw new MissingTileError(ref);
  }
  if (!response.ok) {
    throw new Error(`ubin gagal dimuat: HTTP ${response.status}`);
  }

  const width = Number(response.headers.get("X-Izul-Width") ?? 0);
  const height = Number(response.headers.get("X-Izul-Height") ?? 0);
  const stride = Number(response.headers.get("X-Izul-Stride") ?? width * 4);
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
    throw new Error("ubin tanpa dimensi yang sah");
  }

  // `ArrayBuffer` explicitly, not `ArrayBufferLike`: `ImageData` refuses a
  // view that might be backed by a `SharedArrayBuffer`.
  const buffer: ArrayBuffer = await response.arrayBuffer();
  const raw: Uint8ClampedArray<ArrayBuffer> = new Uint8ClampedArray(buffer);
  const tight = stride === width * 4 ? raw : repack(raw, width, height, stride);
  bgraToRgbaInPlace(tight);

  const image = new ImageData(tight, width, height);
  // `createImageBitmap` hands the pixels to the compositor; from here on the
  // draw is a GPU blit rather than a CPU copy.
  const bitmap = await createImageBitmap(image);
  return { bitmap, width, height, bytes: width * height * 4 };
}

/** Copies a padded bitmap into a tightly packed one. */
function repack(
  src: Uint8ClampedArray,
  width: number,
  height: number,
  stride: number,
): Uint8ClampedArray<ArrayBuffer> {
  const out = new Uint8ClampedArray(new ArrayBuffer(width * height * 4));
  const rowBytes = width * 4;
  for (let y = 0; y < height; y++) {
    const from = y * stride;
    out.set(src.subarray(from, from + rowBytes), y * rowBytes);
  }
  return out;
}
