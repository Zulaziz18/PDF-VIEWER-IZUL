/**
 * The notepad beside a selected sticky note.
 *
 * On the page a note is only a small badge (izul-model `note_icon`); what it
 * says is written and read here, on a sheet of ruled yellow paper next to it —
 * the way a PDF reader opens a note's pop-up. Asked for after 7.0.0, where the
 * note's text lived only in the properties panel and the badge was stretched
 * over the page to make room for it.
 *
 * The text is the note's `/Contents`, so readers like Acrobat and Edge show the
 * same words in their own pop-up.
 */

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useStore } from "zustand";
import type { AnnotObject } from "@/annots/types";
import type { DocumentStore } from "@/state/documentSession";
import type { ViewportRenderer } from "@/viewport/renderer";
import { t } from "@/i18n";

const WIDTH = 240;
const HEIGHT = 190;
const GAP = 8;
/** How long typing must pause before the text becomes an undo step. */
const COMMIT_MS = 500;

type Box = { left: number; top: number; right: number; bottom: number };

/**
 * Where the notepad goes: to the right of the badge, or to its left when the
 * viewport has no room on the right, kept inside the viewport either way.
 */
export function placeNotepad(
  badge: Box,
  view: { width: number; height: number },
  size = { width: WIDTH, height: HEIGHT },
): { left: number; top: number } {
  const right = badge.right + GAP;
  const wanted = right + size.width <= view.width - GAP ? right : badge.left - GAP - size.width;
  const left = Math.min(Math.max(GAP, wanted), Math.max(GAP, view.width - size.width - GAP));
  const top = Math.min(Math.max(GAP, badge.top), Math.max(GAP, view.height - size.height - GAP));
  return { left, top };
}

function noteText(obj: AnnotObject): string {
  return "Note" in obj.payload ? obj.payload.Note.text : "";
}

function withNoteText(obj: AnnotObject, text: string): AnnotObject {
  const next = structuredClone(obj);
  if ("Note" in next.payload) next.payload.Note.text = text;
  return next;
}

export function NotePopup(props: {
  store: DocumentStore;
  renderer: () => ViewportRenderer | null;
  scroller: () => HTMLElement | null;
}): React.JSX.Element | null {
  const selection = useStore(props.store, (s) => s.selection);
  const annots = useStore(props.store, (s) => s.annots);
  const tool = useStore(props.store, (s) => s.tool);
  const zoom = useStore(props.store, (s) => s.zoom);
  const note =
    tool === null && selection.length === 1
      ? (props.store.getState().selectedObjects()[0] ?? null)
      : null;
  const target = note?.kind === "Note" ? note : null;
  void annots;

  const [place, setPlace] = useState<{ left: number; top: number } | null>(null);
  const [draft, setDraft] = useState("");
  const pending = useRef<{ obj: AnnotObject; text: string } | null>(null);
  const timer = useRef<number | null>(null);
  const area = useRef<HTMLTextAreaElement>(null);

  const flush = (): void => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    const p = pending.current;
    pending.current = null;
    if (p && noteText(p.obj) !== p.text) {
      void props.store.getState().replaceAnnots([withNoteText(p.obj, p.text)]);
    }
  };

  // A different note: commit what was typed into the last one, then load.
  const id = target?.id ?? null;
  useEffect(() => {
    flush();
    if (target) setDraft(noteText(target));
    return flush;
    // The text is loaded only when the note changes, never while typing.
  }, [id]);

  // Follow the badge as the page scrolls and zooms.
  useLayoutEffect(() => {
    if (!target) {
      setPlace(null);
      return;
    }
    const scroller = props.scroller();
    const update = (): void => {
      const rect = props.renderer()?.screenRectOf(target);
      // Scrolled out of sight, the badge takes its notepad with it rather
      // than leaving it pinned to an edge, pointing at nothing.
      const offscreen =
        !rect ||
        !scroller ||
        rect.right < 0 ||
        rect.bottom < 0 ||
        rect.left > scroller.clientWidth ||
        rect.top > scroller.clientHeight;
      if (offscreen) {
        setPlace(null);
        return;
      }
      setPlace(placeNotepad(rect, { width: scroller.clientWidth, height: scroller.clientHeight }));
    };
    update();
    // The renderer lays out a new zoom in its own effect, which runs after
    // this one: measure again once it has.
    const frame = requestAnimationFrame(update);
    scroller?.addEventListener("scroll", update, { passive: true });
    return () => {
      cancelAnimationFrame(frame);
      scroller?.removeEventListener("scroll", update);
    };
  }, [target, zoom]);

  // A note just placed opens ready to type into.
  useEffect(() => {
    if (target && noteText(target) === "") area.current?.focus({ preventScroll: true });
  }, [id]);

  if (!target || !place) return null;
  return (
    <section
      aria-label={t("note.popup")}
      className="izul-notepad absolute z-20 flex flex-col rounded-[6px] shadow-[0_6px_20px_rgba(0,0,0,0.22)]"
      style={{ left: place.left, top: place.top, width: WIDTH, height: HEIGHT }}
      // The page underneath must not start a gesture or a text selection.
      onPointerDown={(e) => e.stopPropagation()}
    >
      <header className="izul-notepad-head flex items-center justify-between h-7 px-2.5 rounded-t-[6px]">
        <span className="text-[12px] font-medium">{t("note.popup")}</span>
        <button
          type="button"
          aria-label={t("note.close")}
          title={t("note.close")}
          className="w-6 h-6 grid place-items-center rounded-[4px] text-[16px] leading-none hover:bg-black/10"
          onClick={() => {
            flush();
            props.store.getState().select([]);
          }}
        >
          ×
        </button>
      </header>
      <textarea
        ref={area}
        aria-label={t("note.text")}
        placeholder={t("note.placeholder")}
        value={draft}
        spellCheck={false}
        onChange={(e) => {
          const text = e.target.value;
          setDraft(text);
          pending.current = { obj: target, text };
          if (timer.current !== null) window.clearTimeout(timer.current);
          timer.current = window.setTimeout(flush, COMMIT_MS);
        }}
        onBlur={flush}
        onKeyDown={(e) => {
          // Delete and Backspace here edit the text; they must not reach the
          // viewport's "delete the selected object".
          e.stopPropagation();
          if (e.key === "Escape") {
            flush();
            props.store.getState().select([]);
          }
        }}
        className="izul-notepad-paper flex-1 resize-none px-2.5 pt-[3px] pb-1 text-[13px] leading-[22px] outline-none rounded-b-[6px]"
      />
    </section>
  );
}
