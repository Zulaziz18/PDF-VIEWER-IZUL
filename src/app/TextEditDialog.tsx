/**
 * "Edit Teks" (Phase 7): what the selected text becomes, where the result
 * goes, and — when the backend refuses — why, with the dialog left open so
 * the text can be changed and tried again.
 */

import { useEffect, useRef, useState, type JSX } from "react";
import { Icon } from "@/design/Icon";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { useUi } from "@/state/uiStore";
import { Choice } from "./ExportDialog";
import { nameOf } from "./fileActions";
import { editedName, replaceText } from "./textEdit";

export function TextEditDialog(): JSX.Element | null {
  const sel = useUi((s) => s.editingText);
  const doc = useDocument((s) => s.doc);
  const path = useDocument((s) => s.path);
  const ref = useRef<HTMLDialogElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [text, setText] = useState("");
  const [asCopy, setAsCopy] = useState(true);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (sel === null) return;
    setText(sel.text);
    setAsCopy(true);
    setRunning(false);
    setError(null);
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    input.current?.select();
  }, [sel]);

  if (sel === null || doc === null) return null;
  const close = (): void => {
    ref.current?.close();
    useUi.getState().setEditingText(null);
  };
  const go = async (): Promise<void> => {
    if (running || text === sel.text) return;
    setRunning(true);
    setError(null);
    const outcome = await replaceText(doc, sel, text, asCopy);
    setRunning(false);
    if (outcome === "aborted") return;
    if (outcome.ok) close();
    else setError(outcome.error);
  };

  return (
    <dialog
      ref={ref}
      aria-labelledby="textedit-title"
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
          <Icon name="editText" size={28} tone="blue" />
          <h2 id="textedit-title" className="text-[17px] font-semibold">
            {t("textedit.title")}
          </h2>
        </div>
        <p className="text-[13px] text-[var(--izul-text-dim)]">{t("textedit.explain")}</p>

        <div className="flex flex-col gap-1.5">
          <span className="text-[12px] font-semibold text-[var(--izul-text-dim)]">{t("textedit.was")}</span>
          <p className="px-2 py-1.5 rounded-[6px] bg-[var(--izul-surface-raised)] text-[13px] break-words">{sel.text}</p>
        </div>
        <label className="flex flex-col gap-1.5">
          <span className="text-[12px] font-semibold text-[var(--izul-text-dim)]">{t("textedit.becomes")}</span>
          <input
            ref={input}
            value={text}
            disabled={running}
            onChange={(e) => {
              setText(e.target.value.replace(/[\r\n]+/gu, " "));
              setError(null);
            }}
            className="h-9 px-2 rounded-[6px] text-[14px] border border-[var(--izul-border)] bg-[var(--izul-canvas)]"
          />
        </label>

        {error !== null && (
          <p role="alert" className="flex gap-2 p-3 rounded-[8px] bg-[var(--izul-danger-soft)] text-[13px]">
            <Icon name="warning" size={20} tone="rose" />
            <span>{error}</span>
          </p>
        )}

        <fieldset className="flex flex-col gap-1" disabled={running}>
          <legend className="mb-1.5 text-[12px] font-semibold text-[var(--izul-text-dim)]">{t("redact.where")}</legend>
          <Choice name="textedit-where" checked={asCopy} onChange={() => setAsCopy(true)}>
            {t("redact.asCopy")}
          </Choice>
          <p className="ml-6 -mt-1 mb-1 text-[12px] text-[var(--izul-text-dim)] truncate" title={path ?? ""}>
            {path ? nameOf(editedName(path)) : ""}
          </p>
          <Choice name="textedit-where" checked={!asCopy} onChange={() => setAsCopy(false)}>
            {t("textedit.overwrite")}
          </Choice>
        </fieldset>

        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={close}
            disabled={running}
            className="h-9 px-4 rounded-[8px] text-[13px] border border-[var(--izul-border)] hover:bg-[var(--izul-surface-raised)] disabled:opacity-40"
          >
            {t("close.cancel")}
          </button>
          <button
            type="submit"
            disabled={running || text === sel.text}
            className="h-9 px-4 rounded-[8px] text-[13px] bg-[var(--izul-accent)] text-[var(--izul-on-accent)] font-medium hover:brightness-110 disabled:opacity-40"
          >
            {running ? t("textedit.running") : t("textedit.go")}
          </button>
        </div>
      </form>
    </dialog>
  );
}
