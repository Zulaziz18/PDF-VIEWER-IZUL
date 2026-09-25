/**
 * Title bar and tab strip in one (SPEC 12, revised 2026-09-23).
 *
 * Laid out after WPS Office: a "Beranda" tab fixed at the left that shows the
 * home screen without closing anything, one tab per open document, a "+" that
 * opens the home screen for picking the next file, the rest of the bar as the
 * window's drag handle, and the window controls at the right.
 *
 * The window is undecorated, which comes with an obligation this bar must
 * meet: it has **no minimise, maximise or close buttons of its own**, so if
 * the bar does not draw them the only way out is Alt+F4. They also need
 * permissions — Tauri v2 refuses a window command the capability file has not
 * granted, silently (CLAUDE.md, bug #1). `core:window:allow-minimize`,
 * `allow-toggle-maximize`, `allow-close`, `allow-start-dragging` and
 * `allow-is-maximized` are in `src-tauri/capabilities/default.json` for
 * exactly this reason, and `allow-destroy` since Phase 4: the close button
 * asks the window to close, `useCloseGuard` settles unsaved work, and only
 * then is the window destroyed.
 *
 * Tabs are reordered with the HTML5 drag API rather than pointer maths: it is
 * two dozen lines instead of two hundred, and a tab strip is not a canvas.
 */

import { useEffect, useState, type JSX, type KeyboardEvent } from "react";
import { useStore } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { FileBadge } from "@/design/FileBadge";
import { Icon } from "@/design/Icon";
import { Logo } from "@/design/Logo";
import type { DocumentStore } from "@/state/documentSession";
import { useWorkspace, type Tab } from "@/state/workspaceStore";
import { t } from "@/i18n";
import { requestCloseTab } from "./fileActions";
import { TAB_DRAG_TYPE } from "./PanelGrid";

interface VersionInfo {
  name: string;
  codename: string;
  version: string;
  phase: string;
  phaseName: string;
  pdfiumVersion: string;
}

const TAB_BASE =
  "group relative h-[30px] flex items-center gap-2 px-3 rounded-t-[8px] select-none cursor-default text-[13px] transition-colors duration-[120ms]";
const TAB_ACTIVE = "bg-[var(--izul-surface)] text-[var(--izul-text)]";
const TAB_IDLE = "text-[var(--izul-text-dim)] hover:bg-[var(--izul-chrome-hover)] hover:text-[var(--izul-text)]";

/** The orange dot for edits that closing would lose (SPEC 10). */
function UnsavedDot(props: { store: DocumentStore }): JSX.Element | null {
  const dirty = useStore(props.store, (s) => s.dirty);
  if (!dirty) return null;
  return (
    <span
      role="img"
      aria-label={t("tabs.unsaved")}
      className="w-2 h-2 rounded-full bg-[var(--izul-unsaved)] shrink-0"
    />
  );
}

function DocTab(props: {
  tab: Tab;
  active: boolean;
  store: DocumentStore | undefined;
  dragging: boolean;
  onDragStart: () => void;
  onDrop: () => void;
  onDragEnd: () => void;
}): JSX.Element {
  const { tab } = props;
  const workspace = useWorkspace.getState;
  return (
    <div
      role="tab"
      aria-selected={props.active}
      tabIndex={props.active ? 0 : -1}
      draggable
      onDragStart={(e) => {
        // Also a payload a split-view panel understands: dropping a tab on a
        // panel shows the document there (Phase 5).
        e.dataTransfer.setData(TAB_DRAG_TYPE, String(tab.doc));
        e.dataTransfer.effectAllowed = "move";
        props.onDragStart();
      }}
      onDragOver={(e) => e.preventDefault()}
      onDrop={props.onDrop}
      onDragEnd={props.onDragEnd}
      onClick={() => void workspace().activate(tab.doc)}
      aria-keyshortcuts="Delete"
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          void workspace().activate(tab.doc);
        } else if (e.key === "Delete") {
          e.preventDefault();
          void requestCloseTab(tab.doc);
        }
      }}
      // The middle button closes a tab everywhere else; a reader who expects it
      // and does not get it assumes the click missed.
      onAuxClick={(e) => {
        if (e.button === 1) {
          e.preventDefault();
          void requestCloseTab(tab.doc);
        }
      }}
      title={tab.path}
      className={[
        TAB_BASE,
        "w-[216px] min-w-[120px] shrink",
        props.active ? TAB_ACTIVE : TAB_IDLE,
        props.dragging ? "opacity-50" : "",
      ].join(" ")}
    >
      <FileBadge size={18} />
      <span className="flex-1 truncate">{tab.name}</span>
      {props.store && <UnsavedDot store={props.store} />}
      {/* A mouse target only, as in every browser's tab strip: a button
          inside a tab is a control nested in a control, which a screen reader
          cannot reach anyway. The keyboard closes a tab with Delete or Ctrl+W
          (Phase 8, axe `nested-interactive`). */}
      <span
        aria-hidden="true"
        title={t("tabs.close")}
        onClick={(e) => {
          e.stopPropagation();
          void requestCloseTab(tab.doc);
        }}
        className={[
          "shrink-0 w-5 h-5 grid place-items-center rounded-[4px] hover:bg-[var(--izul-chrome-hover)]",
          props.active ? "" : "opacity-0 group-hover:opacity-100",
        ].join(" ")}
      >
        <Icon name="dismiss" size={16} />
      </span>
    </div>
  );
}

