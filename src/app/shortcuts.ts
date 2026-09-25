/**
 * Keyboard shortcuts (SPEC 12), read from the command registry and the
 * user's keymap (Phase 8) — the table under F1 is the table that runs.
 *
 * Scrolling keys — arrows, Page Up/Down, Home, End — are deliberately not
 * bound: the viewport's scrolling element handles them natively, which is
 * both correct and what a screen reader expects.
 */

import { useEffect } from "react";
import { comboOf, type KeyLike } from "@/state/keymap";
import { useKeymap } from "@/state/keymapStore";
import { available, COMMAND_BY_ID } from "./commands";

/** Whether the key press lands in something that takes text. */
function typingIn(target: EventTarget | null): boolean {
  const el = target as { tagName?: string; isContentEditable?: boolean } | null;
  const tag = el?.tagName?.toUpperCase();
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el?.isContentEditable === true;
}

/** Where a key press lands, as far as the shortcuts care. */
export interface KeyContext {
  /** Focus is in something that takes text. */
  readonly typing: boolean;
  /** A modal dialog is open. */
  readonly dialog: boolean;
}

/** The command a key press runs now, or `null`. Pure apart from the stores. */
export function commandFor(e: KeyLike, where: KeyContext): string | null {
  const keymap = useKeymap.getState();
  // The F1 dialog is waiting for a new key: that key is its answer.
  if (keymap.recording !== null) return null;
  const combo = comboOf(e);
  if (combo === null) return null;
  const id = keymap.lookup.get(combo);
  if (id === undefined) return null;
  const cmd = COMMAND_BY_ID.get(id);
  if (!cmd || !available(cmd)) return null;
  if (!cmd.whileTyping && where.typing) return null;
  // A dialog in front owns the keyboard, except for what is meant to reach
  // past it (saving while a dialog is open is still saving).
  if (where.dialog && !cmd.whileTyping) return null;
  return id;
}

export function useShortcuts(): void {
  useEffect(() => {
    void useKeymap.getState().load();
    const onKey = (e: KeyboardEvent): void => {
      const id = commandFor(e, {
        typing: typingIn(e.target),
        dialog: document.querySelector("dialog[open]") !== null,
      });
      if (id === null) return;
      e.preventDefault();
      COMMAND_BY_ID.get(id)?.run();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
