/**
 * Per-document UI state (SPEC 10's DocumentSession).
 *
 * One store per document. Business logic lives here and in the Rust crates,
 * never inside a component (SPEC 0).
 */

import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { TileHandle } from "@/viewport/tileSource";

export interface PageSize {
  readonly width: number;
  readonly height: number;
}

export interface OpenedDoc {
  readonly doc: number;
  readonly page_count: number;
  readonly page_sizes: Array<[number, number]>;
  readonly permissions: number;
  readonly encrypted: boolean;
}

export interface DocumentState {
  doc: number | null;
  path: string | null;
  pageCount: number;
  pageSizes: PageSize[];
  encrypted: boolean;
  page: number;
  zoom: number;
  /**
   * Layout epoch. Incremented on every zoom or layout change so the worker can
   * discard tiles for a view the user has already left (SPEC 6).
   */
  generation: number;
  busy: boolean;
  error: string | null;

  open(path: string): Promise<void>;
  close(): Promise<void>;
  setZoom(zoom: number): void;
  setPage(page: number): void;
  requestTile(
    page: number,
    rect: { left: number; bottom: number; right: number; top: number },
    destW: number,
    destH: number,
    sharp: boolean,
  ): Promise<TileHandle>;
}

export const useDocument = create<DocumentState>((set, get) => ({
  doc: null,
  path: null,
  pageCount: 0,
  pageSizes: [],
  encrypted: false,
  page: 0,
  zoom: 1,
  generation: 1,
  busy: false,
  error: null,

  async open(path: string) {
    set({ busy: true, error: null });
    try {
      const opened = await invoke<OpenedDoc>("open_document", { path });
      set({
        doc: opened.doc,
        path,
        pageCount: opened.page_count,
        pageSizes: opened.page_sizes.map(([width, height]) => ({ width, height })),
        encrypted: opened.encrypted,
        page: 0,
        generation: get().generation + 1,
        busy: false,
        error: null,
      });
    } catch (e) {
      set({ busy: false, error: String(e) });
    }
  },

  async close() {
    const { doc } = get();
    if (doc !== null) {
      await invoke("close_document", { doc });
    }
    set({ doc: null, path: null, pageCount: 0, pageSizes: [], page: 0, error: null });
  },

  setZoom(zoom: number) {
    // 10 %–1600 % per SPEC 11.1. Clamped here rather than at each call site so
    // no control can put the viewport into a scale the renderer would refuse.
    const clamped = Math.min(16, Math.max(0.1, zoom));
    set({ zoom: clamped, generation: get().generation + 1 });
  },

  setPage(page: number) {
    const { pageCount } = get();
    set({ page: Math.min(Math.max(0, page), Math.max(0, pageCount - 1)) });
  },

  async requestTile(page, rect, destW, destH, sharp) {
    const { doc, generation } = get();
    if (doc === null) {
      throw new Error("tidak ada dokumen terbuka");
    }
    return invoke<TileHandle>("render_tile", {
      doc,
      page,
      left: rect.left,
      bottom: rect.bottom,
      right: rect.right,
      top: rect.top,
      destW,
      destH,
      generation,
      sharp,
    });
  },
}));
