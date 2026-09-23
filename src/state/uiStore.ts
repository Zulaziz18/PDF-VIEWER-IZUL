/**
 * Window-level interface state that belongs to no document: which ribbon tab
 * is showing, and which text-markup tool is armed.
 *
 * Per-document state (zoom, tool, selection) stays in `documentSession`; this
 * is the small remainder that is the same whichever tab is in front — a user
 * who switched to the "Komentar" ribbon to mark up one document expects it to
 * still be there when they look at the next.
 */

import { create } from "zustand";

/** The ribbon tabs that exist. A tab joins this list in the phase that makes
 * every button on it work (SPEC 0: no dead controls). */
export type RibbonTab = "home" | "edit" | "pages" | "comment" | "convert";

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

export type MarkupKind = "Highlight" | "Underline" | "StrikeOut";

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
}));
