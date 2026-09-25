/**
 * "Edit Teks" (Phase 7): replacing a document's own text, within one line.
 *
 * The replacement is made while saving — like OCR and redaction — because it
 * has to be exact: the rest of the line where it was, the new text in the
 * file's own embedded font. The backend refuses what it cannot do that way
 * (a font not embedded, a letter the font lacks, text that would run into
 * the next word) and says why; the dialog shows that reason and stays open.
 */

import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { PdfRect } from "@/annots/types";
import { t } from "@/i18n";
import type { SaveReport } from "@/state/documentSession";
import { useUi, type TextSelection } from "@/state/uiStore";
import { useWorkspace } from "@/state/workspaceStore";
import { fill } from "./fileActions";
import { reopen } from "./redaction";
import { viewport } from "./viewportHandle";

export type { TextSelection };

/** Whether boxes lie on one line: each overlaps the first vertically. */
export function oneLine(quads: readonly PdfRect[]): boolean {
  const first = quads[0];
  if (!first) return false;
  return quads.every((q) => Math.min(q.top, first.top) - Math.max(q.bottom, first.bottom) > 0);
}

export function union(quads: readonly PdfRect[]): PdfRect | null {
  if (quads.length === 0) return null;
  return quads.reduce((a, b) => ({
    left: Math.min(a.left, b.left),
    bottom: Math.min(a.bottom, b.bottom),
    right: Math.max(a.right, b.right),
    top: Math.max(a.top, b.top),
  }));
}

/** The text selected in the viewport, or why there is none to edit. */
export function currentTextSelection(): TextSelection | "none" | "lines" {
  const found = viewport()?.selectionQuads();
  const text = document.getSelection()?.toString().trim() ?? "";
  if (!found || text.length === 0) return "none";
  if (!oneLine(found.quads) || /[\r\n]/u.test(text)) return "lines";
  const rect = union(found.quads);
  return rect === null ? "none" : { page: found.page, rect, text };
}

/** What the "Edit Teks" button does. */
export function startTextEdit(): void {
  const sel = currentTextSelection();
  const ui = useUi.getState();
  if (sel === "none") ui.notify({ kind: "ok", text: t("textedit.selectFirst") });
  else if (sel === "lines") ui.notify({ kind: "error", text: t("textedit.oneLine") });
  else ui.setEditingText(sel);
}

/** `<name> (disunting).pdf` beside `path`. */
export function editedName(path: string): string {
  const dot = path.toLowerCase().endsWith(".pdf") ? path.length - 4 : path.length;
  return `${path.slice(0, dot)}${t("textedit.suffix")}.pdf`;
}

export type ReplaceOutcome = { readonly ok: true } | { readonly ok: false; readonly error: string } | "aborted";

/** Replaces the selection's text and saves; reopens the tab on the result. */
export async function replaceText(
  doc: number,
  sel: TextSelection,
  text: string,
  asCopy: boolean,
): Promise<ReplaceOutcome> {
  const tab = useWorkspace.getState().tabs.find((x) => x.doc === doc);
  if (!tab) return { ok: false, error: t("textedit.noTab") };
  let target: string | null = null;
  if (asCopy) {
    const chosen = await saveDialog({
      defaultPath: editedName(tab.path),
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (chosen === null) return "aborted";
    target = /\.pdf$/iu.test(chosen) ? chosen : `${chosen}.pdf`;
  }
  let report: SaveReport;
  try {
    report = await invoke<SaveReport>("text_replace", {
      doc,
      page: sel.page,
      rect: sel.rect,
      text,
      target,
    });
  } catch (e) {
    return { ok: false, error: String(e) };
  }
  await reopen(doc, report.path);
  useUi.getState().notify({
    kind: "ok",
    text: fill(t("textedit.done"), { before: report.text?.before ?? sel.text, after: text }),
  });
  return { ok: true };
}
