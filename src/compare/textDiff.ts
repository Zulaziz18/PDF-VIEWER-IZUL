/**
 * Text differences between two pages, for compare mode (Phase 5).
 *
 * Word by word, not character by character: a reader comparing two versions
 * of a contract wants "this word changed", not a confetti of single letters
 * that happen to match across different words. The alignment is a longest
 * common subsequence over the words, so an inserted sentence marks only the
 * sentence, not everything after it.
 *
 * Pure: characters in, boxes out. The boxes are in the same display space
 * the text layer and search highlights use, so they land on the glyphs.
 */

import type { PdfRect } from "@/viewport/geometry";
import type { TextChar } from "@/viewport/textLayer";

export interface Word extends PdfRect {
  readonly text: string;
}

/** Characters into words: whitespace separates, and so does a jump to
 * another line (a hyphen-less line wrap must not glue two words). */
export function words(chars: readonly TextChar[]): Word[] {
  const out: Word[] = [];
  let text = "";
  let box: { left: number; bottom: number; right: number; top: number } | null = null;
  const flush = (): void => {
    if (text.length > 0 && box) out.push({ text, ...box });
    text = "";
    box = null;
  };
  for (const ch of chars) {
    if (/\s/.test(ch.c) || ch.c.length === 0) {
      flush();
      continue;
    }
    if (box) {
      const height = Math.max(1, box.top - box.bottom);
      const sameLine = Math.abs(ch.bottom - box.bottom) < height * 0.5;
      if (!sameLine) flush();
    }
    text += ch.c;
    box = box
      ? {
          left: Math.min(box.left, ch.left),
          bottom: Math.min(box.bottom, ch.bottom),
          right: Math.max(box.right, ch.right),
          top: Math.max(box.top, ch.top),
        }
      : { left: ch.left, bottom: ch.bottom, right: ch.right, top: ch.top };
  }
  flush();
  return out;
}

/**
 * Which words of each side are not part of the longest common subsequence.
 * Returns index lists into `a` and `b`.
 *
 * Quadratic in time and memory; a dense page is a few hundred words, and
 * above `limit` squared cells the pages are compared by their common prefix
 * and suffix only rather than stalling the window.
 */
export function changedWords(
  a: readonly string[],
  b: readonly string[],
  limit = 3000,
): { a: number[]; b: number[] } {
  // Common prefix and suffix first: most pages differ in a small part.
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const midA = a.slice(start, endA);
  const midB = b.slice(start, endB);
  const range = (from: number, to: number): number[] =>
    Array.from({ length: Math.max(0, to - from) }, (_, i) => from + i);
  if (midA.length === 0 || midB.length === 0 || midA.length * midB.length > limit * limit) {
    return { a: range(start, endA), b: range(start, endB) };
  }
  const n = midA.length;
  const m = midB.length;
  // lcs[i][j]: LCS length of midA[i..] and midB[j..].
  const lcs: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    const row = lcs[i] as Uint32Array;
    const below = lcs[i + 1] as Uint32Array;
    for (let j = m - 1; j >= 0; j--) {
      row[j] = midA[i] === midB[j] ? (below[j + 1] as number) + 1 : Math.max(below[j] as number, row[j + 1] as number);
    }
  }
  const keptA = new Set<number>();
  const keptB = new Set<number>();
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (midA[i] === midB[j]) {
      keptA.add(i);
      keptB.add(j);
      i++;
      j++;
    } else if ((lcs[i + 1]?.[j] ?? 0) >= (lcs[i]?.[j + 1] ?? 0)) {
      i++;
    } else {
      j++;
    }
  }
  return {
    a: range(0, n).filter((k) => !keptA.has(k)).map((k) => k + start),
    b: range(0, m).filter((k) => !keptB.has(k)).map((k) => k + start),
  };
}

/** Neighbouring marked words on one line become one box. */
export function mergeOnLines(boxes: readonly PdfRect[]): PdfRect[] {
  const out: { left: number; bottom: number; right: number; top: number }[] = [];
  for (const r of boxes) {
    const last = out[out.length - 1];
    const height = Math.max(1, r.top - r.bottom);
    if (
      last &&
      Math.abs(last.bottom - r.bottom) < height * 0.5 &&
      r.left - last.right < height * 1.5 &&
      r.left >= last.left
    ) {
      last.right = Math.max(last.right, r.right);
      last.top = Math.max(last.top, r.top);
      last.bottom = Math.min(last.bottom, r.bottom);
    } else {
      out.push({ left: r.left, bottom: r.bottom, right: r.right, top: r.top });
    }
  }
  return out;
}

/** The boxes to mark on each page. */
export function textDiff(
  a: readonly TextChar[],
  b: readonly TextChar[],
): { a: PdfRect[]; b: PdfRect[]; changed: number } {
  const wa = words(a);
  const wb = words(b);
  const changed = changedWords(
    wa.map((w) => w.text),
    wb.map((w) => w.text),
  );
  const pick = (ws: readonly Word[], idx: readonly number[]): PdfRect[] =>
    mergeOnLines(idx.flatMap((k) => (ws[k] ? [ws[k] as Word] : [])));
  return {
    a: pick(wa, changed.a),
    b: pick(wb, changed.b),
    changed: changed.a.length + changed.b.length,
  };
}
