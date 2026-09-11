/**
 * Per-document UI state (SPEC 10's DocumentSession).
 *
 * One store per document — for now literally one, because tabs arrive in
 * Phase 2 and the shape here is what each tab will get its own copy of.
 * Business logic lives here and in the Rust crates, never inside a component
 * (SPEC 0).
 *
 * The store holds decisions, not pixels: what is open, at what zoom, in which
 * layout. The viewport reads it and paints; nothing here ever touches a canvas.
 */

import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { PageMetrics, Rotation } from "@/viewport/geometry";
import { clampZoom, displaySize, fitPage, fitWidth, zoomIn, zoomOut } from "@/viewport/geometry";
import type { ViewMode } from "@/viewport/layout";
import { parseViewMode } from "@/viewport/layout";
import type { TextChar } from "@/viewport/textLayer";

export interface OpenedDoc {
  readonly doc: number;
  readonly path: string;
  readonly page_count: number;
  readonly page_sizes: Array<[number, number]>;
  readonly permissions: number;
  readonly encrypted: boolean;
  readonly restored: SavedView | null;
}

export interface SavedView {
  readonly page: number;
  readonly scroll_y: number;
  readonly zoom: number;
  readonly rotation: number;
  readonly view_mode: string;
}

export interface OutlineEntry {
  readonly title: string;
  readonly depth: number;
  readonly page: number | null;
  readonly y: number | null;
}

interface PageTextReply {
  readonly page: number;
  readonly text: string;
  readonly chars: Array<{ c: string; left: number; bottom: number; right: number; top: number }>;
}

/** How the zoom level is chosen. Fit modes follow the window; custom does not. */
export type ZoomMode = "custom" | "fitWidth" | "fitPage" | "actual";

/** What the sidebar is showing (SPEC 11.1). */
export type SidebarTab = "thumbnails" | "outline";

export interface DocumentState {
  doc: number | null;
  path: string | null;
  pageCount: number;
  pageSizes: PageMetrics[];
  encrypted: boolean;
  permissions: number;

  page: number;
  zoom: number;
  zoomMode: ZoomMode;
  docRotation: Rotation;
  pageRotation: Record<number, Rotation>;
  viewMode: ViewMode;
  /**
   * Layout epoch. Incremented on every zoom, rotation or layout change so the
   * worker can discard tiles for a view the user has already left (SPEC 6).
   */
  generation: number;

  sidebarOpen: boolean;
  sidebarTab: SidebarTab;
  outline: OutlineEntry[];
  texts: Map<number, readonly TextChar[]>;

  /** Viewport size in CSS pixels; fit modes are computed from it. */
  viewportWidth: number;
  viewportHeight: number;
  /** Set once after opening, for the viewport to restore the scroll position. */
  pendingScrollY: number | null;

  busy: boolean;
  error: string | null;

  open(path: string): Promise<void>;
  close(): Promise<void>;
  setZoom(zoom: number, mode?: ZoomMode): void;
  zoomIn(): void;
  zoomOut(): void;
  setZoomMode(mode: ZoomMode): void;
  setViewport(width: number, height: number): void;
  setViewMode(mode: ViewMode): void;
  rotateDocument(quarters: number): void;
  rotatePage(page: number, quarters: number): void;
  setPage(page: number): void;
  toggleSidebar(): void;
  setSidebarTab(tab: SidebarTab): void;
  loadOutline(): Promise<void>;
  loadText(pages: readonly number[]): Promise<void>;
  saveView(scrollY: number): Promise<void>;
  consumePendingScroll(): number | null;
}

/** Zoom for the current fit mode, or the current zoom when it is custom. */
function zoomForMode(
  mode: ZoomMode,
  page: PageMetrics | undefined,
  rotation: Rotation,
  width: number,
  height: number,
  fallback: number,
): number {
  if (!page || mode === "custom") return fallback;
  if (mode === "actual") return 1;
  const size = displaySize(page, rotation);
  // The window's own padding: the layout keeps 24 px around the document and
  // 16 px between pages, and a "fit" that ignored them would clip.
  const usableW = Math.max(64, width - 64);
  const usableH = Math.max(64, height - 64);
  return clampZoom(
    mode === "fitWidth" ? fitWidth(size, usableW, 1) : fitPage(size, usableW, usableH, 1),
  );
}

