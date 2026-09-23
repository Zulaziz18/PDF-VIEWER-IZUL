/**
 * What the "Halaman" ribbon does (SPEC 11.3, Phase 5).
 *
 * Every operation acts on the pages selected in the page panel, or — with
 * nothing selected — on the page being read, which is what a user who has
 * never opened the panel expects "Hapus Halaman" to mean. Each is one undo
 * step, and nothing touches the file until it is saved.
 */

import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { targetPages } from "@/state/pageSelection";
import { useUi } from "@/state/uiStore";

function fill(text: string, values: Record<string, string | number>): string {
  return text.replace(/\{(\w+)\}/g, (hole, key: string) => String(values[key] ?? hole));
}

function targets(): number[] {
  const s = useDocument.getState();
  return targetPages(s.pageSelection, s.page);
}

/** A blank page after the current one, the size of the current one. */
export async function insertBlankPage(): Promise<void> {
  const s = useDocument.getState();
  const size = s.pageSizes[s.page] ?? { width: 595, height: 842 };
  const ok = await s.pageCommand({
    kind: "insertBlank",
    at: s.page + 1,
    count: 1,
    width: size.width,
    height: size.height,
  });
  if (ok) useDocument.getState().setPage(s.page + 1);
}

export async function deletePages(): Promise<void> {
  const pages = targets();
  const ok = await useDocument.getState().pageCommand({ kind: "delete", pages });
  if (ok) {
    useDocument.getState().setPageSelection([]);
    // Undo is the safety net, so it is named rather than a confirmation asked.
    useUi.getState().notify({ kind: "ok", text: fill(t("pages.deleted"), { count: pages.length }) });
  }
}

export async function rotatePages(quarters: number): Promise<void> {
  await useDocument.getState().pageCommand({ kind: "rotate", pages: targets(), quarters });
}

export async function duplicatePages(): Promise<void> {
  const pages = targets();
  const ok = await useDocument.getState().pageCommand({ kind: "duplicate", pages });
  if (ok) {
    useUi.getState().notify({ kind: "ok", text: fill(t("pages.duplicated"), { count: pages.length }) });
  }
}

/** "Gabung": every page of another PDF, after the current page. */
export async function mergeFile(): Promise<void> {
  const chosen = await openDialog({
    multiple: false,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (typeof chosen !== "string") return;
  const s = useDocument.getState();
  const before = s.pageCount;
  const ok = await s.insertFile(chosen, s.page + 1);
  if (ok) {
    const added = useDocument.getState().pageCount - before;
    useUi.getState().notify({
      kind: "ok",
      text: fill(t("pages.merged"), { count: added, name: chosen.split(/[\\/]/).pop() ?? chosen }),
    });
  }
}

export function extractPages(): void {
  useUi.getState().setExporting("pages");
}

export function splitDocument(): void {
  useUi.getState().setExporting("split");
}
