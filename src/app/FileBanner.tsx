/**
 * The strip above the page that says the file on disk is no longer the one
 * the tab has: another program wrote it, or it is gone.
 *
 * Not a dialog. The user may be halfway through a sentence in a text box, and
 * a modal that took the keyboard away would be worse than the problem it
 * reports. The strip stays until it is answered.
 */

import type { JSX } from "react";
import { Icon } from "@/design/Icon";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { acknowledgeDiskChange, reloadFromDisk, saveDocument } from "./fileActions";

const ACTION =
  "h-7 px-3 rounded-[6px] text-[12px] border border-[var(--izul-border)] bg-[var(--izul-surface)] hover:bg-[var(--izul-surface-raised)]";

export function FileBanner(): JSX.Element | null {
  const changed = useDocument((s) => s.file.changedOnDisk);
  const missing = useDocument((s) => s.file.missing);
  const dirty = useDocument((s) => s.dirty);
  if (!changed && !missing) return null;
  return (
    <div
      role="alert"
      className="shrink-0 flex items-center gap-3 px-3 py-2 bg-[var(--izul-warning-soft)] border-b border-[var(--izul-border)] text-[13px]"
    >
      <Icon name="warning" size={20} tone="amber" />
      <p className="flex-1 min-w-0">
        {missing ? t("file.missing") : dirty ? t("file.changedDirty") : t("file.changed")}
      </p>
      {missing ? (
        <button type="button" className={ACTION} onClick={() => void saveDocument(undefined, "saveAs")}>
          {t("menu.saveAs")}
        </button>
      ) : (
        <>
          <button
            type="button"
            className={`${ACTION} border-[var(--izul-accent)] text-[var(--izul-accent)] font-medium`}
            onClick={() => void reloadFromDisk()}
          >
            {t("file.reload")}
          </button>
          <button type="button" className={ACTION} title={t("file.ignoreHint")} onClick={() => void acknowledgeDiskChange()}>
            {t("file.ignore")}
          </button>
        </>
      )}
    </div>
  );
}
