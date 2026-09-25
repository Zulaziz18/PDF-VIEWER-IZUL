/**
 * Presentation mode (F5) and focus mode (F11), SPEC 11.1.
 *
 * Presentation is the document alone, full screen, on black: one page fitted
 * to the screen at a time, turned with the keys every presenter's clicker
 * sends (Page Down, Right, Space, Enter; Page Up, Left, Backspace), Home and
 * End, and Escape to leave. The page's own view — mode and zoom — is put back
 * on the way out, so presenting does not change how the document reads.
 *
 * Focus mode folds the chrome away and keeps the reader's view as it is: the
 * page, the scroll, nothing else. F11 again (or Escape) brings it back.
 *
 * `step` is pure, for the tests.
 */

import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useDocument } from "@/state/documentStore";
import { useUi } from "@/state/uiStore";
import type { ViewMode } from "@/viewport/layout";
import type { ZoomMode } from "@/state/documentSession";
import { goToPage } from "./actions";
import { allViewports, viewport } from "./viewportHandle";
import { setCanvasInversion } from "@/annots/canvas";
import { currentTheme, onThemeChange, type Theme } from "@/design/theme";
import { setPageInversion } from "@/viewport/tileSource";

/** The page a presentation key turns to, or `null` when it is not one. */
export function step(key: string, shift: boolean, page: number, count: number): number | null {
  const last = Math.max(0, count - 1);
  switch (key) {
    case "PageDown":
    case "ArrowRight":
    case "ArrowDown":
    case "Enter":
    case "n":
      return Math.min(page + 1, last);
    case " ":
      return shift ? Math.max(page - 1, 0) : Math.min(page + 1, last);
    case "PageUp":
    case "ArrowLeft":
    case "ArrowUp":
    case "Backspace":
    case "p":
      return Math.max(page - 1, 0);
    case "Home":
      return 0;
    case "End":
      return last;
    default:
      return null;
  }
}

function fullscreen(on: boolean): void {
  // Outside Tauri (the screenshot harness) there is no window to change.
  try {
    void getCurrentWindow()
      .setFullscreen(on)
      .catch(() => undefined);
  } catch {
    // Nothing to do.
  }
}

export function usePresentation(): void {
  const presenting = useUi((s) => s.presenting);
  const focusMode = useUi((s) => s.focusMode);
  const doc = useDocument((s) => s.doc);

  // Leaving the document leaves the presentation.
  useEffect(() => {
    if (doc === null && useUi.getState().presenting) useUi.getState().setPresenting(false);
  }, [doc]);

  useEffect(() => {
    if (!presenting) return;
    const store = useDocument.getState();
    const before: { viewMode: ViewMode; zoomMode: ZoomMode; zoom: number; page: number } = {
      viewMode: store.viewMode,
      zoomMode: store.zoomMode,
      zoom: store.zoom,
      page: store.page,
    };
    store.setViewMode("single");
    store.setZoomMode("fitPage");
    fullscreen(true);
    const solo = (page: number): void => viewport()?.setSoloPage(page);
    solo(before.page);
    // Once the layout has the new zoom, the current page is the one shown —
    // unless a key has already turned it.
    let turned = false;
    const settle = window.setTimeout(() => {
      if (!turned) goToPage(before.page);
    }, 60);
    // Each page fitted on its own: a portrait page after a landscape cover
    // must not keep the cover's zoom and run off the screen.
    let pending = 0;
    const turn = (page: number): void => {
      turned = true;
      const s = useDocument.getState();
      s.setPage(page);
      s.setZoomMode("fitPage");
      solo(page);
      window.clearTimeout(pending);
      pending = window.setTimeout(() => goToPage(page), 30);
    };

    let wheelAt = 0;
    const onKey = (e: KeyboardEvent): void => {
      const s = useDocument.getState();
      const next = step(e.key, e.shiftKey, s.page, s.pageCount);
      if (next === null) return;
      e.preventDefault();
      e.stopPropagation();
      turn(next);
    };
    // A wheel notch is a page, not a scroll that would show two halves.
    const onWheel = (e: WheelEvent): void => {
      e.preventDefault();
      const now = performance.now();
      if (now - wheelAt < 250 || Math.abs(e.deltaY) < 4) return;
      wheelAt = now;
      const s = useDocument.getState();
      turn(Math.min(Math.max(s.page + (e.deltaY > 0 ? 1 : -1), 0), Math.max(0, s.pageCount - 1)));
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("wheel", onWheel, { passive: false, capture: true });
    return () => {
      window.clearTimeout(settle);
      window.clearTimeout(pending);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("wheel", onWheel, { capture: true });
      fullscreen(false);
      viewport()?.setSoloPage(null);
      const s = useDocument.getState();
      const page = s.page;
      s.setViewMode(before.viewMode);
      if (before.zoomMode === "custom") s.setZoom(before.zoom, "custom");
      else s.setZoomMode(before.zoomMode);
      window.setTimeout(() => goToPage(page), 60);
    };
  }, [presenting]);

  useEffect(() => {
    if (!focusMode) return;
    fullscreen(true);
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape" && !document.querySelector("dialog[open]")) {
        e.preventDefault();
        useUi.getState().setFocusMode(false);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      fullscreen(false);
    };
  }, [focusMode]);
}

/**
 * Dark mode's page inversion (Phase 8): on while the theme is dark and the
 * user has asked for it. Changing it redraws every viewport — the tiles of
 * the other kind are simply other tiles.
 */
export function usePageInversion(): void {
  const wanted = useUi((s) => s.invertPages);
  useEffect(() => {
    const apply = (theme: Theme): void => {
      const on = wanted && theme === "dark";
      setCanvasInversion(on);
      if (setPageInversion(on)) for (const v of allViewports()) v.requestFrame();
      // The page panel's thumbnails are fetched again under the new key.
      if (useUi.getState().pagesInverted !== on) useUi.setState({ pagesInverted: on });
    };
    apply(currentTheme());
    return onThemeChange(apply);
  }, [wanted]);
}
