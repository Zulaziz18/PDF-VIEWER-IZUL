/**
 * Light or dark, following Windows.
 *
 * `tauri.conf.json` leaves the window's `theme` at `null`, which is "follow the
 * system", and WebView2 reports the Windows app mode through
 * `prefers-color-scheme`. The tokens switch on `data-theme` rather than on the
 * media query directly so that a stored preference (Phase 8) can override the
 * system by writing one attribute.
 */

export type Theme = "light" | "dark";

export function applySystemTheme(root: HTMLElement = document.documentElement): () => void {
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  const apply = (): void => {
    root.dataset["theme"] = query.matches ? "dark" : "light";
  };
  apply();
  query.addEventListener("change", apply);
  return () => query.removeEventListener("change", apply);
}
