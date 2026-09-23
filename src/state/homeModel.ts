/**
 * The home screen's rules, as pure functions.
 *
 * Grouping, formatting and the "frequently used" list are decisions a user
 * sees on every start; keeping them out of the component is what lets them be
 * tested without a window (SPEC 0).
 *
 * Every timestamp here is **seconds** since the Unix epoch, the unit the store
 * writes (`izul-store` uses `as_secs()` throughout). `now` is milliseconds,
 * because that is what `Date.now()` returns; the conversion happens once, in
 * this file.
 */

export interface RecentFile {
  readonly path: string;
  readonly name: string;
  readonly last_opened: number | null;
  readonly pinned: boolean;
  readonly available: boolean;
  readonly cover: string | null;
  readonly size: number;
  readonly modified: number;
}

export interface FolderEntry {
  readonly name: string;
  readonly path: string;
  readonly is_dir: boolean;
  readonly size: number | null;
  readonly modified: number | null;
}

export interface KnownFolder {
  readonly kind: "desktop" | "documents" | "downloads" | "drive";
  readonly path: string;
}

export interface RecentGroup {
  readonly key: "last30" | "earlier";
  readonly files: readonly RecentFile[];
}

const DAY_SECONDS = 86_400;

/**
 * Splits the recent list into "the last 30 days" and "earlier", by when each
 * file was last *opened* — the grouping WPS uses, and the one that answers
 * "what was I working on". Empty groups are dropped. Order inside a group is
 * the order given, which the backend already sorts by recency.
 */
export function groupRecent(files: readonly RecentFile[], nowMs: number): RecentGroup[] {
  const cutoff = nowMs / 1000 - 30 * DAY_SECONDS;
  const last30 = files.filter((f) => (f.last_opened ?? 0) >= cutoff);
  const earlier = files.filter((f) => (f.last_opened ?? 0) < cutoff);
  const groups: RecentGroup[] = [];
  if (last30.length > 0) groups.push({ key: "last30", files: last30 });
  if (earlier.length > 0) groups.push({ key: "earlier", files: earlier });
  return groups;
}

/** The folder a path lives in, for either separator. */
export function parentFolder(path: string): string {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  if (cut <= 0) return path;
  const parent = path.slice(0, cut);
  // "C:" alone is not a folder anyone recognises; "C:\" is.
  return /^[A-Za-z]:$/.test(parent) ? `${parent}\\` : parent;
}

/** The last segment of a path — a folder's display name. */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const cut = Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/"));
  const name = cut >= 0 ? trimmed.slice(cut + 1) : trimmed;
  return name.length > 0 ? name : path;
}

/**
 * Folders the recent files come from, most used first.
 *
 * Counted over the recent list rather than stored: it is the recent list seen
 * from one level up, so it can never disagree with it.
 */
export function frequentFolders(
  files: readonly RecentFile[],
  limit = 4,
  exclude: readonly string[] = [],
): string[] {
  // Folders the navigation already names (Desktop, Documents…) would appear
  // twice, once under their local name and once under the file system's.
  const skip = new Set(exclude.map((p) => p.replace(/[\\/]+$/, "").toLowerCase()));
  const counts = new Map<string, { count: number; latest: number }>();
  for (const f of files) {
    const folder = parentFolder(f.path);
    if (skip.has(folder.toLowerCase())) continue;
    const seen = counts.get(folder) ?? { count: 0, latest: 0 };
    counts.set(folder, {
      count: seen.count + 1,
      latest: Math.max(seen.latest, f.last_opened ?? 0),
    });
  }
  return [...counts.entries()]
    .sort((a, b) => b[1].count - a[1].count || b[1].latest - a[1].latest)
    .slice(0, limit)
    .map(([folder]) => folder);
}

/** "16,5 MB", "482 KB" — Indonesian decimal comma, one decimal from MB up. */
export function formatBytes(bytes: number | null): string {
  if (bytes === null || !Number.isFinite(bytes) || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  const text = unit === 0 ? String(Math.round(value)) : value.toFixed(1).replace(".", ",");
  return `${text} ${units[unit] ?? ""}`;
}

/**
 * A date as the table shows it: `2026-09-20`, like WPS's own column.
 *
 * ISO order rather than a spelled-out month: it sorts by eye, it is the same
 * width on every row, and it cannot be misread as month-first.
 */
export function formatDate(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds <= 0) return "";
  const d = new Date(seconds * 1000);
  const pad = (n: number): string => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Date and time, for the file-information panel where there is room. */
export function formatDateTime(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds <= 0) return "";
  const d = new Date(seconds * 1000);
  const pad = (n: number): string => String(n).padStart(2, "0");
  return `${formatDate(seconds)} ${pad(d.getHours())}.${pad(d.getMinutes())}`;
}

/**
 * The folder column: the parent folder with the user's home folder shortened,
 * because every row starting with `C:\Users\<name>\` is noise.
 */
export function displayFolder(path: string, home: string | null): string {
  const folder = parentFolder(path);
  if (home && home.length > 0) {
    const h = home.replace(/[\\/]+$/, "");
    if (folder.toLowerCase() === h.toLowerCase()) return "~";
    if (folder.toLowerCase().startsWith(`${h.toLowerCase()}\\`) || folder.startsWith(`${h}/`)) {
      return `~${folder.slice(h.length)}`;
    }
  }
  return folder;
}

/**
 * A path that wraps at its separators instead of in the middle of a folder
 * name: a zero-width space after each `\\` or `/`.
 */
export function breakablePath(path: string): string {
  return path.replace(/([\\/])/g, "$1\u200b");
}
