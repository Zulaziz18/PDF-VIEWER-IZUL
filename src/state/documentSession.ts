/**
 * One document's state (SPEC 10's DocumentSession).
 *
 * Phase 1 had a single global store because there was a single document. Tabs
 * make that shape wrong in a way that is worth being explicit about: zoom,
 * rotation, reading position, outline, extracted text and search results all
 * belong to *a document*, not to the window, and a window-level store would
 * have to key every one of them by document id — which is the same thing, less
 * safely, and with every component having to remember to pass the key.
 *
 * So each tab gets its own store, created here, and the workspace holds the
 * map of them. A tab that is closed drops its store and everything in it; a
 * tab that goes inactive keeps its store (a few kilobytes of decisions) while
 * the backend drops its bitmaps (megabytes). That split is the whole of
 * SPEC 10's inactive-tab memory rule.
 *
 * Business logic lives here and in the Rust crates, never inside a component
 * (SPEC 0). The store holds decisions, not pixels.
 */

import { createStore, type StoreApi } from "zustand/vanilla";
import { invoke } from "@tauri-apps/api/core";
import type { PageMetrics, PdfRect, Rotation } from "@/viewport/geometry";
import { clampZoom, displaySize, fitPage, fitWidth, zoomIn, zoomOut } from "@/viewport/geometry";
import type { ViewMode } from "@/viewport/layout";
import { parseViewMode } from "@/viewport/layout";
import type { TextChar } from "@/viewport/textLayer";
import type { AnnotKind, AnnotObject, DisplayListOut, EditResult } from "@/annots/types";
import { NEW_OBJECT_ID } from "@/annots/types";
import { loadImage, referencedImages } from "@/annots/images";

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

/** One match on a page, with the boxes to highlight it. */
export interface PageHit {
  readonly char_index: number;
  readonly char_count: number;
  /** One box per line: a match that wraps must not paint over what is between. */
  readonly rects: readonly PdfRect[];
}

/** A match the index found: which page of which file, and how it reads. */
export interface IndexHit {
  readonly file_id: number;
  readonly path: string;
  readonly name: string;
  readonly page: number;
  readonly snippet: string;
  readonly doc: number | null;
}

/**
 * Which of SPEC 11.1's tiers the search box is running.
 *
 * `regex` is deliberately a mode of its own rather than a checkbox on the
 * others: it can only run over text somebody has extracted, so it covers the
 * open document alone, and the panel says so. See `src-tauri/src/textsearch.rs`
 * for why that is structural and not a shortcut.
 */
export type SearchScope = "document" | "library" | "regex";

export interface SearchState {
  readonly open: boolean;
  readonly query: string;
  readonly scope: SearchScope;
  readonly caseSensitive: boolean;
  readonly wholeWord: boolean;
  readonly running: boolean;
  readonly error: string | null;
  /** Pages the index says contain the query, in relevance order. */
  readonly results: readonly IndexHit[];
  /** Index into {@link results} of the hit the user is on. */
  readonly cursor: number;
  /**
   * Epoch of the newest query. Raised on every keystroke, so an answer to a
   * prefix the user has already typed past is dropped rather than drawn.
   */
  readonly generation: number;
}

/** The file behind a tab, as last measured. */
export interface FileState {
  /** Bytes on disk, or null before the first measurement. */
  readonly size: number | null;
  /** Another program wrote the file after this application opened or saved it. */
  readonly changedOnDisk: boolean;
  readonly missing: boolean;
  /** When this application last saved it, in milliseconds. */
  readonly savedAt: number | null;
  /** A save is running; the buttons wait for it. */
  readonly saving: boolean;
}

/** What `save_document` answers. */
export interface SaveReport {
  readonly path: string;
  readonly bytes: number;
  readonly annotations: number;
}

/** What `file_status` answers. */
export interface FileStatusReply {
  readonly changed_on_disk: boolean;
  readonly missing: boolean;
  readonly dirty: boolean;
  readonly size: number;
}

/** How the zoom level is chosen. Fit modes follow the window; custom does not. */
export type ZoomMode = "custom" | "fitWidth" | "fitPage" | "actual";

/** What the sidebar is showing (SPEC 11.1). Search results are one of its
 * tabs, as SPEC 11.1 lists them, reached from the icon rail like the rest. */
