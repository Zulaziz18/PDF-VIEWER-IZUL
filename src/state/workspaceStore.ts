/**
 * The window: which documents are open, which one is in front, and what the
 * inactive ones are allowed to keep (SPEC 10).
 *
 * One store per document lives in `documentSession`; this one holds the map of
 * them and the order they sit in. The split matters for memory as much as for
 * tidiness — a tab's *decisions* (zoom, position, outline) are kilobytes and
 * are kept for as long as the tab exists, while a tab's *bitmaps* are megabytes
 * and are given back the moment the tab stops being looked at.
 *
 * The backend keeps its own registry of the same tabs, because it is the one
 * that writes the session row. This store never invents a tab the backend has
 * not confirmed: every tab here exists because `open_document` returned.
 */

import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { forgetImages } from "@/annots/images";
import { createDocumentSession, type DocumentStore, type OpenedDoc } from "./documentSession";
import {
  SINGLE,
  assign,
  clampRatio,
  closeDoc,
  decodePanels,
  encodePanels,
  focus,
  setLayout as layoutFor,
  showDoc,
  type Layout,
  type PanelState,
} from "./panels";

/** One tab, as the tab bar draws it. */
export interface Tab {
  readonly doc: number;
  readonly path: string;
  readonly name: string;
  readonly pageCount: number;
}

/**
 * How many documents keep their full-resolution bitmaps.
 *
 * SPEC 10 asks an inactive tab to give its pixels back; it does not say how
 * many tabs to hold, and the honest answer is that holding exactly one is
 * wrong. A reader comparing two documents flips between them every few seconds,
 * and trimming on every flip would re-render both pages every time. Three is
 * the smallest number that makes a two-document comparison free, with one spare
 * for the tab the user just came from.
 */
export const WARM_TABS = 3;

/** A tab restored from the last run, before it is reopened. */
export interface RestorableTab {
  readonly path: string;
  readonly name: string;
  readonly pinned: boolean;
  readonly active: boolean;
  readonly available: boolean;
}

export interface WorkspaceState {
  tabs: Tab[];
  activeDoc: number | null;
  sessions: Map<number, DocumentStore>;
  /**
   * Documents in the order they were last looked at, newest first. This is what
   * decides which tabs stay warm, and it is deliberately recency rather than
   * tab position: the two tabs a user is comparing are rarely neighbours.
   */
  recent: number[];
  busy: boolean;
  error: string | null;
  /**
   * Whether the home screen is in front. WPS keeps a "Beranda" tab pinned at
   * the left of the tab strip; clicking it shows the recent files without
   * closing anything, and clicking a document tab goes back. With nothing
   * open, the home screen is all there is.
   */
  home: boolean;
  /** Split view (Phase 5): which document is in which panel. */
  panels: PanelState;
  /** Two panels side by side, scrolling together, differences marked. */
  compare: boolean;

  showHome(): void;
  setLayout(layout: Layout): void;
  /** A tab dropped onto a panel. */
  assignPanel(panel: number, doc: number): void;
  focusPanel(panel: number): void;
  setRatio(axis: "x" | "y", ratio: number): void;
  setCompare(on: boolean): void;
  /** Reads the remembered layout; called once at startup. */
  loadPanels(): Promise<void>;
  openFile(path: string): Promise<number | null>;
  closeTab(doc: number): Promise<void>;
  closeAll(): Promise<void>;
  activate(doc: number): Promise<void>;
  reorder(order: readonly number[]): Promise<void>;
  session(doc: number): DocumentStore | undefined;
  /** True when any open document has edits that closing would lose. */
  hasEdits(): boolean;
  /** The tabs with unsaved edits, in tab order. */
  dirtyTabs(): Tab[];
  /** A "save as" moved the tab to another file. */
  renameTab(doc: number, path: string): void;
  activeSession(): DocumentStore;
  restoreSession(): Promise<void>;
  openStartupFiles(): Promise<void>;
}

