/**
 * The About dialog (SPEC 0: the version from `version.json` is shown in the
 * title bar *and* in About; SPEC 15: a way to find the log folder).
 *
 * A native `<dialog>`, so focus is trapped and Escape closes it without any
 * code of ours.
 */

import { useEffect, useRef, useState, type JSX } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Logo } from "@/design/Logo";
import { t } from "@/i18n";
import { useUi } from "@/state/uiStore";

interface VersionInfo {
  name: string;
  codename: string;
  version: string;
  phase: string;
  phaseName: string;
  pdfiumVersion: string;
}

export function About(): JSX.Element | null {
  const open = useUi((s) => s.aboutOpen);
  const ref = useRef<HTMLDialogElement>(null);
  const [info, setInfo] = useState<VersionInfo | null>(null);
  const [logs, setLogs] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    ref.current?.showModal();
    void invoke<VersionInfo>("app_version").then(setInfo).catch(() => setInfo(null));
    void invoke<string>("log_folder").then(setLogs).catch(() => setLogs(null));
  }, [open]);

  if (!open) return null;
  const facts: Array<[string, string]> = info
    ? [
        [t("about.version"), `${info.version} (${info.codename})`],
        [t("about.phase"), `${info.phase} — ${info.phaseName}`],
        [t("about.pdfium"), `PDFium ${info.pdfiumVersion}`],
      ]
    : [];
  return (
    <dialog
      ref={ref}
      aria-labelledby="about-title"
      onClose={() => useUi.getState().setAboutOpen(false)}
      className="m-auto w-[420px] max-w-[calc(100vw-32px)] p-0 rounded-[12px] border border-[var(--izul-border)] bg-[var(--izul-surface)] text-[var(--izul-text)] shadow-[0_12px_40px_rgba(0,0,0,0.25)] backdrop:bg-black/30"
    >
      <div className="p-6 flex flex-col gap-4">
        <div className="flex items-center gap-3">
          <Logo size={40} />
          <div>
            <h2 id="about-title" className="text-[20px] font-semibold">{info?.name ?? t("app.name")}</h2>
            <p className="text-[13px] text-[var(--izul-text-dim)]">{t("app.tagline")}</p>
          </div>
        </div>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-[13px]">
          {facts.map(([k, v]) => (
            <div key={k} className="contents">
              <dt className="text-[var(--izul-text-dim)]">{k}</dt>
              <dd className="m-0">{v}</dd>
            </div>
          ))}
          {logs && (
            <div className="contents">
              <dt className="text-[var(--izul-text-dim)]">{t("about.logs")}</dt>
              <dd className="m-0 break-all select-text">{logs}</dd>
            </div>
          )}
        </dl>
        <p className="text-[12px] text-[var(--izul-text-dim)]">{t("about.licenses")}</p>
        <form method="dialog" className="flex justify-end">
          <button className="h-9 px-4 rounded-[8px] bg-[var(--izul-accent)] text-[var(--izul-on-accent)] font-medium">
            {t("about.close")}
          </button>
        </form>
      </div>
    </dialog>
  );
}
