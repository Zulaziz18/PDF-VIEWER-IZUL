/**
 * The page panel: thumbnails, and since Phase 5 the place pages are
 * rearranged — selected with click, Ctrl+click and Shift+click, dragged to a
 * new place, deleted with the Delete key (SPEC 11.3).
 *
 * Thumbnails are the viewport's preview tier at the same URI, so a page whose
 * thumbnail has been drawn is also a page the viewport can show instantly.
 * The strip is windowed: only the slots in view are mounted, so it costs the
 * same at 500 pages as at 5.
 *
 * Dragging uses the HTML5 drag API with a payload of our own type. A drop in
 * this panel moves pages within the document; a drop coming from another
 * document (in split view) copies them in, or moves them with Shift — the
 * rule file managers have taught everyone.
 */

import { useEffect, useRef, useState, type JSX } from "react";
import { t } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import {
  PAGE_DRAG_TYPE,
  clickSelect,
  decodePageDrag,
  dropBefore,
  encodePageDrag,
  moveChangesNothing,
} from "@/state/pageSelection";
import { paintThumbnail } from "@/viewport/thumbnails";
import { viewport } from "./viewportHandle";

/** Height of one slot in CSS pixels; fixed, which is what allows windowing. */
const THUMB_SLOT = 200;

function Thumbnail(props: {
  source: { doc: number | null; page: number };
  label: number;
  rotation: number;
  generation: number;
  current: boolean;
  selected: boolean;
  blankAspect: number;
}): JSX.Element {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [drawn, setDrawn] = useState(false);
  const { doc, page } = props.source;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    if (doc === null) {
      // A blank page: white, in its proportions, with nothing to fetch.
      canvas.width = 128;
      canvas.height = Math.round(128 * props.blankAspect);
      const ctx = canvas.getContext("2d");
      if (ctx) {
        ctx.fillStyle = "#ffffff";
        ctx.fillRect(0, 0, canvas.width, canvas.height);
      }
      setDrawn(true);
      return;
    }
    const abort = new AbortController();
    void paintThumbnail(
      canvas,
      { doc, page, rotation: props.rotation, generation: props.generation },
      abort.signal,
    ).then((ok) => {
      if (!abort.signal.aborted) setDrawn(ok);
    });
    return () => abort.abort();
  }, [doc, page, props.rotation, props.generation, props.blankAspect]);

  return (
    <div
      className={[
        "w-full h-full flex flex-col items-center justify-center gap-1 p-2 rounded-[8px]",
        props.selected ? "bg-[var(--izul-accent-soft)]" : "hover:bg-[var(--izul-surface-raised)]",
      ].join(" ")}
    >
      <canvas
        ref={canvasRef}
        className={[
          "max-w-full max-h-[160px] w-auto h-auto rounded-[2px] bg-white",
          props.current || props.selected
            ? "outline outline-2 outline-[var(--izul-accent)]"
            : "outline outline-1 outline-[var(--izul-border)]",
          drawn ? "" : "opacity-0",
        ].join(" ")}
      />
      <span className="text-[12px] text-[var(--izul-text-dim)] tabular-nums">{props.label}</span>
    </div>
  );
}

