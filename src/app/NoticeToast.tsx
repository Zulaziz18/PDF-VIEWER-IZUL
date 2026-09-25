/**
 * What just happened, in the bottom corner: "Tersimpan", "Diekspor", or why
 * not. A success fades on its own; a failure stays until it is dismissed,
 * because a save that failed is the one message that must not be missed.
 */

import { useEffect, type JSX } from "react";
import { Icon } from "@/design/Icon";
import { t } from "@/i18n";
import { useUi } from "@/state/uiStore";

const OK_MS = 6000;

export function NoticeToast(): JSX.Element | null {
  const notice = useUi((s) => s.notice);

  useEffect(() => {
    // An ok with a detail carries a warning worth reading ("the marks are
    // saved but not applied"); it stays until closed.
    if (!notice || notice.kind !== "ok" || notice.detail) return;
    const timer = window.setTimeout(() => {
      if (useUi.getState().notice === notice) useUi.getState().notify(null);
    }, OK_MS);
    return () => window.clearTimeout(timer);
  }, [notice]);

  if (!notice) return null;
  const error = notice.kind === "error";
  return (
    <div
      role={error ? "alert" : "status"}
      className="fixed right-4 bottom-12 z-40 w-[380px] max-w-[calc(100vw-32px)] flex gap-3 p-3 rounded-[10px] border border-[var(--izul-border)] bg-[var(--izul-surface)] text-[var(--izul-text)] shadow-[0_8px_24px_rgba(0,0,0,0.18)]"
    >
      <Icon name={error ? "error" : "checkmark"} size={20} tone={error ? "rose" : "green"} />
      <div className="flex-1 min-w-0">
        <p className="text-[13px] font-medium break-words">{notice.text}</p>
        {notice.detail && (
          <p className="mt-0.5 text-[12px] text-[var(--izul-text-dim)] break-all select-text">{notice.detail}</p>
        )}
      </div>
      <button
        type="button"
        aria-label={t("notice.dismiss")}
        title={t("notice.dismiss")}
        onClick={() => useUi.getState().notify(null)}
        className="shrink-0 w-6 h-6 grid place-items-center rounded-[6px] hover:bg-[var(--izul-chrome-hover)]"
      >
        <Icon name="dismiss" size={16} />
      </button>
    </div>
  );
}
