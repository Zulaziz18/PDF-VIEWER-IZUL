/**
 * The properties of whatever is selected (SPEC 11.2).
 *
 * Every control writes a whole object back through `replaceAnnots`, which is
 * one undo step and one round trip — the same path a finished drag takes. There
 * is no "apply" button and no local draft, because a second copy of an object's
 * state is a second thing that can disagree with the model.
 *
 * Only the controls that apply to the selected kinds are shown. A stroke-width
 * slider over a sticky note would be a control that does nothing, and a panel
 * full of those teaches people to ignore it.
 */

import { useDocument } from "@/state/documentStore";
import type { AnnotObject, Rgba } from "@/annots/types";
import { cssColor, rgba } from "@/annots/types";
import { t } from "@/i18n";

/** A small, deliberately boring palette: these are annotation colours, and the
 * point is that they read on white paper, not that they are pretty. */
const SWATCHES: readonly Rgba[] = [
  rgba(220, 38, 38),
  rgba(234, 88, 12),
  rgba(202, 138, 4),
  rgba(22, 163, 74),
  rgba(37, 99, 235),
  rgba(124, 58, 237),
  rgba(15, 23, 42),
  rgba(255, 255, 255),
];

/** What a redaction area can become once applied: opaque, and plain. */
const REDACT_SWATCHES: readonly Rgba[] = [
  rgba(0, 0, 0),
  rgba(64, 64, 64),
  rgba(128, 128, 128),
  rgba(255, 255, 255),
];

/** Whether a colour is this swatch, alpha aside (a highlight keeps its own). */
function sameHue(a: Rgba | null, b: Rgba): boolean {
  if (a === null) return false;
  const near = (x: number, y: number): boolean => Math.abs(x - y) < 1 / 255;
  return near(a.r, b.r) && near(a.g, b.g) && near(a.b, b.b);
}

function colorOf(obj: AnnotObject): Rgba | null {
  const p = obj.payload;
  if ("Markup" in p) return p.Markup.color;
  if ("FreeText" in p) return p.FreeText.color;
  if ("Ink" in p) return p.Ink.color;
  if ("Line" in p) return p.Line.color;
  if ("Shape" in p) return p.Shape.style.stroke;
  if ("Polygon" in p) return p.Polygon.style.stroke;
  if ("Note" in p) return p.Note.color;
  if ("Stamp" in p) return p.Stamp.color;
  return null;
}

function withColor(obj: AnnotObject, color: Rgba): AnnotObject {
  const next = structuredClone(obj);
  const p = next.payload;
  // The alpha already on the object is kept: a highlight is a wash, and
  // changing its hue should not make it opaque.
  const keepAlpha = (c: Rgba | null): Rgba => ({ ...color, a: c?.a ?? color.a });
  if ("Markup" in p) p.Markup.color = keepAlpha(p.Markup.color);
  else if ("FreeText" in p) p.FreeText.color = keepAlpha(p.FreeText.color);
  else if ("Ink" in p) p.Ink.color = keepAlpha(p.Ink.color);
  else if ("Line" in p) p.Line.color = keepAlpha(p.Line.color);
  else if ("Shape" in p) p.Shape.style.stroke = keepAlpha(p.Shape.style.stroke);
  else if ("Polygon" in p) p.Polygon.style.stroke = keepAlpha(p.Polygon.style.stroke);
  else if ("Note" in p) p.Note.color = keepAlpha(p.Note.color);
  else if ("Stamp" in p) p.Stamp.color = keepAlpha(p.Stamp.color);
  return next;
}

function strokeWidthOf(obj: AnnotObject): number | null {
  const p = obj.payload;
  if ("Ink" in p) return p.Ink.width;
  if ("Line" in p) return p.Line.width;
  if ("Shape" in p) return p.Shape.style.stroke_width;
  if ("Polygon" in p) return p.Polygon.style.stroke_width;
  return null;
}

