/**
 * "Cetak" (Ctrl+P, SPEC 11.1): which pages, with or without annotations, at
 * what scale and quality. The printer, copies, paper and the preview come
 * after, in Windows' own print dialog — the one every other program uses.
 */

import { useEffect, useMemo, useRef, useState, type JSX } from "react";
import { Icon } from "@/design/Icon";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { parsePageRange } from "@/state/pageRange";
import { useUi } from "@/state/uiStore";
import { Choice, rangeMessage } from "./ExportDialog";
import { fill } from "./fileActions";
import { printDocument, type PrintScale } from "./print";

type Scope = "all" | "current" | "range";

export function PrintDialog(): JSX.Element | null {
  const open = useUi((s) => s.printing);
  const doc = useDocument((s) => s.doc);
  const page = useDocument((s) => s.page);
  const pageCount = useDocument((s) => s.pageCount);
  const ref = useRef<HTMLDialogElement>(null);
  const [scope, setScope] = useState<Scope>("all");
  const [range, setRange] = useState("");
  const [annotations, setAnnotations] = useState(true);
  const [scale, setScale] = useState<PrintScale>("fit");
  const [fine, setFine] = useState(true);
  const [running, setRunning] = useState(false);

  useEffect(() => {
    if (!open || doc === null) return;
    setScope("all");
    setRange(`${page + 1}`);
    setRunning(false);
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    // `page` seeds the range only when the dialog opens.
  }, [open, doc]);

  const parsed = useMemo(() => {
    if (scope === "all") return { ok: true as const, pages: Array.from({ length: pageCount }, (_, i) => i) };
    if (scope === "current") return { ok: true as const, pages: [page] };
    return parsePageRange(range, pageCount);
  }, [scope, range, page, pageCount]);

  if (!open || doc === null) return null;
  const close = (): void => {
    ref.current?.close();
    useUi.getState().setPrinting(false);
  };
  const go = async (): Promise<void> => {
    if (running || !parsed.ok || parsed.pages.length === 0) return;
    setRunning(true);
    const pages = parsed.pages;
    close();
    await printDocument(doc, { pages, annotations, scale, dpi: fine ? 300 : 200 });
    setRunning(false);
  };
  const count = parsed.ok ? parsed.pages.length : 0;

  return (
    <dialog
      ref={ref}
      aria-labelledby="print-title"
      onCancel={(e) => {
        e.preventDefault();
        if (!running) close();
      }}
      className="m-auto w-[480px] max-w-[calc(100vw-32px)] p-0 rounded-[12px] border border-[var(--izul-border)] bg-[var(--izul-surface)] text-[var(--izul-text)] shadow-[0_12px_40px_rgba(0,0,0,0.25)] backdrop:bg-black/30"
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
          <Icon name="print" size={28} tone="blue" />
          <h2 id="print-title" className="text-[17px] font-semibold">
            {t("print.title")}
          </h2>
        </div>

        <fieldset className="flex flex-col gap-1">
          <legend className="mb-1.5 text-[12px] font-semibold text-[var(--izul-text-dim)]">{t("print.which")}</legend>
          <Choice name="print-scope" checked={scope === "all"} onChange={() => setScope("all")}>
            {t("export.all")} ({pageCount})
          </Choice>
          <Choice name="print-scope" checked={scope === "current"} onChange={() => setScope("current")}>
            {fill(t("ocr.current"), { page: page + 1 })}
          </Choice>
          <div className="flex items-center gap-2">
            <Choice name="print-scope" checked={scope === "range"} onChange={() => setScope("range")}>
              {t("export.range")}
            </Choice>
            <input
              aria-label={t("export.range")}
              value={range}
              placeholder={t("export.rangeHint")}
              onFocus={() => setScope("range")}
              onChange={(e) => setRange(e.target.value)}
              className="h-7 w-40 px-2 rounded-[6px] text-[13px] border border-[var(--izul-border)] bg-[var(--izul-surface-raised)]"
            />
          </div>
          {scope === "range" && !parsed.ok && <p className="ml-6 text-[12px] text-[var(--izul-danger)]">{rangeMessage(parsed.error)}</p>}
        </fieldset>

        <fieldset className="flex flex-col gap-1">
          <legend className="mb-1.5 text-[12px] font-semibold text-[var(--izul-text-dim)]">{t("print.scale")}</legend>
          <Choice name="print-scale" checked={scale === "fit"} onChange={() => setScale("fit")}>
            {t("print.fit")}
          </Choice>
          <Choice name="print-scale" checked={scale === "actual"} onChange={() => setScale("actual")}>
            {t("print.actual")}
          </Choice>
        </fieldset>

        <div className="flex flex-col gap-1">
          <label className="flex items-center gap-2 h-7 text-[13px]">
            <input
              type="checkbox"
              checked={annotations}
              onChange={(e) => setAnnotations(e.target.checked)}
              className="accent-[var(--izul-accent)]"
            />
            {t("print.annotations")}
          </label>
          <label className="flex items-center gap-2 h-7 text-[13px]">
            <input type="checkbox" checked={fine} onChange={(e) => setFine(e.target.checked)} className="accent-[var(--izul-accent)]" />
            {t("print.fine")}
          </label>
        </div>

        <p className="text-[12px] text-[var(--izul-text-dim)]">{t("print.next")}</p>

        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={close}
            className="h-9 px-4 rounded-[8px] text-[13px] border border-[var(--izul-border)] hover:bg-[var(--izul-surface-raised)]"
          >
            {t("close.cancel")}
          </button>
          <button
            type="submit"
            disabled={running || !parsed.ok || count === 0}
            className="h-9 px-4 rounded-[8px] text-[13px] bg-[var(--izul-accent)] text-[var(--izul-on-accent)] font-medium hover:brightness-110 disabled:opacity-40"
          >
            {fill(t("print.go"), { count })}
          </button>
        </div>
      </form>
    </dialog>
  );
}