export type SidebarTab = "thumbnails" | "outline" | "annots" | "search";

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
  /** Boxes to paint over matches, per page, in display space. */
  highlights: Map<number, readonly PdfRect[]>;
  search: SearchState;

  /** Annotations of this document, by page (SPEC 8). */
  annots: Map<number, AnnotObject[]>;
  /**
   * What the canvas backend draws, by page.
   *
   * Built in Rust by the same pure function the appearance-stream backend
   * reads, and carried here as data. The frontend never works out an
   * annotation's geometry itself — that is the whole of SPEC 3.2's guarantee.
   */
  annotLists: Map<number, DisplayListOut[]>;
  /** Pixels for the image annotations on screen, by `ImageRef`. */
  annotImages: Map<number, CanvasImageSource>;
  /** Ids of the selected objects. */
  selection: number[];
  /** Which tool the pointer is in. `null` is the selection arrow. */
  tool: AnnotKind | null;
  canUndo: boolean;
  canRedo: boolean;
  /**
   * The document has changes its file does not. The backend's word, carried
   * in every edit's reply — not `canUndo`, which stays true after a save and
   * turns false when the user undoes past one.
   */
  dirty: boolean;
  /** What is known about the file on disk (Phase 4). */
  file: FileState;
  /** Set when an edit was refused — a missing font, a locked object. */
  annotError: string | null;

  /** Viewport size in CSS pixels; fit modes are computed from it. */
  viewportWidth: number;
  viewportHeight: number;
  /** Set once after opening, for the viewport to restore the scroll position. */
  pendingScrollY: number | null;
  /** Set when a search result asks the viewport to jump to a page. */
  pendingPage: number | null;

  busy: boolean;
  error: string | null;

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
  consumePendingPage(): number | null;

  toggleSearch(open?: boolean): void;
  setSearchQuery(query: string): void;
  setSearchScope(scope: SearchScope): void;
  setSearchOption(option: "caseSensitive" | "wholeWord", value: boolean): void;
  runSearch(): Promise<void>;
  gotoResult(delta: number): void;
  /** Fetches highlight boxes for pages that are on screen. */
  loadHighlights(pages: readonly number[]): Promise<void>;

  setTool(tool: AnnotKind | null): void;
  select(ids: readonly number[]): void;
  loadAnnots(pages: readonly number[]): Promise<void>;
  addAnnot(object: AnnotObject): Promise<AnnotObject | null>;
  replaceAnnots(objects: readonly AnnotObject[]): Promise<void>;
  deleteSelected(): Promise<void>;
  undoAnnot(): Promise<void>;
  redoAnnot(): Promise<void>;
  /** Takes the undo, redo and unsaved state from an edit's reply. */
  applyEdit(result: EditResult): void;
  /** Drops every loaded page's annotations and loads them again — after a
   * restored draft replaced them behind the frontend's back. */
  reloadAnnots(): Promise<void>;
  setFileStatus(status: FileStatusReply): void;
  setSaving(saving: boolean): void;
  markSaved(report: SaveReport): void;
  /** The selected objects, in paint order. */
  selectedObjects(): AnnotObject[];
}

export type DocumentStore = StoreApi<DocumentState>;

const EMPTY_SEARCH: SearchState = {
  open: false,
  query: "",
  scope: "document",
  caseSensitive: false,
  wholeWord: false,
  running: false,
  error: null,
  results: [],
  cursor: -1,
  generation: 0,
};

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

/**
 * Builds the store for one open document.
 *
 * `opened` is `null` for the placeholder session the window uses when no
 * document is open. Having one shape for both means every component reads the
 * same state whether or not anything is open, instead of each one carrying its
 * own null check.
 */
