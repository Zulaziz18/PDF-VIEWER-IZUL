/**
 * Compare mode (SPEC 10: "dua dokumen berdampingan, scroll tersinkron,
 * perbedaan teks dan visual disorot").
 *
 * Two documents in the two panels of the "columns" layout. Page *n* of one
 * is compared with page *n* of the other — the pairing a reader checking two
 * versions of a document expects; documents whose pages have shifted are
 * lined up by moving pages first, which Phase 5 also makes possible.
 *
 * - **Scrolling** is kept aligned by position — page plus how far into it —
 *   not by pixels, so two documents with different page sizes stay on the
 *   same page.
 * - **Differences** are word differences when both pages have text
 *   (`textDiff.ts`), and a coarse visual comparison when either has none —
 *   a scan, a drawing (`compare.rs`). Only the pages around the one being
 *   read are compared, and each pair once until one of them changes.
 */

import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { textDiff } from "@/compare/textDiff";
import type { DocumentStore } from "@/state/documentSession";
import { useWorkspace } from "@/state/workspaceStore";
import { displaySize, type PdfRect, type Rotation } from "@/viewport/geometry";
import type { TextChar } from "@/viewport/textLayer";
import { viewportOf } from "./viewportHandle";

/** A region as fractions of the page: x rightwards, y downwards. */
export interface NormBox {
  readonly x0: number;
  readonly y0: number;
  readonly x1: number;
  readonly y1: number;
}

interface VisualDiff {
  readonly boxes: readonly NormBox[];
  readonly fraction: number;
}

/** A normalised box on a page of `width` × `height` display points. */
export function normToDisplay(box: NormBox, width: number, height: number): PdfRect {
  return {
    left: box.x0 * width,
    right: box.x1 * width,
    top: height * (1 - box.y0),
    bottom: height * (1 - box.y1),
  };
}

export type CompareMethod = "text" | "visual";

interface CompareStatus {
  sync: boolean;
  /** The page being read on the left, and what was found there. */
  page: number | null;
  differences: number | null;
  method: CompareMethod | null;
  running: boolean;
  setSync(on: boolean): void;
}

export const useCompare = create<CompareStatus>((set) => ({
  sync: true,
  page: null,
  differences: null,
  method: null,
  running: false,
  setSync(sync) {
    set({ sync });
  },
}));

/**
 * The characters of one page, fetched directly. Not through the session's
 * text cache: that cache claims a page with an empty list *before* its
 * request returns (so a second frame does not ask again), and a page read at
 * that moment looks like a page with no text — a scan — which would send it
 * to the visual comparison for no reason.
 */
async function pageChars(store: DocumentStore, page: number): Promise<readonly TextChar[]> {
  const cached = store.getState().texts.get(page);
  if (cached && cached.length > 0) return cached;
  const from = store.getState().sourceOf(page);
  if (from.doc === null) return [];
  try {
    const reply = await invoke<{ chars: TextChar[] }>("page_text", {
      doc: from.doc,
      page: from.page,
      withBoxes: true,
      rotation: rotationOf(store, page),
    });
    return reply.chars;
  } catch {
    return [];
  }
}

function rotationOf(store: DocumentStore, page: number): Rotation {
  const s = store.getState();
  return ((((s.docRotation + (s.pageRotation[page] ?? 0)) % 4) + 4) % 4) as Rotation;
}

/** How long a scroll we caused ourselves is not echoed back. */
const ECHO_MS = 80;

/**
 * Runs compare mode between the documents in the two panels while it is on.
 * Returns the scroll handler each panel's viewport calls.
 */
