/**
 * Painting one page preview into a small canvas, for the sidebar.
 *
 * Lives here rather than in the component because it is canvas work (SPEC 4),
 * and because it goes through exactly the same tile path as the viewport: the
 * sidebar's thumbnail and the viewport's low-resolution tier are the same
 * bitmap at the same URI, so opening the sidebar costs no extra rendering.
 */

import { PRIORITY, loadTile, type TileRef } from "./tileSource";

/** Maximum edge of a thumbnail, matching the viewport's preview tier. */
export const THUMB_EDGE = 256;

export interface ThumbnailRequest {
  readonly doc: number;
  readonly page: number;
  readonly rotation: number;
  readonly generation: number;
}

/**
 * Draws a page preview into `canvas`, sizing the canvas to the bitmap.
 *
 * Returns false when the tile was superseded or the document went away, which
 * the caller treats as "try again on the next render" rather than an error.
 */
export async function paintThumbnail(
  canvas: HTMLCanvasElement,
  req: ThumbnailRequest,
  signal?: AbortSignal,
): Promise<boolean> {
  const ref: TileRef = {
    doc: req.doc,
    page: req.page,
    rotation: req.rotation,
    scale: THUMB_EDGE,
    col: 0,
    row: 0,
    tier: "preview",
  };
  try {
    const tile = await loadTile(ref, req.generation, PRIORITY.prefetch, signal);
    if (signal?.aborted) {
      tile.bitmap.close();
      return false;
    }
    canvas.width = tile.width;
    canvas.height = tile.height;
    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) {
      tile.bitmap.close();
      return false;
    }
    ctx.drawImage(tile.bitmap, 0, 0);
    tile.bitmap.close();
    return true;
  } catch {
    return false;
  }
}
