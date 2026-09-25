import { describe, expect, it, vi } from "vitest";

/**
 * The shortcuts are the registry and the user's keymap: what F1 lists is
 * what a key press runs. And the rules the hand-written table had are kept —
 * Ctrl+S saves while typing, a bare letter does not turn pages while typing.
 */

async function load() {
  vi.resetModules();
  const stored = new Map<string, string>();
  vi.doMock("@tauri-apps/api/core", () => ({
    invoke: async (cmd: string, args: { key?: string; value?: string }) => {
      if (cmd === "pref_get") return stored.get(args.key ?? "") ?? null;
      if (cmd === "pref_set") {
        stored.set(args.key ?? "", args.value ?? "");
        return null;
      }
      return null;
    },
  }));
  vi.doMock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({}) }));
  vi.doMock("@tauri-apps/plugin-dialog", () => ({ open: async () => null, save: async () => null }));
  const shortcuts = await import("../shortcuts");
  const { useKeymap } = await import("@/state/keymapStore");
  const { useWorkspace } = await import("@/state/workspaceStore");
  return { shortcuts, useKeymap, useWorkspace, stored };
}

const press = (k: string, mods: { ctrl?: boolean; shift?: boolean } = {}) => ({
  key: k,
  ctrlKey: mods.ctrl ?? false,
  shiftKey: mods.shift ?? false,
  altKey: false,
});
const page = { typing: false, dialog: false };
const typing = { typing: true, dialog: false };

describe("shortcuts", () => {
  it("runs the command the keymap names, and follows a change", async () => {
    const { shortcuts, useKeymap } = await load();
    expect(shortcuts.commandFor(press("o", { ctrl: true }), page)).toBe("file.open");
    expect(shortcuts.commandFor(press("P", { ctrl: true, shift: true }), page)).toBe("app.palette");
    expect(shortcuts.commandFor(press("F1"), page)).toBe("app.shortcuts");

    useKeymap.getState().setKeys("app.palette", ["Ctrl+K"]);
    expect(shortcuts.commandFor(press("k", { ctrl: true }), page)).toBe("app.palette");
    expect(shortcuts.commandFor(press("P", { ctrl: true, shift: true }), page)).toBeNull();
  });

  it("moves a key taken from another command, and stores only the difference", async () => {
    const { useKeymap, stored } = await load();
    useKeymap.getState().setKeys("app.shortcuts", ["Ctrl+O"]);
    expect(useKeymap.getState().keys.get("file.open")).toEqual([]);
    expect(useKeymap.getState().lookup.get("Ctrl+O")).toBe("app.shortcuts");
    expect(JSON.parse(stored.get("ui.shortcuts") ?? "{}")).toEqual({ "app.shortcuts": ["Ctrl+O"], "file.open": [] });

    useKeymap.getState().resetAll();
    expect(useKeymap.getState().lookup.get("Ctrl+O")).toBe("file.open");
    expect(stored.get("ui.shortcuts")).toBe("{}");
  });

  it("reads the stored keymap back", async () => {
    const first = await load();
    first.useKeymap.getState().setKeys("file.open", ["F2"]);
    const raw = first.stored.get("ui.shortcuts");
    const second = await load();
    second.stored.set("ui.shortcuts", raw ?? "");
    await second.useKeymap.getState().load();
    expect(second.shortcuts.commandFor(press("F2"), page)).toBe("file.open");
  });

  it("saves while typing, but does not turn pages or open tools", async () => {
    const { shortcuts, useWorkspace } = await load();
    // A document in front, or document commands are not available at all.
    useWorkspace.setState({ activeDoc: 1 });
    const session = useWorkspace.getState().activeSession();
    session.setState({ doc: 1 });
    expect(shortcuts.commandFor(press("s", { ctrl: true }), typing)).toBe("file.save");
    expect(shortcuts.commandFor(press("n"), typing)).toBeNull();
    expect(shortcuts.commandFor(press("n"), page)).toBe("view.nextPage");
  });

  it("needs a document for document commands", async () => {
    const { shortcuts } = await load();
    expect(shortcuts.commandFor(press("s", { ctrl: true }), page)).toBeNull();
    expect(shortcuts.commandFor(press("o", { ctrl: true }), page)).toBe("file.open");
  });

  it("leaves a dialog the keys it does not mean to pass", async () => {
    const { shortcuts } = await load();
    expect(shortcuts.commandFor(press("F1"), { typing: false, dialog: true })).toBe("app.shortcuts");
    expect(shortcuts.commandFor(press("Escape"), { typing: false, dialog: true })).toBeNull();
  });

  it("leaves a key alone that a focused control already handled", async () => {
    const { shortcuts } = await load();
    const handled = { ...press("o", { ctrl: true }), defaultPrevented: true };
    expect(shortcuts.commandFor(handled, page)).toBeNull();
  });

  it("pauses while F1 waits for a new key", async () => {
    const { shortcuts, useKeymap } = await load();
    useKeymap.getState().setRecording("file.open");
    expect(shortcuts.commandFor(press("o", { ctrl: true }), page)).toBeNull();
  });
});

describe("the command registry", () => {
  it("has unique ids, valid default keys, and no key claimed twice", async () => {
    await load();
    const { COMMANDS } = await import("../commands");
    const { normalize } = await import("@/state/keymap");
    const ids = COMMANDS.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
    const owner = new Map<string, string>();
    for (const c of COMMANDS) {
      for (const k of c.keys) {
        const n = normalize(k);
        expect(n, `${c.id}: ${k}`).not.toBeNull();
        const other = owner.get(n ?? "");
        expect(other, `${k} milik ${other} dan ${c.id}`).toBeUndefined();
        owner.set(n ?? "", c.id);
      }
    }
    // The keys SPEC 12 lists are all bound.
    for (const k of ["Ctrl+O", "Ctrl+W", "Ctrl+Tab", "Ctrl+S", "Ctrl+Shift+S", "Ctrl+F", "Ctrl+Shift+F", "Ctrl+Z", "Ctrl+Y", "Ctrl+0", "Ctrl+Plus", "Ctrl+-", "Ctrl+P", "F5", "F11", "Escape", "F1", "Ctrl+Shift+P"]) {
      expect(owner.has(k), k).toBe(true);
    }
  });
});
