/**
 * User bookmarks (SPEC 11.1, Phase 8): kept per file in the application's
 * database, never written into the PDF (`src-tauri/src/bookmarks.rs`).
 */

import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { useDocument } from "@/state/documentStore";
import { t } from "@/i18n";

export interface Bookmark {
  readonly id: number;
  readonly page: number;
  readonly label: string;
  readonly created_at: number;
}

interface BookmarksState {
  /** The document the list belongs to. */
  doc: number | null;
  list: readonly Bookmark[];
  error: string | null;
  load(doc: number): Promise<void>;
  add(doc: number, page: number, label: string): Promise<void>;
  rename(doc: number, id: number, label: string): Promise<void>;
  remove(doc: number, id: number): Promise<void>;
}

export const useBookmarks = create<BookmarksState>((set) => {
  const apply = async (doc: number, call: Promise<Bookmark[]>): Promise<void> => {
    try {
      set({ doc, list: await call, error: null });
    } catch (e) {
      set({ error: String(e) });
    }
  };
  return {
    doc: null,
    list: [],
    error: null,
    load: (doc) => apply(doc, invoke<Bookmark[]>("bookmarks_list", { doc })),
    add: (doc, page, label) => apply(doc, invoke<Bookmark[]>("bookmark_add", { doc, page, label })),
    rename: (doc, id, label) => apply(doc, invoke<Bookmark[]>("bookmark_rename", { doc, id, label })),
    remove: (doc, id) => apply(doc, invoke<Bookmark[]>("bookmark_remove", { doc, id })),
  };
});

/** The name a new bookmark gets: its page, which the user can rename. */
export function defaultLabel(page: number): string {
  return `${t("status.page")} ${page + 1}`;
}

/** Ctrl+B: a bookmark on the page being read, and the panel that shows it. */
export async function bookmarkHere(): Promise<void> {
  const s = useDocument.getState();
  if (s.doc === null) return;
  await useBookmarks.getState().add(s.doc, s.page, defaultLabel(s.page));
  s.setSidebarTab("bookmarks");
}
