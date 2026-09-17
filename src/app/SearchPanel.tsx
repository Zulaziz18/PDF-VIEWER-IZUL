/**
 * Search, in the three tiers SPEC 11.1 asks for, plus regex on its own footing.
 *
 * * **Dokumen ini** — the open document, answered by its FTS5 index. Pages, in
 *   relevance order; the highlights on the page the reader lands on come from
 *   PDFium, which is the only thing that knows where on the page the words are.
 * * **Semua dokumen** — every file that has ever been indexed. Hits carry the
 *   file they came from, and one click opens it in a tab.
 * * **Regex** — one page of the open document at a time, and the panel says so
 *   in as many words. SPEC 11.1 forbids presenting it as the equal of the
 *   other two, and the reason is real: an index of tokens has no ordered text
 *   for a pattern to run against, so a regex can only see what somebody has
 *   extracted. Saying "this document only" where the user can read it is the
 *   difference between a limitation and a lie.
 *
 * Indexing progress is shown while it runs, because a search that returns
 * nothing from a half-indexed document is otherwise indistinguishable from a
 * document that does not contain the word.
 */

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useDocument } from "@/state/documentStore";
import type { SearchScope } from "@/state/documentStore";
import { useWorkspace } from "@/state/workspaceStore";
import { viewport } from "./viewportHandle";
import { t } from "@/i18n";

/** How long the typing has to stop before a query is sent. */
const DEBOUNCE_MS = 220;

interface IndexProgress {
  pages_done: number;
  page_count: number;
  running: boolean;
  error: string | null;
}

const SCOPES: ReadonlyArray<{ id: SearchScope; label: () => string }> = [
  { id: "document", label: () => t("search.scope.document") },
  { id: "library", label: () => t("search.scope.library") },
  { id: "regex", label: () => t("search.scope.regex") },
];

