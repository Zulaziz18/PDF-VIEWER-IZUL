/**
 * The annotation list (SPEC 11.1's sidebar, SPEC 11.2's panel).
 *
 * Grouped by page, filterable by kind, and clicking an entry both selects the
 * object and takes the viewport to it. It is the only way to find an annotation
 * on page 340 of a document without scrolling there, which is what makes it
 * worth its space rather than a duplicate of what is already on screen.
 *
 * It lists what the *store* has, which is the pages that have been looked at.
 * A document whose page 340 has never been on screen has nothing to list there
 * yet, and saying so plainly beats pretending the list is complete.
 */

import { useEffect, useState } from "react";
import { useDocument } from "@/state/documentStore";
import type { AnnotKind, AnnotObject } from "@/annots/types";
import { ANNOT_KINDS, cssColor } from "@/annots/types";
import { viewport } from "./viewportHandle";
import { t } from "@/i18n";

/** A one-line description of an object, for the list. */
function describe(obj: AnnotObject): string {
  const p = obj.payload;
  if ("FreeText" in p && p.FreeText.text.trim().length > 0) return p.FreeText.text;
  if ("Stamp" in p) return p.Stamp.label;
  if ("Note" in p && p.Note.text.trim().length > 0) return p.Note.text;
  if (obj.author_note.trim().length > 0) return obj.author_note;
  return t(`kind.${obj.kind}` as never);
}

function swatchOf(obj: AnnotObject): string | null {
  const p = obj.payload;
  const color =
    "Markup" in p
      ? p.Markup.color
      : "FreeText" in p
        ? p.FreeText.color
        : "Ink" in p
          ? p.Ink.color
          : "Line" in p
            ? p.Line.color
            : "Shape" in p
              ? p.Shape.style.stroke
              : "Polygon" in p
                ? p.Polygon.style.stroke
                : "Note" in p
                  ? p.Note.color
                  : "Stamp" in p
                    ? p.Stamp.color
                    : null;
  return color ? cssColor(color) : null;
}

export function AnnotationList(): React.JSX.Element {
  const annots = useDocument((s) => s.annots);
  const selection = useDocument((s) => s.selection);
  const pageCount = useDocument((s) => s.pageCount);
  const [filter, setFilter] = useState<AnnotKind | "all">("all");
  const store = useDocument.getState;

  // The pages nobody has scrolled to have not been fetched, so the list would
  // silently under-report. Asking for all of them once the panel is open is a
  // handful of cheap calls and makes the count honest.
  useEffect(() => {
    const pages = Array.from({ length: pageCount }, (_, i) => i).filter((p) => !annots.has(p));
    if (pages.length > 0) void store().loadAnnots(pages.slice(0, 200));
    // Only when the document changes size, not on every annotation edit.
  }, [pageCount]);

  const pages = [...annots.entries()]
    .map(([page, objects]) => ({
      page,
      objects: objects.filter((o) => filter === "all" || o.kind === filter),
    }))
    .filter((entry) => entry.objects.length > 0)
    .sort((a, b) => a.page - b.page);

  const total = pages.reduce((sum, entry) => sum + entry.objects.length, 0);

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="p-2 border-b border-[var(--izul-border)]">
        <select
          aria-label={t("annot.filter")}
          value={filter}
          onChange={(e) => setFilter(e.target.value as AnnotKind | "all")}
          className="w-full rounded-[8px] bg-[var(--izul-canvas)] border border-[var(--izul-border)] px-2 py-1 text-[12px]"
        >
          <option value="all">{t("annot.allKinds")}</option>
          {ANNOT_KINDS.map((kind) => (
            <option key={kind} value={kind}>
              {t(`kind.${kind}` as never)}
            </option>
          ))}
        </select>
      </div>

      {total === 0 ? (
        <p className="p-3 text-[13px] text-[var(--izul-text-dim)]">{t("annot.empty")}</p>
      ) : (
        <ul className="flex-1 min-h-0 overflow-auto">
          {pages.map((entry) => (
            <li key={entry.page}>
              <h3 className="px-3 py-1 text-[11px] uppercase tracking-wide text-[var(--izul-text-dim)] bg-[var(--izul-surface)] sticky top-0">
                {t("status.page")} {entry.page + 1}
              </h3>
              <ul>
                {entry.objects.map((obj) => {
                  const swatch = swatchOf(obj);
                  return (
                    <li key={obj.id}>
                      <button
                        type="button"
                        onClick={() => {
                          store().select([obj.id]);
                          store().setPage(obj.page);
                          viewport()?.goToPage(obj.page);
                        }}
                        className={[
                          "w-full text-left px-3 py-1.5 flex items-center gap-2 text-[13px]",
                          selection.includes(obj.id)
                            ? "bg-[var(--izul-surface-raised)]"
                            : "hover:bg-[var(--izul-surface-raised)]",
                        ].join(" ")}
                      >
                        {swatch !== null && (
                          <span
                            aria-hidden="true"
                            style={{ background: swatch }}
                            className="w-3 h-3 rounded-[3px] shrink-0 border border-[var(--izul-border)]"
                          />
                        )}
                        <span className="truncate">{describe(obj)}</span>
                        {obj.locked && (
                          <span aria-label={t("props.locked")} className="ml-auto text-[11px]">
                            🔒
                          </span>
                        )}
                      </button>
                    </li>
                  );
                })}
              </ul>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
