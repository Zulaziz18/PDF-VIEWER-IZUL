/**
 * "Terapkan Redaksi" (Phase 6): what is about to be taken out, where the
 * result goes, and one button to do it.
 *
 * Applying cannot be undone once written, so the dialog says what it will do
 * in plain words, counts the marks — including marks saved in the file on
 * pages not opened this session, which the backend reads in first — and
 * defaults to writing a new file beside the original.
 */

import { useEffect, useRef, useState, type JSX } from "react";
import { Icon } from "@/design/Icon";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { useUi } from "@/state/uiStore";
import { Choice } from "./ExportDialog";
import { fill, nameOf } from "./fileActions";
import { applyRedaction, previewRedaction, redactedName, type RedactionPreview } from "./redaction";

export function RedactDialog(): JSX.Element | null {
  const open = useUi((s) => s.redacting);
  const doc = useDocument((s) => s.doc);
  const path = useDocument((s) => s.path);
  const ref = useRef<HTMLDialogElement>(null);
  const [preview, setPreview] = useState<RedactionPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [asCopy, setAsCopy] = useState(true);
  const [running, setRunning] = useState(false);

  useEffect(() => {
    if (!open || doc === null) return;
    setPreview(null);
    setError(null);
    setAsCopy(true);
    setRunning(false);
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    let live = true;
    previewRedaction(doc)
      .then((p) => {
        if (live) setPreview(p);
      })
      .catch((e: unknown) => {
        if (live) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, [open, doc]);

  if (!open || doc === null) return null;
  const close = (): void => {
    ref.current?.close();
    useUi.getState().setRedacting(false);
  };
  const go = async (): Promise<void> => {
    if (running || !preview || preview.marks === 0) return;
    setRunning(true);
    const done = await applyRedaction(doc, asCopy);
    setRunning(false);
    if (done) close();
  };
  const nothing = preview !== null && preview.marks === 0;

  return (
    <dialog
      ref={ref}
      aria-labelledby="redact-title"
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
          <Icon name="redactApply" size={28} tone="rose" />
          <h2 id="redact-title" className="text-[17px] font-semibold">
            {t("redact.title")}
          </h2>
        </div>

        <div className="flex flex-col gap-2 text-[13px]" aria-live="polite">
          {error !== null ? (
            <p className="text-[var(--izul-danger)] break-words">{error}</p>
          ) : preview === null ? (
            <p className="text-[var(--izul-text-dim)]">{t("redact.loading")}</p>
          ) : nothing ? (
            <p>{t("redact.none")}</p>
          ) : (
            <>
              <p className="font-semibold">
                {fill(t("redact.summary"), { marks: preview.marks, pages: preview.pages.length })}
              </p>
              {preview.annotations > 0 && (
                <p>{fill(t("redact.alsoAnnots"), { count: preview.annotations })}</p>
              )}
            </>
          )}
          {!nothing && (
            <p className="flex gap-2 p-3 rounded-[8px] bg-[var(--izul-danger-soft)] text-[var(--izul-text)]">
              <Icon name="warning" size={20} tone="rose" />
              <span>{t("redact.warning")}</span>
            </p>
          )}
        </div>

        {!nothing && (
          <fieldset className="flex flex-col gap-1">
            <legend className="mb-1.5 text-[12px] font-semibold text-[var(--izul-text-dim)]">{t("redact.where")}</legend>
            <Choice name="redact-where" checked={asCopy} onChange={() => setAsCopy(true)}>
              {t("redact.asCopy")}
            </Choice>
            <p className="ml-6 -mt-1 mb-1 text-[12px] text-[var(--izul-text-dim)] truncate" title={path ?? ""}>
              {path ? nameOf(redactedName(path)) : ""} — {t("redact.asCopyHint")}
            </p>
            <Choice name="redact-where" checked={!asCopy} onChange={() => setAsCopy(false)}>
              {t("redact.overwrite")}
            </Choice>
            <p className="ml-6 -mt-1 text-[12px] text-[var(--izul-text-dim)]">{t("redact.overwriteHint")}</p>
          </fieldset>
        )}

        <div className="flex justify-end gap-2">
          <button
            type="button"
            disabled={running}
            onClick={close}
            className="h-9 px-4 rounded-[8px] text-[13px] border border-[var(--izul-border)] hover:bg-[var(--izul-surface-raised)] disabled:opacity-40"
          >
            {t("close.cancel")}
          </button>
          {!nothing && (
            <button
              type="submit"
              disabled={running || preview === null || preview.marks === 0}
              className="h-9 px-4 rounded-[8px] text-[13px] bg-[var(--izul-danger-fill)] text-[var(--izul-on-danger)] font-medium hover:brightness-110 disabled:opacity-40"
            >
              {running ? t("redact.running") : t("redact.go")}
            </button>
          )}
        </div>
      </form>
    </dialog>
  );
}
