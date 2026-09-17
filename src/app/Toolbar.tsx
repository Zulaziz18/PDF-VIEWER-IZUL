/**
 * The reading toolbar (SPEC 12).
 *
 * Read mode only, which is all Phase 1 has: navigation, zoom, rotation and
 * layout. The annotation tools appear here in Phase 3 behind the mode switch,
 * which is why the controls are already grouped rather than in one row.
 *
 * Every control is a real button with a label and a title, because the whole
 * application has to be operable without a mouse and legible to a screen
 * reader (SPEC 14).
 */

import type { JSX } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useDocument, type ZoomMode } from "@/state/documentStore";
import { useWorkspace } from "@/state/workspaceStore";
import type { ViewMode } from "@/viewport/layout";
import { viewport } from "./viewportHandle";
import { t } from "@/i18n";
import type { StringKey } from "@/i18n";

function Button(props: {
  onClick: () => void;
  label: StringKey;
  children: React.ReactNode;
  active?: boolean;
  disabled?: boolean;
}): JSX.Element {
  return (
    <button
      type="button"
      onClick={props.onClick}
      disabled={props.disabled ?? false}
      aria-pressed={props.active ?? undefined}
      aria-label={t(props.label)}
      title={t(props.label)}
      className={[
        "h-8 min-w-8 px-2 rounded-[8px] text-[13px] flex items-center justify-center",
        "transition-colors duration-[120ms] ease-[cubic-bezier(0.32,0.72,0,1)]",
        props.active === true
          ? "bg-[var(--izul-accent)] text-white"
          : "hover:bg-[var(--izul-surface-raised)] disabled:opacity-40",
      ].join(" ")}
    >
      {props.children}
    </button>
  );
}

const VIEW_MODE_LABELS: Record<ViewMode, StringKey> = {
  single: "view.single",
  dual: "view.dual",
  dual_cover: "view.dualCover",
  horizontal: "view.horizontal",
};

const ZOOM_MODE_LABELS: Record<Exclude<ZoomMode, "custom">, StringKey> = {
  fitWidth: "zoom.fitWidth",
  fitPage: "zoom.fitPage",
  actual: "zoom.actual",
};

/** Opens a file into a new tab. The dialog is the plugin's, so it is the
 * platform's own picker rather than something we drew. */
async function pickAndOpen(): Promise<void> {
  const chosen = await openDialog({
    multiple: true,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  const paths = Array.isArray(chosen) ? chosen : typeof chosen === "string" ? [chosen] : [];
  for (const path of paths) {
    await useWorkspace.getState().openFile(path);
  }
}

export function Toolbar(): JSX.Element {
  const page = useDocument((s) => s.page);
  const pageCount = useDocument((s) => s.pageCount);
  const zoom = useDocument((s) => s.zoom);
  const zoomMode = useDocument((s) => s.zoomMode);
  const viewMode = useDocument((s) => s.viewMode);
  const sidebarOpen = useDocument((s) => s.sidebarOpen);
  const store = useDocument.getState;

  const go = (target: number): void => {
    const clamped = Math.min(Math.max(0, target), Math.max(0, pageCount - 1));
    store().setPage(clamped);
    viewport()?.goToPage(clamped);
  };

  return (
    <div className="h-11 shrink-0 flex items-center gap-1 px-2 border-b border-[var(--izul-border)] bg-[var(--izul-surface)]">
      {/* The way into a second document. It existed only on the empty state and
          as Ctrl+O, which meant that once anything was open there was no
          visible way to open anything else — a reader should not have to know a
          shortcut to use the tabs the application just grew. */}
      <Button onClick={() => void pickAndOpen()} label="toolbar.open">
        📂
      </Button>
      <Button onClick={() => store().toggleSidebar()} label="toolbar.sidebar" active={sidebarOpen}>
        ☰
      </Button>
      <div className="w-px h-5 bg-[var(--izul-border)] mx-1" />

      <Button onClick={() => go(page - 1)} label="nav.previous" disabled={page <= 0}>
        ‹
      </Button>
      <label className="flex items-center gap-1 text-[13px]">
        <span className="sr-only">{t("nav.page")}</span>
        <input
          type="number"
          min={1}
          max={Math.max(1, pageCount)}
          value={page + 1}
          onChange={(e) => go(Number(e.target.value) - 1)}
          aria-label={t("nav.page")}
          className="w-14 h-8 px-2 rounded-[8px] bg-[var(--izul-surface-raised)] border border-[var(--izul-border)] text-right"
        />
        <span className="text-[var(--izul-text-dim)]">/ {pageCount}</span>
      </label>
      <Button onClick={() => go(page + 1)} label="nav.next" disabled={page >= pageCount - 1}>
        ›
      </Button>

      <div className="w-px h-5 bg-[var(--izul-border)] mx-1" />
      <Button onClick={() => store().zoomOut()} label="zoom.out">
        −
      </Button>
      <span className="w-14 text-center text-[13px] tabular-nums" aria-live="polite">
        {Math.round(zoom * 100)}%
      </span>
      <Button onClick={() => store().zoomIn()} label="zoom.in">
        +
      </Button>
      {(Object.keys(ZOOM_MODE_LABELS) as Array<keyof typeof ZOOM_MODE_LABELS>).map((mode) => (
        <Button
          key={mode}
          onClick={() => store().setZoomMode(mode)}
          label={ZOOM_MODE_LABELS[mode]}
          active={zoomMode === mode}
        >
          {t(ZOOM_MODE_LABELS[mode])}
        </Button>
      ))}

      <div className="w-px h-5 bg-[var(--izul-border)] mx-1" />
      <Button onClick={() => store().rotateDocument(-1)} label="rotate.left">
        ⟲
      </Button>
      <Button onClick={() => store().rotateDocument(1)} label="rotate.right">
        ⟳
      </Button>
      <Button onClick={() => store().rotatePage(page, 1)} label="rotate.page">
        ⤾
      </Button>

      <div className="w-px h-5 bg-[var(--izul-border)] mx-1" />
      {(Object.keys(VIEW_MODE_LABELS) as ViewMode[]).map((mode) => (
        <Button
          key={mode}
          onClick={() => store().setViewMode(mode)}
          label={VIEW_MODE_LABELS[mode]}
          active={viewMode === mode}
        >
          {t(VIEW_MODE_LABELS[mode])}
        </Button>
      ))}
    </div>
  );
}
