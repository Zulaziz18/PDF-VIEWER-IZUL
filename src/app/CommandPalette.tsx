/**
 * The command palette (Ctrl+Shift+P, SPEC 12): every command by name, with
 * its shortcut beside it — the way to find a command without knowing which
 * ribbon tab it is under, and to learn its key while doing so.
 */

import { useEffect, useMemo, useRef, useState, type JSX } from "react";
import { t } from "@/i18n";
import { display, fuzzyScore } from "@/state/keymap";
import { useKeymap } from "@/state/keymapStore";
import { useDocument } from "@/state/documentStore";
import { useUi } from "@/state/uiStore";
import { COMMANDS, GROUP_LABEL, type Command } from "./commands";

/** The commands matching `query`, best first; all, in table order, when empty. */
export function rank(query: string, commands: readonly Command[], label: (c: Command) => string): Command[] {
  if (query.trim().length === 0) return [...commands];
  return commands
    .map((c) => ({ c, s: fuzzyScore(query, `${label(c)} ${t(GROUP_LABEL[c.group])}`) }))
    .filter((x): x is { c: Command; s: number } => x.s !== null)
    .sort((a, b) => b.s - a.s)
    .map((x) => x.c);
}

export function CommandPalette(): JSX.Element | null {
  const open = useUi((s) => s.paletteOpen);
  const hasDoc = useDocument((s) => s.doc !== null);
  const keys = useKeymap((s) => s.keys);
  const ref = useRef<HTMLDialogElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [at, setAt] = useState(0);

  const offered = useMemo(
    () => COMMANDS.filter((c) => !c.hidden && (!c.needsDoc || hasDoc) && c.id !== "app.palette"),
    [hasDoc],
  );
  const list = useMemo(() => rank(query, offered, (c) => t(c.label)), [query, offered]);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setAt(0);
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    input.current?.focus();
  }, [open]);

  useEffect(() => setAt(0), [query]);

  if (!open) return null;
  const close = (): void => {
    ref.current?.close();
    useUi.getState().setPaletteOpen(false);
  };
  const run = (cmd: Command | undefined): void => {
    if (!cmd) return;
    close();
    // After the dialog is gone, so what the command opens gets the focus.
    queueMicrotask(() => cmd.run());
  };

  return (
    <dialog
      ref={ref}
      aria-label={t("palette.command")}
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      onClick={(e) => {
        if (e.target === ref.current) close();
      }}
      className="mt-[12vh] mx-auto w-[560px] max-w-[calc(100vw-32px)] p-0 rounded-[12px] border border-[var(--izul-border)] bg-[var(--izul-surface)] text-[var(--izul-text)] shadow-[0_12px_40px_rgba(0,0,0,0.25)] backdrop:bg-black/20"
    >
      <input
        ref={input}
        role="combobox"
        aria-expanded="true"
        aria-controls="palette-list"
        aria-activedescendant={list[at] ? `palette-${list[at].id}` : undefined}
        aria-label={t("palette.placeholder")}
        placeholder={t("palette.placeholder")}
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setAt((i) => Math.min(i + 1, Math.max(0, list.length - 1)));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setAt((i) => Math.max(i - 1, 0));
          } else if (e.key === "Enter") {
            e.preventDefault();
            run(list[at]);
          }
        }}
        className="w-full h-12 px-4 text-[15px] bg-transparent border-b border-[var(--izul-border)] outline-none"
      />
      <ul id="palette-list" role="listbox" aria-label={t("palette.command")} className="max-h-[50vh] overflow-auto py-1">
        {list.length === 0 && <li className="px-4 py-3 text-[13px] text-[var(--izul-text-dim)]">{t("palette.none")}</li>}
        {list.map((cmd, i) => (
          <li
            key={cmd.id}
            id={`palette-${cmd.id}`}
            role="option"
            aria-selected={i === at}
            onMouseMove={() => setAt(i)}
            onClick={() => run(cmd)}
            className={[
              "flex items-center gap-3 px-4 h-9 cursor-pointer text-[13px]",
              i === at ? "bg-[var(--izul-accent-soft)]" : "",
            ].join(" ")}
          >
            <span className="w-24 shrink-0 text-[11px] text-[var(--izul-text-dim)] truncate">{t(GROUP_LABEL[cmd.group])}</span>
            <span className="flex-1 min-w-0 truncate">{t(cmd.label)}</span>
            {(keys.get(cmd.id) ?? []).slice(0, 2).map((k) => (
              <kbd
                key={k}
                className="px-1.5 h-5 grid place-items-center rounded-[4px] border border-[var(--izul-border)] bg-[var(--izul-surface-raised)] text-[11px] font-mono"
              >
                {display(k)}
              </kbd>
            ))}
          </li>
        ))}
      </ul>
    </dialog>
  );
}
