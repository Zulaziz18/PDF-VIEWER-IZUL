/**
 * The viewport: everything that decides what is painted, and when.
 *
 * No React in this folder (SPEC 4). React decides *what is on screen* — which
 * document, which zoom, which sidebar; this file decides what is *painted*, at
 * whatever rate the monitor asks for. Mixing the two would put a reconciliation
 * pass inside the frame budget.
 *
 * The pipeline, in the order a page reaches the glass (SPEC 9):
 *
 * 1. **Placeholder.** Every page has its real size from the page tree before
 *    anything is rendered, so the scroll bar is correct from the first frame
 *    and a page never appears as a white rectangle of the wrong shape.
 * 2. **Preview.** A thumbnail-resolution render of the whole page, upscaled.
 *    It costs a couple of milliseconds and is what the user sees while
 *    scrolling fast. It is also the sidebar's thumbnail — the same bitmap,
 *    rendered once.
 * 3. **Tiles.** 512x512 at the current scale, drawn over the preview as they
 *    arrive. Only the visible ones are requested.
 * 4. **Prefetch.** Previews for the pages the scroll is heading towards.
 *
 * Cancellation runs through all of it: every request carries the layout epoch
 * it belongs to, and when the epoch changes the in-flight fetches are aborted
 * and the backend drops everything queued for the old one.
 */

import { BitmapCache, BITMAP_BUDGET_BYTES } from "./bitmapCache";
import { Canvas2DSurface } from "./canvas";
import type { PageMetrics, PdfRect, Rotation, Scale } from "./geometry";
import { displaySize, pageSizePx, scaleKey, tilesCovering, zoomAbout } from "./geometry";
import type { Layout, ViewMode, ViewRect } from "./layout";
import { boxOf, dominantPage, layoutDocument, scrollToPage, visiblePages } from "./layout";
import { prefetchPages, ScrollTracker } from "./prediction";
import { buildHighlightLayer } from "./highlights";
import type { TextChar } from "./textLayer";
import { buildTextLayer, fitTextLayer, groupIntoLines } from "./textLayer";
import type { Priority, TileRef } from "./tileSource";
import {
  loadTile,
  MissingTileError,
  PRIORITY,
  StaleTileError,
  SupersededTileError,
  tileKey,
} from "./tileSource";

/** Maximum edge of a preview bitmap, in pixels. Doubles as the thumbnail size. */
export const PREVIEW_EDGE = 256;

/** Gap between pages and padding around the document, in CSS pixels (SPEC 12's 8px grid). */
const PAGE_GAP = 16;
const PAGE_PADDING = 24;

/** How far beyond the viewport a page still counts as visible, in CSS pixels. */
const VISIBLE_MARGIN = 200;

/** The elements the viewport owns. */
export interface RendererHost {
  readonly canvas: HTMLCanvasElement;
  /** The scrolling element; the renderer reads its offsets and listens to it. */
  readonly scroller: HTMLElement;
  /** Sized to the whole document so the scroll bar is real. */
  readonly spacer: HTMLElement;
  /** Holds one absolutely positioned text layer per visible page. */
  readonly textLayer: HTMLElement;
}

/** Everything the renderer needs from the store to paint a frame. */
export interface RendererState {
  readonly doc: number | null;
  readonly pageSizes: readonly PageMetrics[];
  readonly zoom: number;
  readonly mode: ViewMode;
  readonly docRotation: Rotation;
  readonly pageRotation: Readonly<Record<number, Rotation>>;
  readonly generation: number;
  /** Character boxes per page, filled in as the backend answers. */
  readonly texts: ReadonlyMap<number, readonly TextChar[]>;
  /** Boxes to paint over search matches, per page, in display space. */
  readonly highlights: ReadonlyMap<number, readonly PdfRect[]>;
}

export interface RendererEvents {
  /** The page the user is looking at changed. */
  onPage(page: number): void;
  /** The scroll offset changed, in content-space CSS pixels. */
  onScroll(x: number, y: number): void;
  /** A zoom gesture, already clamped, with the anchor to keep still. */
  onZoom(zoom: number): void;
  /** The viewport's CSS size changed; fit modes recompute from it. */
  onViewport(width: number, height: number): void;
  /** These pages are on screen and their text has not been fetched yet. */
  onWantText(pages: readonly number[]): void;
}

export interface FrameStats {
  /** Milliseconds spent in the last paint. */
  lastFrameMs: number;
  /** Tiles drawn in the last paint. */
  tilesDrawn: number;
  /** Tiles the last paint wanted but did not have. */
  tilesMissing: number;
  /** Requests in flight. */
  pending: number;
  bitmapBytes: number;
}

