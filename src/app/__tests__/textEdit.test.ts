import { describe, expect, it, vi } from "vitest";
import { editedName, oneLine, union } from "../textEdit";

const box = (left: number, bottom: number, right: number, top: number) => ({ left, bottom, right, top });

describe("text edit helpers", () => {
  it("tells one line from two", () => {
    // Two boxes of one line, a word apart.
    expect(oneLine([box(10, 700, 50, 712), box(55, 699, 90, 711)])).toBe(true);
    // The next line down: no vertical overlap with the first.
    expect(oneLine([box(10, 700, 50, 712), box(10, 684, 60, 696)])).toBe(false);
    expect(oneLine([])).toBe(false);
  });

  it("unites the selection's boxes", () => {
    expect(union([box(10, 700, 50, 712), box(55, 699, 90, 711)])).toEqual(box(10, 699, 90, 712));
    expect(union([])).toBeNull();
  });

  it("names the copy beside the original", () => {
    expect(editedName("C:\\Dok\\surat.pdf")).toBe("C:\\Dok\\surat (disunting).pdf");
    expect(editedName("/x/surat")).toBe("/x/surat (disunting).pdf");
  });
});

/**
 * A refusal is shown in the dialog, which stays open; the tab is not
 * reopened (nothing was written). A success reopens the tab on the file.
 */
describe("replaceText", () => {
  it("returns the refusal and leaves the tab alone; reopens after a success", async () => {
    vi.resetModules();
    const calls: string[] = [];
    let refuse = true;
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: async (cmd: string, args: unknown) => {
        calls.push(cmd);
        if (cmd === "open_document") {
          const path = (args as { path: string }).path;
          return { doc: 4, path, page_count: 1, page_sizes: [[595, 842]], permissions: 0, encrypted: false, restored: null };
        }
        if (cmd === "text_replace") {
          if (refuse) throw "Teks tidak diganti, berkas tidak diubah: font \"Helvetica\" tidak tertanam";
          return { path: "/x/surat.pdf", bytes: 1, annotations: 0, restructured: null, redaction: null, text: { before: "Budi", glyphs: 4 } };
        }
        if (cmd === "annot_list" || cmd === "annot_display_lists") return [];
        return null;
      },
    }));
    vi.doMock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({}) }));
    vi.doMock("@tauri-apps/plugin-dialog", () => ({ open: async () => null, save: async () => null }));
    const ws = await import("@/state/workspaceStore");
    const doc = await ws.useWorkspace.getState().openFile("/x/surat.pdf");
    if (doc === null) throw new Error("not opened");
    const { replaceText } = await import("../textEdit");
    const { useUi } = await import("@/state/uiStore");
    const sel = { page: 0, rect: box(10, 700, 50, 712), text: "Budi" };

    const refused = await replaceText(doc, sel, "Rina", false);
    expect(refused).toEqual({ ok: false, error: expect.stringContaining("tidak tertanam") as string });
    const opens = calls.filter((c) => c === "open_document").length;

    refuse = false;
    expect(await replaceText(doc, sel, "Rina", false)).toEqual({ ok: true });
    expect(calls.filter((c) => c === "open_document").length).toBe(opens + 1);
    expect(useUi.getState().notice?.text).toContain("“Budi” menjadi “Rina”");
  });
});