/**
 * The store components read when nothing is open.
 *
 * A real session with `doc: null`, not a bag of undefineds: every component
 * then reads one shape whether or not a document is open, and the empty state
 * is a state rather than a special case.
 */
const PLACEHOLDER: DocumentStore = createDocumentSession(null);

/**
 * Which documents should give their bitmaps back.
 *
 * Pure, and exported, because this is the rule the pass criterion in SPEC 17
 * is about — fifty documents open with memory under control — and a rule that
 * can only be observed by watching a memory graph is a rule nobody can check.
 */
export function tabsToTrim(recent: readonly number[], warm: number = WARM_TABS): number[] {
  return recent.slice(warm);
}

export const useWorkspace = create<WorkspaceState>((set, get) => ({
  tabs: [],
  activeDoc: null,
  sessions: new Map(),
  recent: [],
  busy: false,
  error: null,
  home: true,
  panels: SINGLE,
  compare: false,

  showHome() {
    set({ home: true });
  },

  setLayout(layout: Layout) {
    const { tabs, activeDoc, panels } = get();
    const next = layoutFor(panels, layout, tabs.map((t) => t.doc), activeDoc);
    // Comparing is two panels side by side; any other layout ends it.
    set({ panels: next, compare: get().compare && layout === "columns", home: false });
    savePanels(next);
    void trimCold(get().recent);
  },

  assignPanel(panel: number, doc: number) {
    if (!get().sessions.has(doc)) return;
    set({ panels: assign(get().panels, panel, doc), activeDoc: doc, home: false });
    void invoke("activate_document", { doc }).catch(() => undefined);
    void trimCold(get().recent);
  },

  focusPanel(panel: number) {
    const panels = focus(get().panels, panel);
    const doc = panels.panels[panels.focused] ?? null;
    // An empty panel takes the focus, and the next tab chosen goes into it;
    // the ribbon keeps acting on the document that was in front.
    if (doc === null) {
      set({ panels });
      return;
    }
    if (doc === get().activeDoc && panels.focused === get().panels.focused) return;
    const recent = [doc, ...get().recent.filter((d) => d !== doc)];
    set({ panels, activeDoc: doc, recent });
    void invoke("activate_document", { doc }).catch(() => undefined);
  },

  setRatio(axis: "x" | "y", ratio: number) {
    const panels = { ...get().panels, ratio: { ...get().panels.ratio, [axis]: clampRatio(ratio) } };
    set({ panels });
    savePanels(panels);
  },

  setCompare(on: boolean) {
    if (on && get().panels.layout !== "columns") get().setLayout("columns");
    set({ compare: on });
  },

  async loadPanels() {
    try {
      const saved = decodePanels(await invoke<string | null>("pref_get", { key: PANELS_PREF }));
      set({ panels: { ...get().panels, ratio: saved.ratio } });
      if (saved.layout !== "single") get().setLayout(saved.layout);
    } catch {
      // A first run, or a store that will not answer: one panel.
    }
  },

  session(doc: number) {
    return get().sessions.get(doc);
  },

  hasEdits() {
    return get().dirtyTabs().length > 0;
  },

  dirtyTabs() {
    // The backend's `dirty`, not `canUndo`: a saved document keeps its undo
    // history, and a document undone past its last save has changes again.
    return get().tabs.filter((tab) => get().sessions.get(tab.doc)?.getState().dirty === true);
  },

  renameTab(doc: number, path: string) {
    set({
      tabs: get().tabs.map((tab) =>
        tab.doc === doc ? { ...tab, path, name: path.split(/[\\/]/).pop() ?? path } : tab,
      ),
    });
  },

  activeSession() {
    const { activeDoc, sessions } = get();
    return (activeDoc !== null ? sessions.get(activeDoc) : undefined) ?? PLACEHOLDER;
  },

  async openFile(path: string) {
    // Already open: focus it rather than opening a second copy. Two tabs on one
    // file is a Phase 5 feature (split view); doing it by accident here would
    // only double the memory of a document the user thinks they opened once.
    const existing = get().tabs.find((t) => t.path === path);
    if (existing) {
      await get().activate(existing.doc);
      return existing.doc;
    }
    // A tab only exists once `open_document` has returned, so the check above
    // cannot see an open that is still in flight. Two calls for the same path
    // that overlap — session restore racing the startup files, or React's
    // development double-mount running the startup effect twice — would both
    // pass it and produce two tabs on one file. The in-flight map is the
    // missing half of the guard: the second caller waits for the first instead
    // of starting a second open.
    const running = opening.get(path);
    if (running) return await running;
    const attempt = openUnguarded(path, get, set);
    opening.set(path, attempt);
    try {
      return await attempt;
    } finally {
      opening.delete(path);
    }
  },

  async closeTab(doc: number) {
    const { tabs, sessions } = get();
    const index = tabs.findIndex((t) => t.doc === doc);
    if (index < 0) return;
    const nextSessions = new Map(sessions);
    nextSessions.delete(doc);
    const nextTabs = tabs.filter((t) => t.doc !== doc);
    // The neighbour that took its place, or the one before it when the last tab
    // closed — the same rule the backend registry follows, so the two cannot
    // disagree about which tab is in front.
    let successor =
      get().activeDoc === doc
        ? (nextTabs[index] ?? nextTabs[nextTabs.length - 1])?.doc ?? null
        : get().activeDoc;
    let panels = closeDoc(get().panels, doc, successor);
    if (panels.layout !== "single" && get().activeDoc === doc) {
      // In a split, the front moves to a document still on screen, not to a
      // tab the user did not put beside the others.
      const visible = panels.panels.findIndex((d) => d !== null);
      if (visible >= 0) {
        successor = panels.panels[visible] ?? null;
        panels = focus(panels, visible);
      } else if (successor !== null) {
        panels = showDoc(panels, successor);
      }
    }
    set({
      panels,
      tabs: nextTabs,
      sessions: nextSessions,
      activeDoc: successor,
      recent: get().recent.filter((d) => d !== doc),
      error: null,
      home: nextTabs.length === 0 ? true : get().home,
    });
    // The images of a closed document are megabytes the browser would otherwise
    // hold for the rest of the run.
    forgetImages(doc);
    try {
      await invoke("close_document", { doc });
    } catch (e) {
      // The tab is gone from the window either way; a backend that could not
      // close it cleanly is a log entry, not a dialog.
      console.warn("dokumen tidak dapat ditutup", e);
    }
  },

  async closeAll() {
    for (const tab of [...get().tabs]) {
      await get().closeTab(tab.doc);
    }
  },

  async activate(doc: number) {
    if (!get().sessions.has(doc)) return;
    const recent = [doc, ...get().recent.filter((d) => d !== doc)];
    set({ activeDoc: doc, recent, home: false, panels: showDoc(get().panels, doc) });
    try {
      await invoke("activate_document", { doc });
    } catch {
      // Losing the focus hint costs the session row one wrong `is_active`.
    }
    void trimCold(recent);
  },

  async reorder(order: readonly number[]) {
    const byId = new Map(get().tabs.map((t) => [t.doc, t]));
    const tabs = order.flatMap((d) => {
      const tab = byId.get(d);
      return tab ? [tab] : [];
    });
    for (const tab of get().tabs) {
      if (!order.includes(tab.doc)) tabs.push(tab);
    }
    set({ tabs });
    try {
      await invoke("reorder_tabs", { order: tabs.map((t) => t.doc) });
    } catch {
      // Order is cosmetic until the next run; a failed write is not worth a
      // dialog in front of a drag the user just finished.
    }
  },

  async restoreSession() {
    let tabs: RestorableTab[];
    try {
      tabs = await invoke<RestorableTab[]>("restore_session");
    } catch {
      // No stored arrangement, or a database that will not open: the window
      // comes up empty, which is what it did before sessions existed.
      return;
    }
    let activePath: string | null = null;
    for (const tab of tabs) {
      if (!tab.available) continue;
      const doc = await get().openFile(tab.path);
      if (doc !== null && tab.active) activePath = tab.path;
    }
    // Focus is applied last: opening each document takes focus as it goes, and
    // the tab that had it is rarely the one opened last.
    const wanted = get().tabs.find((t) => t.path === activePath);
    if (wanted) await get().activate(wanted.doc);
  },

  async openStartupFiles() {
    try {
      const files = await invoke<string[]>("startup_files");
      for (const file of files) {
        await get().openFile(file);
      }
    } catch {
      // Nothing on the command line is the normal case.
    }
  },
}));

