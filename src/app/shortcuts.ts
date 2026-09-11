/**
 * Keyboard shortcuts (SPEC 12).
 *
 * Only the ones Phase 1 can honour. They live in one place rather than on the
 * controls themselves so that Phase 8 can make them user-editable by changing
 * this table and nothing else, and so that the list in the manual is derived
 * from the same source as the behaviour.
 *
 * Scrolling keys — arrows, Page Up/Down, Home, End — are deliberately absent:
 * the viewport's scrolling element handles them natively, which is both
 * correct and what a screen reader expects.
 */

import { useEffect } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useDocument } from "@/state/documentStore";
import { viewport } from "./viewportHandle";

async function pickFile(): Promise<void> {
  const chosen = await openDialog({
    multiple: false,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (typeof chosen === "string") {
    await useDocument.getState().open(chosen);
  }
}

export function useShortcuts(): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const store = useDocument.getState();
      // Typing a page number into the toolbar must not trigger shortcuts.
      const target = e.target;
      const typing =
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        (target instanceof HTMLElement && target.isContentEditable);

      if (e.ctrlKey || e.metaKey) {
        switch (e.key) {
          case "o":
            e.preventDefault();
            void pickFile();
            return;
          case "w":
            e.preventDefault();
            void store.close();
            return;
          case "0":
            e.preventDefault();
            store.setZoomMode("actual");
            return;
          case "+":
          case "=":
            e.preventDefault();
            store.zoomIn();
            return;
          case "-":
            e.preventDefault();
            store.zoomOut();
            return;
          default:
            return;
        }
      }
      if (typing) return;
      if (e.key === "Escape") {
        (document.activeElement as HTMLElement | null)?.blur();
        return;
      }
      // Page-at-a-time navigation, which is what the page buttons do.
      if (e.key === "n" || e.key === "j") {
        const next = Math.min(store.page + 1, Math.max(0, store.pageCount - 1));
        store.setPage(next);
        viewport()?.goToPage(next);
      } else if (e.key === "p" || e.key === "k") {
        const prev = Math.max(0, store.page - 1);
        store.setPage(prev);
        viewport()?.goToPage(prev);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
