import { describe, expect, it, vi } from "vitest";
import type * as BackgroundModule from "../background";

/**
 * "Hapus Latar" must redraw the picture. The edit result only carries the
 * undo state, so the page's annotations have to be fetched again — the first
 * version did not, and the harness screenshot showed the old picture after
 * a successful run.
 */

const picture = {
  id: 7,
  page: 1,
  kind: "Image",
  rect: { left: 0, bottom: 0, right: 10, top: 10 },
  rotation: 0,
  opacity: 1,
  z: 0,
  locked: false,
  created_at: 0,
  modified_at: 0,
  author_note: "",
  payload: { Image: { image: 2, crop: { left: 0, bottom: 0, right: 1, top: 1 }, opacity: 1 } },
};

async function harness(): Promise<{ calls: Array<{ cmd: string; args: unknown }>; remove: typeof BackgroundModule.removeBackground; doc: number }> {
  vi.resetModules();
  const calls: Array<{ cmd: string; args: unknown }> = [];
  vi.doMock("@tauri-apps/api/core", () => ({
    invoke: async (cmd: string, args: unknown) => {
      calls.push({ cmd, args });
      if (cmd === "open_document") {
        const path = (args as { path: string }).path;
        return { doc: 3, path, page_count: 2, page_sizes: [[595, 842], [595, 842]], permissions: 0, encrypted: false, restored: null };
      }
      if (cmd === "annot_remove_background") {
        return {
          edit: { objects: [picture], can_undo: true, can_redo: false, dirty: true, map_revision: 0 },
          device: "CPU",
          paper: true,
        };
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
  const { removeBackground } = await import("../background");
  return { calls, remove: removeBackground, doc };
}

describe("Hapus Latar", () => {
  it("fetches the page's annotations again after the edit", async () => {
    const h = await harness();
    await h.remove(7, "OnPaper");
    const cmds = h.calls.map((c) => c.cmd);
    const at = cmds.indexOf("annot_remove_background");
    expect(at).toBeGreaterThanOrEqual(0);
    const after = h.calls.slice(at + 1);
    expect(after.some((c) => c.cmd === "annot_display_lists" || c.cmd === "annot_list")).toBe(true);
  });
});