const EMPTY_LAYOUT: Layout = {
  pages: [],
  rows: [],
  width: 1,
  height: 1,
  mode: "single",
  zoom: 1,
  horizontal: false,
};

export class ViewportRenderer {
  #host: RendererHost;
  #events: RendererEvents;
  #surface: Canvas2DSurface;
  #bitmaps = new BitmapCache<ImageBitmap>(BITMAP_BUDGET_BYTES);
  #tracker = new ScrollTracker();

  #state: RendererState = {
    doc: null,
    pageSizes: [],
    zoom: 1,
    mode: "single",
    docRotation: 0,
    pageRotation: {},
    generation: 0,
    texts: new Map(),
    highlights: new Map(),
  };
  #layout: Layout = EMPTY_LAYOUT;
  #dpr = 1;
  #frame = 0;
  #abort = new AbortController();
  #pending = new Set<string>();
  #textSignatures = new Map<number, string>();
  #highlightSignatures = new Map<number, string>();
  #reportedPage = -1;
  #stats: FrameStats = {
    lastFrameMs: 0,
    tilesDrawn: 0,
    tilesMissing: 0,
    pending: 0,
    bitmapBytes: 0,
  };
  /** Set while a zoom is in flight, so the anchor can be restored afterwards. */
  #zoomAnchor: { contentX: number; contentY: number; clientX: number; clientY: number } | null =
    null;
  #resize: ResizeObserver;
  #dprWatcher: MediaQueryList | null = null;
  #onScroll = (): void => {
    const now = performance.now();
    this.#tracker.sample(
      this.#layout.horizontal ? this.#host.scroller.scrollLeft : this.#host.scroller.scrollTop,
      now,
    );
    this.#events.onScroll(this.#host.scroller.scrollLeft, this.#host.scroller.scrollTop);
    this.requestFrame();
  };
  #onWheel = (e: WheelEvent): void => {
    // Ctrl+wheel is zoom, on a mouse and on a trackpad pinch alike: the browser
    // reports a pinch as a wheel event with ctrlKey set (SPEC 11.1).
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();
    const rect = this.#host.scroller.getBoundingClientRect();
    const clientX = e.clientX - rect.left;
    const clientY = e.clientY - rect.top;
    const scroll = {
      x: this.#host.scroller.scrollLeft,
      y: this.#host.scroller.scrollTop,
    };
    // A wheel notch is ~100 units; the exponent turns any device's deltas into
    // the same perceptual step and keeps the zoom smooth on a trackpad.
    const factor = Math.exp(-e.deltaY / 400);
    const next = this.#state.zoom * factor;
    this.#zoomAnchor = {
      contentX: scroll.x + clientX,
      contentY: scroll.y + clientY,
      clientX,
      clientY,
    };
    this.#events.onZoom(next);
  };

  constructor(host: RendererHost, events: RendererEvents) {
    this.#host = host;
    this.#events = events;
    this.#surface = new Canvas2DSurface(host.canvas);
    this.#dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;

    host.scroller.addEventListener("scroll", this.#onScroll, { passive: true });
    host.scroller.addEventListener("wheel", this.#onWheel, { passive: false });

    this.#resize = new ResizeObserver(() => {
      this.#events.onViewport(host.scroller.clientWidth, host.scroller.clientHeight);
      this.requestFrame();
    });
    this.#resize.observe(host.scroller);
    this.#watchDpr();
  }

  /** Applies new state and repaints. Called by the React shell. */
  update(state: RendererState): void {
    const previous = this.#state;
    this.#state = state;

    const relayout =
      previous.doc !== state.doc ||
      previous.zoom !== state.zoom ||
      previous.mode !== state.mode ||
      previous.docRotation !== state.docRotation ||
      previous.pageRotation !== state.pageRotation ||
      previous.pageSizes !== state.pageSizes;

    if (previous.generation !== state.generation) {
      // A new layout epoch: everything in flight belongs to a view the user has
      // already left (SPEC 6).
      this.#abort.abort();
      this.#abort = new AbortController();
      this.#pending.clear();
    }
    if (previous.doc !== state.doc) {
      this.#bitmaps.clear();
      this.#textSignatures.clear();
      this.#highlightSignatures.clear();
      this.#host.textLayer.replaceChildren();
      this.#tracker.reset();
      this.#reportedPage = -1;
    }
    if (relayout) {
      this.#relayout();
      if (previous.zoom !== state.zoom) {
        this.#applyZoomAnchor(previous.zoom, state.zoom);
        // Bitmaps rendered for another scale are dead weight in the compositor:
        // they can never be drawn again at this zoom.
        const live = `/${scaleKey(this.#scale())}/`;
        this.#bitmaps.keepOnly((key) => key.includes(live) || key.endsWith("/preview"));
      }
    }
    this.requestFrame();
  }

  /** Scrolls so a page's top-left corner is at the viewport's origin. */
  goToPage(page: number, offsetPoints?: number): void {
    const target = scrollToPage(this.#layout, page, PAGE_PADDING / 2);
    let y = target.y;
    if (offsetPoints !== undefined) {
      const box = boxOf(this.#layout, page);
      const size = this.#displaySizeOf(page);
      if (box && size) {
        // A destination's y is measured from the page's bottom in points.
        y = box.y + Math.max(0, (size.height - offsetPoints) * this.#state.zoom) - PAGE_PADDING / 2;
      }
    }
    this.#tracker.reset();
    this.#host.scroller.scrollTo({ left: target.x, top: Math.max(0, y), behavior: "auto" });
    this.requestFrame();
  }

  /** The scroll offset, for the store to persist as the reading position. */
  scrollOffset(): { x: number; y: number } {
    return { x: this.#host.scroller.scrollLeft, y: this.#host.scroller.scrollTop };
  }

  /** Restores a scroll offset, e.g. the one saved when the file was last open. */
  restoreScroll(x: number, y: number): void {
    this.#tracker.reset();
    this.#host.scroller.scrollTo({ left: x, top: y, behavior: "auto" });
    this.requestFrame();
  }

  stats(): FrameStats {
    return { ...this.#stats, pending: this.#pending.size, bitmapBytes: this.#bitmaps.bytes };
  }

  /** The current layout, for callers that need page boxes (the sidebar). */
  layout(): Layout {
    return this.#layout;
  }

  destroy(): void {
    this.#abort.abort();
    this.#resize.disconnect();
    this.#dprWatcher?.removeEventListener("change", this.#onDprChange);
    this.#host.scroller.removeEventListener("scroll", this.#onScroll);
    this.#host.scroller.removeEventListener("wheel", this.#onWheel);
    if (this.#frame) cancelAnimationFrame(this.#frame);
    this.#bitmaps.clear();
  }

  requestFrame(): void {
    if (this.#frame) return;
    this.#frame = requestAnimationFrame(() => {
      this.#frame = 0;
      this.#draw();
    });
  }

  // ---- internals ---------------------------------------------------------

  #onDprChange = (): void => {
    this.#dpr = window.devicePixelRatio || 1;
    // Tiles are rendered for a specific pixel density; on a move to another
    // monitor every one of them is the wrong resolution (SPEC 9).
    this.#bitmaps.keepOnly(() => false);
    this.#watchDpr();
    this.requestFrame();
  };

  #watchDpr(): void {
    if (typeof window === "undefined" || !window.matchMedia) return;
    this.#dprWatcher?.removeEventListener("change", this.#onDprChange);
    this.#dprWatcher = window.matchMedia(`(resolution: ${this.#dpr}dppx)`);
    this.#dprWatcher.addEventListener("change", this.#onDprChange);
  }

  #scale(): Scale {
    return { zoom: this.#state.zoom, dpr: this.#dpr };
  }

  #rotationOf = (page: number): Rotation => {
    const extra = this.#state.pageRotation[page] ?? 0;
    return (((this.#state.docRotation + extra) % 4) + 4) % 4 as Rotation;
  };

  #displaySizeOf(page: number): PageMetrics | undefined {
    const size = this.#state.pageSizes[page];
    return size ? displaySize(size, this.#rotationOf(page)) : undefined;
  }

  #relayout(): void {
    this.#layout = layoutDocument({
      pageSizes: this.#state.pageSizes,
      zoom: this.#state.zoom,
      mode: this.#state.mode,
      rotationOf: this.#rotationOf,
      gap: PAGE_GAP,
      padding: PAGE_PADDING,
      viewportWidth: this.#host.scroller.clientWidth,
    });
    this.#host.spacer.style.width = `${this.#layout.width}px`;
    this.#host.spacer.style.height = `${this.#layout.height}px`;
  }

  /** Keeps the point under the cursor still across a zoom (SPEC 11.1). */
  #applyZoomAnchor(oldZoom: number, newZoom: number): void {
    const anchor = this.#zoomAnchor;
    this.#zoomAnchor = null;
    const scroller = this.#host.scroller;
    if (anchor) {
      const next = zoomAbout(
        { x: anchor.clientX, y: anchor.clientY },
        { x: anchor.contentX - anchor.clientX, y: anchor.contentY - anchor.clientY },
        oldZoom,
        newZoom,
      );
      scroller.scrollTo({ left: Math.max(0, next.x), top: Math.max(0, next.y), behavior: "auto" });
      return;
    }
    // No cursor to anchor to — a toolbar or keyboard zoom — so hold the centre
    // of the viewport instead, which is where the eye is.
    const cx = scroller.clientWidth / 2;
    const cy = scroller.clientHeight / 2;
    const next = zoomAbout(
      { x: cx, y: cy },
      { x: scroller.scrollLeft, y: scroller.scrollTop },
      oldZoom,
      newZoom,
    );
    scroller.scrollTo({ left: Math.max(0, next.x), top: Math.max(0, next.y), behavior: "auto" });
  }

  #view(): ViewRect {
    const s = this.#host.scroller;
    return { x: s.scrollLeft, y: s.scrollTop, w: s.clientWidth, h: s.clientHeight };
  }

  #draw(): void {
    const started = performance.now();
    const state = this.#state;
    const view = this.#view();
    const dpr = this.#dpr;
    this.#surface.resize(view.w, view.h, dpr);
    this.#surface.clear(
      getComputedStyle(this.#host.canvas).getPropertyValue("--izul-canvas") || "#f5f5f4",
    );
    if (state.doc === null) {
      this.#stats = { ...this.#stats, lastFrameMs: performance.now() - started };
      return;
    }

    const pages = visiblePages(this.#layout, view, VISIBLE_MARGIN);
    let drawn = 0;
    let missing = 0;
    const wantText: number[] = [];

    for (const page of pages) {
      const box = boxOf(this.#layout, page);
      const size = this.#displaySizeOf(page);
      if (!box || !size) continue;

      // Page pixel origin on the canvas, rounded once so tiles within a page
      // stay exactly adjacent instead of drifting by a rounding step each.
      const originX = Math.round((box.x - view.x) * dpr);
      const originY = Math.round((box.y - view.y) * dpr);
      const { w: pw, h: ph } = pageSizePx(size, this.#scale());

      // 1. The page is white even before anything has been rendered, so the
      //    user never sees the canvas background where a page should be.
      this.#surface.drawPlaceholder({ x: originX, y: originY, w: pw, h: ph }, "#ffffff");

      // 2. The preview tier, upscaled to the page's box.
      const previewRef = this.#previewRef(page);
      const preview = this.#bitmaps.get(tileKey(previewRef));
      if (preview) {
        this.#surface.drawTile(preview, { x: originX, y: originY, w: pw, h: ph });
      } else {
        this.#request(previewRef, PRIORITY.preview);
      }

      // 3. Sharp tiles over the top, for the part of the page on screen.
      const visibleInPage = {
        x: Math.max(0, (view.x - box.x) * dpr),
        y: Math.max(0, (view.y - box.y) * dpr),
        w: view.w * dpr,
        h: view.h * dpr,
      };
      for (const tile of tilesCovering(visibleInPage, size, this.#scale())) {
        const ref = this.#tileRef(page, tile.col, tile.row);
        const bitmap = this.#bitmaps.get(tileKey(ref));
        if (bitmap) {
          this.#surface.drawTile(bitmap, {
            x: originX + tile.px.x,
            y: originY + tile.px.y,
            w: tile.px.w,
            h: tile.px.h,
          });
          drawn++;
        } else {
          missing++;
          this.#request(ref, PRIORITY.visible);
        }
      }

      if (!state.texts.has(page)) wantText.push(page);
    }
    this.#surface.present();

    // 4. Prefetch: previews for where the scroll is going.
    for (const page of prefetchPages(pages, this.#tracker.velocity, state.pageSizes.length)) {
      const ref = this.#previewRef(page);
      if (!this.#bitmaps.has(tileKey(ref))) this.#request(ref, PRIORITY.prefetch);
    }

    this.#syncTextLayer(pages);
    this.#syncHighlightLayer(pages);
    if (wantText.length > 0) this.#events.onWantText(wantText);

    const page = dominantPage(this.#layout, view);
    if (page !== this.#reportedPage) {
      this.#reportedPage = page;
      this.#events.onPage(page);
    }

    this.#stats = {
      lastFrameMs: performance.now() - started,
      tilesDrawn: drawn,
      tilesMissing: missing,
      pending: this.#pending.size,
      bitmapBytes: this.#bitmaps.bytes,
    };
  }

  #previewRef(page: number): TileRef {
    return {
      doc: this.#state.doc ?? 0,
      page,
      rotation: this.#rotationOf(page),
      scale: PREVIEW_EDGE,
      col: 0,
      row: 0,
      tier: "preview",
    };
  }

  #tileRef(page: number, col: number, row: number): TileRef {
    return {
      doc: this.#state.doc ?? 0,
      page,
      rotation: this.#rotationOf(page),
      scale: scaleKey(this.#scale()),
      col,
      row,
      tier: "sharp",
    };
  }

  #request(ref: TileRef, priority: Priority): void {
    const key = tileKey(ref);
    if (this.#pending.has(key) || this.#bitmaps.has(key)) return;
    this.#pending.add(key);
    const generation = this.#state.generation;
    const signal = this.#abort.signal;
    void loadTile(ref, generation, priority, signal)
      .then((tile) => {
        this.#pending.delete(key);
        if (signal.aborted) {
          tile.bitmap.close();
          return;
        }
        this.#bitmaps.set(key, tile.bitmap, tile.bytes);
        this.requestFrame();
      })
      .catch((e: unknown) => {
        this.#pending.delete(key);
        if (signal.aborted || e instanceof DOMException) return;
        // A superseded or recycled tile is the pipeline working as designed,
        // not something to report: the next frame asks again.
        if (
          e instanceof SupersededTileError ||
          e instanceof StaleTileError ||
          e instanceof MissingTileError
        ) {
          return;
        }
        console.warn("ubin gagal", e);
      });
  }

  /**
   * Rebuilds the text layers of the visible pages, and only those.
   *
   * A signature per page means a layer is rebuilt when its text, zoom or
   * rotation changes and never merely because the user scrolled.
   */
  #syncTextLayer(pages: readonly number[]): void {
    const wanted = new Set(pages);
    for (const page of Array.from(this.#textSignatures.keys())) {
      if (!wanted.has(page)) {
        this.#host.textLayer.querySelector(`[data-page="${page}"]`)?.remove();
        this.#textSignatures.delete(page);
      }
    }
    for (const page of pages) {
      const chars = this.#state.texts.get(page);
      const box = boxOf(this.#layout, page);
      const size = this.#displaySizeOf(page);
      if (!chars || !box || !size) continue;
      const signature = `${chars.length}:${this.#state.zoom}:${this.#rotationOf(page)}`;
      if (this.#textSignatures.get(page) === signature) continue;

      const layer = buildTextLayer(
        groupIntoLines(chars),
        { x: box.x, y: box.y, zoom: this.#state.zoom, pageHeight: size.height },
        document,
      );
      layer.dataset["page"] = String(page);
      this.#host.textLayer.querySelector(`[data-page="${page}"]`)?.remove();
      this.#host.textLayer.appendChild(layer);
      fitTextLayer(layer);
      this.#textSignatures.set(page, signature);
    }
  }

  /**
   * Paints the search matches of the visible pages.
   *
   * Same shape as the text layer and for the same reasons: one element per
   * page, rebuilt only when what it shows actually changed. The signature
   * includes the zoom and the rotation because a highlight is positioned in
   * display space — a box left over from another zoom is not merely stale, it
   * is over the wrong words.
   */
  #syncHighlightLayer(pages: readonly number[]): void {
    const wanted = new Set(pages);
    for (const page of Array.from(this.#highlightSignatures.keys())) {
      const rects = this.#state.highlights.get(page);
      if (!wanted.has(page) || !rects || rects.length === 0) {
        this.#host.textLayer.querySelector(`[data-hl="${page}"]`)?.remove();
        this.#highlightSignatures.delete(page);
      }
    }
    for (const page of pages) {
      const rects = this.#state.highlights.get(page);
      const box = boxOf(this.#layout, page);
      const size = this.#displaySizeOf(page);
      if (!rects || rects.length === 0 || !box || !size) continue;
      const signature = `${rects.length}:${this.#state.zoom}:${this.#rotationOf(page)}`;
      if (this.#highlightSignatures.get(page) === signature) continue;

      const layer = buildHighlightLayer(
        rects,
        { x: box.x, y: box.y, zoom: this.#state.zoom, pageHeight: size.height },
        document,
      );
      layer.dataset["hl"] = String(page);
      this.#host.textLayer.querySelector(`[data-hl="${page}"]`)?.remove();
      this.#host.textLayer.appendChild(layer);
      this.#highlightSignatures.set(page, signature);
    }
  }
}
