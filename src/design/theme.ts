/**
 * Light or dark: following Windows, or as the user chose (Phase 8).
 *
 * `tauri.conf.json` leaves the window's `theme` at `null`, which is "follow the
 * system", and WebView2 reports the Windows app mode through
 * `prefers-color-scheme`. The tokens switch on `data-theme` rather than on the
 * media query directly, so a stored preference overrides the system by
 * writing one attribute.
 */

export type Theme = "light" | "dark";
export type ThemePref = "system" | Theme;

export const THEME_PREFS: readonly ThemePref[] = ["system", "light", "dark"];

/** The theme a preference comes to, given what the system says. */
export function resolveTheme(pref: ThemePref, systemDark: boolean): Theme {
  if (pref === "system") return systemDark ? "dark" : "light";
  return pref;
}

let stop: (() => void) | null = null;
const listeners = new Set<(theme: Theme) => void>();

/** Called with the theme in force whenever it changes. */
export function onThemeChange(listener: (theme: Theme) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The theme in force now. */
export function currentTheme(root: HTMLElement = document.documentElement): Theme {
  return root.dataset["theme"] === "dark" ? "dark" : "light";
}

/** Applies `pref`, following the system while it is "system". */
export function applyTheme(pref: ThemePref, root: HTMLElement = document.documentElement): void {
  stop?.();
  stop = null;
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  const apply = (): void => {
    const theme = resolveTheme(pref, query.matches);
    root.dataset["theme"] = theme;
    for (const l of listeners) l(theme);
  };
  apply();
  if (pref === "system") {
    query.addEventListener("change", apply);
    stop = () => query.removeEventListener("change", apply);
  }
}

/** Following Windows, as the application starts before the preference is read. */
export function applySystemTheme(root: HTMLElement = document.documentElement): () => void {
  applyTheme("system", root);
  return () => stop?.();
}
