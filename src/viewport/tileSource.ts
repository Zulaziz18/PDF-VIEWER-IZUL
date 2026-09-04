/**
 * Fetches rendered tiles over the `izul://` protocol and turns them into
 * `ImageBitmap`s.
 *
 * This is the only module permitted to call `fetch` (see eslint.config.js). It
 * reaches a custom scheme served by our own process out of shared memory, never
 * the network — SPEC 4's offline rule is intact.
 *
 * Why not `invoke`: Tauri's command bridge serialises payloads as JSON, so a
 * 1 MiB BGRA tile would arrive as several MiB of base64 and cost a parse on the
 * UI thread. The whole 16 ms frame budget would be gone before anything drew.
 */

export interface TileHandle {
  readonly doc: number;
  readonly page: number;
  readonly slot: number;
  readonly epoch: number;
  readonly width: number;
  readonly height: number;
  readonly stride: number;
  readonly uri: string;
}

/** A tile whose pixels are ready to draw. */
export interface DecodedTile {
  readonly bitmap: ImageBitmap;
  readonly width: number;
  readonly height: number;
}

/**
 * Raised when the worker recycled the slot before the page asked for it.
 *
 * Normal during fast scrolling, and not an error the user should ever hear
 * about: the viewport simply requests the tile again at the current generation.
 */
export class StaleTileError extends Error {
  constructor(readonly handle: TileHandle) {
    super(`ubin sudah kedaluwarsa: slot ${handle.slot} epoch ${handle.epoch}`);
    this.name = "StaleTileError";
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
export async function loadTile(handle: TileHandle, signal?: AbortSignal): Promise<DecodedTile> {
  const response = await fetch(handle.uri, signal ? { signal } : {});
  if (response.status === 410) {
    throw new StaleTileError(handle);
  }
  if (!response.ok) {
    throw new Error(`ubin gagal dimuat: HTTP ${response.status}`);
  }

  const width = Number(response.headers.get("X-Izul-Width") ?? handle.width);
  const height = Number(response.headers.get("X-Izul-Height") ?? handle.height);
  const stride = Number(response.headers.get("X-Izul-Stride") ?? handle.stride);
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
  return { bitmap, width, height };
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
