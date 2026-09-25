/**
 * "Kenali Teks (OCR)" (Phase 7): scanned pages made searchable.
 *
 * Recognition runs in the backend as part of a save (`ocr_apply`), one page
 * at a time; this module picks the target, watches the progress, and reopens
 * the tab on the result — its text layer, search results and selection were
 * all made from a file without the words.
 */

import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { t } from "@/i18n";
import type { SaveReport } from "@/state/documentSession";
import { useUi } from "@/state/uiStore";
import { useWorkspace } from "@/state/workspaceStore";
import { fill, nameOf } from "./fileActions";
import { reopen } from "./redaction";

/** The message a cancelled run fails with — `saving::OCR_CANCELLED`. */
export const OCR_CANCELLED = "OCR dibatalkan; berkas tidak diubah.";

export interface OcrProgress {
  readonly running: boolean;
  readonly done: number;
  readonly total: number;
}

export interface OcrRequest {
  /** Write a new file beside the original (asks where) rather than over it. */
  readonly asCopy: boolean;
  /** 0-based pages, or `null` for all. */
  readonly pages: readonly number[] | null;
  /** Read pages that already have text too. */
  readonly force: boolean;
  readonly onProgress?: (p: OcrProgress) => void;
}

export type OcrOutcome = "done" | "cancelled" | "failed" | "aborted";

/** Whether the models are installed. */
export function ocrAvailable(): Promise<boolean> {
  return invoke<boolean>("ocr_available");
}

/** `<name> (OCR).pdf` beside `path`. */
export function ocredName(path: string): string {
  const dot = path.toLowerCase().endsWith(".pdf") ? path.length - 4 : path.length;
  return `${path.slice(0, dot)}${t("ocr.suffix")}.pdf`;
}

export function cancelOcr(doc: number): Promise<void> {
  return invoke("ocr_cancel", { doc });
}

/** The notice for a finished run. */
export function ocrDoneText(report: SaveReport): string {
  const o = report.ocr;
  if (!o) return t("ocr.doneNone");
  const base =
    o.pages_read > 0
      ? fill(t("ocr.done"), { words: o.words, pages: o.pages_read, name: nameOf(report.path) })
      : t("ocr.doneNone");
  return o.pages_had_text > 0 ? `${base} ${fill(t("ocr.skipped"), { count: o.pages_had_text })}` : base;
}

/**
 * Runs OCR on `doc` and saves the result. `"aborted"` when the user closed
 * the file dialog; `"cancelled"` when they stopped the run (nothing written).
 */
export async function applyOcr(doc: number, req: OcrRequest): Promise<OcrOutcome> {
  const tab = useWorkspace.getState().tabs.find((x) => x.doc === doc);
  if (!tab) return "failed";
  let target: string | null = null;
  if (req.asCopy) {
    const chosen = await saveDialog({
      defaultPath: ocredName(tab.path),
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (chosen === null) return "aborted";
    target = /\.pdf$/i.test(chosen) ? chosen : `${chosen}.pdf`;
  }
  let polling = true;
  const poll = async (): Promise<void> => {
    while (polling) {
      await new Promise((r) => setTimeout(r, 250));
      if (!polling) return;
      try {
        req.onProgress?.(await invoke<OcrProgress>("ocr_progress", { doc }));
      } catch {
        // A missed reading is not worth stopping the run for.
      }
    }
  };
  void poll();
  let report: SaveReport;
  try {
    report = await invoke<SaveReport>("ocr_apply", {
      doc,
      target,
      pages: req.pages === null ? null : [...req.pages],
      force: req.force,
    });
  } catch (e) {
    polling = false;
    const text = String(e);
    if (text.includes(OCR_CANCELLED)) {
      useUi.getState().notify({ kind: "ok", text: OCR_CANCELLED });
      return "cancelled";
    }
    useUi.getState().notify({ kind: "error", text: t("ocr.failed"), detail: text });
    return "failed";
  }
  polling = false;
  await reopen(doc, report.path);
  useUi.getState().notify({ kind: "ok", text: ocrDoneText(report) });
  return "done";
}
