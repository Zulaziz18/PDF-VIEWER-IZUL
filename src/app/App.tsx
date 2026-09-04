/**
 * Application shell.
 *
 * Phase 0's window: a title bar, one document surface, and a status bar. Tabs,
 * split panes and the command palette arrive in Phases 2 and 8; this file is
 * their eventual home, which is why the layout is already a column of
 * independent regions rather than one block.
 */

import { EmptyState } from "./EmptyState";
import { PageView } from "./PageView";
import { StatusBar } from "./StatusBar";
import { TitleBar } from "./TitleBar";
import { useDocument } from "@/state/documentStore";
import { t } from "@/i18n";

export function App(): React.JSX.Element {
  const doc = useDocument((s) => s.doc);
  const error = useDocument((s) => s.error);

  return (
    <div className="h-full flex flex-col bg-[var(--izul-canvas)]">
      <TitleBar />
      {error !== null && (
        <div
          role="alert"
          className="px-3 py-2 bg-[var(--izul-danger)] text-white text-[13px]"
        >
          {t("err.open")}: {error}
        </div>
      )}
      {doc === null ? <EmptyState /> : <PageView />}
      <StatusBar />
    </div>
  );
}
