/**
 * The bottom bar (SPEC 12, revised 2026-09-23), after WPS Office's: page
 * navigation at the left, view modes and the zoom slider at the right, and
 * between them the status SPEC 12 asks for — render state, the worker pool and
 * the cache.
 *
 * The worker count is shown because a crashed and restarted worker is a
 * visible event, and hiding it would make the crash-isolation behaviour
 * impossible to observe.
 */

import { useEffect, useState, type JSX } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconButton, MenuButton } from "@/design/controls";
import { Icon, type IconName } from "@/design/Icon";
import { t, type StringKey } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { sliderToZoom, zoomToSlider } from "@/viewport/geometry";
import type { ViewMode } from "@/viewport/layout";
import { goToPage } from "./actions";

interface CacheStats {
  hits: number;
  misses: number;
  entries: number;
  bytes: number;
  budget_bytes: number;
}

interface Health {
  pool_size: number;
  live_workers: number;
  sandbox_is_security_boundary: boolean;
  render: { cache: CacheStats; queued: number; inflight: number; rendered: number };
}

function megabytes(bytes: number): string {
  return `${Math.round(bytes / (1024 * 1024))} MB`;
}

const VIEW_MODES: ReadonlyArray<{ mode: ViewMode; icon: IconName; label: StringKey }> = [
  { mode: "single", icon: "viewSingle", label: "view.single" },
  { mode: "dual", icon: "viewDual", label: "view.dual" },
  { mode: "dual_cover", icon: "viewCover", label: "view.dualCover" },
  { mode: "horizontal", icon: "viewHorizontal", label: "view.horizontal" },
];

const ZOOM_PRESETS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4];

function PageBox(props: { page: number; pageCount: number }): JSX.Element {
  const [draft, setDraft] = useState<string | null>(null);
  const shown = draft ?? `${props.page + 1}/${props.pageCount}`;
  return (
    <input
      type="text"
      inputMode="numeric"
      aria-label={t("nav.page")}
      value={shown}
      onFocus={(e) => {
        setDraft(String(props.page + 1));
        requestAnimationFrame(() => e.target.select());
      }}
      onChange={(e) => setDraft(e.target.value.replace(/[^0-9]/g, ""))}
      onBlur={() => setDraft(null)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          const n = Number(draft);
          if (Number.isFinite(n) && n > 0) goToPage(n - 1);
          (e.target as HTMLInputElement).blur();
        } else if (e.key === "Escape") {
          (e.target as HTMLInputElement).blur();
        }
      }}
      className="w-[76px] h-6 px-2 text-center text-[12px] tabular-nums rounded-[6px] bg-[var(--izul-surface)] border border-[var(--izul-border)]"
    />
  );
}