/**
 * Asks the backend to drop the bitmaps of every tab that has gone cold.
 *
 * Deliberately fire-and-forget: trimming is an optimisation, and making the
 * user wait for one would be the opposite of the point.
 */
function trimCold(recent: readonly number[]): void {
  // A document on screen in any panel is being looked at, however long ago
  // its tab was clicked.
  const visible = new Set(
    useWorkspace.getState().panels.panels.filter((d): d is number => d !== null),
  );
  for (const doc of tabsToTrim(recent.filter((d) => !visible.has(d)), Math.max(0, WARM_TABS - visible.size))) {
    void invoke("trim_document", { doc }).catch(() => {
      // A tab that could not be trimmed keeps its bitmaps until it is closed.
    });
  }
}

/**
 * The open itself, with the duplicate guard already applied by `openFile`.
 *
 * Separate so the in-flight map holds exactly one promise per path: the guard
 * has to sit outside the work it guards, or it would be re-entered by its own
 * awaits.
 */
async function openUnguarded(
  path: string,
  get: () => WorkspaceState,
  set: (partial: Partial<WorkspaceState>) => void,
): Promise<number | null> {
  set({ busy: true, error: null });
  try {
    const opened = await invoke<OpenedDoc>("open_document", { path });
    const previous = get().activeSession().getState();
    const store = createDocumentSession(opened, {
      width: previous.viewportWidth,
      height: previous.viewportHeight,
    });
    const sessions = new Map(get().sessions);
    sessions.set(opened.doc, store);
    const tab: Tab = {
      doc: opened.doc,
      path: opened.path,
      name: opened.path.split(/[\\/]/).pop() ?? opened.path,
      pageCount: opened.page_count,
    };
    set({
      sessions,
      tabs: [...get().tabs, tab],
      activeDoc: opened.doc,
      panels: showDoc(get().panels, opened.doc),
      recent: [opened.doc, ...get().recent.filter((d) => d !== opened.doc)],
      busy: false,
      error: null,
      home: false,
    });
    void store.getState().loadOutline();
    void trimCold(get().recent);
    afterOpen?.(opened.doc, opened.path);
    return opened.doc;
  } catch (e) {
    set({ busy: false, error: String(e) });
    return null;
  }
}

const PANELS_PREF = "ui.panels";

function savePanels(panels: PanelState): void {
  void invoke("pref_set", { key: PANELS_PREF, value: encodePanels(panels) }).catch(() => undefined);
}

/** Opens in flight, by path. See `openFile`. */
const opening = new Map<string, Promise<number | null>>();

/**
 * Called once for every document that finishes opening, whoever opened it —
 * the picker, the home screen, a drop, the restored session. Phase 4 asks
 * about waiting drafts here (`fileActions.ts`); registered from outside so
 * this store does not import the dialogs that ask.
 */
let afterOpen: ((doc: number, path: string) => void) | null = null;

export function setAfterOpen(hook: ((doc: number, path: string) => void) | null): void {
  afterOpen = hook;
}
