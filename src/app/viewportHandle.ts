/**
 * The live viewports, so that panels outside them can ask one to scroll.
 *
 * One per document on screen — split view shows up to four (Phase 5) — and
 * `viewport()` answers for the document in front, which is the one the
 * sidebar, the search panel and the keyboard act on. A module-level map
 * rather than a context: the viewport is imperative, and threading refs
 * through the tree would make every component between the sidebar and the
 * canvas aware of something none of them use.
 */

import type { ViewportRenderer } from "@/viewport/renderer";
import { useWorkspace } from "@/state/workspaceStore";

const byDoc = new Map<number, ViewportRenderer>();

export function registerViewport(doc: number, renderer: ViewportRenderer | null): void {
  if (renderer) byDoc.set(doc, renderer);
  else byDoc.delete(doc);
}

/** The viewport showing `doc`, if one is. */
export function viewportOf(doc: number): ViewportRenderer | null {
  return byDoc.get(doc) ?? null;
}

/** The viewport of the document in front. */
export function viewport(): ViewportRenderer | null {
  const active = useWorkspace.getState().activeDoc;
  return active === null ? null : viewportOf(active);
}
