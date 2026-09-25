/**
 * Hosts the imperative viewport surface.
 *
 * The component's whole job is to own four elements and hand the renderer the
 * current state. It never touches pixels itself — that boundary is what SPEC 4
 * is protecting.
 *
 * The layering is what makes both painting and selection work at once: the
 * canvas sits underneath, the scrolling element above it carries the real
 * scroll bars and the transparent text layer, and the canvas is repainted from
 * the scroll offset every frame.
 */

import { useEffect, useRef } from "react";
import { useStore } from "zustand";
import { ViewportRenderer } from "@/viewport/renderer";
import { DEFAULT_STYLE, objectFromDrawn } from "@/annots/factory";
import type { DocumentState, DocumentStore } from "@/state/documentSession";
import { registerViewport } from "./viewportHandle";
import { t } from "@/i18n";

/** How long the scroll must be still before the reading position is stored. */
const SAVE_IDLE_MS = 1200;

/**
 * One document's viewport. `store` is that document's session: with split
 * view (Phase 5) several are on screen, and each must read and write its
 * own document, not whichever one happens to be in front.
 */
export function Viewport(props: {
  store: DocumentStore;
  onFocus?: () => void;
  /** Called on every scroll, after the renderer has moved (compare mode). */
  onScrolled?: () => void;
}): React.JSX.Element {
  const { store: session } = props;
  const useDocument = <T,>(selector: (s: DocumentState) => T): T => useStore(session, selector);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const scrollerRef = useRef<HTMLDivElement>(null);
  const spacerRef = useRef<HTMLDivElement>(null);
  const textRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<ViewportRenderer | null>(null);
  const saveTimer = useRef<number | null>(null);

  const doc = useDocument((s) => s.doc);
  const pageSizes = useDocument((s) => s.pageSizes);
  const zoom = useDocument((s) => s.zoom);
  const viewMode = useDocument((s) => s.viewMode);
  const docRotation = useDocument((s) => s.docRotation);
  const pageRotation = useDocument((s) => s.pageRotation);
  const generation = useDocument((s) => s.generation);
  const texts = useDocument((s) => s.texts);
  const highlights = useDocument((s) => s.highlights);
  const annots = useDocument((s) => s.annots);
  const annotLists = useDocument((s) => s.annotLists);
  const annotImages = useDocument((s) => s.annotImages);
  const selection = useDocument((s) => s.selection);
  const tool = useDocument((s) => s.tool);
  const pagesView = useDocument((s) => s.pagesView);
  const pagesEpoch = useDocument((s) => s.pagesEpoch);
  const marks = useDocument((s) => s.marks);
  // Read through a ref: the renderer's callbacks are built once, and must
  // call whatever the panel passes now.
  const onScrolled = useRef(props.onScrolled);
  onScrolled.current = props.onScrolled;

  useEffect(() => {
    const canvas = canvasRef.current;
    const scroller = scrollerRef.current;
    const spacer = spacerRef.current;
    const textLayer = textRef.current;
    if (!canvas || !scroller || !spacer || !textLayer) return;

    const store = session.getState;
    const renderer = new ViewportRenderer(
      { canvas, scroller, spacer, textLayer },
      {
        onPage: (page) => store().setPage(page),
        onScroll: (_x, y) => {
          onScrolled.current?.();
          if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
          saveTimer.current = window.setTimeout(() => {
            void store().saveView(y);
          }, SAVE_IDLE_MS);
        },
        onZoom: (next) => store().setZoom(next),
        onViewport: (w, h) => store().setViewport(w, h),
        onWantText: (pages) => {
          void store().loadText(pages);
          // The same pages, and the only ones worth asking about: highlights are
          // fetched per page from the worker, so asking for a page nobody is
          // looking at would be a round trip for nothing.
          void store().loadHighlights(pages);
        },
        onWantAnnots: (pages) => void store().loadAnnots(pages),
        onSelect: (ids) => store().select(ids),
        onTransform: (objects) => void store().replaceAnnots(objects),
        onDraw: (drawn) => {
          const kind = store().tool;
          if (kind === null) return;
          const object = objectFromDrawn(kind, drawn, DEFAULT_STYLE, Date.now());
          if (object) void store().addAnnot(object);
          else store().setTool(null);
        },
      },
    );
    rendererRef.current = renderer;
    const docId = store().doc;
    if (docId !== null) registerViewport(docId, renderer);
    store().setViewport(scroller.clientWidth, scroller.clientHeight);

    // Annotation editing is pointer work on the scrolling element. It is
    // attached here rather than as React handlers because the renderer owns the
    // gesture, and a React re-render in the middle of a drag would be a frame
    // the pointer did not get.
    const down = (e: PointerEvent): void => {
      if (e.button !== 0) return;
      const editing = store().tool !== null || store().selection.length > 0;
      if (!renderer.onAnnotPointerDown(e)) return;
      if (renderer.gestureActive) {
        // Only capture once a gesture really started, so an ordinary click on
        // the page still reaches the text layer for selection.
        if (editing || store().selection.length > 0) {
          scroller.setPointerCapture(e.pointerId);
          e.preventDefault();
        }
      }
    };
    const move = (e: PointerEvent): void => {
      if (renderer.gestureActive) renderer.onAnnotPointerMove(e);
    };
    const up = (e: PointerEvent): void => {
      if (!renderer.gestureActive) return;
      renderer.onAnnotPointerUp(e);
      if (scroller.hasPointerCapture(e.pointerId)) scroller.releasePointerCapture(e.pointerId);
    };
    scroller.addEventListener("pointerdown", down);
    scroller.addEventListener("pointermove", move);
    scroller.addEventListener("pointerup", up);
    scroller.addEventListener("pointercancel", up);

    return () => {
      scroller.removeEventListener("pointerdown", down);
      scroller.removeEventListener("pointermove", move);
      scroller.removeEventListener("pointerup", up);
      scroller.removeEventListener("pointercancel", up);
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
      if (docId !== null) registerViewport(docId, null);
      rendererRef.current = null;
      renderer.destroy();
    };
    // One renderer per session: a different document is a different panel
    // content, and gets a fresh renderer rather than a repainted one.
  }, [session]);

  useEffect(() => {
    rendererRef.current?.update({
      doc,
      pageSizes,
      zoom,
      mode: viewMode,
      docRotation,
      pageRotation,
      generation,
      texts,
      highlights,
      annots,
      annotLists,
      annotImages,
      selection,
      tool,
      pagesView,
      pagesEpoch,
      marks,
    });
  }, [
    doc,
    pageSizes,
    zoom,
    viewMode,
    docRotation,
    pageRotation,
    generation,
    texts,
    highlights,
    annots,
    annotLists,
    annotImages,
    selection,
    tool,
    pagesView,
    pagesEpoch,
    marks,
  ]);

  // A search result asks the viewport to go somewhere. The store cannot scroll
  // — it has no renderer — so it leaves the page behind and this picks it up.
  const pendingPage = useDocument((s) => s.pendingPage);
  useEffect(() => {
    if (pendingPage === null) return;
    const page = session.getState().consumePendingPage();
    if (page !== null) {
      session.getState().setPage(page);
      rendererRef.current?.goToPage(page);
    }
  }, [pendingPage]);

  // A document that has been open before reopens where it was left (SPEC 11.1).
  // Applied after the first layout, so the offset means what it meant then.
  useEffect(() => {
    if (doc === null) return;
    const pending = session.getState().consumePendingScroll();
    if (pending !== null) {
      rendererRef.current?.restoreScroll(0, pending);
    }
  }, [doc, pageSizes]);

  return (
    <div
      className="relative flex-1 min-h-0 min-w-0 overflow-hidden bg-[var(--izul-canvas)]"
      onPointerDownCapture={props.onFocus}
      onFocusCapture={props.onFocus}
    >
      <canvas ref={canvasRef} className="absolute inset-0 block" aria-hidden="true" />
      <div
        ref={scrollerRef}
        // Focusable so the whole viewport is operable from the keyboard: the
        // browser's own scrolling handles arrows, Page Up/Down, Home and End
        // once something inside it has focus (SPEC 14).
        tabIndex={0}
        role="region"
        aria-label={t("viewport.label")}
        className="absolute inset-0 overflow-auto outline-none"
        // A drawing tool takes the pointer, so the text layer underneath must
        // not also start a selection with it.
        style={tool === null ? undefined : { cursor: "crosshair" }}
      >
        <div ref={spacerRef} className="relative">
          <div
            ref={textRef}
            className={`izul-text-layer absolute inset-0 ${tool === null ? "select-text" : "pointer-events-none"}`}
            role="document"
            aria-label={t("viewport.textLayer")}
          />
        </div>
      </div>
    </div>
  );
}
