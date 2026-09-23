/**
 * Phase 4: what happens to the file — save, save as, export, closing with
 * unsaved work, drafts, and a file another program changed.
 *
 * Kept out of the components for the same reason `actions.ts` is: the menu,
 * the ribbon, the tab's close button, Ctrl+S and the window's own close all
 * reach the same few operations, and each is written once, here.
 *
 * The rules, and where they come from:
 *
 * - Nothing is ever lost silently (SPEC 8). Closing a tab or the window with
 *   unsaved edits asks "save / don't save / cancel"; only the middle answer
 *   throws work away, and only then is the draft discarded.
 * - A draft is offered when its document opens; one made against a file that
 *   has since changed says so and is applied only if the user asks for exactly
 *   that.
 * - Autosave writes drafts, never the file. Writing the file is always the
 *   user's decision.
 */

import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { EditResult } from "@/annots/types";
import { t } from "@/i18n";
import type { FileStatusReply, SaveReport } from "@/state/documentSession";
import { formatBytes, formatDateTime } from "@/state/homeModel";
import { formatPageRange } from "@/state/pageRange";
import { useUi } from "@/state/uiStore";
import { setAfterOpen, useWorkspace } from "@/state/workspaceStore";

/** How often drafts are written (SPEC 8: "tiap 20 detik"). */
export const AUTOSAVE_MS = 20_000;
/** How often the file in front is checked for changes by other programs. */
export const WATCH_MS = 2_500;

interface DraftInfo {
  readonly updated_at: number;
  readonly objects: number;
  readonly matches_file: boolean;
}

/** Fills `{name}`-style holes in a translated string. */
function fill(text: string, values: Record<string, string | number>): string {
  return text.replace(/\{(\w+)\}/g, (hole, key: string) => String(values[key] ?? hole));
}

