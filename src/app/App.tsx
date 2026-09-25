/**
 * Application shell, laid out after WPS Office's PDF editor (SPEC 12, revised
 * 2026-09-23):
 *
 *   title bar with the tabs · menu row · ribbon
 *   icon rail · side panel · viewport · properties panel
 *   bottom bar
 *
 * or, when the "Beranda" tab is in front, the home screen under the title bar.
 * The viewport stays mounted while the home screen is showing, so going back
 * to a document costs nothing: the tabs are still open, their tiles still
 * cached, their scroll positions still where they were.
 *
 * Three things happen once, at startup, and they happen in this order for a
 * reason: files named on the command line are what the user just double-clicked
 * and must win the focus, so the restored session is opened first and the
 * startup files after it. A *later* double-click reaches the running window
 * through the single-instance channel instead — see `openFiles.ts`.
 */

import { useEffect } from "react";
import { About } from "./About";
import { BottomBar } from "./BottomBar";
import { ExportDialog } from "./ExportDialog";
import { FileBanner } from "./FileBanner";
import { FormBanner } from "./FormsPanel";
import { NoticeToast } from "./NoticeToast";
import { PanelGrid } from "./PanelGrid";
import { PromptDialog } from "./PromptDialog";
import { RedactDialog } from "./RedactDialog";
import { OcrDialog } from "./OcrDialog";
import { TextEditDialog } from "./TextEditDialog";
import { CommandPalette } from "./CommandPalette";
import { PrintDialog } from "./PrintDialog";
import { ShortcutsDialog } from "./ShortcutsDialog";
import { SplitDialog } from "./SplitDialog";
import { Home } from "./Home";
import { PropertiesPanel } from "./PropertiesPanel";
import { MenuBar, Ribbon } from "./Ribbon";
import { LeftRail, Sidebar } from "./Sidebar";
import { TitleBar } from "./TitleBar";
import { useArmedMarkup } from "./armedMarkup";
import { useDropTarget } from "./dropTarget";
import { installDraftOffer, useAutosave, useCloseGuard, useFileWatch } from "./fileActions";
import { useOpenFilesFromOtherInstance } from "./openFiles";
import { useShortcuts } from "./shortcuts";
import { BareHint } from "./BareHint";
import { usePageInversion, usePresentation } from "./presentation";
import { useDocument } from "@/state/documentStore";
import { useUi } from "@/state/uiStore";
import { useWorkspace } from "@/state/workspaceStore";
import { t } from "@/i18n";

let startupDone = false;

// Before anything opens: the restored session's documents are the ones most
// likely to have a draft waiting.
installDraftOffer();

export function App(): React.JSX.Element {
  const doc = useDocument((s) => s.doc);
  const home = useWorkspace((s) => s.home);
  const error = useWorkspace((s) => s.error);
  const dropping = useDropTarget();
  useShortcuts();
  useArmedMarkup();
  useOpenFilesFromOtherInstance();
  useCloseGuard();
  useAutosave();
  useFileWatch();
  usePresentation();
  usePageInversion();
  const presenting = useUi((s) => s.presenting);
  const focusMode = useUi((s) => s.focusMode);
  const bare = (presenting || focusMode) && doc !== null;

  useEffect(() => {
    // Once per run, not once per mount. React's development mode mounts every
    // component twice on purpose, and the second mount would restore the same
    // session a second time; the guard lives outside the component because that
    // is the only place the two mounts share.
    if (startupDone) return;
    startupDone = true;
    // The stored theme and page inversion, before the first document draws.
    void useUi.getState().loadPrefs();
    const workspace = useWorkspace.getState();
    void workspace
      .restoreSession()
      .then(() => workspace.openStartupFiles())
      // The remembered split, once there are documents to put in it.
      .then(() => workspace.loadPanels())
      .catch(() => {
        // Neither is something the user asked for in this moment; a failure to
        // restore must not be the first thing they see.
      });
  }, []);

  const showHome = (home || doc === null) && !bare;

  return (
    <div
      className="h-full flex flex-col bg-[var(--izul-chrome)]"
      data-presenting={presenting ? "" : undefined}
      // Black around the page while presenting: the canvas reads this.
      style={presenting ? ({ "--izul-canvas": "#000000" } as React.CSSProperties) : undefined}
    >
      {!bare && <TitleBar />}
      {error !== null && (
        <div role="alert" className="px-3 py-2 bg-[var(--izul-danger)] text-white text-[13px]">
          {t("err.open")}: {error}
        </div>
      )}
      {showHome && <Home dropping={dropping} />}
      {doc !== null && (
        <div className={showHome ? "hidden" : "flex-1 min-h-0 flex flex-col"}>
          {!bare && (
            <>
              <MenuBar />
              <Ribbon />
              <FileBanner />
              <FormBanner />
            </>
          )}
          <div
            className={[
              "flex-1 min-h-0 flex border-t border-[var(--izul-border)]",
              dropping ? "outline outline-2 -outline-offset-2 outline-[var(--izul-accent)]" : "",
            ].join(" ")}
          >
            {!bare && <LeftRail />}
            {!bare && <Sidebar />}
            <PanelGrid />
            {!bare && <PropertiesPanel />}
          </div>
          {!bare && <BottomBar />}
          {bare && <BareHint presenting={presenting} />}
        </div>
      )}
      <About />
      <ExportDialog />
      <SplitDialog />
      <RedactDialog />
      <OcrDialog />
      <TextEditDialog />
      <CommandPalette />
      <PrintDialog />
      <ShortcutsDialog />
      <PromptDialog />
      <NoticeToast />
    </div>
  );
}
