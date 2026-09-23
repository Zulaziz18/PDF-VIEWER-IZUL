/**
 * Split view (SPEC 10): one, two or four panels, each showing one open
 * document with its own viewport, scroll and zoom. The focused panel is the
 * document in front — the ribbon, the sidebar and the keyboard act on it —
 * and clicking into a panel focuses it.
 *
 * Tabs are dragged from the title bar onto a panel to show them there;
 * pages dragged from the page panel onto another document's panel are
 * copied into it after its current page (moved with Shift). The dividers are
 * dragged, and where they sit is remembered.
 */

import { useRef, useState, type JSX } from "react";
import { FileBadge } from "@/design/FileBadge";
import { Icon } from "@/design/Icon";
import { t } from "@/i18n";
import type { DocumentStore } from "@/state/documentSession";
import { PAGE_DRAG_TYPE, decodePageDrag } from "@/state/pageSelection";
import { useWorkspace } from "@/state/workspaceStore";
import { Viewport } from "./Viewport";
import { toggleCompare, useCompare, useCompareMode } from "./compare";

/** What a tab carries when dragged out of the title bar. */
export const TAB_DRAG_TYPE = "application/x-izul-tab";

function Divider(props: { axis: "x" | "y"; host: React.RefObject<HTMLDivElement | null> }): JSX.Element {
  const vertical = props.axis === "x";
  return (
    <div
      role="separator"
      aria-orientation={vertical ? "vertical" : "horizontal"}
      aria-label={t("panels.divider")}
      tabIndex={0}
      onKeyDown={(e) => {
        const step = e.key === (vertical ? "ArrowLeft" : "ArrowUp") ? -0.05 : e.key === (vertical ? "ArrowRight" : "ArrowDown") ? 0.05 : 0;
        if (step !== 0) {
          e.preventDefault();
          const ws = useWorkspace.getState();
          ws.setRatio(props.axis, ws.panels.ratio[props.axis] + step);
        }
      }}
      onPointerDown={(e) => {
        const host = props.host.current;
        if (!host) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        const rect = host.getBoundingClientRect();
        const move = (ev: PointerEvent): void => {
          const r = vertical ? (ev.clientX - rect.left) / rect.width : (ev.clientY - rect.top) / rect.height;
          useWorkspace.getState().setRatio(props.axis, r);
        };
        const up = (): void => {
          window.removeEventListener("pointermove", move);
          window.removeEventListener("pointerup", up);
        };
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", up);
      }}
      className={[
        "shrink-0 bg-[var(--izul-border)] hover:bg-[var(--izul-accent)] focus-visible:bg-[var(--izul-accent)] outline-none",
        vertical ? "w-[5px] cursor-col-resize" : "h-[5px] cursor-row-resize",
      ].join(" ")}
    />
  );
}

