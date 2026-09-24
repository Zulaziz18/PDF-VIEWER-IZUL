/**
 * "Pecah Dokumen" (SPEC 11.3): one PDF per part — every N pages, by ranges
 * the user types, or one per top-level bookmark. The parts are written as
 * `<name>-1.pdf`, `<name>-2.pdf`… into a folder the platform picker asks for.
 *
 * What the choice produces is shown before anything is written ("4 berkas:
 * 1-3, 4-6, …"), because a split is many files and a mistake is many files
 * to delete by hand.
 */

import { useEffect, useMemo, useRef, useState, type JSX } from "react";
import { Icon } from "@/design/Icon";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { formatPageRange, parseRangeGroups, rangesEvery, rangesFromOutline } from "@/state/pageRange";
import { useUi } from "@/state/uiStore";
import { Choice, rangeMessage } from "./ExportDialog";
import { exportSplit } from "./fileActions";

type Mode = "every" | "ranges" | "bookmarks";

export function SplitDialog(): JSX.Element | null {
  const kind = useUi((s) => s.exporting);
  const doc = useDocument((s) => s.doc);
  const pageCount = useDocument((s) => s.pageCount);
  const outline = useDocument((s) => s.outline);
  const ref = useRef<HTMLDialogElement>(null);
  const [mode, setMode] = useState<Mode>("every");
  const [every, setEvery] = useState(1);
  const [ranges, setRanges] = useState("");
  const [running, setRunning] = useState(false);

  // Bookmarks point at the file's pages; after a rearrangement some point
  // elsewhere, and some nowhere.
  const byBookmark = useMemo(() => {
    const store = useDocument.getState();
    const shown = outline.map((e) => ({
      depth: e.depth,
      page: e.page === null ? null : store.displayOfOwn(e.page),
    }));
    return rangesFromOutline(shown, pageCount);
  }, [outline, pageCount]);

  useEffect(() => {
    if (kind !== "split") return;
    setMode(byBookmark.length > 1 ? "bookmarks" : "every");
    setEvery(Math.max(1, Math.min(10, Math.ceil(pageCount / 2))));
    setRanges("");
    setRunning(false);
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    // Only when it opens.
  }, [kind]);

  const plan = useMemo(() => {
    if (mode === "every") return { ok: true as const, groups: rangesEvery(every, pageCount) };
    if (mode === "bookmarks") return { ok: true as const, groups: byBookmark };
    const r = parseRangeGroups(ranges, pageCount);
    return r.ok ? { ok: true as const, groups: r.groups } : r;
  }, [mode, every, ranges, pageCount, byBookmark]);

  if (kind !== "split" || doc === null) return null;
  const close = (): void => {
    ref.current?.close();
    useUi.getState().setExporting(null);
  };
  const go = async (): Promise<void> => {
    if (!plan.ok || plan.groups.length === 0 || running) return;
    setRunning(true);
    const done = await exportSplit(doc, plan.groups);
    setRunning(false);
    if (done) close();
  };

  const preview = plan.ok
    ? plan.groups.length === 0
      ? t("split.noParts")
      : `${plan.groups.length} ${t("split.files")}: ${plan.groups
          .slice(0, 6)
          .map((g) => formatPageRange(g))
          .join(" · ")}${plan.groups.length > 6 ? " · …" : ""}`
    : `${t("split.group")} ${plan.group + 1}: ${rangeMessage(plan.error)}`;

  return (
    <dialog
      ref={ref}
      aria-labelledby="split-title"
      onCancel={(e) => {
        e.preventDefault();
        if (!running) close();
      }}
      className="m-auto w-[500px] max-w-[calc(100vw-32px)] p-0 rounded-[12px] border border-[var(--izul-border)] bg-[var(--izul-surface)] text-[var(--izul-text)] shadow-[0_12px_40px_rgba(0,0,0,0.25)] backdrop:bg-black/30"
    >
      <form
        method="dialog"
        onSubmit={(e) => {
          e.preventDefault();
          void go();
        }}
        className="p-6 flex flex-col gap-5"
      >
        <div className="flex items-center gap-3">
          <Icon name="split" size={28} tone="orange" />
          <h2 id="split-title" className="text-[17px] font-semibold">
            {t("split.title")}
          </h2>
        </div>

        <fieldset className="flex flex-col gap-1">
          <legend className="mb-1.5 text-[12px] font-semibold text-[var(--izul-text-dim)]">{t("split.how")}</legend>
          <div className="flex items-center gap-2">
            <Choice name="split" checked={mode === "every"} onChange={() => setMode("every")}>
              {t("split.every")}
            </Choice>
            <input
              type="number"
              min={1}
              max={Math.max(1, pageCount)}
              aria-label={t("split.everyCount")}
              value={every}
              onFocus={() => setMode("every")}
              onChange={(e) => setEvery(Math.max(1, Number(e.target.value) || 1))}
              className="w-16 h-7 px-2 text-[13px] rounded-[6px] bg-[var(--izul-surface)] border border-[var(--izul-border)]"
            />
            <span className="text-[13px]">{t("split.pagesEach")}</span>
          </div>
          <Choice name="split" checked={mode === "ranges"} onChange={() => setMode("ranges")}>
            {t("split.ranges")}
          </Choice>
          <input
            type="text"
            aria-label={t("split.ranges")}
            value={ranges}
            placeholder={t("split.rangesHint")}
            onFocus={() => setMode("ranges")}
            onChange={(e) => setRanges(e.target.value)}
            className="ml-6 h-8 px-2 text-[13px] rounded-[6px] bg-[var(--izul-surface)] border border-[var(--izul-border)] focus:border-[var(--izul-accent)] outline-none"
          />
          <Choice
            name="split"
            checked={mode === "bookmarks"}
            disabled={byBookmark.length === 0}
            onChange={() => setMode("bookmarks")}
          >
            {t("split.bookmarks")}
          </Choice>
          {byBookmark.length === 0 && (
            <p className="ml-6 text-[12px] text-[var(--izul-text-dim)]">{t("sidebar.noOutline")}</p>
          )}
        </fieldset>

        <p
          aria-live="polite"
          className={[
            "min-h-[36px] text-[12px] break-words",
            plan.ok && plan.groups.length > 0 ? "text-[var(--izul-text-dim)]" : "text-[var(--izul-danger)]",
          ].join(" ")}
        >
          {preview}
        </p>

        <div className="flex justify-end gap-2">
          <button
            type="button"
            disabled={running}
            onClick={close}
            className="h-9 px-4 rounded-[8px] text-[13px] border border-[var(--izul-border)] hover:bg-[var(--izul-surface-raised)] disabled:opacity-40"
          >
            {t("close.cancel")}
          </button>
          <button
            type="submit"
            disabled={!plan.ok || plan.groups.length === 0 || running}
            className="h-9 px-4 rounded-[8px] text-[13px] bg-[var(--izul-accent)] text-[var(--izul-on-accent)] font-medium hover:brightness-110 disabled:opacity-40"
          >
            {running ? t("export.running") : t("export.goImages")}
          </button>
        </div>
      </form>
    </dialog>
  );
}
