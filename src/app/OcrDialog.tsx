/**
 * "Kenali Teks (OCR)" (Phase 7): which pages, where the result goes, and the
 * progress while it runs — with a way to stop that leaves the file as it was.
 */

import { useEffect, useMemo, useRef, useState, type JSX } from "react";
import { Icon } from "@/design/Icon";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { parsePageRange } from "@/state/pageRange";
import { useUi } from "@/state/uiStore";
import { Choice, rangeMessage } from "./ExportDialog";
import { fill, nameOf } from "./fileActions";
import { applyOcr, cancelOcr, ocrAvailable, ocredName, type OcrProgress } from "./ocr";

type Scope = "all" | "current" | "range";

export function OcrDialog(): JSX.Element | null {
  const open = useUi((s) => s.ocring);
  const doc = useDocument((s) => s.doc);
  const path = useDocument((s) => s.path);
  const page = useDocument((s) => s.page);
  const pageCount = useDocument((s) => s.pageCount);
  const ref = useRef<HTMLDialogElement>(null);
  const [available, setAvailable] = useState<boolean | null>(null);
  const [scope, setScope] = useState<Scope>("all");
  const [range, setRange] = useState("");
  const [force, setForce] = useState(false);
  const [asCopy, setAsCopy] = useState(true);
  const [progress, setProgress] = useState<OcrProgress | null>(null);
  const [running, setRunning] = useState(false);

  useEffect(() => {
    if (!open || doc === null) return;
    setScope("all");
    setRange(`${page + 1}`);
    setForce(false);
    setAsCopy(true);
    setProgress(null);
    setRunning(false);
    setAvailable(null);
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    let live = true;
    ocrAvailable()
      .then((a) => live && setAvailable(a))
      .catch(() => live && setAvailable(false));
    return () => {
      live = false;
    };
    // `page` only seeds the range when the dialog opens, so it is not a
    // dependency: turning pages behind the dialog must not reset it.
  }, [open, doc]);

  const parsed = useMemo(() => {
    if (scope === "all") return { ok: true as const, pages: null };
    if (scope === "current") return { ok: true as const, pages: [page] };
    return parsePageRange(range, pageCount);
  }, [scope, range, page, pageCount]);

  if (!open || doc === null) return null;
  const close = (): void => {
    ref.current?.close();
    useUi.getState().setOcring(false);
  };
  const go = async (): Promise<void> => {
    if (running || !parsed.ok || available !== true) return;
    setRunning(true);
    const outcome = await applyOcr(doc, { asCopy, pages: parsed.pages, force, onProgress: setProgress });
    setRunning(false);
    setProgress(null);
    if (outcome === "done" || outcome === "cancelled") close();
  };
  const count = parsed.ok ? (parsed.pages === null ? pageCount : parsed.pages.length) : 0;

  return (
    <dialog
      ref={ref}
      aria-labelledby="ocr-title"
      onCancel={(e) => {
        e.preventDefault();
        if (!running) close();
      }}
      className="m-auto w-[520px] max-w-[calc(100vw-32px)] p-0 rounded-[12px] border border-[var(--izul-border)] bg-[var(--izul-surface)] text-[var(--izul-text)] shadow-[0_12px_40px_rgba(0,0,0,0.25)] backdrop:bg-black/30"
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
          <Icon name="ocr" size={28} tone="teal" />
          <h2 id="ocr-title" className="text-[17px] font-semibold">
            {t("ocr.title")}
          </h2>
        </div>
        <p className="text-[13px] text-[var(--izul-text-dim)]">{t("ocr.explain")}</p>

        {available === false ? (
          <p className="flex gap-2 p-3 rounded-[8px] bg-[var(--izul-danger-soft)] text-[13px]">
            <Icon name="warning" size={20} tone="rose" />
            <span>{t("ocr.noModels")}</span>
          </p>
        ) : (
          <>
            <fieldset className="flex flex-col gap-1" disabled={running}>
              <legend className="mb-1.5 text-[12px] font-semibold text-[var(--izul-text-dim)]">{t("ocr.which")}</legend>
              <Choice name="ocr-scope" checked={scope === "all"} onChange={() => setScope("all")}>
                {t("export.all")} ({pageCount})
              </Choice>
              <Choice name="ocr-scope" checked={scope === "current"} onChange={() => setScope("current")}>
                {fill(t("ocr.current"), { page: page + 1 })}
              </Choice>
              <div className="flex items-center gap-2">
                <Choice name="ocr-scope" checked={scope === "range"} onChange={() => setScope("range")}>
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
              {scope === "range" && !parsed.ok && (
                <p className="ml-6 text-[12px] text-[var(--izul-danger)]">{rangeMessage(parsed.error)}</p>
              )}
              <label className="flex items-center gap-2 h-7 mt-1 text-[13px]">
                <input
                  type="checkbox"
                  checked={force}
                  onChange={(e) => setForce(e.target.checked)}
                  className="accent-[var(--izul-accent)]"
                />
                {t("ocr.force")}
              </label>
            </fieldset>

            <fieldset className="flex flex-col gap-1" disabled={running}>
              <legend className="mb-1.5 text-[12px] font-semibold text-[var(--izul-text-dim)]">{t("redact.where")}</legend>
              <Choice name="ocr-where" checked={asCopy} onChange={() => setAsCopy(true)}>
                {t("redact.asCopy")}
              </Choice>
              <p className="ml-6 -mt-1 mb-1 text-[12px] text-[var(--izul-text-dim)] truncate" title={path ?? ""}>
                {path ? nameOf(ocredName(path)) : ""}
              </p>
              <Choice name="ocr-where" checked={!asCopy} onChange={() => setAsCopy(false)}>
                {t("ocr.overwrite")}
              </Choice>
            </fieldset>
          </>
        )}

        {running && (
          <div className="flex flex-col gap-1.5" aria-live="polite">
            <p className="text-[13px]">
              {progress && progress.total > 0
                ? fill(t("ocr.progress"), { done: Math.min(progress.done + 1, progress.total), total: progress.total })
                : t("ocr.starting")}
            </p>
            <div className="h-1.5 rounded-full bg-[var(--izul-surface-raised)] overflow-hidden">
              <div
                className="h-full bg-[var(--izul-accent)] transition-[width] duration-150"
                style={{ width: `${progress && progress.total > 0 ? (100 * progress.done) / progress.total : 0}%` }}
              />
            </div>
          </div>
        )}

        <div className="flex justify-end gap-2">
          {running ? (
            <button
              type="button"
              onClick={() => void cancelOcr(doc)}
              className="h-9 px-4 rounded-[8px] text-[13px] border border-[var(--izul-border)] hover:bg-[var(--izul-surface-raised)]"
            >
              {t("ocr.stop")}
            </button>
          ) : (
            <button
              type="button"
              onClick={close}
              className="h-9 px-4 rounded-[8px] text-[13px] border border-[var(--izul-border)] hover:bg-[var(--izul-surface-raised)]"
            >
              {t("close.cancel")}
            </button>
          )}
          {available !== false && (
            <button
              type="submit"
              disabled={running || !parsed.ok || available !== true || count === 0}
              className="h-9 px-4 rounded-[8px] text-[13px] bg-[var(--izul-accent)] text-[var(--izul-on-accent)] font-medium hover:brightness-110 disabled:opacity-40"
            >
              {running ? t("ocr.running") : fill(t("ocr.go"), { count })}
            </button>
          )}
        </div>
      </form>
    </dialog>
  );
}
