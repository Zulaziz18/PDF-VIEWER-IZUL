/**
 * What the application shows before anything is open.
 *
 * SPEC 12 asks for this to be designed seriously rather than left as a blank
 * grey rectangle: it is the first thing a user sees.
 */

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";

export function EmptyState(): React.JSX.Element {
  const openDoc = useDocument((s) => s.open);
  const [recent, setRecent] = useState<string[]>([]);

  useEffect(() => {
    void invoke<string[]>("recent_files").then(setRecent).catch(() => setRecent([]));
  }, []);

  async function pick(): Promise<void> {
    const chosen = await openDialog({
      multiple: false,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof chosen === "string") {
      await openDoc(chosen);
    }
  }

  return (
    <div className="flex-1 grid place-items-center">
      <div className="w-[420px] max-w-full">
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
          <ul className="mt-2 space-y-1">
            {recent.map((path) => (
              <li key={path}>
                <button
                  type="button"
                  onClick={() => void openDoc(path)}
                  title={path}
                  className="w-full text-left truncate rounded-[8px] px-2 py-1.5 hover:bg-[var(--izul-surface-raised)]"
                >
                  {path.split(/[\\/]/).pop()}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