export function createDocumentSession(
  opened: OpenedDoc | null,
  viewport: { width: number; height: number } = { width: 1024, height: 768 },
): DocumentStore {
  const sizes: PageMetrics[] =
    opened?.page_sizes.map(([width, height]) => ({ width, height })) ?? [];
  const restored = opened?.restored ?? null;
  const rotation = ((restored?.rotation ?? 0) % 4) as Rotation;
  const zoomMode: ZoomMode = restored ? "custom" : "fitWidth";
  const zoom = restored
    ? clampZoom(restored.zoom)
    : zoomForMode("fitWidth", sizes[0], rotation, viewport.width, viewport.height, 1);

  return createStore<DocumentState>((set, get) => ({
    doc: opened?.doc ?? null,
    path: opened?.path ?? null,
    pageCount: opened?.page_count ?? 0,
    pageSizes: sizes,
    encrypted: opened?.encrypted ?? false,
    permissions: opened?.permissions ?? 0,

    page: restored?.page ?? 0,
    zoom,
    zoomMode,
    docRotation: rotation,
    pageRotation: {},
    viewMode: parseViewMode(restored?.view_mode ?? "single"),
    // Starts at 1, not 0: the backend treats "older than the current epoch" as
    // superseded, and epoch zero would make the first frame indistinguishable
    // from a request left over from nothing.
    generation: 1,

    sidebarOpen: true,
    sidebarTab: "thumbnails",
    outline: [],
    texts: new Map(),
    highlights: new Map(),
    search: EMPTY_SEARCH,

    annots: new Map(),
    annotLists: new Map(),
    annotImages: new Map(),
    selection: [],
    tool: null,
    canUndo: false,
    canRedo: false,
    dirty: false,
    file: { size: null, changedOnDisk: false, missing: false, savedAt: null, saving: false },
    annotError: null,

    viewportWidth: viewport.width,
    viewportHeight: viewport.height,
    pendingScrollY: restored?.scroll_y ?? null,
    pendingPage: null,

    busy: false,
    error: null,

    setZoom(next: number, mode: ZoomMode = "custom") {
      // 10 %–1600 % per SPEC 11.1. Clamped here rather than at each call site so
      // no control can put the viewport into a scale the renderer would refuse.
      const clamped = clampZoom(next);
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
      const { pageSizes, page, docRotation, viewportWidth, viewportHeight, zoom: current } = get();
      const next = zoomForMode(
        mode,
        pageSizes[page],
        docRotation,
        viewportWidth,
        viewportHeight,
        current,
      );
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
      const next = (((state.docRotation + quarters) % 4) + 4) % 4;
      const nextZoom = zoomForMode(
        state.zoomMode,
        state.pageSizes[state.page],
        next as Rotation,
        state.viewportWidth,
        state.viewportHeight,
        state.zoom,
      );
      // Highlights are in display space, so a rotation invalidates every box
      // that has been fetched. Keeping them would paint over the wrong words.
      bumpGeneration(set, get, {
        docRotation: next as Rotation,
        zoom: nextZoom,
        highlights: new Map(),
      });
    },

    rotatePage(page: number, quarters: number) {
      const state = get();
      const current = state.pageRotation[page] ?? 0;
      const next = ((((current + quarters) % 4) + 4) % 4) as Rotation;
      // A new object, not a mutation: the renderer compares by identity to decide
      // whether the layout has to be rebuilt.
      const pageRotation = { ...state.pageRotation, [page]: next };
      const highlights = new Map(state.highlights);
      highlights.delete(page);
      bumpGeneration(set, get, { pageRotation, highlights });
    },

    setPage(page: number) {
      const { pageCount } = get();
      set({ page: Math.min(Math.max(0, page), Math.max(0, pageCount - 1)) });
    },

    toggleSidebar() {
      const open = !get().sidebarOpen;
      // The search panel lives in the sidebar, so its `open` flag — which gates
      // the debounced query and the index poll — follows the sidebar's.
      set({
        sidebarOpen: open,
        search: { ...get().search, open: open && get().sidebarTab === "search" },
      });
    },

    setSidebarTab(tab: SidebarTab) {
      set({ sidebarTab: tab, sidebarOpen: true, search: { ...get().search, open: tab === "search" } });
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
      const { doc, page, zoom: current, docRotation, viewMode } = get();
      if (doc === null) return;
      try {
        await invoke("save_view_state", {
          doc,
          page,
          scrollY,
          zoom: current,
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

    consumePendingPage() {
      const pending = get().pendingPage;
      if (pending !== null) set({ pendingPage: null });
      return pending;
    },

    // ---- search ----------------------------------------------------------

    toggleSearch(open?: boolean) {
      const next = open ?? !get().search.open;
      if (next) {
        set({ sidebarOpen: true, sidebarTab: "search", search: { ...get().search, open: true } });
      } else {
        // Closing search closes the panel it was in, rather than leaving the
        // sidebar showing an empty tab.
        const wasShowing = get().sidebarTab === "search";
        set({
          search: { ...get().search, open: false },
          sidebarOpen: wasShowing ? false : get().sidebarOpen,
          sidebarTab: wasShowing ? "thumbnails" : get().sidebarTab,
        });
      }
    },

    setSearchQuery(query: string) {
      const search = get().search;
      // The epoch moves on every keystroke, which is what makes an answer to
      // "lapor" arrive too late to overwrite the results for "laporan".
      set({
        search: {
          ...search,
          query,
          generation: search.generation + 1,
          error: null,
          results: query.length === 0 ? [] : search.results,
          cursor: -1,
        },
        highlights: new Map(),
      });
    },

    setSearchScope(scope: SearchScope) {
      set({
        search: { ...get().search, scope, results: [], cursor: -1, error: null },
        highlights: new Map(),
      });
    },

    setSearchOption(option, value) {
      set({ search: { ...get().search, [option]: value }, highlights: new Map() });
    },

    async runSearch() {
      const { doc, search } = get();
      const query = search.query.trim();
      if (query.length === 0) {
        set({ search: { ...get().search, results: [], cursor: -1, running: false } });
        return;
      }
      const epoch = search.generation;
      set({ search: { ...get().search, running: true, error: null } });
      try {
        let results: IndexHit[] = [];
        if (search.scope === "library") {
          results = await invoke<IndexHit[]>("search_library", { query, limit: 200 });
        } else if (search.scope === "document" && doc !== null) {
          results = await invoke<IndexHit[]>("search_document", { doc, query, limit: 200 });
        } else {
          // Regex has no index behind it, so there is no list of pages to show;
          // it highlights the page the reader is on, and the panel says so.
          results = [];
        }
        // Another keystroke landed while this was in flight: its answer is the
        // one the user is waiting for.
        if (get().search.generation !== epoch) return;
        set({
          search: { ...get().search, results, running: false, cursor: results.length > 0 ? 0 : -1 },
          highlights: new Map(),
        });
        const first = results[0];
        if (first && (search.scope !== "library" || first.doc === doc)) {
          set({ pendingPage: first.page });
        }
      } catch (e) {
        if (get().search.generation !== epoch) return;
        set({ search: { ...get().search, running: false, error: String(e) } });
      }
    },

    gotoResult(delta: number) {
      const { search, doc } = get();
      if (search.results.length === 0) return;
      const count = search.results.length;
      const cursor = (((search.cursor + delta) % count) + count) % count;
      set({ search: { ...search, cursor } });
      const hit = search.results[cursor];
      // A library hit in another document cannot be jumped to from here; the
      // panel opens it in a tab instead.
      if (hit && (hit.doc === null || hit.doc === doc)) {
        set({ pendingPage: hit.page });
      }
    },

    // ---- annotations -----------------------------------------------------

    setTool(tool: AnnotKind | null) {
      // Picking a tool clears the selection: the handles of the object you were
      // editing are not the thing you are about to draw.
      set({ tool, selection: tool === null ? get().selection : [], annotError: null });
    },

    select(ids: readonly number[]) {
      set({ selection: [...ids] });
    },

    selectedObjects() {
      const { annots, selection } = get();
      const byId = new Map<number, AnnotObject>();
      for (const list of annots.values()) {
        for (const obj of list) byId.set(obj.id, obj);
      }
      return selection.flatMap((id) => {
        const found = byId.get(id);
        return found ? [found] : [];
      });
    },

    async loadAnnots(pages: readonly number[]) {
      const { doc } = get();
      if (doc === null) return;
      for (const page of pages) {
        try {
          const [objects, lists] = await Promise.all([
            invoke<AnnotObject[]>("annot_list", { doc, page }),
            invoke<DisplayListOut[]>("annot_display_lists", { doc, page }),
          ]);
          const nextObjects = new Map(get().annots);
          const nextLists = new Map(get().annotLists);
          nextObjects.set(page, objects);
          nextLists.set(page, lists);
          set({ annots: nextObjects, annotLists: nextLists });

          // Pixels for whatever the lists refer to. Fetched after the lists are
          // in place so the shapes draw immediately and the images fill in,
          // rather than the whole page waiting on one photograph.
          const wanted = referencedImages(lists).filter((ref) => !get().annotImages.has(ref));
          for (const ref of wanted) {
            const image = await loadImage(doc, ref);
            if (image) {
              const next = new Map(get().annotImages);
              next.set(ref, image);
              set({ annotImages: next });
            }
          }
        } catch (e) {
          set({ annotError: String(e) });
        }
      }
    },

    async addAnnot(object: AnnotObject) {
      const { doc } = get();
      if (doc === null) return null;
      try {
        const [created, result] = await invoke<[AnnotObject, EditResult]>("annot_add", {
          doc,
          object: { ...object, id: NEW_OBJECT_ID },
        });
        await get().loadAnnots([object.page]);
        get().applyEdit(result);
        set({ selection: [created.id], tool: null, annotError: null });
        return created;
      } catch (e) {
        // A refusal the user needs to read — a font without the glyphs they
        // typed is the one SPEC 11.2 insists on saying out loud.
        set({ annotError: String(e) });
        return null;
      }
    },

    async replaceAnnots(objects: readonly AnnotObject[]) {
      const { doc } = get();
      if (doc === null || objects.length === 0) return;
      try {
        const result = await invoke<EditResult>("annot_replace", { doc, objects });
        get().applyEdit(result);
        set({ annotError: null });
        await get().loadAnnots([...new Set(objects.map((o) => o.page))]);
      } catch (e) {
        set({ annotError: String(e) });
        // The backend refused, so the frontend's copy is the wrong one: take
        // the backend's word rather than leaving a ghost on screen.
        await get().loadAnnots([...new Set(objects.map((o) => o.page))]);
      }
    },

    async deleteSelected() {
      const { doc, selection } = get();
      if (doc === null || selection.length === 0) return;
      const pages = [...new Set(get().selectedObjects().map((o) => o.page))];
      try {
        const result = await invoke<EditResult>("annot_delete", { doc, ids: selection });
        get().applyEdit(result);
        set({ selection: [], annotError: null });
      } catch (e) {
        set({ annotError: String(e) });
      }
      await get().loadAnnots(pages);
    },

    async undoAnnot() {
      const { doc } = get();
      if (doc === null) return;
      try {
        const result = await invoke<EditResult>("annot_undo", { doc, page: get().page });
        get().applyEdit(result);
        set({ selection: [] });
      } catch (e) {
        set({ annotError: String(e) });
      }
      await get().loadAnnots([...get().annots.keys()]);
    },

    async redoAnnot() {
      const { doc } = get();
      if (doc === null) return;
      try {
        const result = await invoke<EditResult>("annot_redo", { doc, page: get().page });
        get().applyEdit(result);
        set({ selection: [] });
      } catch (e) {
        set({ annotError: String(e) });
      }
      await get().loadAnnots([...get().annots.keys()]);
    },

    applyEdit(result: EditResult) {
      set({ canUndo: result.can_undo, canRedo: result.can_redo, dirty: result.dirty });
    },

    async reloadAnnots() {
      const pages = [...get().annots.keys()];
      set({ annots: new Map(), annotLists: new Map(), selection: [] });
      await get().loadAnnots(pages);
    },

    setFileStatus(status: FileStatusReply) {
      const file = get().file;
      const next = {
        ...file,
        size: status.missing ? file.size : status.size,
        changedOnDisk: status.changed_on_disk,
        missing: status.missing,
      };
      // Only a change is written: this runs on a timer, and a store update
      // re-renders everything that reads the file state.
      if (
        next.size !== file.size ||
        next.changedOnDisk !== file.changedOnDisk ||
        next.missing !== file.missing ||
        status.dirty !== get().dirty
      ) {
        set({ file: next, dirty: status.dirty });
      }
    },

    setSaving(saving: boolean) {
      set({ file: { ...get().file, saving } });
    },

    markSaved(report: SaveReport) {
      set({
        dirty: false,
        path: report.path,
        file: {
          size: report.bytes,
          changedOnDisk: false,
          missing: false,
          savedAt: Date.now(),
          saving: false,
        },
      });
    },

    async loadHighlights(pages: readonly number[]) {
      const { doc, search, docRotation, pageRotation, highlights } = get();
      const query = search.query.trim();
      if (doc === null || query.length === 0) return;
      const epoch = search.generation;
      const wanted = pages.filter((p) => !highlights.has(p));
      if (wanted.length === 0) return;
      // Claimed before awaiting, for the same reason `loadText` claims: the
      // viewport asks again on the very next frame.
      const claimed = new Map(highlights);
      for (const page of wanted) claimed.set(page, []);
      set({ highlights: claimed });

      for (const page of wanted) {
        const rotation = (((docRotation + (pageRotation[page] ?? 0)) % 4) + 4) % 4;
        try {
          const hits =
            search.scope === "regex"
              ? await invoke<PageHit[]>("search_regex_page", {
                  doc,
                  page,
                  pattern: query,
                  caseSensitive: search.caseSensitive,
                  rotation,
                })
              : await invoke<PageHit[]>("search_page", {
                  doc,
                  page,
                  query,
                  caseSensitive: search.caseSensitive,
                  wholeWord: search.wholeWord,
                  rotation,
                  generation: get().generation,
                });
          if (get().search.generation !== epoch) return;
          const next = new Map(get().highlights);
          next.set(page, hits.flatMap((h) => h.rects));
          set({ highlights: next });
        } catch (e) {
          if (get().search.generation !== epoch) return;
          // A regex the user is halfway through typing is not a failure worth
          // shouting about, but it is worth showing once in the panel.
          if (search.scope === "regex") {
            set({ search: { ...get().search, error: String(e) } });
          }
        }
      }
    },
  }));
}

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
