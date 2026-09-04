/**
 * The sidebar: page thumbnails and the document's own outline (SPEC 11.1).
 *
 * Both panels are navigation, so both do the same thing when clicked — move the
 * viewport — and neither holds any state the viewport does not already have.
 *
 * Thumbnails are the viewport's preview tier at the same URI, so a page whose
 * thumbnail has been drawn is also a page the viewport can show instantly, and
 * neither pays for the other.
 */

import { useEffect, useRef, useState, type JSX } from "react";
import { useDocument, type OutlineEntry } from "@/state/documentStore";
import { paintThumbnail } from "@/viewport/thumbnails";
import { viewport } from "./viewportHandle";
import { t } from "@/i18n";

function Thumbnail(props: {
  doc: number;
  page: number;
  rotation: number;
  generation: number;
  active: boolean;
  onSelect: () => void;
}): JSX.Element {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [drawn, setDrawn] = useState(false);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const abort = new AbortController();
    void paintThumbnail(
      canvas,
      {
        doc: props.doc,
        page: props.page,
        rotation: props.rotation,
        generation: props.generation,
      },
      abort.signal,
    ).then((ok) => {
      if (!abort.signal.aborted) setDrawn(ok);
    });
    return () => abort.abort();
  }, [props.doc, props.page, props.rotation, props.generation]);

  return (
    <button
        type="button"
        onClick={props.onSelect}
        aria-current={props.active ? "page" : undefined}
        aria-label={`${t("nav.page")} ${props.page + 1}`}
        className={[
          "w-full flex flex-col items-center gap-1 p-2 rounded-[8px]",
          props.active ? "bg-[var(--izul-surface-raised)]" : "hover:bg-[var(--izul-surface-raised)]",
        ].join(" ")}
      >
        <canvas
          ref={canvasRef}
          className={[
            "max-w-full h-auto rounded-[2px] bg-white",
            props.active ? "outline outline-2 outline-[var(--izul-accent)]" : "",
            drawn ? "" : "opacity-0",
          ].join(" ")}
        />
        <span className="text-[12px] text-[var(--izul-text-dim)] tabular-nums">
          {props.page + 1}
        </span>
    </button>
  );
}

function OutlineList(props: { entries: readonly OutlineEntry[] }): JSX.Element {
  if (props.entries.length === 0) {
    return <p className="p-3 text-[var(--izul-text-dim)]">{t("sidebar.noOutline")}</p>;
  }
  return (
    <ul className="p-1">
      {props.entries.map((entry, index) => (
        <li key={`${index}-${entry.title}`}>
          <button
            type="button"
            disabled={entry.page === null}
            onClick={() => {
              if (entry.page === null) return;
              useDocument.getState().setPage(entry.page);
              viewport()?.goToPage(entry.page, entry.y ?? undefined);
            }}
            title={entry.title}
            style={{ paddingLeft: `${8 + Math.min(entry.depth, 6) * 12}px` }}
            className="w-full text-left truncate py-1.5 pr-2 rounded-[8px] hover:bg-[var(--izul-surface-raised)] disabled:opacity-50"
          >
            {entry.title || t("sidebar.untitled")}
          </button>
        </li>
      ))}
    </ul>
  );
}

/**
 * Height of one thumbnail slot, in CSS pixels.
 *
 * Fixed rather than measured: it is what lets the strip be virtualized, and a
 * portrait page at 256 px tall plus its label fits inside it. A landscape page
 * simply sits smaller in the same slot.
 */
const THUMB_SLOT = 200;

/**
 * The thumbnail strip, windowed.
 *
 * A 500-page document must not put 500 canvases in the DOM, each fetching a
 * preview: the render queue would be full of thumbnails nobody is looking at
 * while the page on screen waits. Only the slots in view are mounted, so the
 * strip costs the same at 500 pages as at 5.
 */