function withStrokeWidth(obj: AnnotObject, width: number): AnnotObject {
  const next = structuredClone(obj);
  const p = next.payload;
  if ("Ink" in p) p.Ink.width = width;
  else if ("Line" in p) p.Line.width = width;
  else if ("Shape" in p) p.Shape.style.stroke_width = width;
  else if ("Polygon" in p) p.Polygon.style.stroke_width = width;
  return next;
}

function textOf(obj: AnnotObject): string | null {
  const p = obj.payload;
  if ("FreeText" in p) return p.FreeText.text;
  if ("Stamp" in p) return p.Stamp.label;
  if ("Note" in p) return p.Note.text;
  return null;
}

function withText(obj: AnnotObject, text: string): AnnotObject {
  const next = structuredClone(obj);
  const p = next.payload;
  if ("FreeText" in p) p.FreeText.text = text;
  else if ("Stamp" in p) p.Stamp.label = text;
  else if ("Note" in p) p.Note.text = text;
  return next;
}

function fontOf(obj: AnnotObject): { family: string; size: number } | null {
  const p = obj.payload;
  if ("FreeText" in p) return p.FreeText.font;
  if ("Stamp" in p) return p.Stamp.font;
  return null;
}

function withFontSize(obj: AnnotObject, size: number): AnnotObject {
  const next = structuredClone(obj);
  const p = next.payload;
  if ("FreeText" in p) p.FreeText.font.size = size;
  else if ("Stamp" in p) p.Stamp.font.size = size;
  return next;
}

/** Families offered. The standard-14 faces need no embedding at all, which is
 * why they are the ones Phase 3 can promise; anything else waits for Phase 4's
 * font embedding rather than being listed and then refused. */
const FAMILIES = ["Times New Roman", "Helvetica", "Courier"];

function withFamily(obj: AnnotObject, family: string): AnnotObject {
  const next = structuredClone(obj);
  const p = next.payload;
  if ("FreeText" in p) p.FreeText.font.family = family;
  else if ("Stamp" in p) p.Stamp.font.family = family;
  return next;
}

