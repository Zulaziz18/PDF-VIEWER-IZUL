/**
 * Status bar: page, zoom, and the health of the worker pool (SPEC 12).
 *
 * The worker count is shown because a crashed and restarted worker is a visible
 * event in Phase 0, and hiding it would make the crash-isolation behaviour
 * impossible to observe.
 */

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";

interface Health {
  pool_size: number;
  live_workers: number;
  sandbox_is_security_boundary: boolean;
}

export function StatusBar(): React.JSX.Element {
  const page = useDocument((s) => s.page);
  const pageCount = useDocument((s) => s.pageCount);
  const zoom = useDocument((s) => s.zoom);
  const busy = useDocument((s) => s.busy);
  const [health, setHealth] = useState<Health | null>(null);

  useEffect(() => {
    const tick = (): void => {
      void invoke<Health>("pool_health").then(setHealth).catch(() => setHealth(null));
    };
    tick();
    const timer = window.setInterval(tick, 2000);
    return () => window.clearInterval(timer);
  }, []);

  return (
    <footer className="flex items-center gap-4 px-3 h-7 border-t border-[var(--izul-border)] bg-[var(--izul-surface)] text-[12px] text-[var(--izul-text-dim)]">
      {pageCount > 0 && (
        <span>
          {t("status.page")} {page + 1} {t("status.of")} {pageCount}
        </span>
      )}
      <span>
        {t("status.zoom")} {Math.round(zoom * 100)}%
      </span>
      <span className="ml-auto">{busy ? t("status.rendering") : t("status.ready")}</span>
      {health && (
        <span title={health.sandbox_is_security_boundary ? t("about.sandboxReal") : t("about.sandboxDev")}>
          {t("status.workers")} {health.live_workers}/{health.pool_size}
        </span>
      )}
    </footer>
  );
}
