/**
 * Painting search matches over the page (SPEC 11.1).
 *
 * The boxes come from the backend in display space — points, origin at the
 * bottom-left, the same space the text layer's character boxes are in — and
 * have to land exactly where the glyphs are, at any zoom and rotation. That is
 * the whole job, and it is the one part of search that is easy to get subtly
 * wrong: a highlight that is off by the page height looks like a highlight that
 * simply does not work.
 *
 * They are drawn as DOM elements rather than onto the canvas on purpose. The
 * canvas is repainted from cached tiles every frame; a highlight painted there
 * would have to be repainted in the same pass and would flicker whenever a tile
 * arrived. As elements they sit above the tiles, under the text layer, and cost
 * nothing per frame.
 */

import type { PdfRect } from "./geometry";
import type { PagePlacement } from "./textLayer";

/** A highlight box in CSS pixels, relative to its page's top-left corner. */
export interface HighlightBox {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

/**
 * Converts one display-space rectangle into a box the page's layer can hold.
 *
 * The y flip is the part worth naming: PDF space counts upwards from the bottom
 * of the page and CSS counts downwards from the top, so the *top* of a box in
 * points becomes its distance from the top of the page in pixels.
 */
export function highlightBox(rect: PdfRect, place: PagePlacement): HighlightBox {
  return {
    left: rect.left * place.zoom,
    top: (place.pageHeight - rect.top) * place.zoom,
    width: Math.max(1, (rect.right - rect.left) * place.zoom),
    height: Math.max(1, (rect.top - rect.bottom) * place.zoom),
  };
}

/**
 * Builds the layer for one page's matches.
 *
 * Detached, like the text layer, so a page's highlights are swapped in one
 * operation instead of appearing box by box.
 */
export function buildHighlightLayer(
  rects: readonly PdfRect[],
  place: PagePlacement,
  doc: Document,
): HTMLElement {
  const host = doc.createElement("div");
  host.className = "izul-highlight-page";
  host.style.position = "absolute";
  host.style.left = `${place.x}px`;
  host.style.top = `${place.y}px`;
  host.setAttribute("aria-hidden", "true");
  for (const rect of rects) {
    const box = highlightBox(rect, place);
    const el = doc.createElement("div");
    el.className = "izul-highlight";
    el.style.position = "absolute";
    el.style.left = `${box.left}px`;
    el.style.top = `${box.top}px`;
    el.style.width = `${box.width}px`;
    el.style.height = `${box.height}px`;
    host.appendChild(el);
  }
  return host;
}
