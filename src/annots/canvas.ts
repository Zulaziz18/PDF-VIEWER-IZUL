/**
 * The canvas backend (SPEC 3.2).
 *
 * The second of the two consumers of a display list. It draws the live proxy —
 * what the user sees while an object is being dragged, and until PDFium's
 * authoritative render of the saved appearance takes over (SPEC 3.3).
 *
 * Like the appearance-stream backend, it translates and never decides. Every
 * curve, arrow head and glyph position arrived already settled; if this file
 * ever computes a shape of its own, the screen and the exported file have two
 * descriptions of it and the parity guarantee is gone.
 *
 * The one thing it does own is the *page-to-pixel* transform, because that is
 * not geometry — it is where the page happens to be on screen at this zoom, and
 * the PDF knows nothing about it.
 */

import type { DisplayList, Matrix, Path, PdfPoint, Rgba } from "./types";

/** Where a page sits on the canvas, in device pixels. */
export interface PagePlacement {
  /** Left edge of the page in canvas pixels. */
  readonly x: number;
  /** Top edge of the page in canvas pixels. */
  readonly y: number;
  /** Device pixels per PDF point. */
  readonly scale: number;
  /** Page height in points, for the y flip. */
  readonly pageHeight: number;
}

/**
 * The transform from PDF user space to canvas pixels.
 *
 * PDF counts y upwards from the bottom of the page and a canvas counts it
 * downwards from the top, so the flip lives here — once, in the only place that
 * knows about screens.
 */
export function pageTransform(place: PagePlacement): DOMMatrix {
  return new DOMMatrix([
    place.scale,
    0,
    0,
    -place.scale,
    place.x,
    place.y + place.pageHeight * place.scale,
  ]);
}

function toDom(m: Matrix): DOMMatrix {
  return new DOMMatrix([m.a, m.b, m.c, m.d, m.e, m.f]);
}

function css(c: Rgba, alpha = 1): string {
  const to255 = (v: number): number => Math.round(Math.min(1, Math.max(0, v)) * 255);
  return `rgba(${to255(c.r)}, ${to255(c.g)}, ${to255(c.b)}, ${c.a * alpha})`;
}

/** Pixel width of whatever kind of image source this is. */
function sourceWidth(source: CanvasImageSource): number {
  if (source instanceof HTMLImageElement) return source.naturalWidth;
  if (typeof VideoFrame !== "undefined" && source instanceof VideoFrame) return source.codedWidth;
  const withWidth = source as { width?: number };
  return typeof withWidth.width === "number" ? withWidth.width : Number.POSITIVE_INFINITY;
}

function sourceHeight(source: CanvasImageSource): number {
  if (source instanceof HTMLImageElement) return source.naturalHeight;
  if (typeof VideoFrame !== "undefined" && source instanceof VideoFrame) return source.codedHeight;
  const withHeight = source as { height?: number };
  return typeof withHeight.height === "number" ? withHeight.height : Number.POSITIVE_INFINITY;
}

function tracePath(ctx: CanvasRenderingContext2D, path: Path): void {
  ctx.beginPath();
  let current: PdfPoint | null = null;
  for (const seg of path.segs) {
    if (seg === "Close") {
      ctx.closePath();
      continue;
    }
    if ("MoveTo" in seg) {
      ctx.moveTo(seg.MoveTo.x, seg.MoveTo.y);
      current = seg.MoveTo;
    } else if ("LineTo" in seg) {
      ctx.lineTo(seg.LineTo.x, seg.LineTo.y);
      current = seg.LineTo;
    } else {
      const { c1, c2, to } = seg.CurveTo;
      // A curve with no current point is a malformed list; starting one here
      // rather than throwing keeps a single bad object from blanking the page.
      if (!current) ctx.moveTo(c1.x, c1.y);
      ctx.bezierCurveTo(c1.x, c1.y, c2.x, c2.y, to.x, to.y);
      current = to;
    }
  }
}

/** PDF blend modes and their canvas names. The two sets agree on everything the
 * display list can express, which is why the vocabulary was kept to these. */
const BLEND: Record<string, GlobalCompositeOperation> = {
  Normal: "source-over",
  Multiply: "multiply",
  Screen: "screen",
  Darken: "darken",
  Lighten: "lighten",
};

/**
 * On a page inverted for dark mode (Phase 8), a highlight multiplied over
 * dark paper would vanish; over the inverted page the colour that marks
 * without hiding the text is the screen of it, which is what multiply is
 * over white. The saved file is unaffected — this is only how it is shown.
 */
let inverted = false;
export function setCanvasInversion(on: boolean): void {
  inverted = on;
}
function blendOf(name: string): GlobalCompositeOperation {
  const mode = BLEND[name] ?? "source-over";
  if (!inverted) return mode;
  return mode === "multiply" ? "screen" : mode === "darken" ? "lighten" : mode;
}

