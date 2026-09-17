/**
 * Pixels for image annotations.
 *
 * The display list carries an `ImageRef`, never bytes (SPEC 3.2): the
 * appearance-stream backend resolves it to a PDF XObject, and the canvas
 * backend resolves it here. Images come back over the same custom protocol the
 * tiles use, for the same reason — the invoke bridge is JSON, and a photograph
 * through it is several megabytes of base64 parsed on the UI thread.
 */

const cache = new Map<string, HTMLImageElement>();

/** The URI an image annotation's pixels live at. */
export function imageUri(doc: number, image: number): string {
  // `http://izul.localhost/...`, not `izul://`: WebView2 on Windows will not
  // fetch a bare custom scheme, and `wry` only translates the http form. The
  // Phase 1 notes in CLAUDE.md have the full story.
  return `http://izul.localhost/image/${doc}/${image}`;
}

/**
 * Loads an image, once per document and handle.
 *
 * Returns `null` when it cannot be loaded, and the canvas backend then draws
 * nothing — which is the honest outcome: a placeholder box would be a picture
 * that is not what the saved file will contain.
 */
export async function loadImage(doc: number, image: number): Promise<HTMLImageElement | null> {
  const key = `${doc}/${image}`;
  const cached = cache.get(key);
  if (cached) return cached;
  const element = new Image();
  element.src = imageUri(doc, image);
  try {
    await element.decode();
  } catch {
    return null;
  }
  cache.set(key, element);
  return element;
}

/** Drops a document's images when its tab closes. */
export function forgetImages(doc: number): void {
  for (const key of [...cache.keys()]) {
    if (key.startsWith(`${doc}/`)) cache.delete(key);
  }
}

/** Every `ImageRef` a set of display lists refers to. */
export function referencedImages(lists: readonly { ops: { ops: readonly unknown[] } }[]): number[] {
  const out = new Set<number>();
  for (const list of lists) {
    for (const op of list.ops.ops) {
      if (op !== null && typeof op === "object" && "DrawImage" in op) {
        const draw = (op as { DrawImage: { image: number } }).DrawImage;
        out.add(draw.image);
      }
    }
  }
  return [...out];
}