export function PropertiesPanel(): React.JSX.Element | null {
  const selection = useDocument((s) => s.selection);
  const annots = useDocument((s) => s.annots);
  const store = useDocument.getState;
  const objects = store().selectedObjects();

  // Subscribed to `annots` so the panel follows an object that was just
  // dragged; the value itself is read through the store.
  void annots;

  if (selection.length === 0 || objects.length === 0) return null;
  const first = objects[0] as AnnotObject;
  const color = colorOf(first);
  const width = strokeWidthOf(first);
  const text = objects.length === 1 ? textOf(first) : null;
  const font = objects.length === 1 ? fontOf(first) : null;
  // A redaction mark's colour is what its area becomes once applied, and the
  // mark itself always looks the same — so no opacity, and its own palette.
  const redact = objects.every((o) => o.kind === "Redact");
  const swatches = redact ? REDACT_SWATCHES : SWATCHES;
  const colorLabel = redact ? t("redact.fill") : t("props.color");

  const apply = (map: (obj: AnnotObject) => AnnotObject): void => {
    void store().replaceAnnots(objects.map(map));
  };

  return (
    <aside
      aria-label={t("props.label")}
      className="w-[260px] shrink-0 flex flex-col gap-4 p-3 border-l border-[var(--izul-border)] bg-[var(--izul-surface)] overflow-auto"
    >
      <header className="flex items-baseline justify-between">
        <h2 className="text-[13px] font-medium">{t("props.label")}</h2>
        <span className="text-[12px] text-[var(--izul-text-dim)]">
          {objects.length === 1 ? t(`kind.${first.kind}` as never) : `${objects.length} objek`}
        </span>
      </header>

      {color !== null && (
        <section>
          <h3 className="text-[12px] text-[var(--izul-text-dim)] mb-1.5">{colorLabel}</h3>
          <div className="flex flex-wrap gap-1.5">
            {swatches.map((swatch, i) => (
              <button
                key={i}
                type="button"
                aria-label={`${colorLabel} ${i + 1}`}
                aria-pressed={sameHue(color, swatch)}
                onClick={() => apply((o) => withColor(o, swatch))}
                style={{ background: cssColor(swatch) }}
                className={[
                  "w-6 h-6 rounded-[6px] border border-[var(--izul-border)]",
                  sameHue(color, swatch) ? "outline outline-2 outline-offset-2 outline-[var(--izul-accent)]" : "",
                ].join(" ")}
              />
            ))}
          </div>
        </section>
      )}

      {!redact && (
        <section>
          <label className="block text-[12px] text-[var(--izul-text-dim)] mb-1">
            {t("props.opacity")} — {Math.round(first.opacity * 100)}%
          </label>
          <input
            type="range"
            min={10}
            max={100}
            value={Math.round(first.opacity * 100)}
            onChange={(e) =>
              apply((o) => ({ ...structuredClone(o), opacity: Number(e.target.value) / 100 }))
            }
            className="w-full"
          />
        </section>
      )}

      {width !== null && (
        <section>
          <label className="block text-[12px] text-[var(--izul-text-dim)] mb-1">
            {t("props.strokeWidth")} — {width.toFixed(1)} pt
          </label>
          <input
            type="range"
            min={0.5}
            max={20}
            step={0.5}
            value={width}
            onChange={(e) => apply((o) => withStrokeWidth(o, Number(e.target.value)))}
            className="w-full"
          />
        </section>
      )}

      {font !== null && (
        <section className="flex flex-col gap-2">
          <label className="text-[12px] text-[var(--izul-text-dim)]">
            {t("props.font")}
            <select
              value={font.family}
              onChange={(e) => apply((o) => withFamily(o, e.target.value))}
              className="mt-1 w-full rounded-[8px] bg-[var(--izul-canvas)] border border-[var(--izul-border)] px-2 py-1 text-[13px]"
            >
              {FAMILIES.map((family) => (
                <option key={family} value={family}>
                  {family}
                </option>
              ))}
            </select>
          </label>
          <label className="text-[12px] text-[var(--izul-text-dim)]">
            {t("props.fontSize")} — {font.size.toFixed(0)} pt
            <input
              type="range"
              min={6}
              max={72}
              value={font.size}
              onChange={(e) => apply((o) => withFontSize(o, Number(e.target.value)))}
              className="w-full"
            />
          </label>
        </section>
      )}

      {text !== null && (
        <section>
          <label className="block text-[12px] text-[var(--izul-text-dim)] mb-1">
            {t("props.text")}
          </label>
          <textarea
            value={text}
            rows={4}
            onChange={(e) => apply((o) => withText(o, e.target.value))}
            className="w-full rounded-[8px] bg-[var(--izul-canvas)] border border-[var(--izul-border)] px-2 py-1.5 text-[13px]"
          />
        </section>
      )}

      <section className="flex flex-col gap-2">
        <label className="flex items-center gap-2 text-[12px]">
          <input
            type="checkbox"
            checked={first.locked}
            onChange={(e) =>
              apply((o) => ({ ...structuredClone(o), locked: e.target.checked }))
            }
          />
          {t("props.locked")}
        </label>
        {/* No rotation for redaction marks: what is applied is upright. */}
        {!redact && (
          <label className="text-[12px] text-[var(--izul-text-dim)]">
            {t("props.rotation")} — {Math.round(first.rotation)}°
            <input
              type="range"
              min={-180}
              max={180}
              value={Math.round(first.rotation)}
              onChange={(e) =>
                apply((o) => ({ ...structuredClone(o), rotation: Number(e.target.value) }))
              }
              className="w-full"
            />
          </label>
        )}
      </section>

      <button
        type="button"
        onClick={() => void store().deleteSelected()}
        className="mt-auto rounded-[8px] border border-[var(--izul-danger)] text-[var(--izul-danger)] py-1.5 text-[13px]"
      >
        {t("annot.delete")}
      </button>
    </aside>
  );
}
