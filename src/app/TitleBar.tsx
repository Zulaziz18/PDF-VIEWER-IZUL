/**
 * Custom title bar (SPEC 12).
 *
 * The window is undecorated so the title bar and the tab strip read as one
 * surface. That decision comes with an obligation this file did not meet until
 * a user pointed it out: an undecorated window has **no minimise, maximise or
 * close buttons of its own**, so if the title bar does not draw them, the only
 * way out of the application is Alt+F4. The controls below are that obligation.
 *
 * They also need permissions. Tauri v2 refuses a window command the capability
 * file has not granted, *silently* — the same trap that made the Open button do
 * nothing in Phase 1 (see CLAUDE.md). `core:window:allow-minimize`,
 * `allow-toggle-maximize`, `allow-close` and `allow-start-dragging` are in
 * `src-tauri/capabilities/default.json` for exactly this reason; the last one is
 * what makes `data-tauri-drag-region` able to move the window at all.
 */

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { confirm } from "@tauri-apps/plugin-dialog";
import { useDocument } from "@/state/documentStore";
import { useWorkspace } from "@/state/workspaceStore";
import { t } from "@/i18n";

interface VersionInfo {
  name: string;
  codename: string;
  version: string;
  phase: string;
  phaseName: string;
  pdfiumVersion: string;
}

export function TitleBar(): React.JSX.Element {
  const [info, setInfo] = useState<VersionInfo | null>(null);
  const path = useDocument((s) => s.path);

  useEffect(() => {
    void invoke<VersionInfo>("app_version").then(setInfo).catch(() => setInfo(null));
  }, []);

  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const window_ = getCurrentWindow();
    const sync = (): void => {
      void window_
        .isMaximized()
        .then(setMaximized)
        .catch(() => setMaximized(false));
    };
    sync();
    let unlisten: (() => void) | null = null;
    void window_.onResized(sync).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, []);

  /**
   * Closes the window, asking first when there is work that closing would lose.
   *
   * Annotations live in memory until Phase 4 writes them to the PDF. Letting the
   * window close silently on top of an hour's marking up would be the worst
   * thing this application could do to someone, and a question costs a second.
   */
  async function close(): Promise<void> {
    if (useWorkspace.getState().hasEdits()) {
      const go = await confirm(t("window.unsavedBody"), {
        title: t("window.unsavedTitle"),
        kind: "warning",
      });
      if (!go) return;
    }
    await getCurrentWindow().close();
  }

  const name = path?.split(/[\\/]/).pop();

  return (
    <header
      data-tauri-drag-region
      className="h-9 shrink-0 flex items-center px-3 gap-3 border-b border-[var(--izul-border)] bg-[var(--izul-surface)] select-none"
    >
      <span className="font-medium">{info?.name ?? "PDF Studio Izul"}</span>
      {info && (
        <span
          className="text-[12px] text-[var(--izul-text-dim)]"
          title={`Fase ${info.phase} — ${info.phaseName} · PDFium ${info.pdfiumVersion}`}
        >
          {info.version} ({info.codename})
        </span>
      )}
      {name && <span className="text-[12px] text-[var(--izul-text-dim)] truncate">— {name}</span>}

      <div className="ml-auto flex items-center -mr-3 h-full">
        <button
          type="button"
          aria-label={t("window.minimize")}
          title={t("window.minimize")}
          onClick={() => void getCurrentWindow().minimize()}
          className="w-11 h-full grid place-items-center hover:bg-[var(--izul-surface-raised)]"
        >
          <span aria-hidden="true">–</span>
        </button>
        <button
          type="button"
          aria-label={maximized ? t("window.restore") : t("window.maximize")}
          title={maximized ? t("window.restore") : t("window.maximize")}
          onClick={() => void getCurrentWindow().toggleMaximize()}
          className="w-11 h-full grid place-items-center hover:bg-[var(--izul-surface-raised)]"
        >
          <span aria-hidden="true" className="text-[11px]">
            {maximized ? "❐" : "▢"}
          </span>
        </button>
        <button
          type="button"
          aria-label={t("window.close")}
          title={t("window.close")}
          onClick={() => void close()}
          className="w-11 h-full grid place-items-center hover:bg-[var(--izul-danger)] hover:text-white"
        >
          <span aria-hidden="true">✕</span>
        </button>
      </div>
    </header>
  );
}
