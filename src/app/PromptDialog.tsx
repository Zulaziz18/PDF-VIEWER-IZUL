/**
 * The question the application is waiting on (`useUi.ask`): save before
 * closing, restore a draft.
 *
 * A native `<dialog>` like About, so focus is trapped and Escape works without
 * code of ours — Escape arrives as `cancel`, which answers with the prompt's
 * cancel id rather than closing it unanswered.
 */

import { useEffect, useRef, type JSX } from "react";
import { Icon } from "@/design/Icon";
import { useUi, type PromptButton } from "@/state/uiStore";

const BUTTON: Record<NonNullable<PromptButton["kind"]>, string> = {
  primary: "bg-[var(--izul-accent)] text-[var(--izul-on-accent)] font-medium hover:brightness-110",
  danger: "border border-[var(--izul-border)] text-[var(--izul-danger)] hover:bg-[var(--izul-surface-raised)]",
  default: "border border-[var(--izul-border)] hover:bg-[var(--izul-surface-raised)]",
};

export function PromptDialog(): JSX.Element | null {
  const prompt = useUi((s) => s.prompt);
  const ref = useRef<HTMLDialogElement>(null);
  const primary = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!prompt) return;
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    // The safe default has the focus, so Enter does what the button says.
    primary.current?.focus();
  }, [prompt]);

  if (!prompt) return null;
  const { spec } = prompt;
  const icon = spec.icon === "draft" ? "draft" : spec.icon === "info" ? "info" : "warning";
  const tone = spec.icon === "draft" ? "blue" : spec.icon === "info" ? "blue" : "amber";
  return (
    <dialog
      ref={ref}
      aria-labelledby="prompt-title"
      aria-describedby="prompt-body"
      onCancel={(e) => {
        e.preventDefault();
        useUi.getState().answer(spec.cancelId);
      }}
      className="m-auto w-[460px] max-w-[calc(100vw-32px)] p-0 rounded-[12px] border border-[var(--izul-border)] bg-[var(--izul-surface)] text-[var(--izul-text)] shadow-[0_12px_40px_rgba(0,0,0,0.25)] backdrop:bg-black/30"
    >
      <div className="p-6 flex gap-4">
        <Icon name={icon} size={28} tone={tone} />
        <div className="flex-1 min-w-0 flex flex-col gap-2">
          <h2 id="prompt-title" className="text-[17px] font-semibold">
            {spec.title}
          </h2>
          <p id="prompt-body" className="text-[13px] break-words">
            {spec.body}
          </p>
          {spec.detail && <p className="text-[12px] text-[var(--izul-text-dim)]">{spec.detail}</p>}
        </div>
      </div>
      <div className="px-6 pb-5 flex justify-end gap-2">
        {spec.buttons.map((b) => (
          <button
            key={b.id}
            ref={b.kind === "primary" ? primary : undefined}
            type="button"
            onClick={() => useUi.getState().answer(b.id)}
            className={["h-9 px-4 rounded-[8px] text-[13px]", BUTTON[b.kind ?? "default"]].join(" ")}
          >
            {b.label}
          </button>
        ))}
      </div>
    </dialog>
  );
}
