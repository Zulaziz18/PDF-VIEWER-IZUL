import { describe, expect, it, vi } from "vitest";
import { tabsToTrim, WARM_TABS } from "../workspaceStore";
import type * as WorkspaceModule from "../workspaceStore";

/**
 * The rule behind SPEC 17's "fifty documents open, memory under control".
 *
 * It is worth testing rather than eyeballing because the failure modes are both
 * invisible in a screenshot: trimming too eagerly re-renders the page every
 * time a reader flips between two documents, and trimming too little is the
 * memory the criterion is about.
 */
describe("tabsToTrim", () => {
  it("keeps the tabs the reader has actually been using", () => {
    const recent = [1, 2, 3, 4, 5];
    expect(tabsToTrim(recent)).toEqual([4, 5]);
    expect(tabsToTrim(recent)).not.toContain(1);
  });

  it("trims nothing while there are fewer tabs than the warm budget", () => {
    expect(tabsToTrim([1])).toEqual([]);
    expect(tabsToTrim([1, 2, 3])).toEqual([]);
  });

  /// Two documents being compared are flipped between every few seconds; a
  /// policy that trimmed on every flip would re-render both pages each time.
  it("leaves a two-document comparison alone", () => {
    let recent = [1, 2];
    for (let i = 0; i < 10; i++) {
      const front = recent[0] as number;
      recent = [recent[1] as number, front];
      expect(tabsToTrim(recent)).toEqual([]);
    }
  });

  it("trims everything past the budget when fifty are open", () => {
    const recent = Array.from({ length: 50 }, (_, i) => i + 1);
    const trimmed = tabsToTrim(recent);
    expect(trimmed).toHaveLength(50 - WARM_TABS);
    expect(trimmed).not.toContain(recent[0]);
    expect(trimmed[trimmed.length - 1]).toBe(50);
  });
});

/**
 * The bug this guards against was reported from a real window: three files on
 * the command line produced six tabs, each one twice.
 *
 * The cause is a race, so the test has to be one too. `openFile`'s "already
 * open" check reads the tab list, and a tab only exists once `open_document`
 * has returned — so two calls that overlap both find nothing and both open.
 * In the application the two callers were React's development double-mount
 * running the startup effect twice; here they are two calls started in the
 * same tick, which is the same thing with the framework taken out.
 */
describe("openFile", () => {
  it("opens one tab when the same path is asked for twice at once", async () => {
    let opens = 0;
    const { useWorkspace } = await freshWorkspace(async (cmd, args) => {
      if (cmd !== "open_document") return null;
      opens += 1;
      // The real command is not instant, and the bug only exists in that gap.
      await new Promise((r) => setTimeout(r, 5));
      return opened(opens, (args as { path: string }).path);
    });

    const both = await Promise.all([
      useWorkspace.getState().openFile("/x/a.pdf"),
      useWorkspace.getState().openFile("/x/a.pdf"),
    ]);

    expect(opens).toBe(1);
    expect(useWorkspace.getState().tabs).toHaveLength(1);
    expect(both[0]).toBe(both[1]);
  });

  it("still opens a second tab for a different file", async () => {
    let opens = 0;
    const { useWorkspace } = await freshWorkspace(async (cmd, args) => {
      if (cmd !== "open_document") return null;
      opens += 1;
      await new Promise((r) => setTimeout(r, 5));
      return opened(opens, (args as { path: string }).path);
    });

    await Promise.all([
      useWorkspace.getState().openFile("/x/a.pdf"),
      useWorkspace.getState().openFile("/x/b.pdf"),
    ]);

    expect(useWorkspace.getState().tabs.map((t) => t.path)).toEqual(["/x/a.pdf", "/x/b.pdf"]);
  });
});

/**
 * A workspace store with the Tauri bridge replaced.
 *
 * The module is re-imported per test because the store — and the map of opens
 * in flight — are module state, and a test that inherited another's tabs would
 * pass or fail for the wrong reason.
 */
async function freshWorkspace(
  handler: (cmd: string, args: unknown) => Promise<unknown>,
): Promise<typeof WorkspaceModule> {
  vi.resetModules();
  vi.doMock("@tauri-apps/api/core", () => ({ invoke: handler }));
  return await import("../workspaceStore");
}

/** The shape `open_document` returns, with the fields a session reads filled. */
function opened(doc: number, path: string) {
  return {
    doc,
    path,
    page_count: 3,
    page_sizes: [
      [595, 842],
      [595, 842],
      [595, 842],
    ],
    permissions: 0,
    encrypted: false,
    restored: null,
  };
}
