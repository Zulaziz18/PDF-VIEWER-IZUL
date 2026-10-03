/**
 * The selectable text layer (SPEC 11.1, SPEC 14).
 *
 * Transparent text positioned over the rendered page, so that selecting,
 * copying and a screen reader all work on the document's real text rather than
 * on a picture of it. The boxes come from PDFium, in display space, which is
 * what makes the selection land on the glyphs in justified and rotated text
 * instead of on a layout we guessed at.
 *
 * Characters are grouped into lines rather than given a span each: a dense page
 * carries four thousand characters, and four thousand absolutely positioned
 * elements per page is a layout cost the frame budget cannot absorb. A line is
 * one element, its width matched to the real text extent by a horizontal
 * scale — the same trade every PDF viewer with selectable text makes.
 */

import type { PdfRect } from "./geometry";

/** One character with its box in display space, as the backend reports it. */
export interface TextChar extends PdfRect {
  readonly c: string;
}

/** A run of characters on one visual line. */
export interface TextLine extends PdfRect {
  readonly text: string;
}

/**
 * Groups characters into lines.
 *
 * Two characters share a line when their boxes overlap vertically by more than
 * half the smaller box's height and the text keeps moving forwards. A jump
 * backwards starts a new line even when the vertical overlap holds, which is
 * what separates the columns of a two-column page.
 */
export function groupIntoLines(chars: readonly TextChar[]): TextLine[] {
  const lines: TextLine[] = [];
  let text = "";
  let box: PdfRect | null = null;

  const flush = (): void => {
    if (box && text.trim().length > 0) {
      lines.push({ ...box, text });
    }
    text = "";
    box = null;
  };

  for (const ch of chars) {
    // Line and paragraph separators carry no box worth keeping.
    if (ch.c === "\n" || ch.c === "\r") {
      flush();
      continue;
    }
    if (!box) {
      box = { left: ch.left, bottom: ch.bottom, right: ch.right, top: ch.top };
      text = ch.c;
      continue;
    }
    const overlap = Math.min(box.top, ch.top) - Math.max(box.bottom, ch.bottom);
    const smaller = Math.min(box.top - box.bottom, ch.top - ch.bottom);
    const sameLine = smaller > 0 && overlap > smaller * 0.5;
    const movesBackwards = ch.right < box.left;
    if (!sameLine || movesBackwards) {
      flush();
      box = { left: ch.left, bottom: ch.bottom, right: ch.right, top: ch.top };
      text = ch.c;
      continue;
    }
    box = {
      left: Math.min(box.left, ch.left),
      bottom: Math.min(box.bottom, ch.bottom),
      right: Math.max(box.right, ch.right),
      top: Math.max(box.top, ch.top),
    };
    text += ch.c;
  }
  flush();
  return lines;
}

/** Where a page sits in content space, in CSS pixels. */
export interface PagePlacement {
  readonly x: number;
  readonly y: number;
  readonly zoom: number;
  /** Display height of the page in points, for the y flip. */
  readonly pageHeight: number;
}

/**
 * Builds the DOM for one page's text.
 *
 * Returns a detached element so the caller can swap a page's layer in one
 * operation rather than mutating a live subtree while the user is selecting.
 */
export function buildTextLayer(
  lines: readonly TextLine[],
  place: PagePlacement,
  doc: Document,
): HTMLElement {
  const host = doc.createElement("div");
  host.className = "izul-text-page";
  host.style.position = "absolute";
  host.style.left = `${place.x}px`;
  host.style.top = `${place.y}px`;
  host.setAttribute("aria-hidden", "false");

  for (const line of lines) {
    const el = doc.createElement("div");
    const height = (line.top - line.bottom) * place.zoom;
    const width = (line.right - line.left) * place.zoom;
    el.textContent = line.text;
    el.style.position = "absolute";
    el.style.left = `${line.left * place.zoom}px`;
    el.style.top = `${(place.pageHeight - line.top) * place.zoom}px`;
    el.style.height = `${height}px`;
    el.style.fontSize = `${height}px`;
    el.style.lineHeight = "1";
    el.style.whiteSpace = "pre";
    el.style.transformOrigin = "0 0";
    // The line is laid out at its natural width in the fallback font and then
    // scaled to the width PDFium measured, so the selection highlight tracks
    // the glyphs even though the font itself is not the document's.
    el.dataset["w"] = String(width);
    host.appendChild(el);
  }
  return host;
}

/**
 * Matches each line's rendered width to the width the PDF says it has.
 *
 * Must run after the element is in the document, because it measures. Split
 * from {@link buildTextLayer} so the building half stays pure enough to test.
 */
export function fitTextLayer(host: HTMLElement): void {
  for (const child of Array.from(host.children)) {
    if (!(child instanceof HTMLElement)) continue;
    const target = Number(child.dataset["w"] ?? 0);
    const actual = child.getBoundingClientRect().width;
    if (target > 0 && actual > 0) {
      child.style.transform = `scaleX(${target / actual})`;
    }
  }
}

/**
 * Where page `page`'s layer goes among the layers already there (their page
 * numbers, in DOM order): before the first later page. A screen reader reads
 * the DOM in its order, and pages arrive in the order they scroll into view.
 */
export function layerSlot(present: readonly number[], page: number): number {
  const at = present.findIndex((p) => p > page);
  return at < 0 ? present.length : at;
}

/**
 * The text inside a rectangle, line by line (SPEC 11.1: "seleksi persegi
 * (Alt+drag)", Phase 8) — a column of a table, a block beside a figure, what
 * a flowing selection cannot take without the text around it.
 *
 * A character counts when its centre is inside, so a glyph the edge only
 * grazes stays out. `rect` is in the same page space as the characters, in
 * any orientation.
 */
export function textInRect(
  chars: readonly TextChar[],
  rect: { left: number; bottom: number; right: number; top: number },
): string {
  const l = Math.min(rect.left, rect.right);
  const r = Math.max(rect.left, rect.right);
  const b = Math.min(rect.bottom, rect.top);
  const t = Math.max(rect.bottom, rect.top);
  const inside = chars.filter((ch) => {
    const x = (ch.left + ch.right) / 2;
    const y = (ch.bottom + ch.top) / 2;
    return x >= l && x <= r && y >= b && y <= t;
  });
  return groupIntoLines(inside)
    .map((line) => line.text.trim())
    .filter((text) => text.length > 0)
    .join("\n");
}
