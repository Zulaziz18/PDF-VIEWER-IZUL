/**
 * Page ranges as a person types them into an export dialog: "1-3, 5, 8-".
 *
 * Pure, because the export writes whatever this returns — a range read one
 * page off is a file with the wrong pages in it, and nobody checks an exported
 * file page by page. The rules are the ones print dialogs have taught everyone:
 *
 * - pages are numbered from 1, as the bottom bar shows them;
 * - `a-b` is inclusive, and `b-a` means the same pages in reverse order —
 *   "10-1" is a deliberate request, not a typo worth refusing;
 * - `a-` runs to the last page and `-b` starts at the first;
 * - commas, semicolons and spaces all separate;
 * - a page asked for twice is exported once, where it first appeared.
 *
 * Returns zero-based page indices in the order asked for, or the reason the
 * text cannot be read, pointing at the part that is wrong.
 */

export type RangeResult =
  | { readonly ok: true; readonly pages: number[] }
  | { readonly ok: false; readonly error: PageRangeError };

export type PageRangeError =
  | { readonly kind: "empty" }
  | { readonly kind: "syntax"; readonly part: string }
  | { readonly kind: "outOfRange"; readonly part: string; readonly pageCount: number };

export function parsePageRange(text: string, pageCount: number): RangeResult {
  // "1 - 3" means "1-3": close up the spaces around a dash before splitting,
  // or the dash would become a part of its own.
  const joined = text
    .replace(/\s*-\s*/g, "-")
    .split(/[,;\s]+/)
    .filter((p) => p.length > 0);
  if (joined.length === 0) return { ok: false, error: { kind: "empty" } };

  const seen = new Set<number>();
  const pages: number[] = [];
  const push = (n: number): void => {
    if (!seen.has(n)) {
      seen.add(n);
      pages.push(n);
    }
  };

  for (const part of joined) {
    const match = /^(\d*)-(\d*)$/.exec(part) ?? /^(\d+)$/.exec(part);
    if (!match) return { ok: false, error: { kind: "syntax", part } };
    const single = match.length === 2;
    const fromText = match[1] ?? "";
    const toText = single ? fromText : (match[2] ?? "");
    if (fromText === "" && toText === "") return { ok: false, error: { kind: "syntax", part } };
    const from = fromText === "" ? 1 : Number(fromText);
    const to = toText === "" ? pageCount : Number(toText);
    if (from < 1 || to < 1 || from > pageCount || to > pageCount) {
      return { ok: false, error: { kind: "outOfRange", part, pageCount } };
    }
    const step = from <= to ? 1 : -1;
    for (let n = from; step > 0 ? n <= to : n >= to; n += step) push(n - 1);
  }
  return { ok: true, pages };
}

/** The same pages written back compactly, for a label: [0,1,2,4] → "1-3, 5". */
export function formatPageRange(pages: readonly number[]): string {
  const out: string[] = [];
  let i = 0;
  while (i < pages.length) {
    const start = pages[i] as number;
    let end = start;
    while (pages[i + 1] === end + 1) {
      end += 1;
      i += 1;
    }
    out.push(start === end ? `${start + 1}` : `${start + 1}-${end + 1}`);
    i += 1;
  }
  return out.join(", ");
}

/** Split points for "Pecah": every `n` pages. */
export function rangesEvery(n: number, pageCount: number): number[][] {
  const size = Math.max(1, Math.floor(n));
  const out: number[][] = [];
  for (let start = 0; start < pageCount; start += size) {
    out.push(Array.from({ length: Math.min(size, pageCount - start) }, (_, i) => start + i));
  }
  return out;
}

/**
 * "1-3; 4-6, 9": one output file per `;`-separated group, each group read
 * like a single range. The first group that cannot be read is the error.
 */
export function parseRangeGroups(
  text: string,
  pageCount: number,
): { ok: true; groups: number[][] } | { ok: false; error: PageRangeError; group: number } {
  const parts = text
    .split(";")
    .map((p) => p.trim())
    .filter((p) => p.length > 0);
  if (parts.length === 0) return { ok: false, error: { kind: "empty" }, group: 0 };
  const groups: number[][] = [];
  for (const [i, part] of parts.entries()) {
    const r = parsePageRange(part, pageCount);
    if (!r.ok) return { ok: false, error: r.error, group: i };
    groups.push(r.pages);
  }
  return { ok: true, groups };
}

/**
 * One file per top-level bookmark: each runs from its bookmark's page to
 * the page before the next one. Pages before the first bookmark — a cover,
 * a table of contents — are a file of their own rather than being lost.
 * Bookmarks that point nowhere, or at a page another already starts at,
 * are skipped.
 */
export function rangesFromOutline(
  outline: readonly { readonly depth: number; readonly page: number | null }[],
  pageCount: number,
): number[][] {
  const starts = [
    ...new Set(
      outline
        .filter((e) => e.depth === 0 && e.page !== null && e.page >= 0 && e.page < pageCount)
        .map((e) => e.page as number),
    ),
  ].sort((a, b) => a - b);
  if (starts.length === 0) return [];
  if (starts[0] !== 0) starts.unshift(0);
  return starts.map((start, i) => {
    const end = (starts[i + 1] ?? pageCount) - 1;
    return Array.from({ length: end - start + 1 }, (_, k) => start + k);
  });
}
