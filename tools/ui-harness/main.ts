/**
 * Entry point of the screenshot harness: install the mocks, start the real
 * application exactly as `src/app/main.tsx` does, and hand the scene scripts
 * in `shoot.mjs` a few levers that a user would pull with the mouse.
 *
 * The levers go through the same stores and the same viewport handle the
 * application's own controls use; Vite serves each module once, so these are
 * the very instances the running application holds.
 */

import { installMocks } from "./mock";

declare global {
  interface Window {
    __izul?: {
      goToPage(page: number): void;
      select(ids: number[]): void;
      sidebar(tab: "thumbnails" | "outline" | "annots"): void;
      layout(layout: "single" | "columns" | "rows" | "grid"): void;
      compare(on: boolean): void;
      pages(selection: number[]): void;
    };
  }
}

const data = window.__HARNESS__;
if (!data) {
  throw new Error("__HARNESS__ belum dipasang — jalankan lewat `npm run ui:shots`");
}
installMocks(data);
await import("@/app/main");

const { useDocument } = await import("@/state/documentStore");
const { viewport } = await import("@/app/viewportHandle");
const { useWorkspace } = await import("@/state/workspaceStore");
window.__izul = {
  goToPage(page) {
    useDocument.getState().setPage(page);
    viewport()?.goToPage(page);
  },
  select(ids) {
    useDocument.getState().select(ids);
  },
  sidebar(tab) {
    useDocument.getState().setSidebarTab(tab);
  },
  layout(layout) {
    useWorkspace.getState().setLayout(layout);
  },
  compare(on) {
    useWorkspace.getState().setCompare(on);
  },
  pages(selection) {
    useDocument.getState().setPageSelection(selection);
  },
};
