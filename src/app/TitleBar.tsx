/**
 * Custom title bar (SPEC 12).
 *
 * The window is undecorated so the title bar and the tab strip read as one
 * surface. The version comes from `version.json` through the backend, so it
 * can never disagree with the About box or the installer.
 */

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useDocument } from "@/state/documentStore";

interface VersionInfo {
  name: string;
  codename: string;
  version: string;
  phase: string;
  phaseName: string;
  pdfiumVersion: string;
}

export function TitleBar(): React.JSX.Element {
  const [info, setInfo] = useState<VersionInfo | null>(null);
  const path = useDocument((s) => s.path);

  useEffect(() => {
    void invoke<VersionInfo>("app_version").then(setInfo).catch(() => setInfo(null));
  }, []);

  const name = path?.split(/[\\/]/).pop();

  return (
    <header
      data-tauri-drag-region
      className="h-9 shrink-0 flex items-center px-3 gap-3 border-b border-[var(--izul-border)] bg-[var(--izul-surface)] select-none"
    >
      <span className="font-medium">{info?.name ?? "PDF Studio Izul"}</span>
      {info && (
        <span
          className="text-[12px] text-[var(--izul-text-dim)]"
          title={`Fase ${info.phase} — ${info.phaseName} · PDFium ${info.pdfiumVersion}`}
        >
          {info.version} ({info.codename})
        </span>
      )}
      {name && <span className="text-[12px] text-[var(--izul-text-dim)] truncate">— {name}</span>}
    </header>
  );
}