function ThumbnailStrip(props: {
  doc: number;
  pageCount: number;
  page: number;
  docRotation: number;
  pageRotation: Readonly<Record<number, number>>;
  generation: number;
}): JSX.Element {
  const scrollerRef = useRef<HTMLDivElement>(null);
  const [window_, setWindow] = useState({ first: 0, last: 12 });

  useEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const recompute = (): void => {
      const first = Math.max(0, Math.floor(el.scrollTop / THUMB_SLOT) - 2);
      const last = Math.min(
        props.pageCount,
        Math.ceil((el.scrollTop + el.clientHeight) / THUMB_SLOT) + 2,
      );
      setWindow((prev) => (prev.first === first && prev.last === last ? prev : { first, last }));
    };
    recompute();
    el.addEventListener("scroll", recompute, { passive: true });
    const observer = new ResizeObserver(recompute);
    observer.observe(el);
    return () => {
      el.removeEventListener("scroll", recompute);
      observer.disconnect();
    };
  }, [props.pageCount]);

  // Keep the current page in view when it changes from the viewport's side.
  useEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const top = props.page * THUMB_SLOT;
    if (top < el.scrollTop || top + THUMB_SLOT > el.scrollTop + el.clientHeight) {
      el.scrollTo({ top: Math.max(0, top - el.clientHeight / 2), behavior: "auto" });
    }
  }, [props.page]);

  const items: JSX.Element[] = [];
  for (let i = window_.first; i < window_.last; i++) {
    items.push(
      <li
        key={i}
        style={{ position: "absolute", top: i * THUMB_SLOT, height: THUMB_SLOT, left: 0, right: 0 }}
      >
        <Thumbnail
          doc={props.doc}
          page={i}
          rotation={(((props.docRotation + (props.pageRotation[i] ?? 0)) % 4) + 4) % 4}
          generation={props.generation}
          active={i === props.page}
          onSelect={() => {
            useDocument.getState().setPage(i);
            viewport()?.goToPage(i);
          }}
        />
      </li>,
    );
  }

  return (
    <div ref={scrollerRef} className="h-full overflow-auto">
      <ul className="relative" style={{ height: props.pageCount * THUMB_SLOT }}>
        {items}
      </ul>
    </div>
  );
}

export function Sidebar(): JSX.Element | null {
  const open = useDocument((s) => s.sidebarOpen);
  const tab = useDocument((s) => s.sidebarTab);
  const doc = useDocument((s) => s.doc);
  const pageCount = useDocument((s) => s.pageCount);
  const page = useDocument((s) => s.page);
  const outline = useDocument((s) => s.outline);
  const docRotation = useDocument((s) => s.docRotation);
  const pageRotation = useDocument((s) => s.pageRotation);
  const generation = useDocument((s) => s.generation);

  if (!open || doc === null) return null;

  return (
    <aside
      className="w-[220px] shrink-0 border-r border-[var(--izul-border)] bg-[var(--izul-surface)] flex flex-col"
      aria-label={t("sidebar.label")}
    >
      <div className="flex p-1 gap-1 border-b border-[var(--izul-border)]" role="tablist">
        {(["thumbnails", "outline"] as const).map((which) => (
          <button
            key={which}
            type="button"
            role="tab"
            aria-selected={tab === which}
            onClick={() => useDocument.getState().setSidebarTab(which)}
            className={[
              "flex-1 h-8 rounded-[8px] text-[13px]",
              tab === which
                ? "bg-[var(--izul-accent)] text-white"
                : "hover:bg-[var(--izul-surface-raised)]",
            ].join(" ")}
          >
            {t(which === "thumbnails" ? "sidebar.thumbnails" : "sidebar.outline")}
          </button>
        ))}
      </div>

      <div className="flex-1 overflow-auto">
        {tab === "thumbnails" ? (
          <ThumbnailStrip
            doc={doc}
            pageCount={pageCount}
            page={page}
            docRotation={docRotation}
            pageRotation={pageRotation}
            generation={generation}
          />
        ) : (
          <OutlineList entries={outline} />
        )}
      </div>
    </aside>
  );
}
