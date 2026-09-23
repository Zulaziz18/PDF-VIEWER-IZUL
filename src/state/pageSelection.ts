/**
 * Selecting and dropping pages in the page panel (Phase 5).
 *
 * Pure, because these are the rules a user's hand learns — click, Ctrl+click,
 * Shift+click, where a dropped page lands — and a rule that is only visible
 * by dragging thumbnails around is one nobody can check.
 */

export interface ClickModifiers {
  readonly toggle: boolean; // Ctrl (Cmd on a Mac)
  readonly range: boolean; // Shift
}

export interface SelectionState {
  readonly pages: readonly number[];
  /** Where a Shift+click range starts. */
  readonly anchor: number | null;
}

/** The selection after clicking `page`, the way file managers do it. */
export function clickSelect(state: SelectionState, page: number, mods: ClickModifiers): SelectionState {
  if (mods.range && state.anchor !== null) {
    const [a, b] = state.anchor <= page ? [state.anchor, page] : [page, state.anchor];
    const span = Array.from({ length: b - a + 1 }, (_, i) => a + i);
    const pages = mods.toggle ? [...new Set([...state.pages, ...span])] : span;
    return { pages: pages.sort((x, y) => x - y), anchor: state.anchor };
  }
  if (mods.toggle) {
    const has = state.pages.includes(page);
    const pages = has ? state.pages.filter((p) => p !== page) : [...state.pages, page];
    return { pages: [...pages].sort((x, y) => x - y), anchor: page };
  }
  return { pages: [page], anchor: page };
}

/**
 * Where a drop at `offsetY` (pixels from the top of the strip) inserts: the
 * page slot under the pointer, before it when in its top half and after it
 * in its bottom half. Returns a "before" index, `count` meaning the end.
 */
export function dropBefore(offsetY: number, slot: number, count: number): number {
  if (count <= 0) return 0;
  const index = Math.floor(Math.max(0, offsetY) / slot);
  if (index >= count) return count;
  const within = Math.max(0, offsetY) - index * slot;
  return within < slot / 2 ? index : index + 1;
}

/**
 * True when moving `pages` before `before` would leave everything where it
 * is — dropping a page onto its own slot — so no undo step is made for it.
 */
export function moveChangesNothing(pages: readonly number[], before: number): boolean {
  const sorted = [...new Set(pages)].sort((a, b) => a - b);
  if (sorted.length === 0) return true;
  const first = sorted[0] as number;
  const last = sorted[sorted.length - 1] as number;
  const contiguous = last - first + 1 === sorted.length;
  return contiguous && before >= first && before <= last + 1;
}

/** What the page operations act on: the selection, or else the current page. */
export function targetPages(selection: readonly number[], current: number): number[] {
  return selection.length > 0 ? [...selection] : [current];
}

/** The drag payload between page panels and viewports. */
export const PAGE_DRAG_TYPE = "application/x-izul-pages";

export interface PageDrag {
  readonly doc: number;
  readonly pages: readonly number[];
}

export function encodePageDrag(drag: PageDrag): string {
  return JSON.stringify(drag);
}

export function decodePageDrag(text: string): PageDrag | null {
  try {
    const v = JSON.parse(text) as unknown;
    if (
      typeof v === "object" &&
      v !== null &&
      typeof (v as PageDrag).doc === "number" &&
      Array.isArray((v as PageDrag).pages) &&
      (v as PageDrag).pages.every((p) => Number.isInteger(p) && p >= 0)
    ) {
      return { doc: (v as PageDrag).doc, pages: [...(v as PageDrag).pages] };
    }
  } catch {
    // Not ours: some other drag passing over the panel.
  }
  return null;
}
