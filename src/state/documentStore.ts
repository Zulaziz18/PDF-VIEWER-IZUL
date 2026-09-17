/**
 * The active document, as the components see it.
 *
 * Phase 1 exported a single store from here; Phase 2 made state per-document
 * (`documentSession`) with the window holding the map of them
 * (`workspaceStore`). This module is what keeps every component that only ever
 * cares about *the document in front* from having to know that: it subscribes
 * to whichever session is active and re-subscribes when the tab changes.
 *
 * `useDocument.getState()` is kept for the imperative callers — the renderer's
 * callbacks and the keyboard shortcuts — which read the store outside React and
 * would otherwise need a second path.
 */

import { useStore } from "zustand";
import { useWorkspace } from "./workspaceStore";
import type { DocumentState } from "./documentSession";

export type {
  DocumentState,
  IndexHit,
  OpenedDoc,
  OutlineEntry,
  PageHit,
  SavedView,
  SearchScope,
  SearchState,
  SidebarTab,
  ZoomMode,
} from "./documentSession";

interface UseDocument {
  <T>(selector: (state: DocumentState) => T): T;
  getState(): DocumentState;
}

function useDocumentImpl<T>(selector: (state: DocumentState) => T): T {
  // Two subscriptions on purpose: one to the workspace, so switching tabs
  // re-renders with the new session, and one to the session itself. Reading the
  // session through `getState()` alone would leave components stale, because a
  // change inside a session does not touch the workspace store at all.
  const store = useWorkspace((s) => (s.activeDoc !== null ? s.sessions.get(s.activeDoc) : undefined));
  const active = useWorkspace((s) => s.activeSession);
  return useStore(store ?? active(), selector);
}

export const useDocument: UseDocument = Object.assign(useDocumentImpl, {
  getState: (): DocumentState => useWorkspace.getState().activeSession().getState(),
});
