/**
 * The export dialog: which pages, and — for images — which format and how
 * sharp. Where the file goes is asked next, by the platform's own picker,
 * because that is the window every Windows user already knows how to drive.
 *
 * "Ekspor Rata" has no dialog: it is the whole document, so the only question
 * is where, and the save picker asks it.
 */

import { useEffect, useMemo, useRef, useState, type JSX } from "react";
import { Icon } from "@/design/Icon";
import { t, type StringKey } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { formatPageRange, parsePageRange, type PageRangeError } from "@/state/pageRange";
import { useUi } from "@/state/uiStore";
import { exportImages, exportPages } from "./fileActions";

type Scope = "all" | "current" | "range";

/** Resolutions, each with what it is for — the number alone means little to
 * most people choosing it. */
const DPI_CHOICES: ReadonlyArray<{ dpi: number; label: StringKey }> = [
  { dpi: 72, label: "export.dpi72" },
  { dpi: 150, label: "export.dpi150" },
  { dpi: 300, label: "export.dpi300" },
  { dpi: 600, label: "export.dpi600" },
];

export function rangeMessage(error: PageRangeError): string {
  switch (error.kind) {
    case "empty":
      return t("export.rangeEmpty");
    case "syntax":
      return `${t("export.rangeSyntax")} “${error.part}”`;
    case "outOfRange":
      return `${t("export.rangeOut")} ${error.pageCount} (“${error.part}”)`;
  }
}

export function Choice(props: { name: string; checked: boolean; onChange: () => void; children: React.ReactNode }): JSX.Element {
  return (
    <label className="flex items-center gap-2 h-7 text-[13px] cursor-default">
      <input type="radio" name={props.name} checked={props.checked} onChange={props.onChange} className="accent-[var(--izul-accent)]" />
      {props.children}
    </label>
  );
}

