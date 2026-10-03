/**
 * Printing (SPEC 11.1), Phase 8.
 *
 * The backend renders the chosen pages as the editor has them — with or
 * without our annotations — and serves them at `izul://print/{job}/{i}`.
 * They go into a container that only exists for the print medium, and the
 * webview's own print dialog takes over: preview, printer, copies, paper.
 * When it closes, the container and the backend's files go.
 *
 * `printLayout` is pure: how each page sits on the paper.
 */

import { invoke } from "@tauri-apps/api/core";
import { t } from "@/i18n";
import { useUi } from "@/state/uiStore";
import { fill } from "./fileActions";

export type PrintScale = "fit" | "actual";

export interface PrintOptions {
  readonly pages: readonly number[];
  readonly annotations: boolean;
  readonly scale: PrintScale;
  /** Render resolution: 200 standard, 300 fine. */
  readonly dpi: number;
}

/**
 * The CSS size of a page image on paper: fitted to the printable area
 * (the stylesheet keeps its proportions, `object-fit: contain`), or at its
 * real size in points (`px * 72 / dpi`) — never larger than the area.
 */
export function printLayout(scale: PrintScale, naturalWidth: number, naturalHeight: number, dpi: number): {
  width: string;
  height: string;
} {
  if (scale === "fit" || naturalWidth <= 0 || naturalHeight <= 0) {
    return { width: "100%", height: "100%" };
  }
  const pt = (px: number): string => `${((px * 72) / dpi).toFixed(2)}pt`;
  return { width: pt(naturalWidth), height: pt(naturalHeight) };
}

/** The page URL, in the spelling WebView2 can load (see `tileSource.ts`). */
export function printUrl(job: number, index: number): string {
  return `http://izul.localhost/print/${job}/${index}`;
}

const CONTAINER = "izul-print";

function cleanup(job: number | null): void {
  document.getElementById(CONTAINER)?.remove();
  if (job !== null) void invoke("print_done", { job }).catch(() => undefined);
}

/** Prepares and prints; resolves when the print dialog has been shown. */
export async function printDocument(doc: number, opts: PrintOptions): Promise<boolean> {
  cleanup(null);
  useUi.getState().notify({ kind: "ok", text: fill(t("print.preparing"), { count: opts.pages.length }) });
  let job: number;
  try {
    const prepared = await invoke<{ job: number; count: number }>("print_prepare", {
      doc,
      pages: [...opts.pages],
      dpi: opts.dpi,
      annotations: opts.annotations,
    });
    job = prepared.job;
  } catch (e) {
    useUi.getState().notify({ kind: "error", text: t("print.failed"), detail: String(e) });
    return false;
  }
  const host = document.createElement("div");
  host.id = CONTAINER;
  host.dataset["scale"] = opts.scale;
  const images: HTMLImageElement[] = [];
  opts.pages.forEach((_, i) => {
    const sheet = document.createElement("div");
    sheet.className = "izul-print-page";
    const img = document.createElement("img");
    img.alt = "";
    img.src = printUrl(job, i);
    sheet.appendChild(img);
    host.appendChild(sheet);
    images.push(img);
  });
  document.body.appendChild(host);
  try {
    await Promise.all(images.map((img) => img.decode()));
  } catch (e) {
    cleanup(job);
    useUi.getState().notify({ kind: "error", text: t("print.failed"), detail: String(e) });
    return false;
  }
  for (const img of images) {
    const size = printLayout(opts.scale, img.naturalWidth, img.naturalHeight, opts.dpi);
    img.style.width = size.width;
    img.style.height = size.height;
  }
  // WebView2's print dialog does not block; the pages must stay until it closes.
  window.addEventListener("afterprint", () => cleanup(job), { once: true });
  window.print();
  return true;
}
