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
import { ViewportRenderer } from "@/viewport/renderer";
import { useDocument } from "@/state/documentStore";
import { setViewport } from "./viewportHandle";
import { t } from "@/i18n";

/** How long the scroll must be still before the reading position is stored. */
const SAVE_IDLE_MS = 1200;

export function Viewport(): React.JSX.Element {
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

  useEffect(() => {
    const canvas = canvasRef.current;
    const scroller = scrollerRef.current;
    const spacer = spacerRef.current;
    const textLayer = textRef.current;
    if (!canvas || !scroller || !spacer || !textLayer) return;

    const store = useDocument.getState;
    const renderer = new ViewportRenderer(
      { canvas, scroller, spacer, textLayer },
      {
        onPage: (page) => store().setPage(page),
        onScroll: (_x, y) => {
          if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
          saveTimer.current = window.setTimeout(() => {
            void store().saveView(y);
          }, SAVE_IDLE_MS);
        },
        onZoom: (next) => store().setZoom(next),
        onViewport: (w, h) => store().setViewport(w, h),
        onWantText: (pages) => void store().loadText(pages),
      },
    );
    rendererRef.current = renderer;
    setViewport(renderer);
    store().setViewport(scroller.clientWidth, scroller.clientHeight);

    return () => {
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
      setViewport(null);
      rendererRef.current = null;
      renderer.destroy();
    };
  }, []);

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
    });
  }, [doc, pageSizes, zoom, viewMode, docRotation, pageRotation, generation, texts]);

  // A document that has been open before reopens where it was left (SPEC 11.1).
  // Applied after the first layout, so the offset means what it meant then.
  useEffect(() => {
    if (doc === null) return;
    const pending = useDocument.getState().consumePendingScroll();
    if (pending !== null) {
      rendererRef.current?.restoreScroll(0, pending);
    }
  }, [doc, pageSizes]);

  return (
    <div className="relative flex-1 min-h-0 overflow-hidden bg-[var(--izul-canvas)]">
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
      >
        <div ref={spacerRef} className="relative">
          <div
            ref={textRef}
            className="izul-text-layer absolute inset-0 select-text"
            aria-label={t("viewport.textLayer")}
          />
        </div>
      </div>
    </div>
  );
}
