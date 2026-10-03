/**
 * Key combinations (Phase 8): how a key press is written down, compared, and
 * stored, so that shortcuts can be changed by the user (SPEC 12) and the
 * list under F1 is the list that runs.
 *
 * A combination is written `Ctrl+Shift+S`: modifiers in a fixed order
 * (Ctrl, Alt, Shift), then the key — letters upper case, `Plus` for "+",
 * `Space` for " ", and the browser's own names for the rest (`F5`, `Tab`,
 * `Delete`, `ArrowLeft`). For a symbol key, Shift is part of the symbol
 * (Shift+= types "+"), so it is not written twice: Ctrl+Shift+= and Ctrl++
 * are both `Ctrl+Plus`.
 *
 * Pure: no store, no DOM — the rules are tested on their own.
 */

export interface KeyLike {
  readonly key: string;
  readonly ctrlKey: boolean;
  readonly metaKey?: boolean;
  readonly altKey: boolean;
  readonly shiftKey: boolean;
}

/** Keys that, pressed alone, are a modifier being held — no combination yet. */
const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta", "AltGraph", "CapsLock", "OS"]);

function keyName(key: string): string | null {
  if (MODIFIER_KEYS.has(key) || key === "Unidentified" || key === "Dead" || key === "") return null;
  if (key === "+") return "Plus";
  if (key === " ") return "Space";
  if (key.length === 1) return key.toUpperCase();
  if (key === "Esc") return "Escape";
  return key;
}

/** Whether a key name is a symbol typed with the help of Shift. */
function isSymbol(name: string): boolean {
  return name === "Plus" || (name.length === 1 && !/[A-Z0-9]/u.test(name));
}

/** The combination a key press makes, or `null` for a modifier alone. */
export function comboOf(e: KeyLike): string | null {
  const name = keyName(e.key);
  if (name === null) return null;
  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey === true) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey && !isSymbol(name)) parts.push("Shift");
  parts.push(name);
  return parts.join("+");
}

/** A stored combination in canonical form, or `null` when it is not one. */
export function normalize(combo: string): string | null {
  const raw = combo.trim();
  if (raw.length === 0) return null;
  // "Ctrl++" ends in the plus key itself.
  const tokens = raw.endsWith("++") ? [...raw.slice(0, -2).split("+"), "+"] : raw.split("+");
  const key = tokens.pop();
  if (key === undefined || key.length === 0) return null;
  const mods = new Set(tokens.map((t) => t.trim().toLowerCase()));
  for (const m of mods) if (!["ctrl", "alt", "shift"].includes(m)) return null;
  return comboOf({
    key: key === "Plus" ? "+" : key === "Space" ? " " : key.length === 1 ? key.toLowerCase() : key,
    ctrlKey: mods.has("ctrl"),
    altKey: mods.has("alt"),
    shiftKey: mods.has("shift"),
  });
}

/** How a combination reads on screen: `Ctrl+Plus` shows as `Ctrl++`. */
export function display(combo: string): string {
  return combo
    .split("+")
    .map((t) => (t === "Plus" ? "+" : t === "ArrowLeft" ? "←" : t === "ArrowRight" ? "→" : t === "ArrowUp" ? "↑" : t === "ArrowDown" ? "↓" : t))
    .join("+");
}

/** Whether a combination may be bound at all: keys Windows owns are not. */
export function bindable(combo: string): boolean {
  const reserved = ["Alt+F4", "Ctrl+Alt+Delete", "Alt+Tab"];
  return !reserved.includes(combo);
}

export type Keymap = ReadonlyMap<string, readonly string[]>;

/** The defaults, with the user's changes on top (a command → its keys). */
export function effective(
  defaults: ReadonlyMap<string, readonly string[]>,
  overrides: Readonly<Record<string, readonly string[]>>,
): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const [id, keys] of defaults) {
    const own = overrides[id];
    const chosen = own !== undefined ? own : keys;
    out.set(id, chosen.map((k) => normalize(k)).filter((k): k is string => k !== null));
  }
  return out;
}

/** Command ids by combination; the first command claiming a key wins. */
export function byCombo(map: Keymap): Map<string, string> {
  const out = new Map<string, string>();
  for (const [id, keys] of map) for (const k of keys) if (!out.has(k)) out.set(k, id);
  return out;
}

/** Other commands already bound to `combo`. */
export function conflicts(map: Keymap, combo: string, except: string): string[] {
  const out: string[] = [];
  for (const [id, keys] of map) if (id !== except && keys.includes(combo)) out.push(id);
  return out;
}

/** Stored form: only what differs from the defaults, as JSON. */
export function encodeOverrides(overrides: Readonly<Record<string, readonly string[]>>): string {
  return JSON.stringify(overrides);
}

/** The stored overrides, keeping only well-formed entries for known commands. */
export function decodeOverrides(raw: string | null, known: ReadonlySet<string>): Record<string, string[]> {
  if (raw === null) return {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {};
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return {};
  const out: Record<string, string[]> = {};
  for (const [id, keys] of Object.entries(parsed as Record<string, unknown>)) {
    if (!known.has(id) || !Array.isArray(keys)) continue;
    out[id] = keys
      .filter((k): k is string => typeof k === "string")
      .map((k) => normalize(k))
      .filter((k): k is string => k !== null);
  }
  return out;
}

/**
 * How well `query` matches `label` for the command palette: every character
 * of the query in order, word starts and runs scoring higher. `null` when it
 * does not match at all.
 */
export function fuzzyScore(query: string, label: string): number | null {
  const q = query.trim().toLocaleLowerCase("id");
  if (q.length === 0) return 0;
  const l = label.toLocaleLowerCase("id");
  if (l.includes(q)) return 1000 - l.indexOf(q) - (l.length - q.length) / 100;
  let score = 0;
  let at = 0;
  let run = 0;
  for (const ch of q) {
    if (ch === " ") continue;
    const found = l.indexOf(ch, at);
    if (found < 0) return null;
    const wordStart = found === 0 || l[found - 1] === " ";
    run = found === at ? run + 1 : 0;
    score += 1 + (wordStart ? 5 : 0) + run * 2;
    at = found + 1;
  }
  return score;
}