export const useDocument = create<DocumentState>((set, get) => ({
  doc: null,
  path: null,
  pageCount: 0,
  pageSizes: [],
  encrypted: false,
  permissions: 0,

  page: 0,
  zoom: 1,
  zoomMode: "fitWidth",
  docRotation: 0,
  pageRotation: {},
  viewMode: "single",
  generation: 1,

  sidebarOpen: true,
  sidebarTab: "thumbnails",
  outline: [],
  texts: new Map(),

  viewportWidth: 1024,
  viewportHeight: 768,
  pendingScrollY: null,

  busy: false,
  error: null,

  async open(path: string) {
    const previous = get().doc;
    if (previous !== null) {
      await invoke("close_document", { doc: previous });
    }
    set({ busy: true, error: null });
    try {
      const opened = await invoke<OpenedDoc>("open_document", { path });
      const sizes = opened.page_sizes.map(([width, height]) => ({ width, height }));
      const restored = opened.restored;
      const rotation = ((restored?.rotation ?? 0) % 4) as Rotation;
      const viewMode = parseViewMode(restored?.view_mode ?? "single");
      const zoomMode: ZoomMode = restored ? "custom" : "fitWidth";
      const { viewportWidth, viewportHeight } = get();
      const zoom = restored
        ? clampZoom(restored.zoom)
        : zoomForMode("fitWidth", sizes[0], rotation, viewportWidth, viewportHeight, 1);

      set({
        doc: opened.doc,
        path: opened.path,
        pageCount: opened.page_count,
        pageSizes: sizes,
        encrypted: opened.encrypted,
        permissions: opened.permissions,
        page: restored?.page ?? 0,
        zoom,
        zoomMode,
        docRotation: rotation,
        pageRotation: {},
        viewMode,
        outline: [],
        texts: new Map(),
        pendingScrollY: restored?.scroll_y ?? null,
        generation: get().generation + 1,
        busy: false,
        error: null,
      });
      void get().loadOutline();
    } catch (e) {
      set({ busy: false, error: String(e) });
    }
  },

  async close() {
    const { doc } = get();
    if (doc !== null) {
      await invoke("close_document", { doc });
    }
    set({
      doc: null,
      path: null,
      pageCount: 0,
      pageSizes: [],
      page: 0,
      outline: [],
      texts: new Map(),
      pendingScrollY: null,
      error: null,
    });
  },

  setZoom(zoom: number, mode: ZoomMode = "custom") {
    // 10 %–1600 % per SPEC 11.1. Clamped here rather than at each call site so
    // no control can put the viewport into a scale the renderer would refuse.
    const clamped = clampZoom(zoom);
    if (clamped === get().zoom && mode === get().zoomMode) return;
    bumpGeneration(set, get, { zoom: clamped, zoomMode: mode });
  },

  zoomIn() {
    get().setZoom(zoomIn(get().zoom));
  },

  zoomOut() {
    get().setZoom(zoomOut(get().zoom));
  },

  setZoomMode(mode: ZoomMode) {
    const { pageSizes, page, docRotation, viewportWidth, viewportHeight, zoom } = get();
    const next = zoomForMode(mode, pageSizes[page], docRotation, viewportWidth, viewportHeight, zoom);
    bumpGeneration(set, get, { zoom: next, zoomMode: mode });
  },

  setViewport(width: number, height: number) {
    const state = get();
    if (state.viewportWidth === width && state.viewportHeight === height) return;
    const next = zoomForMode(
      state.zoomMode,
      state.pageSizes[state.page],
      state.docRotation,
      width,
      height,
      state.zoom,
    );
    if (next !== state.zoom) {
      bumpGeneration(set, get, { viewportWidth: width, viewportHeight: height, zoom: next });
    } else {
      set({ viewportWidth: width, viewportHeight: height });
    }
  },

  setViewMode(mode: ViewMode) {
    if (mode === get().viewMode) return;
    bumpGeneration(set, get, { viewMode: mode });
  },

  rotateDocument(quarters: number) {
    const state = get();
    const rotation = (((state.docRotation + quarters) % 4) + 4) % 4;
    const zoom = zoomForMode(
      state.zoomMode,
      state.pageSizes[state.page],
      rotation as Rotation,
      state.viewportWidth,
      state.viewportHeight,
      state.zoom,
    );
    bumpGeneration(set, get, { docRotation: rotation as Rotation, zoom });
  },

  rotatePage(page: number, quarters: number) {
    const state = get();
    const current = state.pageRotation[page] ?? 0;
    const next = ((((current + quarters) % 4) + 4) % 4) as Rotation;
    // A new object, not a mutation: the renderer compares by identity to decide
    // whether the layout has to be rebuilt.
    const pageRotation = { ...state.pageRotation, [page]: next };
    bumpGeneration(set, get, { pageRotation });
  },

  setPage(page: number) {
    const { pageCount } = get();
    set({ page: Math.min(Math.max(0, page), Math.max(0, pageCount - 1)) });
  },

  toggleSidebar() {
    set({ sidebarOpen: !get().sidebarOpen });
  },

  setSidebarTab(tab: SidebarTab) {
    set({ sidebarTab: tab, sidebarOpen: true });
  },

  async loadOutline() {
    const { doc } = get();
    if (doc === null) return;
    try {
      const outline = await invoke<OutlineEntry[]>("document_outline", { doc });
      set({ outline });
    } catch {
      // A document without an outline is the common case, and a failure to read
      // one is not worth interrupting the reader for; the sidebar shows the
      // thumbnails instead.
      set({ outline: [] });
    }
  },

  async loadText(pages: readonly number[]) {
    const { doc, texts, docRotation, pageRotation } = get();
    if (doc === null) return;
    const missing = pages.filter((p) => !texts.has(p));
    if (missing.length === 0) return;
    // Mark them as claimed before awaiting, so a second frame does not ask for
    // the same page while the first request is still in flight.
    const claimed = new Map(texts);
    for (const page of missing) claimed.set(page, []);
    set({ texts: claimed });

    for (const page of missing) {
      const rotation = (((docRotation + (pageRotation[page] ?? 0)) % 4) + 4) % 4;
      try {
        const reply = await invoke<PageTextReply>("page_text", {
          doc,
          page,
          withBoxes: true,
          rotation,
        });
        const next = new Map(get().texts);
        next.set(page, reply.chars);
        set({ texts: next });
      } catch {
        // A page whose text cannot be read still renders; it simply has no
        // selection layer.
      }
    }
  },

  async saveView(scrollY: number) {
    const { doc, page, zoom, docRotation, viewMode } = get();
    if (doc === null) return;
    try {
      await invoke("save_view_state", {
        doc,
        page,
        scrollY,
        zoom,
        rotation: docRotation,
        viewMode,
      });
    } catch {
      // Losing a reading position is a nuisance, not a failure worth a dialog.
    }
  },

  consumePendingScroll() {
    const pending = get().pendingScrollY;
    if (pending !== null) set({ pendingScrollY: null });
    return pending;
  },
}));

/**
 * Applies a change that invalidates rendered tiles.
 *
 * Every such change is one layout epoch, and the backend is told immediately so
 * that work for the old one is dropped rather than finished (SPEC 6).
 */
function bumpGeneration(
  set: (partial: Partial<DocumentState>) => void,
  get: () => DocumentState,
  patch: Partial<DocumentState>,
): void {
  const generation = get().generation + 1;
  set({ ...patch, generation });
  const { doc } = get();
  if (doc !== null) {
    void invoke("set_generation", { doc, generation }).catch(() => {
      // The backend gates on generation as well; a lost hint costs a few
      // superseded tiles, never correctness.
    });
  }
}
