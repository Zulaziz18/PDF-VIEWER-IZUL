import { describe, expect, it, vi } from "vitest";
import type * as WorkspaceModule from "@/state/workspaceStore";
import type * as UiModule from "@/state/uiStore";
import type * as FileActionsModule from "../fileActions";

/**
 * SPEC 8's promise, tested where it is kept: nothing is thrown away without
 * the user choosing to throw it away.
 *
 * Each test drives the real stores and the real close flow with the Tauri
 * bridge replaced, and answers the prompt the way a user would.
 */

interface Harness {
  calls: Array<{ cmd: string; args: unknown }>;
  ws: typeof WorkspaceModule;
  ui: typeof UiModule;
  actions: typeof FileActionsModule;
}

async function harness(overrides: Record<string, (args: unknown) => unknown> = {}): Promise<Harness> {
  vi.resetModules();
  const calls: Array<{ cmd: string; args: unknown }> = [];
  vi.doMock("@tauri-apps/api/core", () => ({
    invoke: async (cmd: string, args: unknown) => {
      calls.push({ cmd, args });
      const override = overrides[cmd];
      if (override) return override(args);
      if (cmd === "open_document") {
        const path = (args as { path: string }).path;
        return {
          doc: path.length,
          path,
          page_count: 2,
          page_sizes: [
            [595, 842],
            [595, 842],
          ],
          permissions: 0,
          encrypted: false,
          restored: null,
        };
      }
      if (cmd === "draft_status") return null;
      return null;
    },
  }));
  vi.doMock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({}) }));
  vi.doMock("@tauri-apps/plugin-dialog", () => ({
    open: async () => null,
    save: async () => null,
  }));
  const ws = await import("@/state/workspaceStore");
  const ui = await import("@/state/uiStore");
  const actions = await import("../fileActions");
  return { calls, ws, ui, actions };
}

/** Opens a document and gives it unsaved edits, as an annotation would. */
async function openDirty(h: Harness, path: string, dirty = true): Promise<number> {
  const doc = await h.ws.useWorkspace.getState().openFile(path);
  if (doc === null) throw new Error("not opened");
  h.ws.useWorkspace.getState().session(doc)?.getState().applyEdit({
    objects: [],
    can_undo: true,
    can_redo: false,
    dirty,
    map_revision: 0,
  });
  return doc;
}

/** Waits for the prompt, then answers it. */
async function answer(h: Harness, id: string): Promise<string> {
  await vi.waitFor(() => expect(h.ui.useUi.getState().prompt).not.toBeNull());
  const title = h.ui.useUi.getState().prompt?.spec.title ?? "";
  h.ui.useUi.getState().answer(id);
  return title;
}

describe("closing a tab", () => {
  it("asks before closing a document with unsaved edits, and 'cancel' keeps it", async () => {
    const h = await harness();
    const doc = await openDirty(h, "/x/a.pdf");
    const closing = h.actions.requestCloseTab(doc);
    await answer(h, "cancel");
    await closing;
    expect(h.ws.useWorkspace.getState().tabs).toHaveLength(1);
    expect(h.calls.map((c) => c.cmd)).not.toContain("close_document");
    expect(h.calls.map((c) => c.cmd)).not.toContain("draft_discard");
  });

  it("'don't save' discards the draft, then closes", async () => {
    const h = await harness();
    const doc = await openDirty(h, "/x/a.pdf");
    const closing = h.actions.requestCloseTab(doc);
    await answer(h, "discard");
    await closing;
    const cmds = h.calls.map((c) => c.cmd);
    expect(h.ws.useWorkspace.getState().tabs).toHaveLength(0);
    // In this order: a draft left behind would be offered back next time.
    expect(cmds.indexOf("draft_discard")).toBeGreaterThanOrEqual(0);
    expect(cmds.indexOf("draft_discard")).toBeLessThan(cmds.indexOf("close_document"));
  });

  it("'save' that fails keeps the tab open", async () => {
    const h = await harness({
      save_document: () => {
        throw new Error("disk penuh");
      },
    });
    const doc = await openDirty(h, "/x/a.pdf");
    const closing = h.actions.requestCloseTab(doc);
    await answer(h, "save");
    await closing;
    expect(h.ws.useWorkspace.getState().tabs).toHaveLength(1);
    expect(h.ui.useUi.getState().notice?.kind).toBe("error");
    expect(h.ws.useWorkspace.getState().session(doc)?.getState().dirty).toBe(true);
  });

  it("'save' that succeeds writes the file, then closes", async () => {
    const h = await harness({
      save_document: () => ({ path: "/x/a.pdf", bytes: 1234, annotations: 1 }),
    });
    const doc = await openDirty(h, "/x/a.pdf");
    const closing = h.actions.requestCloseTab(doc);
    await answer(h, "save");
    await closing;
    const cmds = h.calls.map((c) => c.cmd);
    expect(cmds.indexOf("save_document")).toBeLessThan(cmds.indexOf("close_document"));
    expect(cmds).not.toContain("draft_discard");
  });

  /// The old rule was `canUndo`: a saved document still has an undo history,
  /// and would have been asked about for nothing.
  it("closes a saved document without asking, even with undo history", async () => {
    const h = await harness();
    const doc = await openDirty(h, "/x/a.pdf", false);
    expect(h.ws.useWorkspace.getState().session(doc)?.getState().canUndo).toBe(true);
    await h.actions.requestCloseTab(doc);
    expect(h.ui.useUi.getState().prompt).toBeNull();
    expect(h.ws.useWorkspace.getState().tabs).toHaveLength(0);
    // And its draft, if one was kept "for later", is left alone.
    expect(h.calls.map((c) => c.cmd)).not.toContain("draft_discard");
  });
});