/**
 * Arrow keys, Home and End move between the tabs (the WAI-ARIA tabs pattern):
 * only the selected tab is in the Tab order, so without them the others are
 * out of the keyboard's reach.
 */
function moveAmongTabs(e: KeyboardEvent<HTMLDivElement>): void {
  const tabs = [...e.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]')];
  const at = tabs.indexOf(document.activeElement as HTMLElement);
  if (at < 0) return;
  const to =
    e.key === "ArrowRight"
      ? (at + 1) % tabs.length
      : e.key === "ArrowLeft"
        ? (at - 1 + tabs.length) % tabs.length
        : e.key === "Home"
          ? 0
          : e.key === "End"
            ? tabs.length - 1
            : null;
  if (to === null) return;
  e.preventDefault();
  tabs[to]?.focus();
}

export function TitleBar(): JSX.Element {
  const tabs = useWorkspace((s) => s.tabs);
  const activeDoc = useWorkspace((s) => s.activeDoc);
  const home = useWorkspace((s) => s.home);
  const sessions = useWorkspace((s) => s.sessions);
  const [dragging, setDragging] = useState<number | null>(null);
  const [info, setInfo] = useState<VersionInfo | null>(null);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    void invoke<VersionInfo>("app_version").then(setInfo).catch(() => setInfo(null));
  }, []);

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
    let cancelled = false;
    void window_
      .onResized(sync)
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  function drop(target: number): void {
    const from = dragging;
    setDragging(null);
    if (from === null || from === target) return;
    const order = tabs.map((tab) => tab.doc);
    const fromIndex = order.indexOf(from);
    const toIndex = order.indexOf(target);
    if (fromIndex < 0 || toIndex < 0) return;
    order.splice(toIndex, 0, ...order.splice(fromIndex, 1));
    void useWorkspace.getState().reorder(order);
  }

  const homeActive = home || activeDoc === null;
  const version = info ? `${info.version}` : "";

  return (
    <header
      data-tauri-drag-region
      className="h-[38px] shrink-0 flex items-end pl-1.5 bg-[var(--izul-chrome)] select-none"
    >
      <div
        role="tablist"
        aria-label={t("tabs.label")}
        onKeyDown={moveAmongTabs}
        className="flex items-end min-w-0 gap-0.5"
      >
        <div
          role="tab"
          aria-selected={homeActive}
          tabIndex={homeActive ? 0 : -1}
          onClick={() => useWorkspace.getState().showHome()}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              useWorkspace.getState().showHome();
            }
          }}
          title={info ? `${info.name} ${info.version} (${info.codename})` : undefined}
          className={[TAB_BASE, "w-[112px] shrink-0", homeActive ? TAB_ACTIVE : TAB_IDLE].join(" ")}
        >
          <Logo size={18} />
          <span className="truncate">{t("tabs.home")}</span>
        </div>
        {tabs.map((tab) => (
          <DocTab
            key={tab.doc}
            tab={tab}
            active={!homeActive && tab.doc === activeDoc}
            store={sessions.get(tab.doc)}
            dragging={dragging === tab.doc}
            onDragStart={() => setDragging(tab.doc)}
            onDrop={() => drop(tab.doc)}
            onDragEnd={() => setDragging(null)}
          />
        ))}
      </div>
      <button
        type="button"
        aria-label={t("tabs.new")}
        title={t("tabs.new")}
        onClick={() => useWorkspace.getState().showHome()}
        className="h-[30px] shrink-0 px-2.5 mb-0 flex items-center gap-1.5 rounded-t-[8px] text-[13px] text-[var(--izul-text-dim)] hover:bg-[var(--izul-chrome-hover)] hover:text-[var(--izul-text)]"
      >
        <Icon name="add" size={16} />
        <span>{t("tabs.newShort")}</span>
      </button>

      <div data-tauri-drag-region className="flex-1 min-w-4 self-stretch" />
      {version && (
        <span
          data-tauri-drag-region
          className="self-center mr-3 text-[12px] text-[var(--izul-text-dim)] whitespace-nowrap"
          title={info ? `${t("about.phase")} ${info.phase} — ${info.phaseName} · PDFium ${info.pdfiumVersion}` : undefined}
        >
          {version}
        </span>
      )}

      <div className="flex self-stretch">
        <button
          type="button"
          aria-label={t("window.minimize")}
          title={t("window.minimize")}
          onClick={() => void getCurrentWindow().minimize()}
          className="w-[46px] grid place-items-center hover:bg-[var(--izul-chrome-hover)]"
        >
          <Icon name="minimize" size={16} />
        </button>
        <button
          type="button"
          aria-label={maximized ? t("window.restore") : t("window.maximize")}
          title={maximized ? t("window.restore") : t("window.maximize")}
          onClick={() => void getCurrentWindow().toggleMaximize()}
          className="w-[46px] grid place-items-center hover:bg-[var(--izul-chrome-hover)]"
        >
          <Icon name={maximized ? "restore" : "maximize"} size={16} />
        </button>
        <button
          type="button"
          aria-label={t("window.close")}
          title={t("window.close")}
          // A request, not a destroy: it arrives at `useCloseGuard` exactly
          // as Alt+F4 does, and unsaved work is settled there for both.
          onClick={() => void getCurrentWindow().close()}
          className="w-[46px] grid place-items-center hover:bg-[var(--izul-danger-fill)] hover:text-[var(--izul-on-danger)]"
        >
          <Icon name="dismiss" size={16} />
        </button>
      </div>
    </header>
  );
}
