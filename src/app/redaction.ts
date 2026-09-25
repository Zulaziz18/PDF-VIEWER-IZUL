/**
 * Redaction (Phase 6): marking what goes, and applying the marks.
 *
 * A mark is an annotation (`Redact`) until it is applied: it can be moved,
 * resized, deleted and undone like any other, and saving keeps it as a
 * standard `/Redact` annotation that other editors understand. Applying is
 * different in kind — what lies under the marks is taken out of the file
 * (`izul-redact`, checked by the worker with PDFium), and there is no undo
 * afterwards. So applying always goes through a dialog, and by default writes
 * a new file next to the original rather than over it.
 *
 * After applying, the tab reopens its file: every bitmap, text layer and
 * search result it held was made from content that no longer exists.
 */

import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { AnnotObject, EditResult, PdfRect } from "@/annots/types";
import { REDACT_FILL, NEW_OBJECT_ID } from "@/annots/types";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import type { SaveReport } from "@/state/documentSession";
import { useUi } from "@/state/uiStore";
import { useWorkspace } from "@/state/workspaceStore";
import { markupSelection } from "./actions";
import { fill, nameOf } from "./fileActions";

/** What applying would take out, as `redact_preview` reports it. */
export interface RedactionPreview {
  readonly marks: number;
  readonly pages: readonly number[];
  readonly annotations: number;
}

/** "Tandai Teks": the selection now, or the next one. */
export function markText(): void {
  markupSelection("Redact");
}

/** "Tandai Area": the drag tool, or back to the arrow if it is already on. */
export function toggleAreaTool(): void {
  useUi.getState().armMarkup(null);
  const store = useDocument.getState();
  store.setTool(store.tool === "Redact" ? null : "Redact");
}

/** "Cari & Tandai": the search panel, where the marking button lives. */
export function openSearchForMarking(): void {
  const store = useDocument.getState();
  store.setSidebarTab("search");
}

/** A mark over each rectangle group, as one undo step. */
export function marksFor(hits: ReadonlyArray<{ page: number; rects: readonly PdfRect[] }>, now = 0): AnnotObject[] {
  return hits
    .filter((h) => h.rects.length > 0)
    .map((h) => {
      const rect = h.rects.reduce((a, b) => ({
        left: Math.min(a.left, b.left),
        bottom: Math.min(a.bottom, b.bottom),
        right: Math.max(a.right, b.right),
        top: Math.max(a.top, b.top),
      }));
      return {
        id: NEW_OBJECT_ID,
        page: h.page,
        kind: "Redact",
        rect,
        rotation: 0,
        opacity: 1,
        z: 0,
        locked: false,
        created_at: now,
        modified_at: now,
        author_note: "",
        payload: { Markup: { quads: [...h.rects], color: REDACT_FILL } },
      } satisfies AnnotObject;
    });
}

/**
 * Marks every hit of the current search for redaction. Returns how many.
 *
 * Every page with a hit is searched again for its boxes, in the space the
 * text layer and the annotations share — the highlights on screen may be of
 * only the pages that have been shown.
 */
export async function markSearchResults(): Promise<number> {
  const store = useDocument.getState();
  const doc = store.doc;
  const query = store.search.query.trim();
  if (doc === null || query.length === 0) return 0;
  const pages =
    store.search.scope === "regex"
      ? Array.from({ length: store.pageCount }, (_, i) => i)
      : [...new Set(store.search.results.filter((r) => r.doc === undefined || r.doc === doc).map((r) => r.page))]
          .map((p) => store.displayOfOwn(p))
          .filter((p): p is number => p !== null);
  const hits: Array<{ page: number; rects: PdfRect[] }> = [];
  for (const page of pages) {
    const s = useDocument.getState();
    const from = s.sourceOf(page);
    if (from.doc === null) continue;
    const rotation = ((((s.docRotation + (s.pageRotation[page] ?? 0)) % 4) + 4) % 4) as 0 | 1 | 2 | 3;
    const found =
      s.search.scope === "regex"
        ? await invoke<Array<{ rects: PdfRect[] }>>("search_regex_page", {
            doc: from.doc,
            page: from.page,
            pattern: query,
            caseSensitive: s.search.caseSensitive,
            rotation,
          }).catch(() => [])
        : await invoke<Array<{ rects: PdfRect[] }>>("search_page", {
            doc: from.doc,
            page: from.page,
            query,
            caseSensitive: s.search.caseSensitive,
            wholeWord: s.search.wholeWord,
            rotation,
            generation: s.generation,
          }).catch(() => []);
    for (const h of found) hits.push({ page, rects: h.rects });
  }
  const objects = marksFor(hits, Date.now());
  if (objects.length === 0) {
    useUi.getState().notify({ kind: "ok", text: t("redact.nothingFound") });
    return 0;
  }
  const [, result] = await invoke<[AnnotObject[], EditResult]>("annot_add_many", { doc, objects });
  const fresh = useDocument.getState();
  fresh.applyEdit(result);
  await fresh.reloadAnnots();
  useUi.getState().notify({ kind: "ok", text: fill(t("redact.marked"), { count: objects.length }) });
  return objects.length;
}

export async function previewRedaction(doc: number): Promise<RedactionPreview> {
  return invoke<RedactionPreview>("redact_preview", { doc });
}

/** `C:\\x\\Surat.pdf` → `C:\\x\\Surat (diredaksi).pdf`. */
export function redactedName(path: string): string {
  const dot = path.toLowerCase().endsWith(".pdf") ? path.length - 4 : path.length;
  return `${path.slice(0, dot)}${t("redact.suffix")}.pdf`;
}

/**
 * Applies the marks of `doc`. `asCopy` asks where to write the new file,
 * starting from `<name> (diredaksi).pdf`; otherwise the file is overwritten.
 * Resolves whether it was applied.
 */
export async function applyRedaction(doc: number, asCopy: boolean): Promise<boolean> {
  const ws = useWorkspace.getState();
  const tab = ws.tabs.find((x) => x.doc === doc);
  if (!tab) return false;
  let target: string | null = null;
  if (asCopy) {
    const chosen = await saveDialog({
      defaultPath: redactedName(tab.path),
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (chosen === null) return false;
    target = /\.pdf$/i.test(chosen) ? chosen : `${chosen}.pdf`;
  }
  let report: SaveReport;
  try {
    report = await invoke<SaveReport>("redact_apply", { doc, target });
  } catch (e) {
    useUi.getState().notify({ kind: "error", text: t("redact.failed"), detail: String(e) });
    return false;
  }
  await reopen(doc, report.path);
  const r = report.redaction;
  if (r) {
    const images = r.images_removed + r.images_cleared + r.images_unsupported;
    useUi.getState().notify({
      kind: "ok",
      text: fill(t("redact.done"), { glyphs: r.glyphs, images, pages: r.pages, name: nameOf(report.path) }),
      ...(r.images_unsupported > 0
        ? { detail: fill(t("redact.unsupported"), { count: r.images_unsupported }) }
        : {}),
    });
  }
  return true;
}

/** Closes the tab and opens `path` in its place. */
export async function reopen(doc: number, path: string): Promise<void> {
  const ws = useWorkspace.getState();
  const index = ws.tabs.findIndex((x) => x.doc === doc);
  await ws.closeTab(doc);
  const next = await useWorkspace.getState().openFile(path);
  if (next === null || index < 0) return;
  const order = useWorkspace.getState().tabs.map((x) => x.doc).filter((d) => d !== next);
  order.splice(index, 0, next);
  await useWorkspace.getState().reorder(order);
}
