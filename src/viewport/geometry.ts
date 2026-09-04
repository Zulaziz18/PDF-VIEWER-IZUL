/**
 * Coordinate conversion between PDF space and device pixels.
 *
 * SPEC 8: the model stores points with the origin bottom-left. Pixels exist
 * only here. Everything in this file is pure, so the conversions have tests and
 * do not need a canvas to exercise.
 */

/** A rectangle in PDF user space: points, origin bottom-left, `top` > `bottom`. */
export interface PdfRect {
  readonly left: number;
  readonly bottom: number;
  readonly right: number;
  readonly top: number;
}

/** A rectangle in device pixels: origin top-left, y growing downward. */
export interface PxRect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

export interface PageMetrics {
  /** Page width in points. */
  readonly width: number;
  /** Page height in points. */
  readonly height: number;
}

/**
 * How PDF points map to device pixels.
 *
 * `dpr` is kept separate from `zoom` on purpose. Zoom is what the user chose;
 * dpr is what the monitor is. Multiplying them into one number early makes it
 * impossible to keep "125 %" showing as 125 % when the window moves to a
 * different-DPI monitor (SPEC 9).
 */
export interface Scale {
  readonly zoom: number;
  readonly dpr: number;
}

export function pixelsPerPoint(s: Scale): number {
  return s.zoom * s.dpr;
}

/** Size of a page in device pixels at a given scale. */
export function pageSizePx(page: PageMetrics, s: Scale): { w: number; h: number } {
  const k = pixelsPerPoint(s);
  return { w: Math.max(1, Math.round(page.width * k)), h: Math.max(1, Math.round(page.height * k)) };
}

/** Zoom that fits a page's width into `viewportPx` device pixels. */
export function fitWidth(page: PageMetrics, viewportPx: number, dpr: number): number {
  if (page.width <= 0) return 1;
  return viewportPx / dpr / page.width;
}

/** Zoom that fits a whole page inside a viewport. */
export function fitPage(
  page: PageMetrics,
  viewportWpx: number,
  viewportHpx: number,
  dpr: number,
): number {
  if (page.width <= 0 || page.height <= 0) return 1;
  return Math.min(viewportWpx / dpr / page.width, viewportHpx / dpr / page.height);
}

/**
 * Converts a device-pixel rectangle within a page into the PDF rectangle it
 * covers, flipping the y axis.
 *
 * This is the function that decides what a tile actually draws, so it is the one
 * that has to be right.
 */
export function pxRectToPdf(r: PxRect, page: PageMetrics, s: Scale): PdfRect {
  const k = pixelsPerPoint(s);
  const left = r.x / k;
  const right = (r.x + r.w) / k;
  // Device y grows down from the page top; PDF y grows up from the page bottom.
  const top = page.height - r.y / k;
  const bottom = page.height - (r.y + r.h) / k;
  return { left, bottom, right, top };
}

/** The inverse of {@link pxRectToPdf}. */
export function pdfRectToPx(r: PdfRect, page: PageMetrics, s: Scale): PxRect {
  const k = pixelsPerPoint(s);
  return {
    x: r.left * k,
    y: (page.height - r.top) * k,
    w: (r.right - r.left) * k,
    h: (r.top - r.bottom) * k,
  };
}

/** Edge length of a tile, matching `izul_ipc::ring::TILE_EDGE`. */
export const TILE_EDGE = 512;

/**
 * The tiles needed to cover a visible region of a page.
 *
 * Tiles are aligned to a fixed grid in page pixel space rather than to the
 * viewport, so panning by a few pixels reuses the same tiles instead of
 * invalidating every one of them.
 */
export function tilesCovering(
  visible: PxRect,
  page: PageMetrics,
  s: Scale,
): Array<{ col: number; row: number; px: PxRect }> {
  const { w: pw, h: ph } = pageSizePx(page, s);
  const x0 = Math.max(0, Math.floor(visible.x / TILE_EDGE));
  const y0 = Math.max(0, Math.floor(visible.y / TILE_EDGE));
  const x1 = Math.min(Math.ceil(pw / TILE_EDGE), Math.ceil((visible.x + visible.w) / TILE_EDGE));
  const y1 = Math.min(Math.ceil(ph / TILE_EDGE), Math.ceil((visible.y + visible.h) / TILE_EDGE));

  const out: Array<{ col: number; row: number; px: PxRect }> = [];
  for (let row = y0; row < y1; row++) {
    for (let col = x0; col < x1; col++) {
      const x = col * TILE_EDGE;
      const y = row * TILE_EDGE;
      out.push({
        col,
        row,
        // Clamp the last row and column so a tile never claims to cover area
        // beyond the page; the worker would reject the source rect.
        px: { x, y, w: Math.min(TILE_EDGE, pw - x), h: Math.min(TILE_EDGE, ph - y) },
      });
    }
  }
  return out;
}

/**
 * Zoom about a fixed point.
 *
 * Returns the scroll offset that keeps `anchorPx` — usually the cursor —
 * pointing at the same spot on the page after the zoom change (SPEC 11.1).
 */
export function zoomAbout(
  anchorPx: { x: number; y: number },
  scroll: { x: number; y: number },
  oldZoom: number,
  newZoom: number,
): { x: number; y: number } {
  const ratio = newZoom / oldZoom;
  return {
    x: (scroll.x + anchorPx.x) * ratio - anchorPx.x,
    y: (scroll.y + anchorPx.y) * ratio - anchorPx.y,
  };
}
