/**
 * Application shell.
 *
 * Phase 2's window: title bar, tab strip, reading toolbar, sidebar, viewport,
 * search panel, status bar. The regions were already independent in Phase 1
 * precisely so that tabs and the search panel could be dropped in beside them
 * rather than through them.
 *
 * Three things happen once, at startup, and they happen in this order for a
 * reason: files named on the command line are what the user just double-clicked
 * and must win the focus, so the restored session is opened first and the
 * startup files after it.
 */

import { useEffect } from "react";
import { EmptyState } from "./EmptyState";
import { SearchPanel } from "./SearchPanel";
import { Sidebar } from "./Sidebar";
import { StatusBar } from "./StatusBar";
import { TabBar } from "./TabBar";
import { TitleBar } from "./TitleBar";
import { Toolbar } from "./Toolbar";
import { Viewport } from "./Viewport";
import { useDropTarget } from "./dropTarget";
import { useShortcuts } from "./shortcuts";
import { useDocument } from "@/state/documentStore";
import { useWorkspace } from "@/state/workspaceStore";
import { t } from "@/i18n";

export function App(): React.JSX.Element {
  const doc = useDocument((s) => s.doc);
  const error = useWorkspace((s) => s.error);
  useShortcuts();
  useDropTarget();

  useEffect(() => {
    const workspace = useWorkspace.getState();
    void workspace
      .restoreSession()
      .then(() => workspace.openStartupFiles())
      .catch(() => {
        // Neither is something the user asked for in this moment; a failure to
        // restore must not be the first thing they see.
      });
  }, []);

  return (
    <div className="h-full flex flex-col bg-[var(--izul-canvas)]">
      <TitleBar />
      <TabBar />
      {error !== null && (
        <div role="alert" className="px-3 py-2 bg-[var(--izul-danger)] text-white text-[13px]">
          {t("err.open")}: {error}
        </div>
      )}
      {doc === null ? (
        <EmptyState />
      ) : (
        <>
          <Toolbar />
          <div className="flex-1 min-h-0 flex">
            <Sidebar />
            <Viewport />
            <SearchPanel />
          </div>
        </>
      )}
      <StatusBar />
    </div>
  );
}
