import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => null }));

const { createDocumentSession } = await import("../documentSession");

function session() {
  return createDocumentSession({
    doc: 7,
    path: "/x/a.pdf",
    page_count: 3,
    page_sizes: [
      [100, 200],
      [100, 200],
      [300, 400],
    ],
    permissions: 0,
    encrypted: false,
    restored: null,
  });
}

/**
 * The translation every fetch goes through once pages have moved. A page one
 * off here is a thumbnail, a text layer or a search highlight belonging to
 * the neighbouring page — the kind of bug that looks like a rendering fault.
 */
describe("page map in the session", () => {
  it("is the identity until a page operation", () => {
    const s = session().getState();
    expect(s.sourceOf(2)).toEqual({ doc: 7, page: 2 });
    expect(s.displayOfOwn(1)).toBe(1);
  });

  it("follows the backend's layout once there is one", () => {
    const store = session();
    const before = store.getState();
    store.getState().applyPagesView({
      mapped: true,
      map_revision: 3,
      sources: ["/x/lain.pdf"],
      pages: [
        { render_doc: 7, page: 2, rotation: 1, width: 300, height: 400, source: 0 },
        { render_doc: null, page: 0, rotation: 0, width: 50, height: 60, source: 0 },
        { render_doc: 7, page: 0, rotation: 0, width: 100, height: 200, source: 0 },
        { render_doc: 12, page: 4, rotation: 0, width: 10, height: 20, source: 1 },
      ],
    });
    const s = store.getState();
    expect(s.pageCount).toBe(4);
    expect(s.pageSizes[1]).toEqual({ width: 50, height: 60 });
    expect(s.pageRotation).toEqual({ 0: 1 });
    expect(s.sourceOf(0)).toEqual({ doc: 7, page: 2 });
    expect(s.sourceOf(1)).toEqual({ doc: null, page: 0 });
    expect(s.sourceOf(3)).toEqual({ doc: 12, page: 4 });
    // The file's page 0 is now shown third; page 1 was deleted.
    expect(s.displayOfOwn(0)).toBe(2);
    expect(s.displayOfOwn(1)).toBeNull();
    // A foreign file's page 4 is not "own page 4".
    expect(s.displayOfOwn(4)).toBeNull();
    // Everything cached by page number starts again.
    expect(s.pagesEpoch).toBe(before.pagesEpoch + 1);
    expect(s.generation).toBeGreaterThan(before.generation);
    expect(s.mapRevision).toBe(3);
  });

  it("goes back to the identity when the backend says the map is gone", () => {
    const store = session();
    store.getState().applyPagesView({
      mapped: true,
      map_revision: 1,
      sources: [],
      pages: [{ render_doc: 7, page: 1, rotation: 0, width: 100, height: 200, source: 0 }],
    });
    store.getState().applyPagesView({
      mapped: false,
      map_revision: 2,
      sources: [],
      pages: [
        { render_doc: 7, page: 0, rotation: 0, width: 100, height: 200, source: 0 },
        { render_doc: 7, page: 1, rotation: 0, width: 100, height: 200, source: 0 },
      ],
    });
    const s = store.getState();
    expect(s.pagesView).toBeNull();
    expect(s.sourceOf(1)).toEqual({ doc: 7, page: 1 });
    expect(s.page).toBeLessThan(2);
  });

  it("keeps a page selection only where pages still exist", () => {
    const store = session();
    store.getState().setPageSelection([2, 0, 2]);
    expect(store.getState().pageSelection).toEqual([0, 2]);
    store.getState().applyPagesView({
      mapped: true,
      map_revision: 1,
      sources: [],
      pages: [{ render_doc: 7, page: 0, rotation: 0, width: 100, height: 200, source: 0 }],
    });
    expect(store.getState().pageSelection).toEqual([0]);
  });
});
