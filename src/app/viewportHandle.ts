/**
 * The one live viewport, so that panels outside it can ask it to scroll.
 *
 * A module-level handle rather than a context: the viewport is imperative and
 * singular, and threading a ref through the tree would make every component
 * between the sidebar and the canvas aware of something none of them use.
 * Phase 2's split view turns this into a handle per panel.
 */

import type { ViewportRenderer } from "@/viewport/renderer";

let current: ViewportRenderer | null = null;

export function setViewport(renderer: ViewportRenderer | null): void {
  current = renderer;
}

export function viewport(): ViewportRenderer | null {
  return current;
}
