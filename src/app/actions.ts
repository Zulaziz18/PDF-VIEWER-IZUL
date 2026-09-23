/**
 * What the buttons do.
 *
 * The ribbon, the menus, the bottom bar and the keyboard shortcuts all reach
 * the same handful of operations, and SPEC 0 keeps logic out of components.
 * Each operation is written once, here, and every control that offers it calls
 * the same function — so a shortcut and a button cannot drift apart.
 */

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { DEFAULT_STYLE, markupFromQuads } from "@/annots/factory";
import { NEW_OBJECT_ID } from "@/annots/types";
import { useDocument } from "@/state/documentStore";
import { useUi, type MarkupKind } from "@/state/uiStore";
import { useWorkspace } from "@/state/workspaceStore";
import { viewport } from "./viewportHandle";
import { t } from "@/i18n";

/** Opens one or more files into tabs, through the platform's own picker. */
export async function pickAndOpen(): Promise<void> {
  const chosen = await openDialog({
    multiple: true,
    filters: [{ name: "PDF", extensions: ["pdf"] }],
  });
  const paths = Array.isArray(chosen) ? chosen : typeof chosen === "string" ? [chosen] : [];
  for (const path of paths) {
    await useWorkspace.getState().openFile(path);
  }
}

/** Moves the viewport to a page, clamped to the document. */
export function goToPage(target: number): void {
  const store = useDocument.getState();
  const clamped = Math.min(Math.max(0, target), Math.max(0, store.pageCount - 1));
  store.setPage(clamped);
  viewport()?.goToPage(clamped);
}

/**
 * Marks up the current text selection on the page. Returns whether something
 * was drawn; a selection outside the page's text layer draws nothing.
 */
export function applyMarkup(kind: MarkupKind): boolean {
  const found = viewport()?.selectionQuads();
  if (!found) return false;
  const object = markupFromQuads(kind, found.page, found.quads, DEFAULT_STYLE, Date.now());
  if (object) void useDocument.getState().addAnnot(object);
  document.getSelection()?.removeAllRanges();
  return object !== null;
}

/**
 * What a markup button does: mark up the selection if there is one, otherwise
 * arm the tool so the next selection is marked up (`useArmedMarkup`), and
 * disarm it when it is pressed again — as the markup tools behave in every
 * editor a user is likely to have used.
 */
export function markupSelection(kind: MarkupKind): void {
  if (applyMarkup(kind)) return;
  const ui = useUi.getState();
  ui.armMarkup(ui.markup === kind ? null : kind);
  useDocument.getState().setTool(null);
}

/**
 * Inserts an image on the current page.
 *
 * A file, not a drag: an image annotation needs pixels before it has a size,
 * and dragging out a box first would mean either distorting the picture or
 * ignoring the box the user just drew. It lands in the middle of the page at
 * its natural aspect ratio and is moved and resized like anything else.
 */
export async function insertImage(): Promise<void> {
  const store = useDocument.getState;
  const state = store();
  if (state.doc === null) return;
  const chosen = await openDialog({
    multiple: false,
    filters: [{ name: t("dialog.images"), extensions: ["png", "jpg", "jpeg", "gif", "webp"] }],
  });
  if (typeof chosen !== "string") return;
  try {
    const image = await invoke<number>("annot_add_image", { doc: state.doc, path: chosen });
    const page = state.page;
    const size = state.pageSizes[page];
    const width = Math.min(240, (size?.width ?? 400) * 0.5);
    const height = width * 0.75;
    const left = ((size?.width ?? 400) - width) / 2;
    const bottom = ((size?.height ?? 600) - height) / 2;
    await store().addAnnot({
      id: NEW_OBJECT_ID,
      page,
      kind: "Image",
      rect: { left, bottom, right: left + width, top: bottom + height },
      rotation: 0,
      opacity: 1,
      z: 0,
      locked: false,
      created_at: Date.now(),
      modified_at: Date.now(),
      author_note: "",
      payload: { Image: { image, crop: { left: 0, bottom: 0, right: 1, top: 1 }, opacity: 1 } },
    });
  } catch (e) {
    console.warn("gambar tidak dapat disisipkan", e);
  }
}
