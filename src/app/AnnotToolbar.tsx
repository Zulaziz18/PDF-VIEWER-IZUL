/**
 * The annotation tools (SPEC 11.2, SPEC 12).
 *
 * One row under the reading toolbar. A tool stays selected until it is used or
 * dismissed, because drawing three arrows in a row is the common case and
 * re-picking the tool each time is the kind of friction that makes people stop
 * annotating.
 *
 * The markup tools (highlight, underline, strike-out) are absent here on
 * purpose: they follow a *text selection*, not a dragged box, and a button that
 * silently does nothing when no text is selected would be worse than no button.
 * They are offered from the selection itself.
 */

import { useDocument } from "@/state/documentStore";
import type { AnnotKind } from "@/annots/types";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { DEFAULT_STYLE, markupFromQuads } from "@/annots/factory";
import { NEW_OBJECT_ID } from "@/annots/types";
import { viewport } from "./viewportHandle";
import { t } from "@/i18n";

/** The kinds a pointer drag can produce, in the order they are drawn. */
const TOOLS: ReadonlyArray<{ kind: AnnotKind; glyph: string; label: () => string }> = [
  { kind: "Ink", glyph: "✎", label: () => t("tool.ink") },
  { kind: "Line", glyph: "╱", label: () => t("tool.line") },
  { kind: "Arrow", glyph: "↗", label: () => t("tool.arrow") },
  { kind: "Rect", glyph: "▭", label: () => t("tool.rect") },
  { kind: "Ellipse", glyph: "◯", label: () => t("tool.ellipse") },
  { kind: "Polygon", glyph: "⬟", label: () => t("tool.polygon") },
  { kind: "FreeText", glyph: "T", label: () => t("tool.text") },
  { kind: "Note", glyph: "🗨", label: () => t("tool.note") },
  { kind: "Stamp", glyph: "✓", label: () => t("tool.stamp") },
];

/**
 * Inserts an image on the current page.
 *
 * A file, not a drag: an image annotation needs pixels before it has a size,
 * and dragging out a box first would mean either distorting the picture or
 * ignoring the box the user just drew. It lands in the middle of the page at
 * its natural aspect ratio and is moved and resized like anything else.
 */
async function insertImage(): Promise<void> {
  const store = useDocument.getState;
  const state = store();
  if (state.doc === null) return;
  const chosen = await openDialog({
    multiple: false,
    filters: [{ name: "Gambar", extensions: ["png", "jpg", "jpeg", "gif", "webp"] }],
  });
  if (typeof chosen !== "string") return;
  try {
    const image = await invoke<number>("annot_add_image", { doc: state.doc, path: chosen });
    const page = state.page;
    const size = state.pageSizes[page];
    const width = Math.min(240, (size?.width ?? 400) * 0.5);
    const height = width * 0.75;
    const left = ((size?.width ?? 400) - width) / 2;
    const bottom = ((size?.height ?? 600) - height) / 2;
    await store().addAnnot({
      id: NEW_OBJECT_ID,
      page,
      kind: "Image",
      rect: { left, bottom, right: left + width, top: bottom + height },
      rotation: 0,
      opacity: 1,
      z: 0,
      locked: false,
      created_at: Date.now(),
      modified_at: Date.now(),
      author_note: "",
      payload: { Image: { image, crop: { left: 0, bottom: 0, right: 1, top: 1 }, opacity: 1 } },
    });
  } catch (e) {
    // Surfaced in the toolbar's own error line, where the rest of the
    // annotation refusals already appear.
    console.warn("gambar tidak dapat disisipkan", e);
  }
}

