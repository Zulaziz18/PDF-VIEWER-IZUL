/**
 * The sidebar: page thumbnails and the document's own outline (SPEC 11.1).
 *
 * Both panels are navigation, so both do the same thing when clicked — move the
 * viewport — and neither holds any state the viewport does not already have.
 *
 * Thumbnails are the viewport's preview tier at the same URI, so a page whose
 * thumbnail has been drawn is also a page the viewport can show instantly, and
 * neither pays for the other.
 */

import type { JSX } from "react";
import { AnnotationList } from "./AnnotationList";
import { AttachmentsPanel } from "./AttachmentsPanel";
import { BookmarksPanel } from "./BookmarksPanel";
import { FormsPanel } from "./FormsPanel";
import { PagePanel } from "./PagePanel";
import { SearchPanel } from "./SearchPanel";
import { Icon, type IconName, type Tone } from "@/design/Icon";
import { IconButton } from "@/design/controls";
import { useDocument, type OutlineEntry, type SidebarTab } from "@/state/documentStore";
import { viewport } from "./viewportHandle";
import { t, type StringKey } from "@/i18n";

function OutlineList(props: { entries: readonly OutlineEntry[] }): JSX.Element {
  if (props.entries.length === 0) {
    return <p className="p-3 text-[var(--izul-text-dim)]">{t("sidebar.noOutline")}</p>;
  }
  return (
    <ul className="p-1">
      {props.entries.map((entry, index) => (
        <li key={`${index}-${entry.title}`}>
          <button
            type="button"
            disabled={entry.page === null}
            onClick={() => {
              if (entry.page === null) return;
              // Bookmarks speak in the file's page numbers; after pages have
              // been moved the page they point at may be elsewhere, or gone.
              const shown = useDocument.getState().displayOfOwn(entry.page);
              if (shown === null) return;
              useDocument.getState().setPage(shown);
              viewport()?.goToPage(shown, entry.y ?? undefined);
            }}
            title={entry.title}
            style={{ paddingLeft: `${8 + Math.min(entry.depth, 6) * 12}px` }}
            className="w-full text-left truncate py-1.5 pr-2 rounded-[8px] hover:bg-[var(--izul-surface-raised)] disabled:opacity-50"
          >
            {entry.title || t("sidebar.untitled")}
          </button>
        </li>
      ))}
    </ul>
  );
}

const PANEL_TITLE: Record<SidebarTab, StringKey> = {
  thumbnails: "sidebar.thumbnails",
  outline: "sidebar.outline",
  annots: "sidebar.annots",
  search: "sidebar.search",
  forms: "sidebar.forms",
  bookmarks: "sidebar.bookmarks",
  attachments: "sidebar.attachments",
};

/** The icon rail's entries, top to bottom (SPEC 12, revised 2026-09-23). */
export const RAIL: ReadonlyArray<{ tab: SidebarTab; icon: IconName; tone: Tone }> = [
  { tab: "thumbnails", icon: "thumbnails", tone: "blue" },
  { tab: "outline", icon: "bookmark", tone: "orange" },
  { tab: "bookmarks", icon: "bookmarks", tone: "rose" },
  { tab: "annots", icon: "comments", tone: "amber" },
  { tab: "search", icon: "search", tone: "violet" },
  { tab: "forms", icon: "formFill", tone: "teal" },
  { tab: "attachments", icon: "attach", tone: "neutral" },
];

/**
 * The vertical icon rail at the window's left edge, as in WPS Office: one icon
 * per side panel. Clicking the panel that is already showing folds it away.
 */
export function LeftRail(): JSX.Element {
  const open = useDocument((s) => s.sidebarOpen);
  const tab = useDocument((s) => s.sidebarTab);
  return (
    <nav
      aria-label={t("sidebar.label")}
      className="w-11 shrink-0 flex flex-col items-center gap-1 pt-2 bg-[var(--izul-chrome)] border-r border-[var(--izul-border)]"
    >
      {RAIL.map((item) => {
        const active = open && tab === item.tab;
        return (
          <button
            key={item.tab}
            type="button"
            aria-pressed={active}
            aria-label={t(PANEL_TITLE[item.tab])}
            title={t(PANEL_TITLE[item.tab])}
            onClick={() => {
              const store = useDocument.getState();
              if (active) store.toggleSidebar();
              else store.setSidebarTab(item.tab);
            }}
            className={[
              "relative w-9 h-9 grid place-items-center rounded-[8px]",
              active ? "bg-[var(--izul-accent-soft)]" : "hover:bg-[var(--izul-chrome-hover)]",
            ].join(" ")}
          >
            {active && (
              <span aria-hidden="true" className="absolute -left-1 top-2 bottom-2 w-[3px] rounded-full bg-[var(--izul-accent)]" />
            )}
            <Icon name={item.icon} size={20} tone={active ? item.tone : "neutral"} />
          </button>
        );
      })}
    </nav>
  );
}

export function Sidebar(): JSX.Element | null {
  const open = useDocument((s) => s.sidebarOpen);
  const tab = useDocument((s) => s.sidebarTab);
  const doc = useDocument((s) => s.doc);
  const outline = useDocument((s) => s.outline);

  if (!open || doc === null) return null;

  return (
    <aside
      className="w-[248px] shrink-0 border-r border-[var(--izul-border)] bg-[var(--izul-surface)] flex flex-col"
      aria-label={t(PANEL_TITLE[tab])}
    >
      <header className="h-10 shrink-0 flex items-center pl-3 pr-1.5 border-b border-[var(--izul-border)]">
        <h2 className="flex-1 text-[13px] font-semibold">{t(PANEL_TITLE[tab])}</h2>
        <IconButton
          icon="dismiss"
          size={16}
          label={t("sidebar.close")}
          onClick={() => {
            const store = useDocument.getState();
            if (tab === "search") store.toggleSearch(false);
            else store.toggleSidebar();
          }}
        />
      </header>

      <div className="flex-1 min-h-0 overflow-auto">
        {tab === "annots" ? (
          <AnnotationList />
        ) : tab === "search" ? (
          <SearchPanel />
        ) : tab === "forms" ? (
          <FormsPanel />
        ) : tab === "thumbnails" ? (
          <PagePanel />
        ) : tab === "bookmarks" ? (
          <BookmarksPanel />
        ) : tab === "attachments" ? (
          <AttachmentsPanel />
        ) : (
          <OutlineList entries={outline} />
        )}
      </div>
    </aside>
  );
}
