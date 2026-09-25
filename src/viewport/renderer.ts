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
import {
  displaySize,
  pageSizePx,
  pixelsPerPoint,
  scaleKey,
  tilesCovering,
  zoomAbout,
  zoomHoldingView,
} from "./geometry";
import type { Layout, ViewMode, ViewRect } from "./layout";
import { boxOf, dominantPage, layoutDocument, scrollToPage, visiblePages } from "./layout";
import { prefetchPages, ScrollTracker } from "./prediction";
import { buildHighlightLayer, rectsSignature } from "./highlights";
import type { PageBox } from "./annotLayer";
import { pageAt, pageBox, toContentPoint, toPagePoint } from "./annotLayer";
import { drawDisplayList } from "@/annots/canvas";
import type { AnnotObject, DisplayListOut, PdfPoint } from "@/annots/types";
import type { HandleId } from "@/annots/interaction";
import { HANDLE_SIZE } from "@/annots/interaction";
import {
  angleTo,
  boundsOf,
  handleAt,
  handlePoint,
  handlesFor,
  normalise,
  pick,
  pickInside,
  resized,
  scaleBetween,
  snapAngle,
} from "@/annots/interaction";
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
  isPageInverted,
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
  /** Annotation objects per page, for hit-testing and selection chrome. */
  readonly annots: ReadonlyMap<number, readonly AnnotObject[]>;
  /** What to draw for them, built in Rust (SPEC 3.2). */
  readonly annotLists: ReadonlyMap<number, readonly DisplayListOut[]>;
  readonly selection: readonly number[];
  /** Pixels for the image annotations, by `ImageRef`. */
  readonly annotImages: ReadonlyMap<number, CanvasImageSource>;
  /** The active tool; `null` is the selection arrow. */
  readonly tool: string | null;
  /**
   * Where each display page comes from once pages have been rearranged
   * (Phase 5); `null` while every page is the document's own, in order. A
   * blank page has no render document and is painted white.
   */
  readonly pagesView?: readonly { readonly render_doc: number | null; readonly page: number }[] | null;
  /** Changes when display page numbers stop meaning what they meant. */
  readonly pagesEpoch?: number;
  /** Differences found by compare mode, per page, in display space. */
  readonly marks?: ReadonlyMap<number, readonly PdfRect[]>;
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
  /** These pages are on screen and their annotations have not been fetched. */
  onWantAnnots(pages: readonly number[]): void;
  /** The selection changed through a click or a rubber band. */
  onSelect(ids: readonly number[]): void;
  /** A gesture finished and these objects have new geometry — one undo step. */
  onTransform(objects: readonly AnnotObject[]): void;
  /** The user drew something with a creation tool. */
  onDraw(draft: DrawnShape): void;
}

/**
 * What a creation tool produced, in page space.
 *
 * Deliberately geometry only: the store decides what kind of object to build
 * from it and with what colours, because that is a matter of the current tool
 * and the last-used style, not of where the pointer went.
 */
