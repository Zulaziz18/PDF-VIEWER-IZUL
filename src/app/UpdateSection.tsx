/**
 * "Periksa pembaruan" in About (7.0.1). The application goes online only when
 * this button is pressed (SPEC 2, amended 3 October 2026): nothing checks at
 * startup or in the background. The check and the install are Rust commands
 * (`src-tauri/src/updates.rs`); the installer's signature is checked there
 * before it runs.
 *
 * Installing ends the application, so unsaved documents are asked about
 * first, exactly as closing the window asks.
 */

import { useEffect, useState, type JSX } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t } from "@/i18n";
import { fill, requestCloseWindow } from "./fileActions";

interface UpdateCheck {
  current: string;
  available: string | null;
  notes: string | null;
  portable: boolean;
  releasesUrl: string;
}

type Phase =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "result"; check: UpdateCheck }
  | { kind: "installing"; done: number; total: number | null }
  | { kind: "error"; message: string };

/** Percentage of a download, or null while the size is unknown. */
export function percent(done: number, total: number | null): number | null {
  if (!total || total <= 0) return null;
  return Math.min(100, Math.round((done / total) * 100));
}

export function UpdateSection(): JSX.Element {
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen<{ done: number; total: number | null }>("izul://update-progress", (e) => {
      setPhase({ kind: "installing", done: e.payload.done, total: e.payload.total });
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const check = (): void => {
    setPhase({ kind: "checking" });
    invoke<UpdateCheck>("update_check")
      .then((c) => setPhase({ kind: "result", check: c }))
      .catch((e: unknown) => setPhase({ kind: "error", message: String(e) }));
  };

  const install = async (): Promise<void> => {
    if (!(await requestCloseWindow())) return;
    setPhase({ kind: "installing", done: 0, total: null });
    try {
      await invoke("update_install");
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  };

  const busy = phase.kind === "checking" || phase.kind === "installing";
  let status: JSX.Element | null = null;
  if (phase.kind === "checking") {
    status = <p>{t("update.checking")}</p>;
  } else if (phase.kind === "error") {
    status = (
      <p role="alert" className="text-[var(--izul-danger)] break-words">
        {phase.message}
      </p>
    );
  } else if (phase.kind === "installing") {
    const p = percent(phase.done, phase.total);
    status = (
      <div className="flex flex-col gap-1">
        <p>{p === null ? t("update.downloading") : fill(t("update.downloadingPct"), { pct: p })}</p>
        <progress className="w-full" max={100} value={p ?? undefined} aria-label={t("update.downloading")} />
      </div>
    );
  } else if (phase.kind === "result") {
    const c = phase.check;
    if (c.portable) {
      status = <p>{fill(t("update.portable"), { url: c.releasesUrl })}</p>;
    } else if (!c.available) {
      status = <p>{fill(t("update.latest"), { version: c.current })}</p>;
    } else {
      status = (
        <div className="flex flex-col gap-2">
          <p className="font-medium text-[var(--izul-text)]">
            {fill(t("update.available"), { version: c.available, current: c.current })}
          </p>
          {c.notes && <p className="whitespace-pre-wrap">{c.notes}</p>}
          <button
            type="button"
            onClick={() => void install()}
            className="self-start h-8 px-3 rounded-[8px] bg-[var(--izul-accent)] text-[var(--izul-on-accent)] text-[13px] font-medium"
          >
            {t("update.install")}
          </button>
        </div>
      );
    }
  }

  return (
    <section aria-labelledby="update-title" className="flex flex-col gap-2 text-[12px] text-[var(--izul-text-dim)]">
      <div className="flex items-center gap-3">
        <h3 id="update-title" className="text-[13px] font-semibold text-[var(--izul-text)]">
          {t("update.title")}
        </h3>
        <button
          type="button"
          disabled={busy}
          onClick={check}
          className="h-8 px-3 rounded-[8px] border border-[var(--izul-border)] text-[13px] text-[var(--izul-text)] hover:bg-[var(--izul-surface-raised)] disabled:opacity-50"
        >
          {t("update.check")}
        </button>
      </div>
      <p>{t("update.privacy")}</p>
      <div aria-live="polite">{status}</div>
    </section>
  );
}