describe("closing the window", () => {
  it("asks about each unsaved document in turn and stops at the first 'cancel'", async () => {
    const h = await harness();
    await openDirty(h, "/x/a.pdf");
    await openDirty(h, "/x/bb.pdf");
    await openDirty(h, "/x/ccc.pdf", false);
    const closing = h.actions.requestCloseWindow();
    await answer(h, "discard");
    await vi.waitFor(() => expect(h.calls.filter((c) => c.cmd === "draft_discard")).toHaveLength(1));
    await answer(h, "cancel");
    expect(await closing).toBe(false);
  });

  it("may close once every unsaved document is settled", async () => {
    const h = await harness();
    await openDirty(h, "/x/a.pdf");
    const closing = h.actions.requestCloseWindow();
    await answer(h, "discard");
    expect(await closing).toBe(true);
  });

  it("closes at once when nothing is unsaved", async () => {
    const h = await harness();
    await openDirty(h, "/x/a.pdf", false);
    expect(await h.actions.requestCloseWindow()).toBe(true);
    expect(h.ui.useUi.getState().prompt).toBeNull();
  });
});

describe("drafts", () => {
  it("offers a waiting draft when its document opens, and restores it on request", async () => {
    const h = await harness({
      draft_status: () => ({ updated_at: 1_700_000_000, objects: 3, matches_file: true }),
      draft_restore: () => ({ objects: [], can_undo: false, can_redo: false, dirty: true, map_revision: 0 }),
    });
    h.actions.installDraftOffer();
    const doc = await h.ws.useWorkspace.getState().openFile("/x/a.pdf");
    await answer(h, "restore");
    await vi.waitFor(() =>
      expect(h.ws.useWorkspace.getState().session(doc ?? -1)?.getState().dirty).toBe(true),
    );
    const restore = h.calls.find((c) => c.cmd === "draft_restore");
    expect(restore?.args).toEqual({ doc, force: false });
  });

  /// SPEC 8: a draft made against a file that has since changed is applied
  /// only when the user asks for exactly that.
  it("says so when the file changed since the draft, and forces only on request", async () => {
    const h = await harness({
      draft_status: () => ({ updated_at: 1_700_000_000, objects: 3, matches_file: false }),
      draft_restore: () => ({ objects: [], can_undo: false, can_redo: false, dirty: true, map_revision: 0 }),
    });
    h.actions.installDraftOffer();
    const doc = await h.ws.useWorkspace.getState().openFile("/x/a.pdf");
    await vi.waitFor(() => expect(h.ui.useUi.getState().prompt).not.toBeNull());
    const spec = h.ui.useUi.getState().prompt?.spec;
    expect(spec?.detail).toBeDefined();
    expect(spec?.buttons.find((b) => b.kind === "primary")?.id).toBe("restore");
    h.ui.useUi.getState().answer("restore");
    await vi.waitFor(() => expect(h.calls.some((c) => c.cmd === "draft_restore")).toBe(true));
    expect(h.calls.find((c) => c.cmd === "draft_restore")?.args).toEqual({ doc, force: true });
  });

  it("'later' keeps the draft", async () => {
    const h = await harness({
      draft_status: () => ({ updated_at: 1_700_000_000, objects: 1, matches_file: true }),
    });
    h.actions.installDraftOffer();
    await h.ws.useWorkspace.getState().openFile("/x/a.pdf");
    await answer(h, "later");
    await new Promise((r) => setTimeout(r, 0));
    const cmds = h.calls.map((c) => c.cmd);
    expect(cmds).not.toContain("draft_discard");
    expect(cmds).not.toContain("draft_restore");
  });
});

describe("saving", () => {
  it("a save with nothing to save does not touch the file", async () => {
    const h = await harness();
    const doc = await openDirty(h, "/x/a.pdf", false);
    expect(await h.actions.saveDocument(doc)).toBe(true);
    expect(h.calls.map((c) => c.cmd)).not.toContain("save_document");
  });

  it("marks the document saved and records the new size", async () => {
    const h = await harness({
      save_document: () => ({ path: "/x/a.pdf", bytes: 4321, annotations: 2 }),
    });
    const doc = await openDirty(h, "/x/a.pdf");
    expect(await h.actions.saveDocument(doc)).toBe(true);
    const state = h.ws.useWorkspace.getState().session(doc)?.getState();
    expect(state?.dirty).toBe(false);
    expect(state?.file.size).toBe(4321);
    expect(h.calls.find((c) => c.cmd === "save_document")?.args).toEqual({ doc, target: null });
  });
});
