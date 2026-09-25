/**
 * Window-level interface state that belongs to no document: which ribbon tab
 * is showing, and which text-markup tool is armed.
 *
 * Per-document state (zoom, tool, selection) stays in `documentSession`; this
 * is the small remainder that is the same whichever tab is in front — a user
 * who switched to the "Komentar" ribbon to mark up one document expects it to
 * still be there when they look at the next.
 */

import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { applyTheme, THEME_PREFS, type ThemePref } from "@/design/theme";
import type { PdfRect } from "@/annots/types";

/** Text selected on a page, as the "Edit Teks" dialog opens with it. */
export interface TextSelection {
  readonly page: number;
  /** Display space: the union of the selection's boxes. */
  readonly rect: PdfRect;
  readonly text: string;
}

/** The ribbon tabs that exist. A tab joins this list in the phase that makes
 * every button on it work (SPEC 0: no dead controls). */
export type RibbonTab = "home" | "edit" | "pages" | "comment" | "protect" | "convert";

/** One button of a {@link Prompt}. */
export interface PromptButton {
  readonly id: string;
  readonly label: string;
  /** `primary` is the default action (Enter); `danger` throws work away. */
  readonly kind?: "primary" | "danger" | "default";
}

/**
 * A question the application has to ask before it can go on — save before
 * closing, restore a draft. In-app rather than the platform's message box
 * because those offer two buttons, and "save / don't save / cancel" is three.
 */
export interface Prompt {
  readonly title: string;
  readonly body: string;
  readonly detail?: string;
  readonly icon?: "warning" | "draft" | "info";
  readonly buttons: readonly PromptButton[];
  /** What Escape and the close button answer. */
  readonly cancelId: string;
}

/** A line at the bottom corner that says what just happened. */
export interface Notice {
  readonly kind: "ok" | "error";
  readonly text: string;
  readonly detail?: string;
}

export type ExportKind = "pages" | "images" | "split";

/** What a text selection can be turned into — the three markups, and a
 * redaction mark over the selected text (Phase 6). */
export type MarkupKind = "Highlight" | "Underline" | "StrikeOut" | "Redact";

export interface UiState {
  ribbon: RibbonTab;
  /**
   * A text-markup tool waiting for a selection. Pressing "Stabilo" with no
   * text selected arms it instead of doing nothing, and every selection made
   * while it is armed is marked up — the way the markup tools behave in every
   * editor a user is likely to have used before.
   */
  markup: MarkupKind | null;
  aboutOpen: boolean;
  /** The question showing, and who is waiting for its answer. */
  prompt: { readonly spec: Prompt; readonly resolve: (id: string) => void } | null;
  notice: Notice | null;
  /** The export dialog, when one is open. */
  exporting: ExportKind | null;
  /** The apply-redaction dialog is open (Phase 6). */
  redacting: boolean;
  /** The "Kenali Teks (OCR)" dialog is open (Phase 7). */
  ocring: boolean;
  /** The "Edit Teks" dialog, with the text it was opened on (Phase 7). */
  editingText: TextSelection | null;
  /** Phase 8: the command palette (Ctrl+Shift+P) and the shortcut list (F1). */
  paletteOpen: boolean;
  shortcutsOpen: boolean;
  /** The print dialog. */
  printing: boolean;
  /** Presentation mode (F5): the page alone, full screen. */
  presenting: boolean;
  /** Focus mode (F11): the chrome folded away, the document stays. */
  focusMode: boolean;
  /** Light, dark, or as Windows is — stored (`ui.theme`). */
  themePref: ThemePref;
  /** Pages drawn inverted in dark mode, pictures left as they are (`ui.invert`). */
  invertPages: boolean;
  /** Whether they are, now: asked for, and the theme is dark. */
  pagesInverted: boolean;
  setRibbon(tab: RibbonTab): void;
  armMarkup(kind: MarkupKind | null): void;
  setAboutOpen(open: boolean): void;
  /**
   * Shows a prompt and resolves with the id of the button pressed. A second
   * prompt while one is showing answers the first with its cancel id: two
   * questions stacked would leave one of them unanswerable.
   */
  ask(spec: Prompt): Promise<string>;
  answer(id: string): void;
  notify(notice: Notice | null): void;
  setExporting(kind: ExportKind | null): void;
  setRedacting(open: boolean): void;
  setOcring(open: boolean): void;
  setEditingText(sel: TextSelection | null): void;
  setPaletteOpen(open: boolean): void;
  setShortcutsOpen(open: boolean): void;
  setPrinting(open: boolean): void;
  setPresenting(on: boolean): void;
  setFocusMode(on: boolean): void;
  setThemePref(pref: ThemePref): void;
  cycleTheme(): void;
  setInvertPages(on: boolean): void;
  /** Reads the stored preferences; called once at startup. */
  loadPrefs(): Promise<void>;
}

function store(key: string, value: string): void {
  void invoke("pref_set", { key, value }).catch(() => undefined);
}

export const useUi = create<UiState>((set, get) => ({
  ribbon: "home",
  markup: null,
  aboutOpen: false,
  setRibbon(ribbon) {
    set({ ribbon });
  },
  armMarkup(markup) {
    set({ markup });
  },
  setAboutOpen(aboutOpen) {
    set({ aboutOpen });
  },
  prompt: null,
  notice: null,
  exporting: null,
  redacting: false,
  ocring: false,
  editingText: null,
  paletteOpen: false,
  shortcutsOpen: false,
  printing: false,
  presenting: false,
  focusMode: false,
  themePref: "system",
  invertPages: false,
  pagesInverted: false,
  ask(spec) {
    const previous = get().prompt;
    if (previous) previous.resolve(previous.spec.cancelId);
    return new Promise<string>((resolve) => {
      set({ prompt: { spec, resolve } });
    });
  },
  answer(id) {
    const current = get().prompt;
    if (!current) return;
    set({ prompt: null });
    current.resolve(id);
  },
  notify(notice) {
    set({ notice });
  },
  setExporting(exporting) {
    set({ exporting });
  },
  setRedacting(redacting) {
    set({ redacting });
  },
  setOcring(ocring) {
    set({ ocring });
  },
  setEditingText(editingText) {
    set({ editingText });
  },
  setPaletteOpen(paletteOpen) {
    set({ paletteOpen });
  },
  setShortcutsOpen(shortcutsOpen) {
    set({ shortcutsOpen });
  },
  setPrinting(printing) {
    set({ printing });
  },
  setPresenting(presenting) {
    set({ presenting });
  },
  setFocusMode(focusMode) {
    set({ focusMode });
  },
  setThemePref(themePref) {
    set({ themePref });
    applyTheme(themePref);
    store("ui.theme", themePref);
  },
  cycleTheme() {
    const at = THEME_PREFS.indexOf(get().themePref);
    get().setThemePref(THEME_PREFS[(at + 1) % THEME_PREFS.length] ?? "system");
  },
  setInvertPages(invertPages) {
    set({ invertPages });
    store("ui.invert", invertPages ? "1" : "0");
  },
  async loadPrefs() {
    const read = (key: string) => invoke<string | null>("pref_get", { key }).catch(() => null);
    const [theme, invert] = await Promise.all([read("ui.theme"), read("ui.invert")]);
    const pref = THEME_PREFS.find((p) => p === theme);
    if (pref !== undefined && pref !== get().themePref) {
      set({ themePref: pref });
      applyTheme(pref);
    }
    if (invert === "1") set({ invertPages: true });
  },
}));
