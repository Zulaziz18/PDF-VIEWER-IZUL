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
import { useWorkspace } from "@/state/workspaceStore";
import { viewport } from "./viewportHandle";

async function pickFile(): Promise<void> {
  const chosen = await openDialog({
    multiple: false,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (typeof chosen === "string") {
    await useWorkspace.getState().openFile(chosen);
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
          case "w": {
            e.preventDefault();
            // Closes the tab, not the window: with tabs, Ctrl+W meaning "quit"
            // would throw away every other document the user has open.
            const active = useWorkspace.getState().activeDoc;
            if (active !== null) void useWorkspace.getState().closeTab(active);
            return;
          }
          case "f":
            e.preventDefault();
            store.toggleSearch(true);
            return;
          case "Tab": {
            e.preventDefault();
            const workspace = useWorkspace.getState();
            const tabs = workspace.tabs;
            if (tabs.length < 2) return;
            const at = tabs.findIndex((tab) => tab.doc === workspace.activeDoc);
            const step = e.shiftKey ? -1 : 1;
            const next = tabs[(((at + step) % tabs.length) + tabs.length) % tabs.length];
            if (next) void workspace.activate(next.doc);
            return;
          }
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
        if (store.search.open) {
          store.toggleSearch(false);
          return;
        }
        (document.activeElement as HTMLElement | null)?.blur();
        return;
      }
      // F3 is what a Windows reader reaches for to step through matches, and
      // it works whether or not the search box has focus.
      if (e.key === "F3") {
        e.preventDefault();
        store.gotoResult(e.shiftKey ? -1 : 1);
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