function EmptyPanel(props: { index: number }): JSX.Element {
  const tabs = useWorkspace((s) => s.tabs);
  const shown = useWorkspace((s) => s.panels.panels);
  const hidden = tabs.filter((tab) => !shown.includes(tab.doc));
  return (
    <div className="h-full flex flex-col items-center justify-center gap-3 p-6 text-center text-[13px] text-[var(--izul-text-dim)]">
      <Icon name="windows" size={28} tone="blue" />
      <p>{hidden.length > 0 ? t("panels.empty") : t("panels.emptyNone")}</p>
      {hidden.length > 0 && (
        <ul className="w-full max-w-[280px] flex flex-col gap-1">
          {hidden.map((tab) => (
            <li key={tab.doc}>
              <button
                type="button"
                onClick={() => useWorkspace.getState().assignPanel(props.index, tab.doc)}
                className="w-full h-8 px-2 flex items-center gap-2 rounded-[6px] text-left text-[var(--izul-text)] hover:bg-[var(--izul-surface-raised)]"
              >
                <FileBadge size={16} />
                <span className="truncate">{tab.name}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function DocPanel(props: {
  index: number;
  doc: number | null;
  store: DocumentStore | undefined;
  focused: boolean;
  split: boolean;
  onScrolled?: () => void;
}): JSX.Element {
  const name = useWorkspace((s) => s.tabs.find((tab) => tab.doc === props.doc)?.name ?? "");
  const [over, setOver] = useState(false);
  const ws = useWorkspace.getState;
  return (
    <section
      aria-label={name || t("panels.emptyLabel")}
      onDragOver={(e) => {
        const types = e.dataTransfer.types;
        if (!types.includes(TAB_DRAG_TYPE) && !types.includes(PAGE_DRAG_TYPE)) return;
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        setOver(false);
        const tab = Number(e.dataTransfer.getData(TAB_DRAG_TYPE));
        if (Number.isFinite(tab) && tab > 0) {
          e.preventDefault();
          ws().assignPanel(props.index, tab);
          return;
        }
        const pages = decodePageDrag(e.dataTransfer.getData(PAGE_DRAG_TYPE));
        if (pages && props.store && pages.doc !== props.doc) {
          e.preventDefault();
          const s = props.store.getState();
          void s.copyPagesFrom(pages.doc, pages.pages, s.page + 1, e.shiftKey);
        }
      }}
      onPointerDownCapture={() => ws().focusPanel(props.index)}
      className={[
        "relative flex-1 min-w-0 min-h-0 flex flex-col",
        props.split && props.focused ? "outline outline-2 -outline-offset-2 outline-[var(--izul-accent)] z-[1]" : "",
        over ? "outline outline-2 -outline-offset-2 outline-dashed outline-[var(--izul-accent)]" : "",
      ].join(" ")}
    >
      {props.split && (
        <header className="h-7 shrink-0 flex items-center gap-2 px-2 text-[12px] bg-[var(--izul-chrome)] border-b border-[var(--izul-border)]">
          {props.doc !== null ? (
            <>
              <FileBadge size={14} />
              <span className={["truncate", props.focused ? "font-semibold" : "text-[var(--izul-text-dim)]"].join(" ")}>{name}</span>
            </>
          ) : (
            <span className="text-[var(--izul-text-dim)]">{t("panels.emptyLabel")}</span>
          )}
        </header>
      )}
      {props.doc !== null && props.store ? (
        <Viewport
          key={props.doc}
          store={props.store}
          onFocus={() => ws().focusPanel(props.index)}
          {...(props.onScrolled ? { onScrolled: props.onScrolled } : {})}
        />
      ) : (
        <EmptyPanel index={props.index} />
      )}
    </section>
  );
}

function CompareBar(): JSX.Element {
  const sync = useCompare((s) => s.sync);
  const page = useCompare((s) => s.page);
  const differences = useCompare((s) => s.differences);
  const method = useCompare((s) => s.method);
  const running = useCompare((s) => s.running);
  const status =
    page === null
      ? ""
      : differences === null
        ? running
          ? t("compare.running")
          : ""
        : `${t("status.page")} ${page + 1}: ${differences === 0 ? t("compare.same") : `${differences} ${t("compare.differences")}`} (${method === "visual" ? t("compare.visual") : t("compare.text")})`;
  return (
    <div className="h-9 shrink-0 flex items-center gap-3 px-3 bg-[var(--izul-accent-soft)] border-b border-[var(--izul-border)] text-[13px]">
      <Icon name="compare" size={20} tone="blue" />
      <span className="font-semibold">{t("compare.title")}</span>
      <label className="flex items-center gap-1.5">
        <input type="checkbox" checked={sync} onChange={(e) => useCompare.getState().setSync(e.target.checked)} className="accent-[var(--izul-accent)]" />
        {t("compare.sync")}
      </label>
      <span className="flex-1 truncate text-[var(--izul-text-dim)]" aria-live="polite">
        {status}
      </span>
      <span className="inline-flex items-center gap-1.5 text-[12px] text-[var(--izul-text-dim)]">
        {/* Not the mark's own class: its multiply blend is for a white page,
            and on the dark bar it would vanish. */}
        <span
          aria-hidden="true"
          className="inline-block w-3 h-3 rounded-[2px] bg-[#e5484d]/45 outline outline-1 outline-[#e5484d]"
        />
        {t("compare.legend")}
      </span>
      <button
        type="button"
        onClick={toggleCompare}
        className="h-7 px-3 rounded-[6px] border border-[var(--izul-border)] bg-[var(--izul-surface)] hover:bg-[var(--izul-surface-raised)]"
      >
        {t("compare.stop")}
      </button>
    </div>
  );
}

export function PanelGrid(): JSX.Element {
  const panels = useWorkspace((s) => s.panels);
  const sessions = useWorkspace((s) => s.sessions);
  const compare = useWorkspace((s) => s.compare);
  const host = useRef<HTMLDivElement>(null);
  const { layout, ratio, focused } = panels;
  const split = layout !== "single";
  const storeOf = (i: number): DocumentStore | undefined => {
    const doc = panels.panels[i];
    return doc === null || doc === undefined ? undefined : sessions.get(doc);
  };
  const comparing = compare && layout === "columns";
  const { onScrolled } = useCompareMode(storeOf(0), storeOf(1), comparing);

  const panel = (i: number): JSX.Element => (
    <DocPanel
      key={i}
      index={i}
      doc={panels.panels[i] ?? null}
      store={storeOf(i)}
      focused={i === focused}
      split={split}
      {...(comparing && (i === 0 || i === 1) ? { onScrolled: () => onScrolled(i as 0 | 1) } : {})}
    />
  );
  const sized = (grow: number, child: JSX.Element): JSX.Element => (
    <div className="min-w-0 min-h-0 flex" style={{ flex: `${grow} 1 0` }}>
      {child}
    </div>
  );

  let body: JSX.Element;
  if (layout === "columns") {
    body = (
      <div ref={host} className="flex-1 min-h-0 flex">
        {sized(ratio.x, panel(0))}
        <Divider axis="x" host={host} />
        {sized(1 - ratio.x, panel(1))}
      </div>
    );
  } else if (layout === "rows") {
    body = (
      <div ref={host} className="flex-1 min-w-0 flex flex-col">
        {sized(ratio.y, panel(0))}
        <Divider axis="y" host={host} />
        {sized(1 - ratio.y, panel(1))}
      </div>
    );
  } else if (layout === "grid") {
    body = (
      <div ref={host} className="flex-1 min-w-0 min-h-0 flex flex-col">
        {sized(
          ratio.y,
          <div className="flex-1 min-w-0 flex">
            {sized(ratio.x, panel(0))}
            <Divider axis="x" host={host} />
            {sized(1 - ratio.x, panel(1))}
          </div>,
        )}
        <Divider axis="y" host={host} />
        {sized(
          1 - ratio.y,
          <div className="flex-1 min-w-0 flex">
            {sized(ratio.x, panel(2))}
            <Divider axis="x" host={host} />
            {sized(1 - ratio.x, panel(3))}
          </div>,
        )}
      </div>
    );
  } else {
    body = <div className="flex-1 min-w-0 min-h-0 flex">{panel(0)}</div>;
  }

  return (
    <div className="flex-1 min-w-0 min-h-0 flex flex-col">
      {comparing && <CompareBar />}
      {body}
    </div>
  );
}
