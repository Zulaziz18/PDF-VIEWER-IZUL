/**
 * The mocked backend for the UI screenshot harness.
 *
 * Loaded only by `tools/ui-harness/index.html`, which only the Vite dev server
 * serves. The production build's single entry is the root `index.html`, so
 * nothing in this folder can reach a shipped binary — the build would have to
 * be pointed here on purpose.
 *
 * Canned answers for everything that is state, and a bridge to
 * `izul-bench`'s `ui-harness` process (through Playwright's `exposeFunction`)
 * for everything that must be real: page sizes, text boxes, outlines and the
 * annotation display lists. Tile pixels do not pass through here at all —
 * Playwright answers the `http://izul.localhost/tile/...` fetches directly.
 */

import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

export interface HarnessDoc {
  readonly doc: number;
  /** What the application shows: a Windows-looking path. */
  readonly path: string;
  readonly page_count: number;
  readonly page_sizes: Array<[number, number]>;
}

export interface HarnessRecent {
  readonly path: string;
  readonly name: string;
  readonly last_opened: number;
  readonly modified: number | null;
  readonly size: number | null;
  readonly page_count: number | null;
  readonly pinned: boolean;
  readonly available: boolean;
  readonly cover: string | null;
}

export interface HarnessData {
  readonly version: Record<string, string>;
  readonly docs: HarnessDoc[];
  readonly recent: HarnessRecent[];
  /** Paths to reopen as the restored session; empty for the home scene. */
  readonly session: string[];
  readonly active: string | null;
  readonly folders: Record<string, unknown>;
  /** Phase 4 states a scene wants to show. */
  readonly flags: HarnessFlags;
  readonly exports: unknown[];
  /** Seconds since the epoch, as the frozen clock reads it. */
  readonly now: number;
}

export interface HarnessFlags {
  /** Every open document has unsaved edits. */
  readonly dirty?: boolean;
  /** The first document has a draft waiting. */
  readonly draft?: boolean;
  /** Another program changed the files on disk. */
  readonly diskChanged?: boolean;
  /** The export history has rows. */
  readonly exports?: boolean;
}

declare global {
  interface Window {
    __HARNESS__?: HarnessData;
    __harnessBackend?: (cmd: string, args: unknown) => Promise<unknown>;
  }
}

export function installMocks(data: HarnessData): void {
  mockWindows("main");
  const byPath = new Map(data.docs.map((d) => [d.path, d]));
  const sizeOf = new Map(data.recent.map((r) => [r.path, r.size ?? 0]));
  const pathOf = new Map(data.docs.map((d) => [d.doc, d.path]));
  const flags = data.flags;

  mockIPC(
    async (cmd, raw) => {
      const args = (raw ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "app_version":
          return data.version;
        case "pool_health":
          return {
            pool_size: 8,
            live_workers: 8,
            quarantined_documents: 0,
            sandbox_is_security_boundary: true,
            render: {
              cache: { hits: 180, misses: 24, entries: 42, bytes: 96 << 20, budget_bytes: 2048 << 20 },
              queued: 0,
              inflight: 0,
              rendered: 204,
            },
          };
        case "recent_files":
          return data.recent;
        case "restore_session":
          return data.session.map((path) => ({
            path,
            name: path.split(/[\\/]/).pop(),
            pinned: false,
            active: path === data.active,
            available: true,
          }));
        case "startup_files":
          return [];
        case "open_document": {
          const found = byPath.get(String(args["path"]));
          if (!found) throw new Error("berkas tidak ditemukan");
          return { ...found, permissions: 0xffffffff, encrypted: false, restored: null };
        }
        case "annot_history":
          return [false, false];
        case "index_progress":
          return { pages_done: 0, page_count: 0, running: false, error: null };
        case "search_document":
        case "search_library":
        case "search_page":
        case "search_regex_page":
          return [];
        case "known_folders":
        case "browse_folder":
          return data.folders[cmd === "known_folders" ? "known" : String(args["path"])] ?? [];
        case "document_outline":
        case "page_text":
        case "annot_list":
        case "annot_display_lists":
          return window.__harnessBackend ? window.__harnessBackend(cmd, args) : [];
        case "file_status":
          return {
            changed_on_disk: flags.diskChanged === true,
            missing: false,
            dirty: flags.dirty === true,
            size: sizeOf.get(pathOf.get(Number(args["doc"])) ?? "") ?? 0,
          };
        case "draft_status":
          return flags.draft === true && args["doc"] === 1
            ? { updated_at: data.now - 47 * 60, objects: 4, matches_file: true }
            : null;
        case "export_history":
          return flags.exports === true ? data.exports : [];
        case "plugin:window|is_maximized":
        case "plugin:window|is_fullscreen":
          return false;
        default:
          // Everything else is a notification the backend would act on and
          // answer with nothing: activate, reorder, set_generation, trim…
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
