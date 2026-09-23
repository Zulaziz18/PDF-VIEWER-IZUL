/**
 * The imperative page surface.
 *
 * No React in this folder (SPEC 4). React's job is to decide *what* is on
 * screen; this file decides what is *painted*, at whatever rate the monitor
 * asks for. Mixing the two would put a reconciliation pass inside the frame
 * budget.
 *
 * The interface here is deliberately narrow — clear, draw a bitmap at a
 * rectangle, present — so that the WebGL2 renderer SPEC 9 anticipates for
 * Phase 8 can replace it without anything above changing.
 */

import type { PxRect } from "./geometry";

export interface Surface {
  /** Resizes the backing store to `w` x `h` device pixels. */
  resize(w: number, h: number, dpr: number): void;
  /** Clears the whole surface to the page background colour. */
  clear(cssColor: string): void;
  /** Draws a decoded tile at a device-pixel rectangle. */
  drawTile(bitmap: ImageBitmap, at: PxRect): void;
  /** Draws a placeholder for a tile that has not arrived yet. */
  drawPlaceholder(at: PxRect, cssColor: string): void;
  /** Signals the end of a frame. */
  present(): void;
  readonly width: number;
  readonly height: number;
}

/**
 * Canvas 2D implementation, used from Phase 1.
 *
 * Deliberately allocation-free per frame: no gradients, no paths built on the
 * fly, no `save`/`restore` pairs. A frame is a sequence of `drawImage` calls,
 * which is what keeps a scroll pinned to the refresh rate.
 */
export class Canvas2DSurface implements Surface {
  #canvas: HTMLCanvasElement;
  #ctx: CanvasRenderingContext2D;
  #width = 0;
  #height = 0;

  constructor(canvas: HTMLCanvasElement) {
    const ctx = canvas.getContext("2d", { alpha: false, desynchronized: true });
    if (!ctx) {
      throw new Error("konteks canvas 2d tidak tersedia");
    }
    this.#canvas = canvas;
    this.#ctx = ctx;
    // Tiles are rendered by PDFium at exactly the size they are drawn, so any
    // smoothing here would only blur pixels that are already correct.
    this.#ctx.imageSmoothingEnabled = false;
  }

  get width(): number {
    return this.#width;
  }

  get height(): number {
    return this.#height;
  }

  resize(w: number, h: number, dpr: number): void {
    const pw = Math.max(1, Math.round(w * dpr));
    const ph = Math.max(1, Math.round(h * dpr));
    if (this.#canvas.width === pw && this.#canvas.height === ph) {
      return;
    }
    // Setting width/height clears the canvas and drops the backing store, so it
    // is done only when the size actually changed.
    this.#canvas.width = pw;
    this.#canvas.height = ph;
    this.#canvas.style.width = `${w}px`;
    this.#canvas.style.height = `${h}px`;
    this.#width = pw;
    this.#height = ph;
    this.#ctx.imageSmoothingEnabled = false;
  }

  clear(cssColor: string): void {
    this.#ctx.fillStyle = cssColor;
    this.#ctx.fillRect(0, 0, this.#width, this.#height);
  }

  drawTile(bitmap: ImageBitmap, at: PxRect): void {
    this.#ctx.drawImage(bitmap, at.x, at.y, at.w, at.h);
  }

  /**
   * A hairline and a soft drop under the page, so a white page still reads as
   * a sheet against a light canvas. Two flat rectangles rather than
   * `shadowBlur`, which would be a blur pass per page per frame during scroll.
   */
  drawPageFrame(at: PxRect, dpr: number): void {
    const ctx = this.#ctx;
    const edge = Math.max(1, Math.round(dpr));
    ctx.fillStyle = "rgba(0, 0, 0, 0.06)";
    ctx.fillRect(at.x - edge, at.y, at.w + 2 * edge, at.h + 3 * edge);
    ctx.fillStyle = "rgba(0, 0, 0, 0.12)";
    ctx.fillRect(at.x - edge, at.y - edge, at.w + 2 * edge, at.h + 2 * edge);
  }

  drawPlaceholder(at: PxRect, cssColor: string): void {
    this.#ctx.fillStyle = cssColor;
    this.#ctx.fillRect(at.x, at.y, at.w, at.h);
  }

  /**
   * The raw 2D context, for the annotation layer.
   *
   * Everything else here is an allocation-free `drawImage` per tile, which is
   * what keeps a scroll at refresh rate. Annotations are paths and text and
   * cannot be expressed that way, so the canvas backend draws them directly —
   * and it is the only caller allowed to, which is why this is a method with a
   * comment rather than a public field.
   */
  context(): CanvasRenderingContext2D {
    return this.#ctx;
  }

  present(): void {
    // Canvas 2D presents implicitly at the end of the task. The method exists so
    // the WebGL2 surface can swap buffers here without changing callers.
  }
}
