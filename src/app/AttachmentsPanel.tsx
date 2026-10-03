/**
 * The "Lampiran" sidebar tab (SPEC 11.1, Phase 8): the files embedded in the
 * document, each saved where the user chooses. Nothing is opened from here —
 * an attachment is a file of any kind from whoever made the PDF, and running
 * it is the file manager's decision, not a PDF reader's.
 */

import { useEffect, useState, type JSX } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useDocument } from "@/state/documentStore";
import { Icon } from "@/design/Icon";
import { t } from "@/i18n";

interface Attachment {
  readonly index: number;
  readonly name: string;
  readonly size: number;
}

/** "12 KB", "3,4 MB": the size the way a file manager writes it. */
export function sizeLabel(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1).replace(".", ",")} MB`;
}

export function AttachmentsPanel(): JSX.Element {
  const doc = useDocument((s) => s.doc);
  const [items, setItems] = useState<readonly Attachment[] | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    setItems(null);
    setStatus(null);
    if (doc === null) return;
    let live = true;
    void invoke<Attachment[]>("attachments_list", { doc })
      .then((list) => live && setItems(list))
      .catch((e: unknown) => live && (setItems([]), setStatus(String(e))));
    return () => {
      live = false;
    };
  }, [doc]);

  async function saveOne(a: Attachment): Promise<void> {
    if (doc === null) return;
    const path = await saveDialog({ defaultPath: a.name });
    if (typeof path !== "string") return;
    try {
      await invoke("attachment_save", { doc, index: a.index, path });
      setStatus(t("attachments.saved").replace("{name}", a.name));
    } catch (e) {
      setStatus(String(e));
    }
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      {items === null ? (
        <p className="p-3 text-[13px] text-[var(--izul-text-dim)]">{t("attachments.loading")}</p>
      ) : items.length === 0 ? (
        <p className="p-3 text-[13px] text-[var(--izul-text-dim)]">{t("attachments.empty")}</p>
      ) : (
        <ul className="flex-1 min-h-0 overflow-auto p-1">
          {items.map((a) => (
            <li key={a.index} className="flex items-center gap-2 px-2 py-1.5 rounded-[8px] hover:bg-[var(--izul-surface-raised)]">
              <Icon name="attach" size={20} tone="neutral" />
              <span className="flex-1 min-w-0">
                <span className="block truncate text-[13px]" title={a.name}>
                  {a.name || t("attachments.unnamed")}
                </span>
                <span className="block text-[11px] text-[var(--izul-text-dim)]">{sizeLabel(a.size)}</span>
              </span>
              <button
                type="button"
                onClick={() => void saveOne(a)}
                aria-label={`${t("attachments.save")} — ${a.name}`}
                className="h-7 px-2 rounded-[6px] text-[12px] border border-[var(--izul-border)] hover:bg-[var(--izul-canvas)]"
              >
                {t("attachments.save")}
              </button>
            </li>
          ))}
        </ul>
      )}
      {status && (
        <p role="status" className="p-2 text-[12px] text-[var(--izul-text-dim)] border-t border-[var(--izul-border)]">
          {status}
        </p>
      )}
    </div>
  );
}
