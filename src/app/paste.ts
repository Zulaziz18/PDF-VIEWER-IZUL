/**
 * Ctrl+V with a picture on the clipboard — a screenshot, an image copied from
 * a browser — puts it on the page (SPEC 11.2, Phase 8).
 *
 * Through the `paste` event rather than `navigator.clipboard.read()`: the
 * event carries the clipboard's files without a permission prompt, and it
 * does not fire for a paste into a text field — nor is it meant to: the
 * field gets its text, the page gets nothing.
 */

import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useDocument } from "@/state/documentStore";
import { placeImage } from "./actions";

/** One clipboard entry, as much of `DataTransferItem` as this needs. */
export interface ClipItem {
  readonly kind: string;
  readonly type: string;
  getAsFile(): Blob | null;
}

/** The picture a paste carries, if it carries one: the first image entry.
 * A copied web image comes as both HTML and a file; the file is the picture. */
export function pastedImage(items: ArrayLike<ClipItem>): Blob | null {
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (item && item.kind === "file" && item.type.startsWith("image/")) return item.getAsFile();
  }
  return null;
}

function typingIn(target: EventTarget | null): boolean {
  const el = target as { tagName?: string; isContentEditable?: boolean } | null;
  const tag = el?.tagName?.toUpperCase();
  return tag === "INPUT" || tag === "TEXTAREA" || el?.isContentEditable === true;
}

export async function pasteImage(doc: number, blob: Blob): Promise<void> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  const image = await invoke<number>("annot_paste_image", bytes, {
    headers: { "x-izul-doc": String(doc) },
  });
  await placeImage(doc, image);
}

export function usePasteImage(): void {
  useEffect(() => {
    const onPaste = (e: ClipboardEvent): void => {
      if (typingIn(e.target) || document.querySelector("dialog[open]")) return;
      const doc = useDocument.getState().doc;
      const items = e.clipboardData?.items;
      if (doc === null || !items) return;
      const blob = pastedImage(items);
      if (!blob) return;
      e.preventDefault();
      void pasteImage(doc, blob).catch((err: unknown) => console.warn("gambar tidak dapat ditempel", err));
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, []);
}
