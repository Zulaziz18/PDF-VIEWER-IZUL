/**
 * The shortcuts as the user has them (Phase 8): the registry's defaults with
 * the user's changes, stored in SQLite under `ui.shortcuts` — only what
 * differs from the defaults, so a default changed in a later version reaches
 * everyone who never touched it.
 */

import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import { COMMANDS } from "@/app/commands";
import { byCombo, decodeOverrides, display, effective, encodeOverrides, normalize } from "./keymap";

const PREF = "ui.shortcuts";

const DEFAULTS: ReadonlyMap<string, readonly string[]> = new Map(
  COMMANDS.map((c) => [c.id, c.keys.map((k) => normalize(k)).filter((k): k is string => k !== null)]),
);
const KNOWN: ReadonlySet<string> = new Set(COMMANDS.map((c) => c.id));

function same(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((k, i) => k === b[i]);
}

interface KeymapState {
  overrides: Record<string, string[]>;
  /** Command → keys, as they apply now. */
  keys: Map<string, string[]>;
  /** Combination → command. */
  lookup: Map<string, string>;
  /** A command waiting for its new key in the F1 dialog: shortcuts pause. */
  recording: string | null;
  load(): Promise<void>;
  setKeys(id: string, keys: readonly string[]): void;
  reset(id: string): void;
  resetAll(): void;
  setRecording(id: string | null): void;
}

function derive(overrides: Record<string, string[]>): Pick<KeymapState, "overrides" | "keys" | "lookup"> {
  const keys = effective(DEFAULTS, overrides);
  return { overrides, keys, lookup: byCombo(keys) };
}

function persist(overrides: Record<string, string[]>): void {
  void invoke("pref_set", { key: PREF, value: encodeOverrides(overrides) }).catch(() => undefined);
}

export const useKeymap = create<KeymapState>((set, get) => ({
  ...derive({}),
  recording: null,
  async load() {
    const raw = await invoke<string | null>("pref_get", { key: PREF }).catch(() => null);
    set(derive(decodeOverrides(raw, KNOWN)));
  },
  setKeys(id, keys) {
    const clean = [...new Set(keys.map((k) => normalize(k)).filter((k): k is string => k !== null))];
    // A key moved here is taken from whoever had it: one key, one command.
    const overrides: Record<string, string[]> = { ...get().overrides };
    for (const [other, has] of get().keys) {
      if (other === id) continue;
      const kept = has.filter((k) => !clean.includes(k));
      if (kept.length !== has.length) overrides[other] = kept;
    }
    overrides[id] = clean;
    for (const [cid, ks] of Object.entries(overrides)) {
      if (same(ks, DEFAULTS.get(cid) ?? [])) delete overrides[cid];
    }
    set(derive(overrides));
    persist(overrides);
  },
  reset(id) {
    const overrides = { ...get().overrides };
    delete overrides[id];
    set(derive(overrides));
    persist(overrides);
  },
  resetAll() {
    set(derive({}));
    persist({});
  },
  setRecording(recording) {
    set({ recording });
  },
}));

export function defaultKeys(id: string): readonly string[] {
  return DEFAULTS.get(id) ?? [];
}

/** "Label (Ctrl+S)" with the key the user has for `id` now, or the label alone. */
export function withKeys(label: string, id: string): string {
  const first = useKeymap.getState().keys.get(id)?.[0];
  return first === undefined ? label : `${label} (${display(first)})`;
}

/** The first key the user has for `id`, as it reads on screen. */
export function keyOf(id: string): string | undefined {
  const first = useKeymap.getState().keys.get(id)?.[0];
  return first === undefined ? undefined : display(first);
}
