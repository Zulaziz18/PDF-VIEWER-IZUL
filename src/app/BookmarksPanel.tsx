/**
 * The "Bookmark Saya" sidebar tab (SPEC 11.1, Phase 8): the reader's own
 * marks, beside the document's outline rather than mixed into it.
 */

import { useEffect, useState, type JSX } from "react";
import { useDocument } from "@/state/documentStore";
import { IconButton } from "@/design/controls";
import { Icon } from "@/design/Icon";
import { t } from "@/i18n";
import { bookmarkHere, useBookmarks, type Bookmark } from "./bookmarks";
import { viewport } from "./viewportHandle";
import { withKeys } from "@/state/keymapStore";

// One empty list, so the selector's answer is stable between renders.
const NONE: readonly Bookmark[] = [];

export function BookmarksPanel(): JSX.Element {
  const doc = useDocument((s) => s.doc);
  const list = useBookmarks((s) => (s.doc === doc ? s.list : NONE));
  const error = useBookmarks((s) => s.error);
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");

  useEffect(() => {
    if (doc !== null) void useBookmarks.getState().load(doc);
  }, [doc]);

  const commit = (id: number): void => {
    setEditing(null);
    if (doc !== null && draft.trim().length > 0) void useBookmarks.getState().rename(doc, id, draft);
  };

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="p-2 border-b border-[var(--izul-border)]">
        <button
          type="button"
          onClick={() => void bookmarkHere()}
          title={withKeys(t("bookmarks.add"), "view.bookmark")}
          className="w-full h-8 px-2 flex items-center justify-center gap-1.5 rounded-[8px] text-[12px] border border-[var(--izul-border)] hover:bg-[var(--izul-surface-raised)]"
        >
          <Icon name="bookmarkAdd" size={16} tone="orange" />
          {t("bookmarks.add")}
        </button>
      </div>
      {error && (
        <p role="alert" className="p-3 text-[12px] text-[var(--izul-danger)]">
          {error}
        </p>
      )}
      {list.length === 0 ? (
        <p className="p-3 text-[13px] text-[var(--izul-text-dim)]">{t("bookmarks.empty")}</p>
      ) : (
        <ul className="flex-1 min-h-0 overflow-auto p-1">
          {list.map((b) => (
            <li key={b.id} className="group flex items-center gap-1 rounded-[8px] hover:bg-[var(--izul-surface-raised)]">
              {editing === b.id ? (
                <input
                  autoFocus
                  aria-label={t("bookmarks.rename")}
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  onBlur={() => commit(b.id)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") commit(b.id);
                    if (e.key === "Escape") setEditing(null);
                  }}
                  className="flex-1 min-w-0 m-1 rounded-[6px] bg-[var(--izul-canvas)] border border-[var(--izul-accent)] px-2 py-1 text-[13px]"
                />
              ) : (
                <button
                  type="button"
                  onClick={() => {
                    useDocument.getState().setPage(b.page);
                    viewport()?.goToPage(b.page);
                  }}
                  onDoubleClick={() => {
                    setDraft(b.label);
                    setEditing(b.id);
                  }}
                  className="flex-1 min-w-0 text-left py-1.5 pl-2 flex items-baseline gap-2"
                >
                  <span className="truncate text-[13px]">{b.label}</span>
                  <span className="shrink-0 text-[11px] text-[var(--izul-text-dim)]">
                    {t("bookmarks.page").replace("{n}", String(b.page + 1))}
                  </span>
                </button>
              )}
              <IconButton
                icon="editText"
                size={16}
                label={`${t("bookmarks.rename")} — ${b.label}`}
                onClick={() => {
                  setDraft(b.label);
                  setEditing(b.id);
                }}
              />
              <IconButton
                icon="dismiss"
                size={16}
                label={`${t("bookmarks.remove")} — ${b.label}`}
                onClick={() => doc !== null && void useBookmarks.getState().remove(doc, b.id)}
              />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
