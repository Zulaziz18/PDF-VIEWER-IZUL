/**
 * The home screen's state: where the navigation is pointing, what is listed
 * there, and which row is selected.
 */

import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { FolderEntry, KnownFolder, RecentFile } from "./homeModel";

export type HomeView =
  | { readonly kind: "recent" }
  | { readonly kind: "starred" }
  | { readonly kind: "pc" }
  | { readonly kind: "folder"; readonly path: string };

export interface HomeState {
  view: HomeView;
  recent: RecentFile[];
  known: KnownFolder[];
  entries: FolderEntry[];
  /** Path of the selected row; drives the file-information panel. */
  selected: string | null;
  loading: boolean;
  error: string | null;

  load(): Promise<void>;
  navigate(view: HomeView): Promise<void>;
  select(path: string | null): void;
  togglePin(path: string): Promise<void>;
}

export const useHome = create<HomeState>((set, get) => ({
  view: { kind: "recent" },
  recent: [],
  known: [],
  entries: [],
  selected: null,
  loading: false,
  error: null,

  async load() {
    const [recent, known] = await Promise.all([
      invoke<RecentFile[]>("recent_files").catch(() => [] as RecentFile[]),
      invoke<KnownFolder[]>("known_folders").catch(() => [] as KnownFolder[]),
    ]);
    set({ recent, known });
  },

  async navigate(view) {
    set({ view, selected: null, error: null, entries: [] });
    if (view.kind !== "folder") return;
    set({ loading: true });
    try {
      const entries = await invoke<FolderEntry[]>("browse_folder", { path: view.path });
      // The user may have clicked somewhere else while this folder was listed.
      if (get().view !== view) return;
      set({ entries, loading: false });
    } catch (e) {
      if (get().view !== view) return;
      set({ loading: false, error: String(e) });
    }
  },

  select(selected) {
    set({ selected });
  },

  async togglePin(path) {
    const file = get().recent.find((f) => f.path === path);
    if (!file) return;
    try {
      await invoke("pin_recent", { path, pinned: !file.pinned });
      const recent = await invoke<RecentFile[]>("recent_files");
      set({ recent });
    } catch {
      // Pinning is a convenience; a failure leaves the list as it was.
    }
  },
}));
