/**
 * Status bar: page, zoom, render state, and the health of the worker pool
 * (SPEC 12).
 *
 * The worker count is shown because a crashed and restarted worker is a visible
 * event, and hiding it would make the crash-isolation behaviour impossible to
 * observe. The cache line is shown because a render pipeline whose behaviour
 * cannot be seen is one whose regressions are found by users.
 */

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";

interface CacheStats {
  hits: number;
  misses: number;
  entries: number;
  bytes: number;
  budget_bytes: number;
}

interface RenderStats {
  cache: CacheStats;
  queued: number;
  inflight: number;
  rendered: number;
}

interface Health {
  pool_size: number;
  live_workers: number;
  sandbox_is_security_boundary: boolean;
  render: RenderStats;
}

function megabytes(bytes: number): string {
  return `${Math.round(bytes / (1024 * 1024))} MB`;
}

export function StatusBar(): React.JSX.Element {
  const page = useDocument((s) => s.page);
  const pageCount = useDocument((s) => s.pageCount);
  const zoom = useDocument((s) => s.zoom);
  const busy = useDocument((s) => s.busy);
  const encrypted = useDocument((s) => s.encrypted);
  const [health, setHealth] = useState<Health | null>(null);

  useEffect(() => {
    const tick = (): void => {
      void invoke<Health>("pool_health")
        .then(setHealth)
        .catch(() => setHealth(null));
    };
    tick();
    const timer = window.setInterval(tick, 2000);
    return () => window.clearInterval(timer);
  }, []);

  const cache = health?.render.cache;
  const lookups = cache ? cache.hits + cache.misses : 0;
  const hitRate = cache && lookups > 0 ? Math.round((cache.hits / lookups) * 100) : null;
  const working = busy || (health?.render.queued ?? 0) + (health?.render.inflight ?? 0) > 0;

  return (
    <footer className="flex items-center gap-4 px-3 h-7 shrink-0 border-t border-[var(--izul-border)] bg-[var(--izul-surface)] text-[12px] text-[var(--izul-text-dim)]">
      {pageCount > 0 && (
        <span>
          {t("status.page")} {page + 1} {t("status.of")} {pageCount}
        </span>
      )}
      <span>
        {t("status.zoom")} {Math.round(zoom * 100)}%
      </span>
      {encrypted && <span>{t("status.encrypted")}</span>}
      <span className="ml-auto" aria-live="polite">
        {working ? t("status.rendering") : t("status.ready")}
      </span>
      {cache && (
        <span title={`${cache.entries} ubin · ${megabytes(cache.budget_bytes)} anggaran`}>
          {t("status.cache")} {megabytes(cache.bytes)}
          {hitRate !== null ? ` · ${hitRate}%` : ""}
        </span>
      )}
      {health && (
        <span
          title={
            health.sandbox_is_security_boundary ? t("about.sandboxReal") : t("about.sandboxDev")
          }
        >
          {t("status.workers")} {health.live_workers}/{health.pool_size}
        </span>
      )}
    </footer>
  );
}
