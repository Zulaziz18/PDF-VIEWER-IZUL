/**
 * The shortcut list under F1 (SPEC 12): every command and its keys, each
 * changeable — press "Ubah", then the new key. A key already in use moves to
 * the command being changed, and the dialog says from which, so nothing is
 * bound twice without the user seeing it. Stored in SQLite (`keymapStore`).
 */

import { useEffect, useMemo, useRef, useState, type JSX } from "react";
import { fill } from "./fileActions";
import { t } from "@/i18n";
import { bindable, comboOf, conflicts, display } from "@/state/keymap";
import { defaultKeys, useKeymap } from "@/state/keymapStore";
import { useUi } from "@/state/uiStore";
import { COMMAND_BY_ID, COMMANDS, GROUP_LABEL, type CommandGroup } from "./commands";

const GROUPS: readonly CommandGroup[] = ["file", "edit", "view", "annotate", "pages", "protect", "convert", "app"];

const SMALL =
  "h-7 px-2.5 rounded-[6px] text-[12px] border border-[var(--izul-border)] hover:bg-[var(--izul-surface-raised)] disabled:opacity-40";

export function ShortcutsDialog(): JSX.Element | null {
  const open = useUi((s) => s.shortcutsOpen);
  const keys = useKeymap((s) => s.keys);
  const recording = useKeymap((s) => s.recording);
  const ref = useRef<HTMLDialogElement>(null);
  const [filter, setFilter] = useState("");
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setFilter("");
    setNote(null);
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
  }, [open]);

  // While a command waits for its key, the next key press is the answer.
  useEffect(() => {
    if (recording === null) return;
    const onKey = (e: KeyboardEvent): void => {
      e.preventDefault();
      e.stopPropagation();
      const store = useKeymap.getState();
      if (e.key === "Escape") {
        store.setRecording(null);
        return;
      }
      const combo = comboOf(e);
      if (combo === null) return;
      if (!bindable(combo)) {
        setNote(fill(t("shortcuts.reserved"), { key: display(combo) }));
        store.setRecording(null);
        return;
      }
      const taken = conflicts(store.keys, combo, recording);
      const current = store.keys.get(recording) ?? [];
      store.setKeys(recording, [combo, ...current.filter((k) => k !== combo)].slice(0, 2));
      setNote(
        taken.length > 0
          ? fill(t("shortcuts.moved"), {
              key: display(combo),
              from: taken.map((id) => t(COMMAND_BY_ID.get(id)?.label ?? "cmd.escape")).join(", "),
            })
          : null,
      );
      store.setRecording(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording]);

  const shown = useMemo(() => {
    const q = filter.trim().toLocaleLowerCase("id");
    return COMMANDS.filter((c) => !c.hidden).filter(
      (c) => q.length === 0 || t(c.label).toLocaleLowerCase("id").includes(q) || (keys.get(c.id) ?? []).some((k) => display(k).toLowerCase().includes(q)),
    );
  }, [filter, keys]);

  if (!open) return null;
  const close = (): void => {
    useKeymap.getState().setRecording(null);
    ref.current?.close();
    useUi.getState().setShortcutsOpen(false);
  };

  return (
    <dialog
      ref={ref}
      aria-labelledby="shortcuts-title"
      onCancel={(e) => {
        e.preventDefault();
        if (recording === null) close();
      }}
      className="m-auto w-[720px] max-w-[calc(100vw-32px)] h-[80vh] p-0 rounded-[12px] border border-[var(--izul-border)] bg-[var(--izul-surface)] text-[var(--izul-text)] shadow-[0_12px_40px_rgba(0,0,0,0.25)] backdrop:bg-black/30"
    >
      <div className="h-full flex flex-col">
        <header className="p-5 pb-3 flex flex-col gap-3 border-b border-[var(--izul-border)]">
          <div className="flex items-center gap-3">
            <h2 id="shortcuts-title" className="flex-1 text-[17px] font-semibold">
              {t("shortcuts.title")}
            </h2>
            <button type="button" className={SMALL} onClick={() => useKeymap.getState().resetAll()}>
              {t("shortcuts.resetAll")}
            </button>
            <button type="button" className={SMALL} onClick={close}>
              {t("close.cancel")}
            </button>
          </div>
          <input
            aria-label={t("shortcuts.filter")}
            placeholder={t("shortcuts.filter")}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            className="h-8 px-2 rounded-[6px] text-[13px] border border-[var(--izul-border)] bg-[var(--izul-canvas)]"
          />
          <p className="text-[12px] text-[var(--izul-text-dim)]" role="status">
            {recording !== null ? fill(t("shortcuts.recording"), { name: t(COMMAND_BY_ID.get(recording)?.label ?? "cmd.escape") }) : (note ?? t("shortcuts.hint"))}
          </p>
        </header>
        <div className="flex-1 min-h-0 overflow-auto px-5 py-2">
          {GROUPS.map((g) => {
            const rows = shown.filter((c) => c.group === g);
            if (rows.length === 0) return null;
            return (
              <section key={g} aria-label={t(GROUP_LABEL[g])} className="py-2">
                <h3 className="py-1 text-[11px] font-semibold uppercase tracking-wide text-[var(--izul-text-dim)]">{t(GROUP_LABEL[g])}</h3>
                <table className="w-full text-[13px]">
                  <tbody>
                    {rows.map((cmd) => {
                      const mine = keys.get(cmd.id) ?? [];
                      const changed = mine.join() !== defaultKeys(cmd.id).join();
                      const waiting = recording === cmd.id;
                      return (
                        <tr key={cmd.id} className="h-9 border-b border-[var(--izul-border)] last:border-0">
                          <td className="pr-3">{t(cmd.label)}</td>
                          <td className="w-[180px]">
                            <span className="flex gap-1 flex-wrap">
                              {mine.length === 0 && <span className="text-[var(--izul-text-dim)]">—</span>}
                              {mine.map((k) => (
                                <kbd
                                  key={k}
                                  className="px-1.5 h-5 grid place-items-center rounded-[4px] border border-[var(--izul-border)] bg-[var(--izul-surface-raised)] text-[11px] font-mono"
                                >
                                  {display(k)}
                                </kbd>
                              ))}
                            </span>
                          </td>
                          <td className="w-[220px] text-right whitespace-nowrap">
                            <span className="inline-flex gap-1">
                              <button
                                type="button"
                                aria-label={fill(t("shortcuts.changeFor"), { name: t(cmd.label) })}
                                aria-pressed={waiting}
                                className={`${SMALL} ${waiting ? "border-[var(--izul-accent)] text-[var(--izul-accent)]" : ""}`}
                                onClick={() => {
                                  setNote(null);
                                  useKeymap.getState().setRecording(waiting ? null : cmd.id);
                                }}
                              >
                                {waiting ? t("shortcuts.pressKey") : t("shortcuts.change")}
                              </button>
                              <button
                                type="button"
                                aria-label={fill(t("shortcuts.clearFor"), { name: t(cmd.label) })}
                                disabled={mine.length === 0}
                                className={SMALL}
                                onClick={() => useKeymap.getState().setKeys(cmd.id, [])}
                              >
                                {t("shortcuts.clear")}
                              </button>
                              <button
                                type="button"
                                aria-label={fill(t("shortcuts.resetFor"), { name: t(cmd.label) })}
                                disabled={!changed}
                                className={SMALL}
                                onClick={() => useKeymap.getState().reset(cmd.id)}
                              >
                                {t("shortcuts.reset")}
                              </button>
                            </span>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </section>
            );
          })}
        </div>
      </div>
    </dialog>
  );
}
