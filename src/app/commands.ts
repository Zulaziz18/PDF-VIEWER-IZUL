/**
 * Every command the application offers, in one table (Phase 8).
 *
 * The keyboard shortcuts, the command palette (Ctrl+Shift+P) and the list
 * under F1 are all read from here, so the three cannot disagree: a command
 * shown with Ctrl+S in the palette is the command Ctrl+S runs. The ribbon
 * keeps its own buttons — they carry state (pressed, disabled) the table
 * does not need — but calls the same functions.
 *
 * Keys are defaults; the user's changes live in SQLite (`keymapStore`).
 */

import type { AnnotKind } from "@/annots/types";
import type { StringKey } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { useUi, type MarkupKind } from "@/state/uiStore";
import { useWorkspace } from "@/state/workspaceStore";
import type { Layout } from "@/state/panels";
import type { ViewMode } from "@/viewport/layout";
import { goToPage, insertImage, markupSelection, pickAndOpen } from "./actions";
import { toggleCompare } from "./compare";
import { exportFlat, requestCloseAll, requestCloseTab, saveDocument } from "./fileActions";
import { deletePages, duplicatePages, extractPages, insertBlankPage, mergeFile, splitDocument } from "./pageActions";
import { markText, openSearchForMarking, toggleAreaTool } from "./redaction";
import { startTextEdit } from "./textEdit";
import { bookmarkHere } from "./bookmarks";

export type CommandGroup = "file" | "edit" | "view" | "annotate" | "pages" | "protect" | "convert" | "app";

export interface Command {
  readonly id: string;
  readonly label: StringKey;
  readonly group: CommandGroup;
  /** Default keys, in `keymap.ts` form. */
  readonly keys: readonly string[];
  /** Needs a document in front. */
  readonly needsDoc?: boolean;
  /**
   * Runs while the focus is in a text box too. Only for keys no one types
   * as text — Ctrl+S is pressed exactly while typing; a bare letter is not.
   */
  readonly whileTyping?: boolean;
  /** Not offered in the palette or F1 (Escape's layered behaviour). */
  readonly hidden?: boolean;
  run(): void;
}

const doc = () => useDocument.getState();
const ui = () => useUi.getState();

function tool(kind: AnnotKind): () => void {
  return () => {
    ui().armMarkup(null);
    doc().setTool(doc().tool === kind ? null : kind);
  };
}

function markup(kind: MarkupKind): () => void {
  return () => markupSelection(kind);
}

function cycleTab(step: number): void {
  const ws = useWorkspace.getState();
  if (ws.tabs.length < 2) return;
  const at = ws.tabs.findIndex((tab) => tab.doc === ws.activeDoc);
  const next = ws.tabs[(((at + step) % ws.tabs.length) + ws.tabs.length) % ws.tabs.length];
  if (next) void ws.activate(next.doc);
}

function viewMode(mode: ViewMode): () => void {
  return () => doc().setViewMode(mode);
}

function layout(l: Layout): () => void {
  return () => useWorkspace.getState().setLayout(l);
}

/** Escape puts down, in order: a tool, an armed markup, a selection, search, focus. */
function escape(): void {
  const store = doc();
  if (ui().presenting) {
    ui().setPresenting(false);
    return;
  }
  if (store.tool !== null) {
    store.setTool(null);
    return;
  }
  if (ui().markup !== null) {
    ui().armMarkup(null);
    return;
  }
  if (store.selection.length > 0) {
    store.select([]);
    return;
  }
  if (store.search.open) {
    store.toggleSearch(false);
    return;
  }
  (document.activeElement as HTMLElement | null)?.blur();
}