export function BottomBar(): JSX.Element {
  const page = useDocument((s) => s.page);
  const pageCount = useDocument((s) => s.pageCount);
  const zoom = useDocument((s) => s.zoom);
  const zoomMode = useDocument((s) => s.zoomMode);
  const viewMode = useDocument((s) => s.viewMode);
  const sidebarOpen = useDocument((s) => s.sidebarOpen);
  const busy = useDocument((s) => s.busy);
  const encrypted = useDocument((s) => s.encrypted);
  const store = useDocument.getState;
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
  const last = Math.max(0, pageCount - 1);

  return (
    <footer className="h-8 shrink-0 flex items-center gap-1 px-2 bg-[var(--izul-chrome)] border-t border-[var(--izul-border)] text-[12px] text-[var(--izul-text-dim)]">
      <IconButton icon="sidebar" label={t("toolbar.sidebar")} pressed={sidebarOpen} onClick={() => store().toggleSidebar()} />
      <span aria-hidden="true" className="w-px h-4 mx-1 bg-[var(--izul-border)]" />
      <IconButton icon="first" size={16} label={t("nav.first")} disabled={page <= 0} onClick={() => goToPage(0)} />
      <IconButton icon="previous" size={16} label={t("nav.previous")} disabled={page <= 0} onClick={() => goToPage(page - 1)} />
      <PageBox page={page} pageCount={pageCount} />
      <IconButton icon="next" size={16} label={t("nav.next")} disabled={page >= last} onClick={() => goToPage(page + 1)} />
      <IconButton icon="last" size={16} label={t("nav.last")} disabled={page >= last} onClick={() => goToPage(last)} />
      <span aria-hidden="true" className="w-px h-4 mx-1 bg-[var(--izul-border)]" />

      <span className="flex items-center gap-1.5 px-1" aria-live="polite">
        <span
          aria-hidden="true"
          className={["w-2 h-2 rounded-full", working ? "bg-[var(--izul-unsaved)]" : "bg-[var(--izul-tone-green)]"].join(" ")}
        />
        {working ? t("status.rendering") : t("status.ready")}
      </span>
      {encrypted && <span className="px-1">{t("status.encrypted")}</span>}
      {health && (
        <span
          className="px-1 whitespace-nowrap"
          title={health.sandbox_is_security_boundary ? t("about.sandboxReal") : t("about.sandboxDev")}
        >
          {t("status.workers")} {health.live_workers}/{health.pool_size}
        </span>
      )}
      {cache && (
        <span className="px-1 whitespace-nowrap max-[1180px]:hidden" title={`${cache.entries} ubin · ${megabytes(cache.budget_bytes)} anggaran`}>
          {t("status.cache")} {megabytes(cache.bytes)}
          {hitRate !== null ? ` · ${hitRate}%` : ""}
        </span>
      )}

      <div className="flex-1" />

      {VIEW_MODES.map((v) => (
        <IconButton
          key={v.mode}
          icon={v.icon}
          size={16}
          label={t(v.label)}
          pressed={viewMode === v.mode}
          onClick={() => store().setViewMode(v.mode)}
        />
      ))}
      <span aria-hidden="true" className="w-px h-4 mx-1 bg-[var(--izul-border)]" />
      <IconButton icon="fitWidth" size={16} label={t("zoom.fitWidthLong")} pressed={zoomMode === "fitWidth"} onClick={() => store().setZoomMode("fitWidth")} />
      <IconButton icon="fitPage" size={16} label={t("zoom.fitPageLong")} pressed={zoomMode === "fitPage"} onClick={() => store().setZoomMode("fitPage")} />
      <span aria-hidden="true" className="w-px h-4 mx-1 bg-[var(--izul-border)]" />

      <MenuButton
        label={t("zoom.level")}
        align="right"
        className="h-6 w-[68px] px-1.5 flex items-center justify-between rounded-[6px] text-[12px] text-[var(--izul-text)] tabular-nums hover:bg-[var(--izul-chrome-hover)]"
        items={ZOOM_PRESETS.map((z) => ({
          label: `${Math.round(z * 100)}%`,
          checked: Math.abs(zoom - z) < 0.005 && zoomMode === "custom",
          onSelect: () => store().setZoom(z),
        }))}
      >
        {Math.round(zoom * 100)}%
        <Icon name="chevronDown" size={16} />
      </MenuButton>
      <IconButton icon="zoomOut" size={16} label={t("zoom.out")} onClick={() => store().zoomOut()} />
      <input
        type="range"
        min={0}
        max={1000}
        step={1}
        aria-label={t("zoom.level")}
        aria-valuetext={`${Math.round(zoom * 100)}%`}
        value={Math.round(zoomToSlider(zoom) * 1000)}
        onChange={(e) => store().setZoom(sliderToZoom(Number(e.target.value) / 1000))}
        className="w-[120px] accent-[var(--izul-accent)]"
      />
      <IconButton icon="zoomIn" size={16} label={t("zoom.in")} onClick={() => store().zoomIn()} />
    </footer>
  );
}