export function AnnotToolbar(): React.JSX.Element {
  const tool = useDocument((s) => s.tool);
  const canUndo = useDocument((s) => s.canUndo);
  const canRedo = useDocument((s) => s.canRedo);
  const selection = useDocument((s) => s.selection);
  const error = useDocument((s) => s.annotError);
  const store = useDocument.getState;

  return (
    <div className="flex flex-col shrink-0 border-b border-[var(--izul-border)] bg-[var(--izul-surface)]">
      <div className="flex items-center gap-1 px-2 h-10">
        <button
          type="button"
          aria-label={t("tool.select")}
          aria-pressed={tool === null}
          title={t("tool.select")}
          onClick={() => store().setTool(null)}
          className={[
            "w-8 h-8 rounded-[8px] text-[15px]",
            tool === null ? "bg-[var(--izul-accent)] text-white" : "hover:bg-[var(--izul-surface-raised)]",
          ].join(" ")}
        >
          ⌖
        </button>
        <span className="w-px h-5 bg-[var(--izul-border)] mx-1" aria-hidden="true" />
        {TOOLS.map((item) => (
          <button
            key={item.kind}
            type="button"
            aria-label={item.label()}
            aria-pressed={tool === item.kind}
            title={item.label()}
            onClick={() => store().setTool(tool === item.kind ? null : item.kind)}
            className={[
              "w-8 h-8 rounded-[8px] text-[15px]",
              tool === item.kind
                ? "bg-[var(--izul-accent)] text-white"
                : "hover:bg-[var(--izul-surface-raised)]",
            ].join(" ")}
          >
            {item.glyph}
          </button>
        ))}

        <button
          type="button"
          aria-label={t("tool.image")}
          title={t("tool.image")}
          onClick={() => void insertImage()}
          className="w-8 h-8 rounded-[8px] text-[15px] hover:bg-[var(--izul-surface-raised)]"
        >
          🖼
        </button>

        <span className="w-px h-5 bg-[var(--izul-border)] mx-1" aria-hidden="true" />
        {/* Markup follows the text selection, so these act on it rather than
            arming a tool. With nothing selected they say so instead of drawing
            an empty highlight somewhere. */}
        {(
          [
            ["Highlight", "▮", "annot.highlight"],
            ["Underline", "U̲", "annot.underline"],
            ["StrikeOut", "S̶", "annot.strikeout"],
          ] as const
        ).map(([kind, glyph, key]) => (
          <button
            key={kind}
            type="button"
            aria-label={t(key)}
            title={t(key)}
            onClick={() => {
              const found = viewport()?.selectionQuads();
              if (!found) {
                store().setTool(null);
                return;
              }
              const object = markupFromQuads(
                kind,
                found.page,
                found.quads,
                DEFAULT_STYLE,
                Date.now(),
              );
              if (object) void store().addAnnot(object);
              document.getSelection()?.removeAllRanges();
            }}
            className="w-8 h-8 rounded-[8px] text-[15px] hover:bg-[var(--izul-surface-raised)]"
          >
            {glyph}
          </button>
        ))}

        <span className="w-px h-5 bg-[var(--izul-border)] mx-1" aria-hidden="true" />
        <button
          type="button"
          aria-label={t("annot.undo")}
          title={t("annot.undo")}
          disabled={!canUndo}
          onClick={() => void store().undoAnnot()}
          className="w-8 h-8 rounded-[8px] hover:bg-[var(--izul-surface-raised)] disabled:opacity-35"
        >
          ↶
        </button>
        <button
          type="button"
          aria-label={t("annot.redo")}
          title={t("annot.redo")}
          disabled={!canRedo}
          onClick={() => void store().redoAnnot()}
          className="w-8 h-8 rounded-[8px] hover:bg-[var(--izul-surface-raised)] disabled:opacity-35"
        >
          ↷
        </button>
        <button
          type="button"
          aria-label={t("annot.delete")}
          title={t("annot.delete")}
          disabled={selection.length === 0}
          onClick={() => void store().deleteSelected()}
          className="w-8 h-8 rounded-[8px] hover:bg-[var(--izul-surface-raised)] disabled:opacity-35"
        >
          🗑
        </button>

        {selection.length > 1 && (
          <span className="ml-2 text-[12px] text-[var(--izul-text-dim)]">
            {selection.length} {t("annot.selected")}
          </span>
        )}
      </div>
      {error !== null && (
        <p role="alert" className="px-3 pb-2 text-[12px] text-[var(--izul-danger)]">
          {error}
        </p>
      )}
    </div>
  );
}
