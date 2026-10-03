import { describe, expect, it } from "vitest";
import {
  byCombo,
  comboOf,
  conflicts,
  decodeOverrides,
  display,
  effective,
  encodeOverrides,
  fuzzyScore,
  normalize,
} from "../keymap";

const key = (k: string, mods: Partial<{ ctrl: boolean; alt: boolean; shift: boolean }> = {}) => ({
  key: k,
  ctrlKey: mods.ctrl ?? false,
  altKey: mods.alt ?? false,
  shiftKey: mods.shift ?? false,
});

describe("comboOf", () => {
  it("writes modifiers in a fixed order and letters upper case", () => {
    expect(comboOf(key("s", { ctrl: true }))).toBe("Ctrl+S");
    expect(comboOf(key("S", { ctrl: true, shift: true }))).toBe("Ctrl+Shift+S");
    expect(comboOf(key("p", { ctrl: true, shift: true, alt: true }))).toBe("Ctrl+Alt+Shift+P");
    expect(comboOf(key("F5"))).toBe("F5");
    expect(comboOf(key("F3", { shift: true }))).toBe("Shift+F3");
    expect(comboOf(key("Tab", { ctrl: true, shift: true }))).toBe("Ctrl+Shift+Tab");
  });

  it("does not count Shift twice on a symbol it typed", () => {
    // Shift+= types "+": Ctrl++ and Ctrl+Shift+= are one combination.
    expect(comboOf(key("+", { ctrl: true, shift: true }))).toBe("Ctrl+Plus");
    expect(comboOf(key("+", { ctrl: true }))).toBe("Ctrl+Plus");
    expect(comboOf(key("=", { ctrl: true }))).toBe("Ctrl+=");
    expect(comboOf(key("-", { ctrl: true }))).toBe("Ctrl+-");
  });

  it("ignores a modifier pressed alone", () => {
    expect(comboOf(key("Control", { ctrl: true }))).toBeNull();
    expect(comboOf(key("Shift", { shift: true }))).toBeNull();
  });
});

describe("normalize and display", () => {
  it("reads what the registry and the store write", () => {
    expect(normalize("ctrl+s")).toBe("Ctrl+S");
    expect(normalize("Shift+Ctrl+s")).toBe("Ctrl+Shift+S");
    expect(normalize("Ctrl++")).toBe("Ctrl+Plus");
    expect(normalize("Ctrl+Plus")).toBe("Ctrl+Plus");
    expect(normalize("Escape")).toBe("Escape");
    expect(normalize("N")).toBe("N");
    expect(normalize("Hyper+S")).toBeNull();
    expect(normalize("")).toBeNull();
  });

  it("shows the plus key as a plus", () => {
    expect(display("Ctrl+Plus")).toBe("Ctrl++");
    expect(display("Ctrl+Shift+S")).toBe("Ctrl+Shift+S");
  });
});

describe("keymaps", () => {
  const defaults = new Map<string, string[]>([
    ["save", ["Ctrl+S"]],
    ["find", ["Ctrl+F"]],
    ["next", ["N", "J"]],
  ]);

  it("puts the user's changes over the defaults", () => {
    const map = effective(defaults, { find: ["Ctrl+Shift+F"], next: [] });
    expect(map.get("save")).toEqual(["Ctrl+S"]);
    expect(map.get("find")).toEqual(["Ctrl+Shift+F"]);
    expect(map.get("next")).toEqual([]);
    expect(byCombo(map).get("Ctrl+Shift+F")).toBe("find");
    expect(byCombo(map).get("N")).toBeUndefined();
  });

  it("finds who else has a key", () => {
    const map = effective(defaults, {});
    expect(conflicts(map, "Ctrl+S", "find")).toEqual(["save"]);
    expect(conflicts(map, "Ctrl+S", "save")).toEqual([]);
  });

  it("keeps only well-formed stored entries for known commands", () => {
    const known = new Set(defaults.keys());
    const raw = encodeOverrides({ save: ["ctrl+shift+s"], ghost: ["Ctrl+G"] });
    expect(decodeOverrides(raw, known)).toEqual({ save: ["Ctrl+Shift+S"] });
    expect(decodeOverrides("{not json", known)).toEqual({});
    expect(decodeOverrides('["a"]', known)).toEqual({});
    expect(decodeOverrides('{"save": ["Hyper+X", 3, "F2"]}', known)).toEqual({ save: ["F2"] });
    expect(decodeOverrides(null, known)).toEqual({});
  });
});

describe("fuzzyScore", () => {
  it("prefers a run of letters to scattered ones, and misses what is absent", () => {
    const run = fuzzyScore("simpan", "Simpan Sebagai…");
    const scattered = fuzzyScore("sms", "Simpan Sebagai…");
    expect(run).not.toBeNull();
    expect(scattered).not.toBeNull();
    expect(run as number).toBeGreaterThan(scattered as number);
    expect(fuzzyScore("xyz", "Simpan")).toBeNull();
    expect(fuzzyScore("", "Simpan")).toBe(0);
  });
});