export function SearchPanel(): React.JSX.Element | null {
  const doc = useDocument((s) => s.doc);
  const search = useDocument((s) => s.search);
  const inputRef = useRef<HTMLInputElement>(null);
  const [progress, setProgress] = useState<IndexProgress | null>(null);

  // Debounced on the query's own epoch: every keystroke raises it, so this
  // effect re-arms rather than stacking one timer per character.
  useEffect(() => {
    if (!search.open) return;
    const timer = window.setTimeout(() => {
      void useDocument.getState().runSearch();
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [search.open, search.generation, search.scope, search.caseSensitive, search.wholeWord]);

  useEffect(() => {
    if (search.open) inputRef.current?.focus();
  }, [search.open]);

  // Only while it is actually running: a finished index has nothing to report
  // and a poll that never stops is a poll that shows up in a profile.
  useEffect(() => {
    if (!search.open || doc === null) return;
    let stop = false;
    const tick = (): void => {
      void invoke<IndexProgress | null>("index_progress", { doc })
        .then((p) => {
          if (stop) return;
          setProgress(p);
          if (p?.running) window.setTimeout(tick, 400);
        })
        .catch(() => setProgress(null));
    };
    tick();
    return () => {
      stop = true;
    };
  }, [search.open, doc]);

  if (!search.open) return null;

  const store = useDocument.getState;
  const indexing = progress?.running === true;
  const results = search.results;

  function goTo(index: number): void {
    const hit = results[index];
    if (!hit) return;
    if (hit.doc !== null && hit.doc !== doc) {
      void useWorkspace.getState().activate(hit.doc);
      return;
    }
    if (hit.doc === null && hit.path) {
      void useWorkspace.getState().openFile(hit.path);
      return;
    }
    store().setPage(hit.page);
    viewport()?.goToPage(hit.page);
  }

  return (
    <aside
      aria-label={t("search.label")}
      className="w-[320px] shrink-0 flex flex-col border-l border-[var(--izul-border)] bg-[var(--izul-surface)]"
    >
      <div className="p-3 flex flex-col gap-2 border-b border-[var(--izul-border)]">
        <div className="flex items-center gap-2">
          <input
            ref={inputRef}
            type="search"
            value={search.query}
            placeholder={search.scope === "regex" ? t("search.placeholderRegex") : t("search.placeholder")}
            aria-label={t("search.label")}
            onChange={(e) => store().setSearchQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                store().gotoResult(e.shiftKey ? -1 : 1);
              } else if (e.key === "Escape") {
                store().toggleSearch(false);
              }
            }}
            className="flex-1 min-w-0 rounded-[8px] bg-[var(--izul-canvas)] border border-[var(--izul-border)] px-2 py-1.5 text-[13px]"
          />
          <button
            type="button"
            aria-label={t("search.close")}
            onClick={() => store().toggleSearch(false)}
            className="w-7 h-7 rounded-[8px] hover:bg-[var(--izul-surface-raised)]"
          >
            ×
          </button>
        </div>

        <div role="tablist" aria-label={t("search.scopeLabel")} className="flex gap-1">
          {SCOPES.map((scope) => (
            <button
              key={scope.id}
              type="button"
              role="tab"
              aria-selected={search.scope === scope.id}
              onClick={() => store().setSearchScope(scope.id)}
              className={[
                "flex-1 rounded-[8px] px-2 py-1 text-[12px]",
                search.scope === scope.id
                  ? "bg-[var(--izul-accent)] text-white"
                  : "hover:bg-[var(--izul-surface-raised)] text-[var(--izul-text-dim)]",
              ].join(" ")}
            >
              {scope.label()}
            </button>
          ))}
        </div>

        <div className="flex items-center gap-3 text-[12px] text-[var(--izul-text-dim)]">
          <label className="flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={search.caseSensitive}
              onChange={(e) => store().setSearchOption("caseSensitive", e.target.checked)}
            />
            {t("search.caseSensitive")}
          </label>
          {search.scope !== "regex" && (
            <label className="flex items-center gap-1.5">
              <input
                type="checkbox"
                checked={search.wholeWord}
                onChange={(e) => store().setSearchOption("wholeWord", e.target.checked)}
              />
              {t("search.wholeWord")}
            </label>
          )}
        </div>

        {search.scope === "regex" && (
          <p className="text-[12px] text-[var(--izul-text-dim)] leading-snug">
            {t("search.regexNote")}
          </p>
        )}
        {indexing && progress && (
          <p className="text-[12px] text-[var(--izul-text-dim)]" aria-live="polite">
            {t("search.indexing")} {progress.pages_done}/{progress.page_count}
          </p>
        )}
        {search.error !== null && (
          <p role="alert" className="text-[12px] text-[var(--izul-danger)]">
            {search.error}
          </p>
        )}
      </div>

      <div className="flex-1 min-h-0 overflow-auto">
        {search.running && results.length === 0 ? (
          <p className="p-3 text-[13px] text-[var(--izul-text-dim)]">{t("search.running")}</p>
        ) : results.length === 0 ? (
          <p className="p-3 text-[13px] text-[var(--izul-text-dim)]">
            {search.query.trim().length === 0
              ? t("search.prompt")
              : search.scope === "regex"
                ? t("search.regexOnPage")
                : t("search.noResults")}
          </p>
        ) : (
          <ul>
            {results.map((hit, index) => (
              <li key={`${hit.file_id}:${hit.page}:${index}`}>
                <button
                  type="button"
                  onClick={() => goTo(index)}
                  className={[
                    "w-full text-left px-3 py-2 border-b border-[var(--izul-border)]",
                    index === search.cursor
                      ? "bg-[var(--izul-surface-raised)]"
                      : "hover:bg-[var(--izul-surface-raised)]",
                  ].join(" ")}
                >
                  <div className="flex items-baseline gap-2">
                    <span className="text-[12px] text-[var(--izul-text-dim)]">
                      {t("status.page")} {hit.page + 1}
                    </span>
                    {search.scope === "library" && (
                      <span className="text-[12px] truncate">{hit.name}</span>
                    )}
                  </div>
                  <p className="text-[13px] leading-snug mt-0.5 line-clamp-3">{hit.snippet}</p>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      {results.length > 0 && (
        <div className="flex items-center gap-2 p-2 border-t border-[var(--izul-border)] text-[12px]">
          <span className="text-[var(--izul-text-dim)]">
            {search.cursor + 1} / {results.length}
          </span>
          <button
            type="button"
            aria-label={t("search.previous")}
            onClick={() => store().gotoResult(-1)}
            className="ml-auto px-2 py-1 rounded-[8px] hover:bg-[var(--izul-surface-raised)]"
          >
            ‹
          </button>
          <button
            type="button"
            aria-label={t("search.next")}
            onClick={() => store().gotoResult(1)}
            className="px-2 py-1 rounded-[8px] hover:bg-[var(--izul-surface-raised)]"
          >
            ›
          </button>
        </div>
      )}
    </aside>
  );
}
