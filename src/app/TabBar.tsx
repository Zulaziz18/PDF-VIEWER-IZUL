/**
 * The tab strip (SPEC 10, SPEC 12).
 *
 * Sits directly under the title bar, so the window reads as one surface. It is
 * the only place the workspace store is driven from by a click, which is why
 * closing and reordering live here rather than in the store's callers.
 *
 * Reordering is a drag with the HTML5 drag API rather than pointer maths: it is
 * two dozen lines instead of two hundred, it is what a screen reader and the
 * keyboard already understand, and a tab strip is not a canvas.
 */

import { useRef, useState } from "react";
import { useWorkspace } from "@/state/workspaceStore";
import { t } from "@/i18n";

export function TabBar(): React.JSX.Element | null {
  const tabs = useWorkspace((s) => s.tabs);
  const activeDoc = useWorkspace((s) => s.activeDoc);
  const [dragging, setDragging] = useState<number | null>(null);
  const strip = useRef<HTMLDivElement>(null);

  // One document is not a tab strip; showing one tab just steals 36 pixels from
  // the page for no information.
  if (tabs.length < 2) return null;

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

  return (
    <div
      ref={strip}
      role="tablist"
      aria-label={t("tabs.label")}
      className="flex items-stretch h-9 shrink-0 gap-1 px-2 border-b border-[var(--izul-border)] bg-[var(--izul-surface)] overflow-x-auto"
    >
      {tabs.map((tab) => {
        const active = tab.doc === activeDoc;
        return (
          <div
            key={tab.doc}
            role="tab"
            aria-selected={active}
            tabIndex={active ? 0 : -1}
            draggable
            onDragStart={() => setDragging(tab.doc)}
            onDragOver={(e) => e.preventDefault()}
            onDrop={() => drop(tab.doc)}
            onDragEnd={() => setDragging(null)}
            onClick={() => void useWorkspace.getState().activate(tab.doc)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                void useWorkspace.getState().activate(tab.doc);
              }
            }}
            // The middle button closes a tab everywhere else; a reader who
            // expects it and does not get it assumes the click missed.
            onAuxClick={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                void useWorkspace.getState().closeTab(tab.doc);
              }
            }}
            title={tab.path}
            className={[
              "group flex items-center gap-2 px-3 max-w-[220px] rounded-t-[8px] cursor-default select-none",
              active
                ? "bg-[var(--izul-canvas)] border-x border-t border-[var(--izul-border)]"
                : "text-[var(--izul-text-dim)] hover:bg-[var(--izul-surface-raised)]",
              dragging === tab.doc ? "opacity-50" : "",
            ].join(" ")}
          >
            <span className="truncate text-[13px]">{tab.name}</span>
            <button
              type="button"
              aria-label={`${t("tabs.close")} — ${tab.name}`}
              onClick={(e) => {
                e.stopPropagation();
                void useWorkspace.getState().closeTab(tab.doc);
              }}
              className="shrink-0 w-4 h-4 leading-none rounded-[4px] opacity-0 group-hover:opacity-100 focus:opacity-100 hover:bg-[var(--izul-surface-raised)]"
            >
              ×
            </button>
          </div>
        );
      })}
    </div>
  );
}
