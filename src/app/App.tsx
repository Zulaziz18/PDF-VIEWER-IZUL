/**
 * Application shell.
 *
 * Phase 1's window: title bar, reading toolbar, sidebar, viewport, status bar.
 * Tabs and split panes arrive in Phase 2 and the command palette in Phase 8;
 * this file is their eventual home, which is why the layout is already a set of
 * independent regions rather than one block.
 */

import { EmptyState } from "./EmptyState";
import { Sidebar } from "./Sidebar";
import { StatusBar } from "./StatusBar";
import { TitleBar } from "./TitleBar";
import { Toolbar } from "./Toolbar";
import { Viewport } from "./Viewport";
import { useShortcuts } from "./shortcuts";
import { useDocument } from "@/state/documentStore";
import { t } from "@/i18n";

export function App(): React.JSX.Element {
  const doc = useDocument((s) => s.doc);
  const error = useDocument((s) => s.error);
  useShortcuts();

  return (
    <div className="h-full flex flex-col bg-[var(--izul-canvas)]">
      <TitleBar />
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
          </div>
        </>
      )}
      <StatusBar />
    </div>
  );
}
