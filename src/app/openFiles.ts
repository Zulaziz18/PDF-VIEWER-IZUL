/**
 * Files handed over by a second launch of the application (SPEC 11.3).
 *
 * With the file association installed, double-clicking a PDF starts the
 * executable again. The single-instance plugin stops that second process from
 * becoming a second window — it passes its command line to the instance already
 * running and exits — and the running instance receives it here.
 *
 * Without this, every double-clicked PDF would open a window with its own pool
 * of eight worker processes, its own tile cache and its own session row, and
 * the tab strip would never fill up no matter how many files the user opened.
 */

import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useWorkspace } from "@/state/workspaceStore";

/** The event the backend emits when another launch hands over its arguments. */
export const OPEN_FILES_EVENT = "izul://open-files";

export function useOpenFilesFromOtherInstance(): void {
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen<string[]>(OPEN_FILES_EVENT, (event) => {
      // Sequential, not `Promise.all`: opening two documents at once would race
      // for which one ends up focused, and the user expects the last file they
      // double-clicked to be the one in front.
      void (async () => {
        for (const path of event.payload) {
          await useWorkspace.getState().openFile(path);
        }
      })();
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // No Tauri host — the browser dev server. Nothing hands files over
        // there, so there is nothing to listen for.
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
}