export const COMMANDS: readonly Command[] = [
  // Berkas
  { id: "file.open", label: "menu.open", group: "file", keys: ["Ctrl+O"], whileTyping: true, run: () => void pickAndOpen() },
  { id: "file.save", label: "menu.save", group: "file", keys: ["Ctrl+S"], needsDoc: true, whileTyping: true, run: () => void saveDocument(undefined, "save") },
  { id: "file.saveAs", label: "menu.saveAs", group: "file", keys: ["Ctrl+Shift+S"], needsDoc: true, whileTyping: true, run: () => void saveDocument(undefined, "saveAs") },
  { id: "file.print", label: "print.command", group: "file", keys: ["Ctrl+P"], needsDoc: true, whileTyping: true, run: () => ui().setPrinting(true) },
  { id: "file.close", label: "menu.closeTab", group: "file", keys: ["Ctrl+W"], needsDoc: true, whileTyping: true, run: () => void requestCloseTab() },
  { id: "file.closeAll", label: "menu.closeAll", group: "file", keys: [], needsDoc: true, run: () => void requestCloseAll() },
  { id: "file.home", label: "menu.home", group: "file", keys: [], run: () => useWorkspace.getState().showHome() },
  { id: "tab.next", label: "cmd.nextTab", group: "file", keys: ["Ctrl+Tab"], whileTyping: true, run: () => cycleTab(1) },
  { id: "tab.previous", label: "cmd.previousTab", group: "file", keys: ["Ctrl+Shift+Tab"], whileTyping: true, run: () => cycleTab(-1) },

  // Edit
  { id: "edit.undo", label: "annot.undo", group: "edit", keys: ["Ctrl+Z"], needsDoc: true, run: () => void doc().undoAnnot() },
  { id: "edit.redo", label: "annot.redo", group: "edit", keys: ["Ctrl+Y", "Ctrl+Shift+Z"], needsDoc: true, run: () => void doc().redoAnnot() },
  {
    id: "edit.delete",
    label: "cmd.deleteSelected",
    group: "edit",
    keys: ["Delete", "Backspace"],
    needsDoc: true,
    run: () => {
      if (doc().selection.length > 0) void doc().deleteSelected();
    },
  },
  { id: "edit.escape", label: "cmd.escape", group: "edit", keys: ["Escape"], hidden: true, run: escape },
  { id: "edit.textEdit", label: "textedit.button", group: "edit", keys: [], needsDoc: true, run: startTextEdit },
  { id: "edit.addText", label: "ribbon.addText", group: "edit", keys: [], needsDoc: true, run: tool("FreeText") },
  { id: "edit.addImage", label: "ribbon.addImage", group: "edit", keys: [], needsDoc: true, run: () => void insertImage() },

  // Tampilan
  { id: "view.bookmark", label: "cmd.bookmark", group: "view", keys: ["Ctrl+B"], needsDoc: true, run: () => void bookmarkHere() },
  { id: "view.find", label: "search.label", group: "view", keys: ["Ctrl+F"], needsDoc: true, whileTyping: true, run: () => doc().toggleSearch(true) },
  {
    id: "view.findLibrary",
    label: "cmd.findLibrary",
    group: "view",
    keys: ["Ctrl+Shift+F"],
    needsDoc: true,
    whileTyping: true,
    run: () => {
      doc().toggleSearch(true);
      doc().setSearchScope("library");
    },
  },
  { id: "view.nextResult", label: "cmd.nextResult", group: "view", keys: ["F3"], needsDoc: true, whileTyping: true, run: () => doc().gotoResult(1) },
  { id: "view.previousResult", label: "cmd.previousResult", group: "view", keys: ["Shift+F3"], needsDoc: true, whileTyping: true, run: () => doc().gotoResult(-1) },
  { id: "view.zoomIn", label: "zoom.in", group: "view", keys: ["Ctrl+Plus", "Ctrl+="], needsDoc: true, whileTyping: true, run: () => doc().zoomIn() },
  { id: "view.zoomOut", label: "zoom.out", group: "view", keys: ["Ctrl+-"], needsDoc: true, whileTyping: true, run: () => doc().zoomOut() },
  { id: "view.actualSize", label: "zoom.actualLong", group: "view", keys: ["Ctrl+0"], needsDoc: true, whileTyping: true, run: () => doc().setZoomMode("actual") },
  { id: "view.fitWidth", label: "zoom.fitWidthLong", group: "view", keys: ["Ctrl+2"], needsDoc: true, run: () => doc().setZoomMode("fitWidth") },
  { id: "view.fitPage", label: "zoom.fitPageLong", group: "view", keys: ["Ctrl+1"], needsDoc: true, run: () => doc().setZoomMode("fitPage") },
  { id: "view.nextPage", label: "cmd.nextPage", group: "view", keys: ["N", "J"], needsDoc: true, run: () => goToPage(doc().page + 1) },
  { id: "view.previousPage", label: "cmd.previousPage", group: "view", keys: ["P", "K"], needsDoc: true, run: () => goToPage(doc().page - 1) },
  { id: "view.firstPage", label: "cmd.firstPage", group: "view", keys: ["Ctrl+Home"], needsDoc: true, run: () => goToPage(0) },
  { id: "view.lastPage", label: "cmd.lastPage", group: "view", keys: ["Ctrl+End"], needsDoc: true, run: () => goToPage(doc().pageCount - 1) },
  { id: "view.rotateLeft", label: "rotate.left", group: "view", keys: [], needsDoc: true, run: () => doc().rotateDocument(-1) },
  { id: "view.rotateRight", label: "rotate.right", group: "view", keys: [], needsDoc: true, run: () => doc().rotateDocument(1) },
  { id: "view.single", label: "cmd.viewSingle", group: "view", keys: [], needsDoc: true, run: viewMode("single") },
  { id: "view.dual", label: "cmd.viewDual", group: "view", keys: [], needsDoc: true, run: viewMode("dual") },
  { id: "view.dualCover", label: "cmd.viewDualCover", group: "view", keys: [], needsDoc: true, run: viewMode("dual_cover") },
  { id: "view.horizontal", label: "cmd.viewHorizontal", group: "view", keys: [], needsDoc: true, run: viewMode("horizontal") },
  { id: "view.sidebar", label: "cmd.sidebar", group: "view", keys: ["F4"], needsDoc: true, run: () => doc().toggleSidebar() },
  { id: "view.formsPanel", label: "sidebar.forms", group: "view", keys: [], needsDoc: true, run: () => doc().setSidebarTab("forms") },
  { id: "view.annotList", label: "ribbon.annotList", group: "view", keys: [], needsDoc: true, run: () => doc().setSidebarTab("annots") },
  { id: "view.layoutSingle", label: "cmd.layoutSingle", group: "view", keys: [], needsDoc: true, run: layout("single") },
  { id: "view.layoutColumns", label: "cmd.layoutColumns", group: "view", keys: [], needsDoc: true, run: layout("columns") },
  { id: "view.layoutRows", label: "cmd.layoutRows", group: "view", keys: [], needsDoc: true, run: layout("rows") },
  { id: "view.layoutGrid", label: "cmd.layoutGrid", group: "view", keys: [], needsDoc: true, run: layout("grid") },
  { id: "view.compare", label: "compare.title", group: "view", keys: [], needsDoc: true, run: toggleCompare },
  { id: "view.present", label: "present.command", group: "view", keys: ["F5"], needsDoc: true, whileTyping: true, run: () => ui().setPresenting(true) },
  { id: "view.focus", label: "focus.command", group: "view", keys: ["F11"], whileTyping: true, run: () => ui().setFocusMode(!ui().focusMode) },
  { id: "view.theme", label: "theme.command", group: "view", keys: [], run: () => ui().cycleTheme() },
  { id: "view.invert", label: "invert.command", group: "view", keys: [], run: () => ui().setInvertPages(!ui().invertPages) },

  // Komentar
  { id: "annotate.highlight", label: "annot.highlight", group: "annotate", keys: ["Ctrl+Shift+H"], needsDoc: true, run: markup("Highlight") },
  { id: "annotate.underline", label: "annot.underline", group: "annotate", keys: ["Ctrl+Shift+U"], needsDoc: true, run: markup("Underline") },
  { id: "annotate.strike", label: "annot.strikeout", group: "annotate", keys: [], needsDoc: true, run: markup("StrikeOut") },
  { id: "annotate.note", label: "tool.note", group: "annotate", keys: [], needsDoc: true, run: tool("Note") },
  { id: "annotate.textbox", label: "tool.text", group: "annotate", keys: [], needsDoc: true, run: tool("FreeText") },
  { id: "annotate.ink", label: "tool.ink", group: "annotate", keys: [], needsDoc: true, run: tool("Ink") },
  { id: "annotate.rect", label: "tool.rect", group: "annotate", keys: [], needsDoc: true, run: tool("Rect") },
  { id: "annotate.ellipse", label: "tool.ellipse", group: "annotate", keys: [], needsDoc: true, run: tool("Ellipse") },
  { id: "annotate.line", label: "tool.line", group: "annotate", keys: [], needsDoc: true, run: tool("Line") },
  { id: "annotate.arrow", label: "tool.arrow", group: "annotate", keys: [], needsDoc: true, run: tool("Arrow") },
  { id: "annotate.stamp", label: "tool.stamp", group: "annotate", keys: [], needsDoc: true, run: tool("Stamp") },

  // Halaman
  { id: "pages.insertBlank", label: "pages.insertBlank", group: "pages", keys: [], needsDoc: true, run: () => void insertBlankPage() },
  { id: "pages.merge", label: "pages.merge", group: "pages", keys: [], needsDoc: true, run: () => void mergeFile() },
  { id: "pages.delete", label: "cmd.deletePages", group: "pages", keys: [], needsDoc: true, run: () => void deletePages() },
  { id: "pages.duplicate", label: "pages.duplicate", group: "pages", keys: [], needsDoc: true, run: () => void duplicatePages() },
  { id: "pages.extract", label: "pages.extract", group: "pages", keys: [], needsDoc: true, run: extractPages },
  { id: "pages.split", label: "pages.split", group: "pages", keys: [], needsDoc: true, run: splitDocument },

  // Lindungi
  { id: "protect.markText", label: "redact.markText", group: "protect", keys: [], needsDoc: true, run: markText },
  { id: "protect.markArea", label: "redact.markArea", group: "protect", keys: [], needsDoc: true, run: toggleAreaTool },
  { id: "protect.markSearch", label: "redact.markSearch", group: "protect", keys: [], needsDoc: true, run: openSearchForMarking },
  { id: "protect.apply", label: "cmd.applyRedaction", group: "protect", keys: [], needsDoc: true, run: () => ui().setRedacting(true) },

  // Konversi
  { id: "convert.images", label: "convert.toImages", group: "convert", keys: [], needsDoc: true, run: () => ui().setExporting("images") },
  { id: "convert.pages", label: "convert.pages", group: "convert", keys: [], needsDoc: true, run: () => ui().setExporting("pages") },
  { id: "convert.flat", label: "convert.flat", group: "convert", keys: [], needsDoc: true, run: () => void exportFlat() },
  { id: "convert.ocr", label: "ocr.button", group: "convert", keys: [], needsDoc: true, run: () => ui().setOcring(true) },

  // Aplikasi
  { id: "app.palette", label: "palette.command", group: "app", keys: ["Ctrl+Shift+P"], whileTyping: true, run: () => ui().setPaletteOpen(true) },
  { id: "app.shortcuts", label: "shortcuts.command", group: "app", keys: ["F1"], whileTyping: true, run: () => ui().setShortcutsOpen(true) },
  { id: "app.about", label: "menu.about", group: "app", keys: [], run: () => ui().setAboutOpen(true) },
];

export const COMMAND_BY_ID: ReadonlyMap<string, Command> = new Map(COMMANDS.map((c) => [c.id, c]));

export const GROUP_LABEL: Readonly<Record<CommandGroup, StringKey>> = {
  file: "menu.file",
  edit: "ribbon.edit",
  view: "ribbon.view",
  annotate: "ribbon.comment",
  pages: "ribbon.pages",
  protect: "ribbon.protect",
  convert: "ribbon.convert",
  app: "cmd.groupApp",
};

/** Whether a command can run now. */
export function available(cmd: Command): boolean {
  return !cmd.needsDoc || useDocument.getState().doc !== null;
}
