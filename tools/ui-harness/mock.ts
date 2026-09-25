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
  /** The OCR models are not installed. */
  readonly noOcr?: boolean;
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

  // Phase 5: a page map per document, applied the way `izul_model::pages`
  // does — enough for the screenshots, not a second implementation to test.
  interface View {
    render_doc: number | null;
    page: number;
    rotation: number;
    width: number;
    height: number;
    source: number;
  }
  const maps = new Map<number, View[]>();
  let revision = 0;
  const viewOf = (doc: number): View[] =>
    maps.get(doc) ??
    (data.docs.find((d) => d.doc === doc)?.page_sizes ?? []).map(([width, height], page) => ({
      render_doc: doc,
      page,
      rotation: 0,
      width,
      height,
      source: 0,
    }));
  const pagesReply = (doc: number) => ({
    view: { pages: viewOf(doc), mapped: maps.has(doc), map_revision: revision, sources: [] },
    edit: { objects: [], can_undo: true, can_redo: false, dirty: true, map_revision: revision },
  });

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
        case "redact_preview":
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
        case "pages_state":
          return pagesReply(Number(args["doc"])).view;
        case "pages_apply": {
          const doc = Number(args["doc"]);
          const cmd = args["cmd"] as { kind: string; pages?: number[]; at?: number; count?: number; width?: number; height?: number; quarters?: number; before?: number };
          const pages = [...viewOf(doc)];
          const sel = [...new Set(cmd.pages ?? [])].sort((a, b) => a - b);
          if (cmd.kind === "insertBlank") {
            const blank = { render_doc: null, page: 0, rotation: 0, width: cmd.width ?? 595, height: cmd.height ?? 842, source: 0 };
            pages.splice(cmd.at ?? pages.length, 0, ...Array.from({ length: cmd.count ?? 1 }, () => ({ ...blank })));
          } else if (cmd.kind === "delete") {
            for (const p of [...sel].reverse()) pages.splice(p, 1);
          } else if (cmd.kind === "duplicate") {
            const copies = sel.map((p) => ({ ...(pages[p] as View) }));
            pages.splice((sel[sel.length - 1] ?? 0) + 1, 0, ...copies);
          } else if (cmd.kind === "rotate") {
            for (const p of sel) {
              const v = pages[p] as View;
              pages[p] = { ...v, rotation: (((v.rotation + (cmd.quarters ?? 1)) % 4) + 4) % 4 };
            }
          } else if (cmd.kind === "move") {
            const moved = sel.map((p) => pages[p] as View);
            const rest = pages.filter((_, i) => !sel.includes(i));
            const at = rest.filter((_, i) => i < (cmd.before ?? 0) - sel.filter((p) => p < (cmd.before ?? 0)).length).length;
            rest.splice(at, 0, ...moved);
            pages.splice(0, pages.length, ...rest);
          }
          maps.set(doc, pages);
          revision += 1;
          return pagesReply(doc);
        }
        case "pref_get":
          return null;
        case "compare_visual":
          throw new Error("tidak tersedia di harness");
        case "plugin:window|is_maximized":
        case "plugin:window|is_fullscreen":
          return false;
        case "ocr_available":
          return flags.noOcr !== true;
        case "ocr_progress":
          return { running: true, done: 2, total: 5 };
        case "ocr_apply":
          // The "ocrrunning" scene photographs the dialog mid-run.
          return new Promise(() => {});
        default:
          // Everything else is a notification the backend would act on and
          // answer with nothing: activate, reorder, set_generation, trim…
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