/**
 * Draws one display list.
 *
 * `images` resolves an `ImageRef` to something drawable. A reference with no
 * image behind it draws nothing rather than a placeholder: an object that looks
 * different on screen from what will be written is exactly what this backend
 * exists to prevent.
 */
export function drawDisplayList(
  ctx: CanvasRenderingContext2D,
  list: DisplayList,
  place: PagePlacement,
  images?: ReadonlyMap<number, CanvasImageSource>,
): void {
  ctx.save();
  const base = pageTransform(place);
  ctx.setTransform(base);
  // Depth of the graphics-state stack this list opened, so an unbalanced list
  // cannot leak a clip into whatever is drawn next.
  let depth = 0;

  for (const op of list.ops) {
    if (op === "PopClip" || op === "PopTransform") {
      if (depth > 0) {
        ctx.restore();
        depth -= 1;
      }
      continue;
    }
    if ("PushClip" in op) {
      ctx.save();
      depth += 1;
      tracePath(ctx, op.PushClip.path);
      ctx.clip(op.PushClip.rule === "EvenOdd" ? "evenodd" : "nonzero");
      continue;
    }
    if ("PushTransform" in op) {
      ctx.save();
      depth += 1;
      const m = base.multiply(toDom(op.PushTransform.matrix));
      ctx.setTransform(m);
      continue;
    }
    if ("FillPath" in op) {
      const { path, color, rule, blend } = op.FillPath;
      ctx.save();
      ctx.globalCompositeOperation = blendOf(blend);
      ctx.fillStyle = css(color);
      tracePath(ctx, path);
      ctx.fill(rule === "EvenOdd" ? "evenodd" : "nonzero");
      ctx.restore();
      continue;
    }
    if ("StrokePath" in op) {
      const { path, color, style, blend } = op.StrokePath;
      ctx.save();
      ctx.globalCompositeOperation = blendOf(blend);
      ctx.strokeStyle = css(color);
      ctx.lineWidth = style.width;
      ctx.lineCap = style.cap === "Round" ? "round" : style.cap === "Square" ? "square" : "butt";
      ctx.lineJoin = style.join === "Round" ? "round" : style.join === "Bevel" ? "bevel" : "miter";
      ctx.miterLimit = style.miter_limit > 0 ? style.miter_limit : 10;
      ctx.setLineDash(style.dash as number[]);
      ctx.lineDashOffset = style.dash_phase;
      tracePath(ctx, path);
      ctx.stroke();
      ctx.restore();
      continue;
    }
    if ("DrawImage" in op) {
      const { image, matrix, opacity } = op.DrawImage;
      const source = images?.get(image);
      if (!source) continue;
      ctx.save();
      ctx.globalAlpha = opacity;
      const placed = base.multiply(toDom(matrix));
      ctx.setTransform(placed);
      // PDFium shows pixels when an image is magnified; a browser smooths by
      // default. Left alone, an inserted screenshot looks soft as a proxy and
      // then snaps to hard pixels the instant the authoritative render replaces
      // it — the visible swap SPEC 3.3 exists to prevent. Matching PDFium on
      // magnification is what keeps the two pictures the same one. Downscaling
      // still smooths, where both do.
      const drawnW = Math.hypot(placed.a, placed.b);
      const drawnH = Math.hypot(placed.c, placed.d);
      const naturalW = sourceWidth(source);
      const naturalH = sourceHeight(source);
      ctx.imageSmoothingEnabled = !(drawnW > naturalW || drawnH > naturalH);
      // The image occupies the unit square, and the matrix places it. Drawing
      // it upside down and flipping back is what keeps the image the right way
      // up under the page's own y flip.
      ctx.translate(0, 1);
      ctx.scale(1, -1);
      ctx.drawImage(source, 0, 0, 1, 1);
      ctx.restore();
      continue;
    }
    if ("DrawText" in op) {
      const { glyphs, size, matrix, color, blend } = op.DrawText;
      ctx.save();
      ctx.globalCompositeOperation = blendOf(blend);
      ctx.fillStyle = css(color);
      // The font here draws glyph *shapes*; every glyph's position came from
      // the layout, which used the metrics the exported file will use. A
      // difference in face shows as different letterforms, never as text in a
      // different place.
      ctx.font = `${size}px "Times New Roman", Times, serif`;
      ctx.textBaseline = "alphabetic";
      for (const glyph of glyphs) {
        const m = base
          .multiply(toDom(matrix))
          .multiply(new DOMMatrix([1, 0, 0, 1, glyph.offset.x, glyph.offset.y]))
          // Text is drawn in a y-up space, so the glyph itself is flipped back.
          .multiply(new DOMMatrix([1, 0, 0, -1, 0, 0]));
        ctx.setTransform(m);
        ctx.fillText(glyph.unicode, 0, 0);
      }
      ctx.restore();
      continue;
    }
  }

  while (depth > 0) {
    ctx.restore();
    depth -= 1;
  }
  ctx.restore();
}