export function ExportDialog(): JSX.Element | null {
  const kind = useUi((s) => s.exporting);
  const doc = useDocument((s) => s.doc);
  const page = useDocument((s) => s.page);
  const pageCount = useDocument((s) => s.pageCount);
  const ref = useRef<HTMLDialogElement>(null);

  const [scope, setScope] = useState<Scope>("all");
  const [range, setRange] = useState("");
  const [format, setFormat] = useState<"png" | "jpg">("png");
  const [dpi, setDpi] = useState<number>(150);
  const [quality, setQuality] = useState(90);
  const [running, setRunning] = useState(false);

  useEffect(() => {
    if (kind === null || kind === "split") return;
    // Pages selected in the page panel are what "extract" most likely means.
    const selected = useDocument.getState().pageSelection;
    setScope(kind === "pages" || selected.length > 0 ? "range" : "all");
    setRange(selected.length > 0 ? formatPageRange(selected) : kind === "pages" ? `${page + 1}` : "");
    setRunning(false);
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    // Only when it opens (`kind` alone in the list): the page moving
    // underneath must not reset what the user has typed.
  }, [kind]);

  const parsed = useMemo(() => {
    if (scope === "all") return { ok: true as const, pages: Array.from({ length: pageCount }, (_, i) => i) };
    if (scope === "current") return { ok: true as const, pages: [page] };
    return parsePageRange(range, pageCount);
  }, [scope, range, page, pageCount]);

  if (kind === null || kind === "split" || doc === null) return null;
  const close = (): void => {
    ref.current?.close();
    useUi.getState().setExporting(null);
  };
  const go = async (): Promise<void> => {
    if (!parsed.ok || running) return;
    setRunning(true);
    const done =
      kind === "pages"
        ? await exportPages(doc, parsed.pages)
        : await exportImages(doc, parsed.pages, format, dpi, quality);
    setRunning(false);
    if (done) close();
  };

  const title = kind === "pages" ? t("export.pagesTitle") : t("export.imagesTitle");
  return (
    <dialog
      ref={ref}
      aria-labelledby="export-title"
      onCancel={(e) => {
        e.preventDefault();
        if (!running) close();
      }}
      className="m-auto w-[480px] max-w-[calc(100vw-32px)] p-0 rounded-[12px] border border-[var(--izul-border)] bg-[var(--izul-surface)] text-[var(--izul-text)] shadow-[0_12px_40px_rgba(0,0,0,0.25)] backdrop:bg-black/30"
    >
      <form
        method="dialog"
        onSubmit={(e) => {
          e.preventDefault();
          void go();
        }}
        className="p-6 flex flex-col gap-5"
      >
        <div className="flex items-center gap-3">
          <Icon name={kind === "pages" ? "extractPages" : "toImages"} size={28} tone={kind === "pages" ? "blue" : "teal"} />
          <h2 id="export-title" className="text-[17px] font-semibold">
            {title}
          </h2>
        </div>

        <fieldset className="flex flex-col gap-0.5">
          <legend className="mb-1.5 text-[12px] font-semibold text-[var(--izul-text-dim)]">{t("export.pages")}</legend>
          <Choice name="scope" checked={scope === "all"} onChange={() => setScope("all")}>
            {t("export.all")} ({pageCount})
          </Choice>
          <Choice name="scope" checked={scope === "current"} onChange={() => setScope("current")}>
            {t("export.current")} ({page + 1})
          </Choice>
          <Choice name="scope" checked={scope === "range"} onChange={() => setScope("range")}>
            {t("export.range")}
          </Choice>
          <input
            type="text"
            aria-label={t("export.range")}
            value={range}
            placeholder={t("export.rangeHint")}
            onFocus={() => setScope("range")}
            onChange={(e) => setRange(e.target.value)}
            className="ml-6 h-8 px-2 text-[13px] rounded-[6px] bg-[var(--izul-surface)] border border-[var(--izul-border)] focus:border-[var(--izul-accent)] outline-none"
          />
          <p
            className={[
              "ml-6 mt-1 min-h-[18px] text-[12px]",
              parsed.ok ? "text-[var(--izul-text-dim)]" : "text-[var(--izul-danger)]",
            ].join(" ")}
            aria-live="polite"
          >
            {scope === "range" && (parsed.ok ? `${parsed.pages.length} ${t("export.pageCount")}` : rangeMessage(parsed.error))}
          </p>
        </fieldset>

        {kind === "images" && (
          <div className="grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-3 text-[13px]">
            <span className="text-[var(--izul-text-dim)]">{t("export.format")}</span>
            <div role="radiogroup" aria-label={t("export.format")} className="flex gap-1">
              {(["png", "jpg"] as const).map((f) => (
                <button
                  key={f}
                  type="button"
                  role="radio"
                  aria-checked={format === f}
                  onClick={() => setFormat(f)}
                  className={[
                    "h-8 px-3 rounded-[6px] border text-[13px]",
                    format === f
                      ? "border-[var(--izul-accent)] bg-[var(--izul-accent-soft)] text-[var(--izul-accent)] font-medium"
                      : "border-[var(--izul-border)] hover:bg-[var(--izul-surface-raised)]",
                  ].join(" ")}
                >
                  {f === "png" ? t("export.png") : t("export.jpg")}
                </button>
              ))}
            </div>
            <label htmlFor="export-dpi" className="text-[var(--izul-text-dim)]">
              {t("export.dpi")}
            </label>
            <select
              id="export-dpi"
              value={dpi}
              onChange={(e) => setDpi(Number(e.target.value))}
              className="h-8 px-2 w-[200px] rounded-[6px] bg-[var(--izul-surface)] border border-[var(--izul-border)]"
            >
              {DPI_CHOICES.map((d) => (
                <option key={d.dpi} value={d.dpi}>
                  {d.dpi} DPI — {t(d.label)}
                </option>
              ))}
            </select>
            {format === "jpg" && (
              <>
                <label htmlFor="export-quality" className="text-[var(--izul-text-dim)]">
                  {t("export.quality")}
                </label>
                <div className="flex items-center gap-3">
                  <input
                    id="export-quality"
                    type="range"
                    min={50}
                    max={100}
                    value={quality}
                    onChange={(e) => setQuality(Number(e.target.value))}
                    className="w-[160px] accent-[var(--izul-accent)]"
                  />
                  <span className="tabular-nums">{quality}</span>
                </div>
              </>
            )}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button
            type="button"
            disabled={running}
            onClick={close}
            className="h-9 px-4 rounded-[8px] text-[13px] border border-[var(--izul-border)] hover:bg-[var(--izul-surface-raised)] disabled:opacity-40"
          >
            {t("close.cancel")}
          </button>
          <button
            type="submit"
            disabled={!parsed.ok || running}
            className="h-9 px-4 rounded-[8px] text-[13px] bg-[var(--izul-accent)] text-[var(--izul-on-accent)] font-medium hover:brightness-110 disabled:opacity-40"
          >
            {running ? t("export.running") : kind === "pages" ? t("export.goPages") : t("export.goImages")}
          </button>
        </div>
      </form>
    </dialog>
  );
}
