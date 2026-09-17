/**
 * What the application shows before anything is open.
 *
 * SPEC 12 asks for this to be designed seriously rather than left as a blank
 * grey rectangle: it is the first thing a user sees. Phase 2 gives the recent
 * list its covers, which is what makes it a shelf rather than a list of
 * filenames — a reader recognises a document's first page long before they read
 * its name.
 *
 * A cover exists only for a file this application has rendered before. Making
 * one on demand would mean opening every file in the list in a worker, which is
 * precisely the cost a recent-files panel must not have, so a file without one
 * gets a plain card and nothing is rendered here at all.
 */

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { t } from "@/i18n";
import { useWorkspace } from "@/state/workspaceStore";
import { useDropTarget } from "./dropTarget";

interface RecentFile {
  readonly path: string;
  readonly name: string;
  readonly last_opened: number | null;
  readonly pinned: boolean;
  /** False when the file has moved or been deleted since it was last opened. */
  readonly available: boolean;
  /** A `data:` PNG of the first page, when one was captured while it was open. */
  readonly cover: string | null;
}

export function EmptyState(): React.JSX.Element {
  const [recent, setRecent] = useState<RecentFile[]>([]);
  const dropping = useDropTarget();

  useEffect(() => {
    void invoke<RecentFile[]>("recent_files")
      .then(setRecent)
      .catch(() => setRecent([]));
  }, []);

  async function pick(): Promise<void> {
    const chosen = await openDialog({
      multiple: false,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof chosen === "string") {
      await useWorkspace.getState().openFile(chosen);
    }
  }

  function togglePin(file: RecentFile): void {
    void invoke("pin_recent", { path: file.path, pinned: !file.pinned })
      .then(() => invoke<RecentFile[]>("recent_files"))
      .then(setRecent)
      .catch(() => {
        // Pinning is a convenience; a failure leaves the list as it was.
      });
  }

  return (
    <div
      className={[
        "flex-1 overflow-auto grid place-items-center p-6",
        dropping ? "outline outline-2 -outline-offset-4 outline-[var(--izul-accent)]" : "",
      ].join(" ")}
    >
      <div className="w-[640px] max-w-full">
        <h1 className="text-[28px] font-semibold tracking-tight">{t("app.name")}</h1>
        <p className="text-[var(--izul-text-dim)] mt-1">{t("empty.subtitle")}</p>

        <button
          type="button"
          onClick={() => void pick()}
          className="mt-6 w-full rounded-[8px] bg-[var(--izul-accent)] text-white py-2.5 font-medium"
        >
          {t("empty.open")}
        </button>

        <h2 className="mt-8 text-[var(--izul-text-dim)] uppercase tracking-wide text-[12px]">
          {t("empty.recent")}
        </h2>
        {recent.length === 0 ? (
          <p className="mt-2 text-[var(--izul-text-dim)]">{t("empty.noRecent")}</p>
        ) : (
          <ul className="mt-3 grid grid-cols-[repeat(auto-fill,minmax(120px,1fr))] gap-3">
            {recent.map((file) => (
              <li key={file.path} className="relative">
                <button
                  type="button"
                  onClick={() => void useWorkspace.getState().openFile(file.path)}
                  disabled={!file.available}
                  title={file.available ? file.path : `${file.path} — ${t("empty.missing")}`}
                  className="w-full text-left rounded-[8px] p-2 hover:bg-[var(--izul-surface-raised)] disabled:opacity-40"
                >
                  <div className="aspect-[3/4] rounded-[6px] overflow-hidden bg-[var(--izul-surface)] border border-[var(--izul-border)] grid place-items-center">
                    {file.cover !== null ? (
                      <img
                        src={file.cover}
                        alt=""
                        className="w-full h-full object-cover object-top"
                      />
                    ) : (
                      <span aria-hidden="true" className="text-[var(--izul-text-dim)] text-[22px]">
                        PDF
                      </span>
                    )}
                  </div>
                  <span className="mt-1.5 block truncate text-[13px]">{file.name}</span>
                </button>
                <button
                  type="button"
                  aria-label={file.pinned ? t("empty.unpin") : t("empty.pin")}
                  aria-pressed={file.pinned}
                  onClick={() => togglePin(file)}
                  className={[
                    "absolute top-3 right-3 w-6 h-6 rounded-[6px] text-[12px] bg-[var(--izul-surface)]/85",
                    file.pinned ? "opacity-100" : "opacity-0 hover:opacity-100 focus:opacity-100",
                  ].join(" ")}
                >
                  {file.pinned ? "★" : "☆"}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