export function useCompareMode(
  a: DocumentStore | undefined,
  b: DocumentStore | undefined,
  on: boolean,
): { onScrolled: (side: 0 | 1) => void } {
  useEffect(() => {
    if (!on || !a || !b) return;
    let token = 0;
    let timer = 0;
    const done = new Set<string>();

    const compare = async (): Promise<void> => {
      const mine = ++token;
      const sa = a.getState();
      const sb = b.getState();
      const centre = sa.page;
      const pages = [centre - 1, centre, centre + 1, centre + 2].filter(
        (p) => p >= 0 && p < sa.pageCount && p < sb.pageCount,
      );
      useCompare.setState({ running: true, page: centre });
      for (const page of pages) {
        const key = `${sa.pagesEpoch}:${sb.pagesEpoch}:${rotationOf(a, page)}:${rotationOf(b, page)}:${page}`;
        if (done.has(key)) continue;
        // Text first: it is exact where it exists.
        const [ta, tb] = await Promise.all([pageChars(a, page), pageChars(b, page)]);
        if (mine !== token) return;
        let method: CompareMethod;
        let count: number;
        if (ta.length > 0 && tb.length > 0) {
          const r = textDiff(ta, tb);
          a.getState().setMarks(page, r.a);
          b.getState().setMarks(page, r.b);
          method = "text";
          count = r.a.length + r.b.length;
        } else {
          const fa = a.getState().sourceOf(page);
          const fb = b.getState().sourceOf(page);
          if (fa.doc === null || fb.doc === null) {
            done.add(key);
            continue;
          }
          const ra = rotationOf(a, page);
          const rb = rotationOf(b, page);
          let diff: VisualDiff;
          try {
            diff = await invoke<VisualDiff>("compare_visual", {
              aDoc: fa.doc,
              aPage: fa.page,
              aRotation: ra,
              bDoc: fb.doc,
              bPage: fb.page,
              bRotation: rb,
            });
          } catch {
            continue;
          }
          if (mine !== token) return;
          const size = (store: DocumentStore, rot: Rotation): { width: number; height: number } => {
            const m = store.getState().pageSizes[page];
            return m ? displaySize(m, rot) : { width: 612, height: 792 };
          };
          const za = size(a, ra);
          const zb = size(b, rb);
          a.getState().setMarks(page, diff.boxes.map((x) => normToDisplay(x, za.width, za.height)));
          b.getState().setMarks(page, diff.boxes.map((x) => normToDisplay(x, zb.width, zb.height)));
          method = "visual";
          count = diff.boxes.length;
        }
        done.add(key);
        if (page === centre) useCompare.setState({ differences: count, method });
      }
      if (mine === token) useCompare.setState({ running: false });
    };

    const schedule = (): void => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => void compare(), 150);
    };
    schedule();
    const watch = (s: ReturnType<DocumentStore["getState"]>, p: ReturnType<DocumentStore["getState"]>): void => {
      if (s.page !== p.page || s.pagesEpoch !== p.pagesEpoch || s.docRotation !== p.docRotation) {
        if (s.pagesEpoch !== p.pagesEpoch) done.clear();
        schedule();
      }
    };
    const offA = a.subscribe(watch);
    const offB = b.subscribe(watch);
    return () => {
      token++;
      window.clearTimeout(timer);
      offA();
      offB();
      a.getState().clearMarks();
      b.getState().clearMarks();
      useCompare.setState({ page: null, differences: null, method: null, running: false });
    };
  }, [a, b, on]);

  return {
    onScrolled: (side: 0 | 1) => {
      if (!on || !a || !b || !useCompare.getState().sync) return;
      const now = performance.now();
      if (now < quiet[side]) return;
      const from = side === 0 ? a : b;
      const to = side === 0 ? b : a;
      const src = viewportOf(from.getState().doc ?? -1);
      const dst = viewportOf(to.getState().doc ?? -1);
      if (!src || !dst) return;
      const at = src.position();
      quiet[side === 0 ? 1 : 0] = now + ECHO_MS;
      dst.scrollToPosition(at.page, at.fraction);
    },
  };
}

/** Per side, until when its scroll events are echoes of our own. */
const quiet: [number, number] = [0, 0];

/** Turns compare mode on for two documents, or off. */
export function toggleCompare(): void {
  const ws = useWorkspace.getState();
  ws.setCompare(!ws.compare);
}