export interface DrawnShape {
  readonly page: number;
  readonly rect: PdfRect;
  readonly from: PdfPoint;
  readonly to: PdfPoint;
  /** Sampled path, for the ink tool. */
  readonly points: readonly PdfPoint[];
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
  #solo: number | null = null;
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
    annots: new Map(),
    annotLists: new Map(),
    annotImages: new Map(),
    selection: [],
    tool: null,
  };
  #layout: Layout = EMPTY_LAYOUT;
  #dpr = 1;
  #frame = 0;
  #abort = new AbortController();
  #pending = new Set<string>();
  #textSignatures = new Map<number, string>();
  #highlightSignatures = new Map<number, string>();
  #markSignatures = new Map<number, string>();
  /** The gesture in progress, if any. */
  #gesture: Gesture | null = null;
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
    if (previous.doc !== state.doc || previous.pagesEpoch !== state.pagesEpoch) {
      this.#bitmaps.clear();
      this.#textSignatures.clear();
      this.#highlightSignatures.clear();
      this.#markSignatures.clear();
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

  /**
   * Drops the bitmaps of pages whose content changed (a form field filled,
   * Phase 7) so they are rendered again; previews included, since a stale
   * thumbnail would show the old value.
   */
  invalidatePages(pages: readonly number[]): void {
    const doc = this.#state.doc;
    if (doc === null || pages.length === 0) return;
    const prefixes = pages.map((p) => `${doc}/${p}/`);
    this.#bitmaps.keepOnly((key) => !prefixes.some((prefix) => key.startsWith(prefix)));
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

  /**
   * Where the reader is, as the page at the top edge of the view and how far
   * into it (0 at its top, 1 at its bottom). Compare mode keeps two
   * documents aligned by this rather than by pixels, because their pages
   * need not be the same size.
   */
  position(): { page: number; fraction: number } {
    const view = this.#view();
    const horizontal = this.#layout.horizontal;
    const page = visiblePages(this.#layout, view, 0)[0] ?? 0;
    const box = boxOf(this.#layout, page);
    if (!box) return { page, fraction: 0 };
    const fraction = horizontal ? (view.x - box.x) / Math.max(1, box.w) : (view.y - box.y) / Math.max(1, box.h);
    return { page, fraction: Math.min(1, Math.max(0, fraction)) };
  }

  /** The inverse of {@link position}; clamps to the pages this document has. */
  scrollToPosition(page: number, fraction: number): void {
    const last = this.#layout.pages.length - 1;
    if (last < 0) return;
    const box = boxOf(this.#layout, Math.min(Math.max(0, page), last));
    if (!box) return;
    const s = this.#host.scroller;
    if (this.#layout.horizontal) {
      s.scrollTo({ left: box.x + fraction * box.w, top: s.scrollTop, behavior: "auto" });
    } else {
      s.scrollTo({ left: s.scrollLeft, top: box.y + fraction * box.h, behavior: "auto" });
    }
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

  /** Draws only `page` (presentation mode), or every page again with `null`. */
  setSoloPage(page: number | null): void {
    if (page === this.#solo) return;
    this.#solo = page;
    this.requestFrame();
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
    // of the viewport, which is where the eye is; see `zoomHoldingView` for
    // the one exception.
    const next = zoomHoldingView(
      { x: scroller.scrollLeft, y: scroller.scrollTop },
      { w: scroller.clientWidth, h: scroller.clientHeight },
      oldZoom,
      newZoom,
    );
    scroller.scrollTo({ left: next.x, top: next.y, behavior: "auto" });
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

    const nearby = visiblePages(this.#layout, view, VISIBLE_MARGIN);
    // Presenting: the one page, its neighbours not drawn at all (Phase 8).
    const pages = this.#solo === null ? nearby : nearby.filter((p) => p === this.#solo);
    let drawn = 0;
    let missing = 0;
    const wantText: number[] = [];
    const wantAnnots: number[] = [];

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
      this.#surface.drawPageFrame({ x: originX, y: originY, w: pw, h: ph }, dpr);
      this.#surface.drawPlaceholder({ x: originX, y: originY, w: pw, h: ph }, isPageInverted() ? "#1c1c1c" : "#ffffff");

      // 2. The preview tier, upscaled to the page's box. A blank page has
      //    nothing to fetch: the white frame above is all of it.
      const previewRef = this.#previewRef(page);
      const preview = previewRef ? this.#bitmaps.get(tileKey(previewRef)) : undefined;
      if (preview) {
        this.#surface.drawTile(preview, { x: originX, y: originY, w: pw, h: ph });
      } else if (previewRef) {
        this.#request(previewRef, PRIORITY.preview);
      }

      // 3. Sharp tiles over the top, for the part of the page on screen.
      const visibleInPage = {
        x: Math.max(0, (view.x - box.x) * dpr),
        y: Math.max(0, (view.y - box.y) * dpr),
        w: view.w * dpr,
        h: view.h * dpr,
      };
      for (const tile of previewRef ? tilesCovering(visibleInPage, size, this.#scale()) : []) {
        const ref = this.#tileRef(page, tile.col, tile.row);
        if (!ref) continue;
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

      // 4. Annotations, from the display lists the model built. The canvas
      //    backend draws them; nothing here works out their geometry.
      const lists = state.annotLists.get(page);
      if (lists && lists.length > 0) {
        const ctx = this.#surface.context();
        const place = {
          x: originX,
          y: originY,
          scale: pixelsPerPoint(this.#scale()),
          pageHeight: size.height,
        };
        for (const list of lists) {
          drawDisplayList(ctx, list.ops, place, state.annotImages);
        }
      }
      if (!state.annots.has(page)) wantAnnots.push(page);

      if (!state.texts.has(page)) wantText.push(page);
    }
    this.#drawSelection(view, dpr);
    this.#surface.present();

    // 4. Prefetch: previews for where the scroll is going.
    for (const page of prefetchPages(pages, this.#tracker.velocity, state.pageSizes.length)) {
      const ref = this.#previewRef(page);
      if (ref && !this.#bitmaps.has(tileKey(ref))) this.#request(ref, PRIORITY.prefetch);
    }

    this.#syncTextLayer(pages);
    this.#syncBoxLayer(pages, this.#state.highlights, this.#highlightSignatures, "hl", "izul-highlight");
    this.#syncBoxLayer(pages, this.#state.marks, this.#markSignatures, "mk", "izul-diff-mark");
    if (wantText.length > 0) this.#events.onWantText(wantText);
    if (wantAnnots.length > 0) this.#events.onWantAnnots(wantAnnots);

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

  /** The document and page whose pixels display page `page` shows, or
   * `null` for a blank page. */
  #source(page: number): { doc: number; page: number } | null {
    const view = this.#state.pagesView;
    if (!view) return this.#state.doc === null ? null : { doc: this.#state.doc, page };
    const entry = view[page];
    if (!entry || entry.render_doc === null) return null;
    return { doc: entry.render_doc, page: entry.page };
  }

  #previewRef(page: number): TileRef | null {
    const from = this.#source(page);
    if (!from) return null;
    return {
      doc: from.doc,
      page: from.page,
      rotation: this.#rotationOf(page),
      scale: PREVIEW_EDGE,
      col: 0,
      row: 0,
      tier: "preview",
    };
  }

  #tileRef(page: number, col: number, row: number): TileRef | null {
    const from = this.#source(page);
    if (!from) return null;
    return {
      doc: from.doc,
      page: from.page,
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
  #syncBoxLayer(
    pages: readonly number[],
    source: ReadonlyMap<number, readonly PdfRect[]> | undefined,
    signatures: Map<number, string>,
    attr: "hl" | "mk",
    variant: "izul-highlight" | "izul-diff-mark",
  ): void {
    const wanted = new Set(pages);
    for (const page of Array.from(signatures.keys())) {
      const rects = source?.get(page);
      if (!wanted.has(page) || !rects || rects.length === 0) {
        this.#host.textLayer.querySelector(`[data-${attr}="${page}"]`)?.remove();
        signatures.delete(page);
      }
    }
    for (const page of pages) {
      const rects = source?.get(page);
      const box = boxOf(this.#layout, page);
      const size = this.#displaySizeOf(page);
      if (!rects || rects.length === 0 || !box || !size) continue;
      const signature = `${rectsSignature(rects)}:${this.#state.zoom}:${this.#rotationOf(page)}`;
      if (signatures.get(page) === signature) continue;

      const layer = buildHighlightLayer(
        rects,
        { x: box.x, y: box.y, zoom: this.#state.zoom, pageHeight: size.height },
        document,
        variant,
      );
      layer.dataset[attr] = String(page);
      this.#host.textLayer.querySelector(`[data-${attr}="${page}"]`)?.remove();
      this.#host.textLayer.appendChild(layer);
      signatures.set(page, signature);
    }
  }

  // ---- annotation editing ------------------------------------------------

  /**
   * Draws the selection frame, its handles, and whatever gesture is in flight.
   *
   * Chrome, not content: none of it is part of the document, so none of it goes
   * through the display list. Drawn on the canvas rather than as elements
   * because it has to move with the object at pointer speed, and a DOM node per
   * handle would be laid out on every frame of a drag.
   */
  #drawSelection(view: ViewRect, dpr: number): void {
    const state = this.#state;
    const ctx = this.#surface.context();
    const selected = this.#selectedObjects();
    const gesture = this.#gesture;

    if (gesture?.kind === "band" && gesture.current) {
      const box = this.#boxOfPage(gesture.page);
      if (box) {
        const a = toContentPoint(box, gesture.origin);
        const b = toContentPoint(box, gesture.current);
        ctx.save();
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.strokeStyle = "rgba(47, 111, 235, 0.9)";
        ctx.fillStyle = "rgba(47, 111, 235, 0.12)";
        ctx.lineWidth = 1;
        const x = Math.min(a.x, b.x) - view.x;
        const y = Math.min(a.y, b.y) - view.y;
        const w = Math.abs(a.x - b.x);
        const h = Math.abs(a.y - b.y);
        ctx.fillRect(x, y, w, h);
        ctx.strokeRect(x, y, w, h);
        ctx.restore();
      }
    }

    if (gesture?.kind === "create" && gesture.current) {
      const box = this.#boxOfPage(gesture.page);
      if (box) {
        ctx.save();
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.strokeStyle = "rgba(47, 111, 235, 0.9)";
        ctx.setLineDash([4, 3]);
        ctx.lineWidth = 1;
        if (gesture.points.length > 1) {
          ctx.beginPath();
          for (const [i, p] of gesture.points.entries()) {
            const c = toContentPoint(box, p);
            if (i === 0) ctx.moveTo(c.x - view.x, c.y - view.y);
            else ctx.lineTo(c.x - view.x, c.y - view.y);
          }
          ctx.stroke();
        } else {
          const a = toContentPoint(box, gesture.origin);
          const b = toContentPoint(box, gesture.current);
          ctx.strokeRect(
            Math.min(a.x, b.x) - view.x,
            Math.min(a.y, b.y) - view.y,
            Math.abs(a.x - b.x),
            Math.abs(a.y - b.y),
          );
        }
        ctx.restore();
      }
    }

    if (selected.length === 0 || state.tool !== null) return;
    const page = selected[0]?.page ?? 0;
    const box = this.#boxOfPage(page);
    const bounds = boundsOf(selected);
    if (!box || !bounds) return;

    const topLeft = toContentPoint(box, { x: bounds.left, y: bounds.top });
    const bottomRight = toContentPoint(box, { x: bounds.right, y: bounds.bottom });
    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.strokeStyle = "rgba(47, 111, 235, 0.95)";
    ctx.lineWidth = 1;
    ctx.setLineDash([3, 2]);
    ctx.strokeRect(
      topLeft.x - view.x,
      topLeft.y - view.y,
      bottomRight.x - topLeft.x,
      bottomRight.y - topLeft.y,
    );
    ctx.setLineDash([]);

    // Handles are drawn at a fixed pixel size so they stay usable at any zoom.
    ctx.fillStyle = "#ffffff";
    const half = HANDLE_SIZE / 2;
    const only = selected.length === 1 ? selected[0] : undefined;
    const marks: HandleId[] = only ? handlesFor(only.kind) : [];
    for (const handle of marks) {
      const p = toContentPoint(box, handlePoint(bounds, handle));
      const x = p.x - view.x;
      const y = p.y - view.y;
      if (handle === "rotate") {
        ctx.beginPath();
        ctx.arc(x, y, half, 0, Math.PI * 2);
        ctx.fill();
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(x, y + half);
        ctx.lineTo(x, toContentPoint(box, { x: 0, y: bounds.top }).y - view.y);
        ctx.stroke();
      } else {
        ctx.fillRect(x - half, y - half, HANDLE_SIZE, HANDLE_SIZE);
        ctx.strokeRect(x - half, y - half, HANDLE_SIZE, HANDLE_SIZE);
      }
    }
    ctx.restore();
  }

  #boxOfPage(page: number): PageBox | null {
    return pageBox(this.#layout, page, this.#state.pageSizes, this.#rotationOf);
  }

  #selectedObjects(): AnnotObject[] {
    const ids = new Set(this.#state.selection);
    const out: AnnotObject[] = [];
    for (const list of this.#state.annots.values()) {
      for (const obj of list) {
        if (ids.has(obj.id)) out.push(obj);
      }
    }
    return out;
  }

  /** Client coordinates to this scroller's content space. */
  #contentOf(event: PointerEvent): { x: number; y: number } {
    const rect = this.#host.scroller.getBoundingClientRect();
    return {
      x: event.clientX - rect.left + this.#host.scroller.scrollLeft,
      y: event.clientY - rect.top + this.#host.scroller.scrollTop,
    };
  }

  #pageUnder(point: { x: number; y: number }): PageBox | null {
    return pageAt(this.#layout, this.#view(), this.#state.pageSizes, this.#rotationOf, point);
  }

  /**
   * Starts a gesture.
   *
   * Which one depends on what is under the pointer, in the order a user
   * expects: a handle of the current selection first (it is drawn on top and is
   * the smallest target), then an object, then empty space.
   */
  onAnnotPointerDown(event: PointerEvent): boolean {
    if (this.#state.doc === null) return false;
    const content = this.#contentOf(event);
    const box = this.#pageUnder(content);
    if (!box) return false;
    const point = toPagePoint(box, content);
    const tool = this.#state.tool;

    if (tool !== null) {
      this.#gesture = {
        kind: "create",
        page: box.page,
        origin: point,
        current: point,
        points: [point],
        handle: null,
        before: [],
      };
      return true;
    }

    const selected = this.#selectedObjects();
    const bounds = boundsOf(selected);
    if (bounds && selected.length === 1) {
      const only = selected[0];
      const handle = only ? handleAt(bounds, point, box.zoom, handlesFor(only.kind)) : null;
      if (handle) {
        this.#gesture = {
          kind: handle === "rotate" ? "rotate" : "resize",
          page: box.page,
          origin: point,
          current: point,
          points: [],
          handle,
          before: selected.map((o) => structuredClone(o)),
        };
        return true;
      }
    }

    const objects = this.#state.annots.get(box.page) ?? [];
    const hit = pick(objects, point, 3 / box.zoom + 3);
    if (hit) {
      const additive = event.shiftKey || event.ctrlKey || event.metaKey;
      const ids = additive
        ? this.#state.selection.includes(hit.id)
          ? this.#state.selection.filter((id) => id !== hit.id)
          : [...this.#state.selection, hit.id]
        : this.#state.selection.includes(hit.id)
          ? this.#state.selection
          : [hit.id];
      this.#events.onSelect(ids);
      const moving = objects.filter((o) => ids.includes(o.id) && !o.locked);
      this.#gesture = {
        kind: "move",
        page: box.page,
        origin: point,
        current: point,
        points: [],
        handle: null,
        before: moving.map((o) => structuredClone(o)),
      };
      return true;
    }

    // Empty space: a rubber band, and the selection is dropped only when the
    // band turns out to be a click.
    this.#gesture = {
      kind: "band",
      page: box.page,
      origin: point,
      current: point,
      points: [],
      handle: null,
      before: [],
    };
    return true;
  }

  onAnnotPointerMove(event: PointerEvent): boolean {
    const gesture = this.#gesture;
    if (!gesture) return false;
    const box = this.#boxOfPage(gesture.page);
    if (!box) return false;
    const point = toPagePoint(box, this.#contentOf(event));
    gesture.current = point;
    if (gesture.kind === "create") gesture.points.push(point);
    if (gesture.kind === "move" || gesture.kind === "resize" || gesture.kind === "rotate") {
      this.#previewTransform(gesture, event.shiftKey);
    }
    this.requestFrame();
    return true;
  }

  /**
   * Moves the *local* copies so the drag is drawn at pointer speed.
   *
   * The backend is told once, when the pointer goes up: one gesture is one undo
   * step (SPEC 8), and a round trip per frame would put the network between the
   * user's hand and the pixels.
   */
  #previewTransform(gesture: Gesture, snap: boolean): void {
    const dx = gesture.current.x - gesture.origin.x;
    const dy = gesture.current.y - gesture.origin.y;
    const bounds = boundsOf(gesture.before);
    if (!bounds) return;
    const live = this.#state.annots.get(gesture.page);
    if (!live) return;

    for (const original of gesture.before) {
      const target = live.find((o) => o.id === original.id);
      if (!target) continue;
      const patched = structuredClone(original);
      if (gesture.kind === "move") {
        translateObject(patched, dx, dy);
      } else if (gesture.kind === "resize" && gesture.handle) {
        const after = resized(bounds, gesture.handle, dx, dy);
        const { origin, sx, sy } = scaleBetween(bounds, after);
        scaleObject(patched, origin, sx, sy);
      } else if (gesture.kind === "rotate") {
        patched.rotation = snapAngle(angleTo(bounds, gesture.current), snap);
      }
      Object.assign(target, patched);
    }
  }

  onAnnotPointerUp(event: PointerEvent): boolean {
    const gesture = this.#gesture;
    this.#gesture = null;
    if (!gesture) return false;
    const box = this.#boxOfPage(gesture.page);
    if (!box) return false;
    const point = toPagePoint(box, this.#contentOf(event));

    switch (gesture.kind) {
      case "create": {
        const rect = normalise({
          left: gesture.origin.x,
          bottom: gesture.origin.y,
          right: point.x,
          top: point.y,
        });
        this.#events.onDraw({
          page: gesture.page,
          rect,
          from: gesture.origin,
          to: point,
          points: gesture.points,
        });
        break;
      }
      case "band": {
        const band = {
          left: gesture.origin.x,
          bottom: gesture.origin.y,
          right: point.x,
          top: point.y,
        };
        const objects = this.#state.annots.get(gesture.page) ?? [];
        const caught = pickInside(objects, band);
        this.#events.onSelect(caught.map((o) => o.id));
        break;
      }
      case "move":
      case "resize":
      case "rotate": {
        const moved = (this.#state.annots.get(gesture.page) ?? []).filter((o) =>
          gesture.before.some((b) => b.id === o.id),
        );
        // A click that moved nothing is not an edit, and must not cost an undo
        // step: pressing on an object to select it would otherwise fill the
        // history with no-ops.
        const changed = moved.some((o) => {
          const before = gesture.before.find((b) => b.id === o.id);
          return before !== undefined && JSON.stringify(before) !== JSON.stringify(o);
        });
        if (changed) this.#events.onTransform(moved.map((o) => structuredClone(o)));
        break;
      }
    }
    this.requestFrame();
    return true;
  }

  /**
   * The quads of the current text selection, in page space.
   *
   * This is what a highlight is made of (SPEC 11.2): one quad per line of
   * selected text, taken from the browser's own selection rectangles rather
   * than re-derived from character boxes. The browser already knows exactly
   * which glyphs are selected — asking it is both simpler and more correct than
   * hit-testing the text layer ourselves.
   *
   * `null` when nothing is selected, or when the selection is not inside this
   * viewport's text layer.
   */
  selectionQuads(): { page: number; quads: PdfRect[] } | null {
    const selection = document.getSelection();
    if (!selection || selection.isCollapsed || selection.rangeCount === 0) return null;
    const range = selection.getRangeAt(0);
    if (!this.#host.textLayer.contains(range.commonAncestorContainer)) return null;

    const scrollerRect = this.#host.scroller.getBoundingClientRect();
    const toContent = (x: number, y: number): { x: number; y: number } => ({
      x: x - scrollerRect.left + this.#host.scroller.scrollLeft,
      y: y - scrollerRect.top + this.#host.scroller.scrollTop,
    });

    const rects = Array.from(range.getClientRects()).filter((r) => r.width > 0 && r.height > 0);
    if (rects.length === 0) return null;

    // Every rect is attributed to the page it starts on; a selection dragged
    // across a page break produces quads on the first page only, which is what
    // a single annotation can honestly cover.
    const first = this.#pageUnder(toContent(rects[0]?.left ?? 0, rects[0]?.top ?? 0));
    if (!first) return null;
    const quads: PdfRect[] = [];
    for (const rect of rects) {
      const topLeft = toPagePoint(first, toContent(rect.left, rect.top));
      const bottomRight = toPagePoint(first, toContent(rect.right, rect.bottom));
      quads.push({
        left: Math.min(topLeft.x, bottomRight.x),
        bottom: Math.min(topLeft.y, bottomRight.y),
        right: Math.max(topLeft.x, bottomRight.x),
        top: Math.max(topLeft.y, bottomRight.y),
      });
    }
    return { page: first.page, quads };
  }

  /** Abandons a gesture — the pointer left the window, or Escape was pressed. */
  cancelGesture(): void {
    this.#gesture = null;
    this.requestFrame();
  }

  get gestureActive(): boolean {
    return this.#gesture !== null;
  }
}

/** A gesture in progress. */
interface Gesture {
  kind: "move" | "resize" | "rotate" | "band" | "create";
  page: number;
  origin: PdfPoint;
  current: PdfPoint;
  points: PdfPoint[];
  handle: HandleId | null;
  /** Copies taken when the gesture started, for the preview and the undo step. */
  before: AnnotObject[];
}

/**
 * Moves an object's geometry, payload and all.
 *
 * A mirror of `AnnotObject::translate` in the model, and the duplication is
 * deliberate rather than an oversight: this one only ever touches the *preview*
 * copies during a drag, and the authoritative move is done by the Rust code
 * when the gesture ends. If the two ever disagree, the object snaps into place
 * on release — visible, and far better than the frontend's idea of the
 * geometry ending up in the file.
 */
function translateObject(obj: AnnotObject, dx: number, dy: number): void {
  const shiftRect = (r: { left: number; bottom: number; right: number; top: number }): void => {
    r.left += dx;
    r.right += dx;
    r.bottom += dy;
    r.top += dy;
  };
  const shiftPoint = (p: { x: number; y: number }): void => {
    p.x += dx;
    p.y += dy;
  };
  shiftRect(obj.rect);
  const payload = obj.payload;
  if ("Markup" in payload) payload.Markup.quads.forEach(shiftRect);
  else if ("Ink" in payload) payload.Ink.strokes.forEach((s) => s.forEach(shiftPoint));
  else if ("Polygon" in payload) payload.Polygon.points.forEach(shiftPoint);
  else if ("Line" in payload) {
    shiftPoint(payload.Line.from);
    shiftPoint(payload.Line.to);
  }
}

/** Scales an object about a pivot; the preview half of `AnnotObject::scale_about`. */
function scaleObject(obj: AnnotObject, origin: PdfPoint, sx: number, sy: number): void {
  const s = (Math.abs(sx) + Math.abs(sy)) / 2;
  const mapPoint = (p: { x: number; y: number }): void => {
    p.x = origin.x + (p.x - origin.x) * sx;
    p.y = origin.y + (p.y - origin.y) * sy;
  };
  const mapRect = (r: { left: number; bottom: number; right: number; top: number }): void => {
    const a = { x: r.left, y: r.bottom };
    const b = { x: r.right, y: r.top };
    mapPoint(a);
    mapPoint(b);
    r.left = Math.min(a.x, b.x);
    r.right = Math.max(a.x, b.x);
    r.bottom = Math.min(a.y, b.y);
    r.top = Math.max(a.y, b.y);
  };
  mapRect(obj.rect);
  const payload = obj.payload;
  if ("Markup" in payload) payload.Markup.quads.forEach(mapRect);
  else if ("Ink" in payload) {
    payload.Ink.strokes.forEach((stroke) => stroke.forEach(mapPoint));
    payload.Ink.width *= s;
  } else if ("Polygon" in payload) {
    payload.Polygon.points.forEach(mapPoint);
    payload.Polygon.style.stroke_width *= s;
  } else if ("Line" in payload) {
    mapPoint(payload.Line.from);
    mapPoint(payload.Line.to);
    payload.Line.width *= s;
    payload.Line.arrow_head *= s;
  } else if ("Shape" in payload) {
    payload.Shape.style.stroke_width *= s;
  } else if ("FreeText" in payload) {
    payload.FreeText.font.size *= s;
  } else if ("Stamp" in payload) {
    payload.Stamp.font.size *= s;
  }
}