function nameOf(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

function folderOf(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return cut > 0 ? path.slice(0, cut) : path;
}

/** The separator the path already uses, so a Windows path stays one. */
function join(folder: string, name: string): string {
  const sep = folder.includes("\\") ? "\\" : "/";
  return folder.endsWith(sep) ? folder + name : folder + sep + name;
}

function stemOf(path: string): string {
  return nameOf(path).replace(/\.pdf$/i, "");
}

function withPdfExtension(path: string): string {
  return /\.pdf$/i.test(path) ? path : `${path}.pdf`;
}

function activeDoc(): number | null {
  const ws = useWorkspace.getState();
  return ws.home ? null : ws.activeDoc;
}

// ---------------------------------------------------------------------------
// Save
// ---------------------------------------------------------------------------

/**
 * Saves a document to its own file, or — `saveAs`, or when its file has gone
 * missing — to one the user picks. Resolves whether the file now holds every
 * edit; a cancelled picker is `false`, the same as a failure, because the
 * callers that care (closing) must not go on either way.
 */
export async function saveDocument(
  doc: number | null = activeDoc(),
  mode: "save" | "saveAs" = "save",
): Promise<boolean> {
  if (doc === null) return false;
  const session = useWorkspace.getState().session(doc);
  if (!session) return false;
  const state = session.getState();
  if (state.file.saving || state.path === null) return false;

  const pickTarget = mode === "saveAs" || state.file.missing;
  if (!pickTarget && !state.dirty) {
    useUi.getState().notify({ kind: "ok", text: t("save.nothing") });
    return true;
  }

  let target: string | null = null;
  if (pickTarget) {
    const chosen = await saveDialog({
      defaultPath: state.path,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (chosen === null) return false;
    target = withPdfExtension(chosen);
  }

  session.getState().setSaving(true);
  useUi.getState().notify({ kind: "ok", text: t("save.running") });
  try {
    const report = await invoke<SaveReport>("save_document", { doc, target });
    session.getState().markSaved(report);
    if (target !== null) useWorkspace.getState().renameTab(doc, report.path);
    // The file now holds the rearranged pages; the tab lays itself out from
    // the file again, and page numbers start meaning file pages once more.
    if (report.restructured) await session.getState().refreshPages();
    useUi.getState().notify({
      kind: "ok",
      text: fill(t("save.done"), { name: nameOf(report.path), size: formatBytes(report.bytes) }),
    });
    return true;
  } catch (e) {
    session.getState().setSaving(false);
    useUi.getState().notify({ kind: "error", text: t("save.failed"), detail: String(e) });
    return false;
  }
}

// ---------------------------------------------------------------------------
// Closing
// ---------------------------------------------------------------------------

/**
 * Settles a tab's unsaved edits before it closes. Resolves whether closing
 * may go ahead.
 */
async function settle(doc: number): Promise<boolean> {
  const ws = useWorkspace.getState();
  const session = ws.session(doc);
  const tab = ws.tabs.find((x) => x.doc === doc);
  if (!session || !tab || !session.getState().dirty) return true;
  // The question is about a document; show that document while asking it.
  await ws.activate(doc);
  const answer = await useUi.getState().ask({
    icon: "warning",
    title: t("close.title"),
    body: fill(t("close.body"), { name: tab.name }),
    detail: t("close.detail"),
    buttons: [
      { id: "save", label: t("close.save"), kind: "primary" },
      { id: "discard", label: t("close.discard"), kind: "danger" },
      { id: "cancel", label: t("close.cancel") },
    ],
    cancelId: "cancel",
  });
  if (answer === "save") return await saveDocument(doc);
  if (answer === "discard") {
    await invoke("draft_discard", { doc }).catch(() => undefined);
    return true;
  }
  return false;
}

/** The tab's close button, Ctrl+W, the middle click and the file menu. */
export async function requestCloseTab(doc: number | null = useWorkspace.getState().activeDoc): Promise<void> {
  if (doc === null) return;
  if (!(await settle(doc))) return;
  await useWorkspace.getState().closeTab(doc);
}

export async function requestCloseAll(): Promise<void> {
  for (const tab of [...useWorkspace.getState().tabs]) {
    if (!(await settle(tab.doc))) return;
    await useWorkspace.getState().closeTab(tab.doc);
  }
}

/** Resolves whether the window may close: every dirty tab settled in turn. */
export async function requestCloseWindow(): Promise<boolean> {
  for (const tab of useWorkspace.getState().dirtyTabs()) {
    if (!(await settle(tab.doc))) return false;
  }
  return true;
}

/**
 * Guards the window's close — the title bar's button and Alt+F4 alike, since
 * both arrive as the same request. Tauri destroys the window after the
 * handler unless it is prevented, which needs `core:window:allow-destroy` in
 * the capability file.
 */
export function useCloseGuard(): void {
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void getCurrentWindow()
      .onCloseRequested(async (event) => {
        if (!(await requestCloseWindow())) event.preventDefault();
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
}

// ---------------------------------------------------------------------------
// Drafts
// ---------------------------------------------------------------------------

/** Paths being reloaded, whose drafts are put back without asking. */
const reloading = new Set<string>();
/** Draft offers, one at a time: a restored session opens several documents. */
let offers: Promise<void> = Promise.resolve();

async function offerDraft(doc: number, path: string): Promise<void> {
  if (reloading.has(path)) return;
  const info = await invoke<DraftInfo | null>("draft_status", { doc }).catch(() => null);
  if (!info) return;
  const session = useWorkspace.getState().session(doc);
  if (!session) return;
  const values = {
    name: nameOf(path),
    count: info.objects,
    time: formatDateTime(info.updated_at),
  };
  const answer = await useUi.getState().ask({
    icon: "draft",
    title: t("draft.title"),
    body: fill(info.matches_file ? t("draft.body") : t("draft.bodyChanged"), values),
    ...(info.matches_file ? {} : { detail: t("draft.detailChanged") }),
    buttons: [
      { id: "restore", label: info.matches_file ? t("draft.restore") : t("draft.restoreAnyway"), kind: "primary" },
      { id: "discard", label: t("draft.discard"), kind: "danger" },
      { id: "later", label: t("draft.later") },
    ],
    cancelId: "later",
  });
  if (answer === "restore") {
    try {
      const result = await invoke<EditResult>("draft_restore", { doc, force: !info.matches_file });
      session.getState().applyEdit(result);
      await session.getState().reloadAnnots();
      useUi.getState().notify({ kind: "ok", text: t("draft.restored") });
    } catch (e) {
      useUi.getState().notify({ kind: "error", text: t("draft.failed"), detail: String(e) });
    }
  } else if (answer === "discard") {
    await invoke("draft_discard", { doc }).catch(() => undefined);
  }
}

/** Registers the draft offer with the workspace; called once at startup. */
export function installDraftOffer(): void {
  setAfterOpen((doc, path) => {
    offers = offers.then(() => offerDraft(doc, path)).catch(() => undefined);
  });
}

/** Writes drafts on a timer and whenever the window loses focus (SPEC 8). */
export function useAutosave(): void {
  useEffect(() => {
    const write = (): void => {
      if (useWorkspace.getState().tabs.length === 0) return;
      void invoke("autosave_drafts").catch(() => undefined);
    };
    const timer = window.setInterval(write, AUTOSAVE_MS);
    window.addEventListener("blur", write);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("blur", write);
    };
  }, []);
}

// ---------------------------------------------------------------------------
// The file on disk
// ---------------------------------------------------------------------------

/** Keeps the tab in front's file state current. */
export function useFileWatch(): void {
  useEffect(() => {
    const check = (): void => {
      const doc = activeDoc();
      if (doc === null) return;
      const session = useWorkspace.getState().session(doc);
      if (!session || session.getState().file.saving) return;
      void invoke<FileStatusReply>("file_status", { doc })
        .then((status) => session.getState().setFileStatus(status))
        .catch(() => undefined);
    };
    check();
    const timer = window.setInterval(check, WATCH_MS);
    window.addEventListener("focus", check);
    const unsubscribe = useWorkspace.subscribe((s, prev) => {
      if (s.activeDoc !== prev.activeDoc || s.home !== prev.home) check();
    });
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", check);
      unsubscribe();
    };
  }, []);
}

/**
 * Opens the file again as it is on disk now, putting unsaved edits back on
 * top. The tab keeps its place in the strip.
 */
export async function reloadFromDisk(doc: number | null = activeDoc()): Promise<void> {
  if (doc === null) return;
  const ws = useWorkspace.getState();
  const session = ws.session(doc);
  const index = ws.tabs.findIndex((x) => x.doc === doc);
  const tab = ws.tabs[index];
  if (!session || !tab) return;
  const dirty = session.getState().dirty;
  reloading.add(tab.path);
  try {
    if (dirty) await invoke("draft_save_now", { doc });
    await ws.closeTab(doc);
    const next = await useWorkspace.getState().openFile(tab.path);
    if (next === null) return;
    const order = useWorkspace.getState().tabs.map((x) => x.doc).filter((d) => d !== next);
    order.splice(index, 0, next);
    await useWorkspace.getState().reorder(order);
    if (dirty) {
      const result = await invoke<EditResult>("draft_restore", { doc: next, force: true });
      const fresh = useWorkspace.getState().session(next);
      fresh?.getState().applyEdit(result);
      await fresh?.getState().reloadAnnots();
    }
    useUi.getState().notify({ kind: "ok", text: fill(t("file.reloaded"), { name: tab.name }) });
  } catch (e) {
    useUi.getState().notify({ kind: "error", text: t("file.reloadFailed"), detail: String(e) });
  } finally {
    reloading.delete(tab.path);
  }
}

/** "Ignore": keep working on what is open; stop warning about this change. */
export async function acknowledgeDiskChange(doc: number | null = activeDoc()): Promise<void> {
  if (doc === null) return;
  await invoke("file_acknowledge", { doc }).catch(() => undefined);
  const session = useWorkspace.getState().session(doc);
  const status = await invoke<FileStatusReply>("file_status", { doc }).catch(() => null);
  if (session && status) session.getState().setFileStatus(status);
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

type ExportSpec =
  | { kind: "flat"; target: string }
  | { kind: "pages"; target: string; pages: number[] }
  | { kind: "split"; folder: string; stem: string; ranges: number[][] }
  | { kind: "images"; folder: string; stem: string; pages: number[]; dpi: number; jpeg_quality: number | null };

async function runExport(doc: number, spec: ExportSpec): Promise<boolean> {
  const ui = useUi.getState();
  ui.notify({ kind: "ok", text: t("export.running") });
  try {
    const written = await invoke<string[]>("export_document", { doc, spec });
    const first = written[0] ?? "";
    ui.notify({
      kind: "ok",
      text:
        spec.kind === "images"
          ? fill(t("export.doneImages"), { count: written.length, folder: nameOf(folderOf(first)) })
          : spec.kind === "split"
            ? fill(t("export.doneSplit"), { count: written.length, folder: nameOf(folderOf(first)) })
            : fill(t("export.done"), { name: nameOf(first) }),
      detail: spec.kind === "images" || spec.kind === "split" ? folderOf(first) : first,
    });
    return true;
  } catch (e) {
    ui.notify({ kind: "error", text: t("export.failed"), detail: String(e) });
    return false;
  }
}

function currentPath(doc: number): string | null {
  return useWorkspace.getState().session(doc)?.getState().path ?? null;
}

/** "Ekspor Rata": every annotation burnt into its page, as a new file. */
export async function exportFlat(doc: number | null = activeDoc()): Promise<boolean> {
  if (doc === null) return false;
  const path = currentPath(doc);
  if (path === null) return false;
  const chosen = await saveDialog({
    defaultPath: join(folderOf(path), `${stemOf(path)}${t("export.flatSuffix")}.pdf`),
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (chosen === null) return false;
  return await runExport(doc, { kind: "flat", target: withPdfExtension(chosen) });
}

/** Some pages as a new PDF, annotations kept editable. */
export async function exportPages(doc: number, pages: number[]): Promise<boolean> {
  const path = currentPath(doc);
  if (path === null || pages.length === 0) return false;
  // "laporan-hal 1-3, 5.pdf" reads well in Explorer; commas stay out of it.
  const label = formatPageRange(pages).replace(/, /g, "_");
  const chosen = await saveDialog({
    defaultPath: join(folderOf(path), `${stemOf(path)}-${t("export.pagesSuffix")} ${label}.pdf`),
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  if (chosen === null) return false;
  return await runExport(doc, { kind: "pages", target: withPdfExtension(chosen), pages });
}

/** Pages as PNG or JPG files in a folder the user picks. */
export async function exportImages(
  doc: number,
  pages: number[],
  format: "png" | "jpg",
  dpi: number,
  quality: number,
): Promise<boolean> {
  const path = currentPath(doc);
  if (path === null || pages.length === 0) return false;
  const folder = await openDialog({ directory: true, defaultPath: folderOf(path) });
  if (typeof folder !== "string") return false;
  return await runExport(doc, {
    kind: "images",
    folder,
    stem: stemOf(path),
    pages,
    dpi,
    jpeg_quality: format === "jpg" ? quality : null,
  });
}

/** "Pecah": one PDF per range, `<name>-1.pdf`, `<name>-2.pdf`… in a folder. */
export async function exportSplit(doc: number, ranges: number[][]): Promise<boolean> {
  const path = currentPath(doc);
  const parts = ranges.filter((r) => r.length > 0);
  if (path === null || parts.length === 0) return false;
  const folder = await openDialog({ directory: true, defaultPath: folderOf(path) });
  if (typeof folder !== "string") return false;
  return await runExport(doc, { kind: "split", folder, stem: stemOf(path), ranges: parts });
}