export function PagePanel(): JSX.Element {
  const doc = useDocument((s) => s.doc);
  const pageCount = useDocument((s) => s.pageCount);
  const page = useDocument((s) => s.page);
  const docRotation = useDocument((s) => s.docRotation);
  const pageRotation = useDocument((s) => s.pageRotation);
  const generation = useDocument((s) => s.generation);
  // Part of each slot's key: after a move, slot 3 holds another page and
  // must not keep the old one's canvas.
  const pagesEpoch = useDocument((s) => s.pagesEpoch);
  const pageSizes = useDocument((s) => s.pageSizes);
  const selection = useDocument((s) => s.pageSelection);
  const scrollerRef = useRef<HTMLDivElement>(null);
  const anchor = useRef<number | null>(null);
  const [window_, setWindow] = useState({ first: 0, last: 12 });
  const [dropAt, setDropAt] = useState<number | null>(null);

  useEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const recompute = (): void => {
      const first = Math.max(0, Math.floor(el.scrollTop / THUMB_SLOT) - 2);
      const last = Math.min(pageCount, Math.ceil((el.scrollTop + el.clientHeight) / THUMB_SLOT) + 2);
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
  }, [pageCount]);

  // Keep the current page in view when it changes from the viewport's side.
  useEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const top = page * THUMB_SLOT;
    if (top < el.scrollTop || top + THUMB_SLOT > el.scrollTop + el.clientHeight) {
      el.scrollTo({ top: Math.max(0, top - el.clientHeight / 2), behavior: "auto" });
    }
  }, [page]);

  if (doc === null) return <div />;
  const store = useDocument.getState;

  const offsetOf = (e: React.DragEvent): number => {
    const el = scrollerRef.current;
    if (!el) return 0;
    return e.clientY - el.getBoundingClientRect().top + el.scrollTop;
  };

  const onDrop = (e: React.DragEvent): void => {
    const drag = decodePageDrag(e.dataTransfer.getData(PAGE_DRAG_TYPE));
    setDropAt(null);
    if (!drag) return;
    e.preventDefault();
    const before = dropBefore(offsetOf(e), THUMB_SLOT, pageCount);
    if (drag.doc === doc) {
      if (moveChangesNothing(drag.pages, before)) return;
      void store().pageCommand({ kind: "move", pages: drag.pages, before });
      store().setPageSelection([]);
    } else {
      void store().copyPagesFrom(drag.doc, drag.pages, before, e.shiftKey);
    }
  };

  const items: JSX.Element[] = [];
  for (let i = window_.first; i < window_.last; i++) {
    const selected = selection.includes(i);
    const size = pageSizes[i];
    items.push(
      <li
        key={`${pagesEpoch}:${i}`}
        role="option"
        aria-selected={selected}
        aria-current={i === page ? "page" : undefined}
        aria-label={`${t("nav.page")} ${i + 1}`}
        tabIndex={i === page ? 0 : -1}
        draggable
        onDragStart={(e) => {
          const pages = selected ? selection : [i];
          if (!selected) store().setPageSelection([i]);
          e.dataTransfer.setData(PAGE_DRAG_TYPE, encodePageDrag({ doc, pages }));
          e.dataTransfer.effectAllowed = "copyMove";
        }}
        onClick={(e) => {
          const next = clickSelect(
            { pages: selection, anchor: anchor.current },
            i,
            { toggle: e.ctrlKey || e.metaKey, range: e.shiftKey },
          );
          anchor.current = next.anchor;
          store().setPageSelection(next.pages);
          store().setPage(i);
          viewport()?.goToPage(i);
        }}
        style={{ position: "absolute", top: i * THUMB_SLOT, height: THUMB_SLOT, left: 0, right: 0 }}
        className="px-1 cursor-default"
      >
        <Thumbnail
          source={store().sourceOf(i)}
          label={i + 1}
          rotation={(((docRotation + (pageRotation[i] ?? 0)) % 4) + 4) % 4}
          generation={generation}
          current={i === page}
          selected={selected}
          blankAspect={size ? size.height / Math.max(1, size.width) : 1.414}
        />
      </li>,
    );
  }

  return (
    <div
      ref={scrollerRef}
      className="h-full overflow-auto outline-none"
      tabIndex={-1}
      onKeyDown={(e) => {
        if ((e.key === "Delete" || e.key === "Backspace") && selection.length > 0) {
          e.preventDefault();
          void store().pageCommand({ kind: "delete", pages: selection });
          store().setPageSelection([]);
        } else if (e.key === "a" && (e.ctrlKey || e.metaKey)) {
          e.preventDefault();
          store().setPageSelection(Array.from({ length: pageCount }, (_, k) => k));
        }
      }}
      onDragOver={(e) => {
        if (!e.dataTransfer.types.includes(PAGE_DRAG_TYPE)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = e.shiftKey ? "move" : "copy";
        setDropAt(dropBefore(offsetOf(e), THUMB_SLOT, pageCount));
      }}
      onDragLeave={() => setDropAt(null)}
      onDrop={onDrop}
    >
      <ul
        role="listbox"
        aria-multiselectable="true"
        aria-label={t("sidebar.thumbnails")}
        className="relative"
        style={{ height: pageCount * THUMB_SLOT }}
      >
        {items}
        {dropAt !== null && (
          <li
            aria-hidden="true"
            className="absolute left-3 right-3 h-[3px] rounded-full bg-[var(--izul-accent)] pointer-events-none"
            style={{ top: dropAt * THUMB_SLOT - 1 }}
          />
        )}
      </ul>
    </div>
  );
}
