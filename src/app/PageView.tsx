/**
 * Hosts the imperative viewport surface.
 *
 * The component's whole job is to own a `<canvas>` element and tell the
 * renderer when the document or the zoom changed. It never touches pixels
 * itself — that boundary is what SPEC 4 is protecting.
 */

import { useEffect, useRef } from "react";
import { Canvas2DSurface } from "@/viewport/canvas";
import { loadTile, StaleTileError } from "@/viewport/tileSource";
import { pageSizePx, pxRectToPdf, tilesCovering } from "@/viewport/geometry";
import { useDocument } from "@/state/documentStore";

export function PageView(): React.JSX.Element {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const surfaceRef = useRef<Canvas2DSurface | null>(null);
  const doc = useDocument((s) => s.doc);
  const page = useDocument((s) => s.page);
  const zoom = useDocument((s) => s.zoom);
  const generation = useDocument((s) => s.generation);
  const pageSizes = useDocument((s) => s.pageSizes);
  const requestTile = useDocument((s) => s.requestTile);

  useEffect(() => {
    const el = canvasRef.current;
    if (!el) return;
    surfaceRef.current = new Canvas2DSurface(el);
  }, []);

  useEffect(() => {
    const surface = surfaceRef.current;
    const metrics = pageSizes[page];
    if (!surface || doc === null || !metrics) return;

    // Abandoning the controller on cleanup is the frontend half of SPEC 6's
    // cancellation: tiles for a superseded generation are never awaited.
    const abort = new AbortController();
    const dpr = window.devicePixelRatio || 1;
    const scale = { zoom, dpr };
    const { w, h } = pageSizePx(metrics, scale);

    surface.resize(w / dpr, h / dpr, dpr);
    surface.clear(getComputedStyle(document.body).getPropertyValue("--izul-canvas") || "#f5f5f4");

    const tiles = tilesCovering({ x: 0, y: 0, w, h }, metrics, scale);
    for (const tile of tiles) {
      // A placeholder goes down first so the user never sees a white page while
      // a tile is in flight (SPEC 9).
      surface.drawPlaceholder(tile.px, "#ffffff");
    }

    void (async () => {
      for (const tile of tiles) {
        if (abort.signal.aborted) return;
        try {
          const source = pxRectToPdf(tile.px, metrics, scale);
          const handle = await requestTile(page, source, tile.px.w, tile.px.h, true);
          if (abort.signal.aborted) return;
          const decoded = await loadTile(handle, abort.signal);
          surface.drawTile(decoded.bitmap, tile.px);
          decoded.bitmap.close();
        } catch (e) {
          // A recycled slot is expected during fast scrolling and is not
          // something to report; anything else is worth a log line.
          if (!(e instanceof StaleTileError) && !abort.signal.aborted) {
            console.warn("ubin gagal", e);
          }
        }
      }
      surface.present();
    })();

    return () => abort.abort();
  }, [doc, page, zoom, generation, pageSizes, requestTile]);

  return (
    <div className="flex-1 overflow-auto grid place-items-start justify-center p-6">
      <canvas ref={canvasRef} className="shadow-sm rounded-[2px] bg-white" />
    </div>
  );
}
