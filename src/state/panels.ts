/**
 * Split view (SPEC 10: "hingga 4 panel, grid 2×2, tab bisa ditarik antar
 * panel, ukuran panel diingat").
 *
 * Pure, and kept apart from the workspace store, because the rules are the
 * whole feature and each is easy to get subtly wrong:
 *
 * - **A document is shown in at most one panel.** Zoom, scroll position and
 *   selection belong to a document's session; two panels on one document
 *   would either share them — scrolling one would scroll the other — or need
 *   a second session for the same file, with two undo stacks editing one set
 *   of annotations. Asking for a document that is already visible focuses
 *   the panel showing it; dragging a tab onto a panel that shows another
 *   document swaps the two.
 * - **Changing the layout keeps what is shown.** Going from four panels to
 *   two keeps the focused document and the next ones; going from one to four
 *   fills the new panels with open tabs, in tab order, rather than leaving
 *   them empty.
 */

export type Layout = "single" | "columns" | "rows" | "grid";

export const PANEL_COUNT: Readonly<Record<Layout, number>> = {
  single: 1,
  columns: 2,
  rows: 2,
  grid: 4,
};

export interface PanelState {
  readonly layout: Layout;
  /** The document in each panel; `null` for an empty one. */
  readonly panels: readonly (number | null)[];
  readonly focused: number;
  /** Where the vertical and horizontal dividers sit, as fractions. */
  readonly ratio: { readonly x: number; readonly y: number };
}

export const SINGLE: PanelState = {
  layout: "single",
  panels: [null],
  focused: 0,
  ratio: { x: 0.5, y: 0.5 },
};

/** A divider may not squeeze a panel below a usable width. */
export function clampRatio(r: number): number {
  if (!Number.isFinite(r)) return 0.5;
  return Math.min(0.85, Math.max(0.15, r));
}

/**
 * Switches layout. `tabs` is the open documents in tab order, `active` the
 * one in front — which stays shown, in the focused panel.
 */
export function setLayout(
  state: PanelState,
  layout: Layout,
  tabs: readonly number[],
  active: number | null,
): PanelState {
  const count = PANEL_COUNT[layout];
  const shown = state.panels.filter((d): d is number => d !== null && tabs.includes(d));
  // The document in front first, then what was already visible, then the
  // rest of the tabs: a new panel gets something to show.
  const order: number[] = [];
  const push = (d: number | null): void => {
    if (d !== null && tabs.includes(d) && !order.includes(d)) order.push(d);
  };
  push(active);
  shown.forEach(push);
  tabs.forEach(push);
  const panels: (number | null)[] = Array.from({ length: count }, (_, i) => order[i] ?? null);
  const focused = active !== null ? Math.max(0, panels.indexOf(active)) : 0;
  return { ...state, layout, panels, focused };
}

/** Brings `doc` into view: its own panel if it has one, else the focused one. */
export function showDoc(state: PanelState, doc: number): PanelState {
  const at = state.panels.indexOf(doc);
  if (at >= 0) return { ...state, focused: at };
  const panels = [...state.panels];
  panels[state.focused] = doc;
  return { ...state, panels };
}

/** A tab dropped on `panel`: shown there, swapping with wherever it was. */
export function assign(state: PanelState, panel: number, doc: number): PanelState {
  if (panel < 0 || panel >= state.panels.length) return state;
  const panels = [...state.panels];
  const from = panels.indexOf(doc);
  if (from === panel) return { ...state, focused: panel };
  if (from >= 0) panels[from] = panels[panel] ?? null;
  panels[panel] = doc;
  return { ...state, panels, focused: panel };
}

/**
 * A document closed. In a single panel the successor tab takes its place; in
 * a split the panel is left empty, because filling it with an unrelated tab
 * would silently change what the user had arranged side by side.
 */
export function closeDoc(state: PanelState, doc: number, successor: number | null): PanelState {
  const panels = state.panels.map((d) => (d === doc ? null : d));
  if (state.layout === "single") panels[0] = successor;
  return { ...state, panels };
}

export function focus(state: PanelState, panel: number): PanelState {
  if (panel < 0 || panel >= state.panels.length) return state;
  return { ...state, focused: panel };
}

/** The persisted form: layout and dividers, never document ids, which only
 * mean something for the run that assigned them. */
export function encodePanels(state: PanelState): string {
  return JSON.stringify({ layout: state.layout, ratio: state.ratio });
}

export function decodePanels(text: string | null): Pick<PanelState, "layout" | "ratio"> {
  try {
    const v = JSON.parse(text ?? "") as { layout?: unknown; ratio?: { x?: unknown; y?: unknown } };
    const layout = typeof v.layout === "string" && v.layout in PANEL_COUNT ? (v.layout as Layout) : "single";
    const x = typeof v.ratio?.x === "number" ? clampRatio(v.ratio.x) : 0.5;
    const y = typeof v.ratio?.y === "number" ? clampRatio(v.ratio.y) : 0.5;
    return { layout, ratio: { x, y } };
  } catch {
    return { layout: "single", ratio: { x: 0.5, y: 0.5 } };
  }
}
