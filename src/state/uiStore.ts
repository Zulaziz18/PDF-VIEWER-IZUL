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
export type RibbonTab = "home" | "edit" | "comment";

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
  setRibbon(tab: RibbonTab): void;
  armMarkup(kind: MarkupKind | null): void;
  setAboutOpen(open: boolean): void;
}

export const useUi = create<UiState>((set) => ({
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
}));
