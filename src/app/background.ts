/**
 * "Hapus Latar" for a picture (Phase 7). The backend runs the segmentation
 * model in a worker and answers with the edit — one undo step — and where
 * the model ran, which the notice says rather than leaving it to be guessed.
 */

import { invoke } from "@tauri-apps/api/core";
import type { EditResult } from "@/annots/types";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { useUi } from "@/state/uiStore";
import { fill } from "./fileActions";

/** What the picture is — the user says; it cannot be told from the picture. */
export type BackgroundKind = "Photo" | "OnPaper";

interface BackgroundResult {
  readonly edit: EditResult;
  readonly device: string;
  readonly paper: boolean;
}

export function backgroundAvailable(): Promise<boolean> {
  return invoke<boolean>("background_available");
}

/** The notice after a run. */
export function backgroundDoneText(kind: BackgroundKind, paper: boolean, device: string): string {
  const where = device.startsWith("DirectML") ? t("bg.gpu") : t("bg.cpu");
  if (kind === "OnPaper" && !paper) return fill(t("bg.donePlainNot"), { where });
  return fill(t("bg.done"), { where });
}

let running = false;

export async function removeBackground(id: number, kind: BackgroundKind): Promise<void> {
  const store = useDocument.getState();
  const doc = store.doc;
  if (doc === null || running) return;
  running = true;
  useUi.getState().notify({ kind: "ok", text: t("bg.running") });
  try {
    const result = await invoke<BackgroundResult>("annot_remove_background", { doc, id, kind });
    const session = useDocument.getState();
    session.applyEdit(result.edit);
    // `applyEdit` only carries the undo state; the page's objects and display
    // lists are fetched again, or the canvas keeps drawing the old picture.
    await session.loadAnnots([...new Set(result.edit.objects.map((o) => o.page))]);
    useUi.getState().notify({ kind: "ok", text: backgroundDoneText(kind, result.paper, result.device) });
  } catch (e) {
    useUi.getState().notify({ kind: "error", text: t("bg.failed"), detail: String(e) });
  } finally {
    running = false;
  }
}
