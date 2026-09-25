import { describe, expect, it, vi } from "vitest";
import { byPage, controlOf, labelOf, sameValue, type FormField } from "../forms";

const rect = { left: 0, bottom: 0, right: 10, top: 10 };

function field(name: string, pages: number[], kind: FormField["kind"] = { Text: { multiline: false } }): FormField {
  return {
    name,
    kind,
    read_only: false,
    widgets: pages.map((p) => [p, rect] as const),
    value: { Text: "" },
    options: [],
    exports: [],
  };
}

describe("form helpers", () => {
  it("groups fields by their first page, pages in order, fields in file order", () => {
    const groups = byPage([field("b", [2]), field("a", [0]), field("c", [2, 0]), field("d", [1])]);
    expect(groups.map(([page, list]) => [page, list.map((f) => f.name)])).toEqual([
      [0, ["a", "c"]],
      [1, ["d"]],
      [2, ["b"]],
    ]);
  });

  it("leaves out a field without widgets rather than putting it on page Infinity", () => {
    expect(byPage([field("ghost", [])])).toEqual([]);
  });

  it("picks the control from the kind", () => {
    expect(controlOf({ Text: { multiline: true } })).toBe("multiline");
    expect(controlOf({ Text: { multiline: false } })).toBe("text");
    expect(controlOf("CheckBox")).toBe("checkbox");
    expect(controlOf("Radio")).toBe("radio");
    expect(controlOf({ ComboBox: { editable: false } })).toBe("select");
    expect(controlOf({ ListBox: { multiple: true } })).toBe("list");
  });

  it("reads the name the author typed out of a machine-made one", () => {
    expect(labelOf("form1[0].nama_lengkap[0]")).toBe("Nama lengkap");
    expect(labelOf("kota")).toBe("Kota");
    expect(labelOf("[0]")).toBe("[0]");
  });

  it("compares values by kind and content", () => {
    expect(sameValue({ Text: "a" }, { Text: "a" })).toBe(true);
    expect(sameValue({ Text: "a" }, { Text: "b" })).toBe(false);
    expect(sameValue({ Choice: [1, 2] }, { Choice: [1, 2] })).toBe(true);
    expect(sameValue({ Choice: [1] }, { Choice: [1, 2] })).toBe(false);
    expect(sameValue({ Checked: false }, { Text: "" })).toBe(false);
  });
});

/**
 * A value is typed into the document it was typed into. The commit happens on
 * blur — and switching tabs is exactly what blurs a text box — so by the time
 * the backend answers, another document can be in front. The first version
 * applied the answer to whichever document that was.
 */
describe("setFormValue", () => {
  it("applies the edit to the document the value was typed into", async () => {
    vi.resetModules();
    let release: () => void = () => undefined;
    const answered = new Promise<void>((r) => (release = r));
    vi.doMock("@tauri-apps/api/core", () => ({
      invoke: async (cmd: string, args: unknown) => {
        if (cmd === "open_document") {
          const path = (args as { path: string }).path;
          return {
            doc: path.includes("satu") ? 1 : 2,
            path,
            page_count: 1,
            page_sizes: [[595, 842]],
            permissions: 0,
            encrypted: false,
            restored: null,
          };
        }
        if (cmd === "form_set") {
          await answered;
          return { objects: [], can_undo: true, can_redo: false, dirty: true, map_revision: 0, repaint: [0] };
        }
        if (cmd === "annot_list" || cmd === "annot_display_lists") return [];
        return null;
      },
    }));
    vi.doMock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({}) }));
    vi.doMock("@tauri-apps/plugin-dialog", () => ({ open: async () => null, save: async () => null }));
    const ws = await import("@/state/workspaceStore");
    const first = await ws.useWorkspace.getState().openFile("/x/satu.pdf");
    const second = await ws.useWorkspace.getState().openFile("/x/dua.pdf");
    if (first === null || second === null) throw new Error("not opened");
    await ws.useWorkspace.getState().activate(first);

    const { setFormValue } = await import("../forms");
    const pending = setFormValue("nama", { Text: "Izul" });
    await ws.useWorkspace.getState().activate(second);
    release();
    expect(await pending).toBe(true);

    const typedInto = ws.useWorkspace.getState().sessions.get(first)?.getState();
    const inFront = ws.useWorkspace.getState().sessions.get(second)?.getState();
    expect(typedInto?.dirty).toBe(true);
    expect(typedInto?.formRevision).toBe(1);
    expect(inFront?.dirty).toBe(false);
    expect(inFront?.formRevision).toBe(0);
  });
});
