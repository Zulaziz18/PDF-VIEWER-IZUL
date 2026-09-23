/**
 * The menu row and the ribbon (SPEC 12, revised 2026-09-23).
 *
 * Laid out after WPS Office's PDF editor: a "Berkas" menu and quick-access
 * buttons at the left of the menu row, the ribbon's tab names in the middle,
 * and under them one ribbon panel of large and small buttons in groups.
 *
 * **A ribbon tab exists only when every button on it works.** SPEC 0 forbids
 * stubs and dead controls, so WPS's "Halaman", "Lindungi" and "Isi & Tanda
 * Tangan" tabs are absent until the phase that builds them adds them here —
 * together with their entry in `RibbonTab`. "Konversi" arrived with Phase 4,
 * carrying the three conversions that phase can do without a network.
 *
 * The ribbon replaces the old contextual toolbar: "Beranda" is reading and
 * navigation, "Edit" and "Komentar" carry the annotation tools. That is the
 * mode switch SPEC 12 asks for, made the way WPS users already know it.
 */

import type { JSX } from "react";
import type { AnnotKind } from "@/annots/types";
import { Icon, type IconName, type Tone } from "@/design/Icon";
import {
  IconButton,
  MenuButton,
  RibbonButton,
  RibbonDivider,
  RibbonSmall,
  RibbonStack,
  type MenuItem,
} from "@/design/controls";
import { t, type StringKey } from "@/i18n";
import { useDocument } from "@/state/documentStore";
import { useUi, type MarkupKind, type RibbonTab } from "@/state/uiStore";
import { useWorkspace } from "@/state/workspaceStore";
import type { ViewMode } from "@/viewport/layout";
import type { Layout } from "@/state/panels";
import { toggleCompare } from "./compare";
import { insertImage, markupSelection, pickAndOpen } from "./actions";
import { exportFlat, requestCloseAll, requestCloseTab, saveDocument } from "./fileActions";
import {
  deletePages,
  duplicatePages,
  extractPages,
  insertBlankPage,
  mergeFile,
  rotatePages,
  splitDocument,
} from "./pageActions";

const TABS: ReadonlyArray<{ id: RibbonTab; label: StringKey }> = [
  { id: "home", label: "ribbon.home" },
  { id: "edit", label: "ribbon.edit" },
  { id: "pages", label: "ribbon.pages" },
  { id: "comment", label: "ribbon.comment" },
  { id: "convert", label: "ribbon.convert" },
];

// ---------------------------------------------------------------------------
// Menu row
// ---------------------------------------------------------------------------

function fileMenu(): MenuItem[] {
  const none = useWorkspace.getState().activeDoc === null;
  return [
    { label: t("menu.open"), icon: "open", tone: "amber", shortcut: "Ctrl+O", onSelect: () => void pickAndOpen() },
    { label: t("menu.home"), icon: "home", tone: "blue", onSelect: () => useWorkspace.getState().showHome() },
    {
      label: t("menu.save"),
      icon: "save",
      tone: "blue",
      shortcut: "Ctrl+S",
      separator: true,
      disabled: none,
      onSelect: () => void saveDocument(),
    },
    {
      label: t("menu.saveAs"),
      icon: "saveAs",
      tone: "blue",
      shortcut: "Ctrl+Shift+S",
      disabled: none,
      onSelect: () => void saveDocument(undefined, "saveAs"),
    },
    {
      label: t("convert.toImages"),
      icon: "toImages",
      tone: "teal",
      separator: true,
      disabled: none,
      onSelect: () => useUi.getState().setExporting("images"),
    },
    {
      label: t("convert.pages"),
      icon: "extractPages",
      tone: "blue",
      disabled: none,
      onSelect: () => useUi.getState().setExporting("pages"),
    },
    { label: t("convert.flat"), icon: "flatten", tone: "violet", disabled: none, onSelect: () => void exportFlat() },
    {
      label: t("menu.closeTab"),
      icon: "dismiss",
      shortcut: "Ctrl+W",
      separator: true,
      disabled: none,
      onSelect: () => void requestCloseTab(),
    },
    {
      label: t("menu.closeAll"),
      disabled: useWorkspace.getState().tabs.length === 0,
      onSelect: () => void requestCloseAll(),
    },
    {
      label: t("menu.about"),
      icon: "info",
      tone: "blue",
      separator: true,
      onSelect: () => useUi.getState().setAboutOpen(true),
    },
  ];
}

export function MenuBar(): JSX.Element {
  const ribbon = useUi((s) => s.ribbon);
  const canUndo = useDocument((s) => s.canUndo);
  const canRedo = useDocument((s) => s.canRedo);
  const searchOpen = useDocument((s) => s.search.open);
  const dirty = useDocument((s) => s.dirty);
  const saving = useDocument((s) => s.file.saving);
  const store = useDocument.getState;

  return (
    <div className="h-[36px] shrink-0 flex items-center gap-1 px-2 bg-[var(--izul-chrome)]">
      <MenuButton
        label={t("menu.file")}
        items={fileMenu()}
        className="h-7 px-2 flex items-center gap-1.5 rounded-[6px] text-[13px] hover:bg-[var(--izul-chrome-hover)]"
      >
        <Icon name="menu" size={16} />
        {t("menu.file")}
      </MenuButton>
      <span aria-hidden="true" className="w-px h-4 mx-1 bg-[var(--izul-border)]" />
      <IconButton icon="open" tone="amber" label={t("menu.open")} hint={`${t("menu.open")} (Ctrl+O)`} onClick={() => void pickAndOpen()} />
      <IconButton
        icon="save"
        tone="blue"
        label={t("menu.save")}
        hint={`${t("menu.save")} (Ctrl+S)`}
        disabled={!dirty || saving}
        onClick={() => void saveDocument()}
      />
      <IconButton
        icon="undo"
        label={t("annot.undo")}
        hint={`${t("annot.undo")} (Ctrl+Z)`}
        disabled={!canUndo}
        onClick={() => void store().undoAnnot()}
      />
      <IconButton
        icon="redo"
        label={t("annot.redo")}
        hint={`${t("annot.redo")} (Ctrl+Y)`}
        disabled={!canRedo}
        onClick={() => void store().redoAnnot()}
      />

      <div role="tablist" aria-label={t("ribbon.label")} className="flex-1 flex justify-center items-stretch h-full gap-1">
        {TABS.map((tab) => {
          const active = tab.id === ribbon;
          return (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={active}
              id={`ribbon-tab-${tab.id}`}
              aria-controls="ribbon-panel"
              onClick={() => useUi.getState().setRibbon(tab.id)}
              className={[
                "relative px-4 text-[13px] rounded-[6px] my-1",
                active ? "font-semibold text-[var(--izul-text)]" : "text-[var(--izul-text)] hover:bg-[var(--izul-chrome-hover)]",
              ].join(" ")}
            >
              {t(tab.label)}
              {active && (
                <span
                  aria-hidden="true"
                  className="absolute left-1/2 -translate-x-1/2 -bottom-1 w-6 h-[3px] rounded-full bg-[var(--izul-brand)]"
                />
              )}
            </button>
          );
        })}
      </div>

      <button
        type="button"
        aria-pressed={searchOpen}
        onClick={() => store().toggleSearch()}
        title={`${t("search.label")} (Ctrl+F)`}
        className={[
          "h-7 w-[180px] px-2 flex items-center gap-2 rounded-[6px] text-[13px] border",
          searchOpen
            ? "border-[var(--izul-accent)] bg-[var(--izul-surface)]"
            : "border-[var(--izul-border)] bg-[var(--izul-surface)] text-[var(--izul-text-dim)] hover:border-[var(--izul-text-dim)]",
        ].join(" ")}
      >
        <Icon name="search" size={16} />
        <span className="truncate">{t("search.trigger")}</span>
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Ribbon panels
// ---------------------------------------------------------------------------

/** Drag tools: arming one clears any armed markup, and pressing it again puts
 * it down. */
function toolProps(kind: AnnotKind, current: AnnotKind | null): { pressed: boolean; onClick: () => void } {
  return {
    pressed: current === kind,
    onClick: () => {
      useUi.getState().armMarkup(null);
      useDocument.getState().setTool(current === kind ? null : kind);
    },
  };
}

const SHAPES: ReadonlyArray<{ kind: AnnotKind; icon: IconName; label: StringKey }> = [
  { kind: "Line", icon: "line", label: "tool.line" },
  { kind: "Arrow", icon: "arrow", label: "tool.arrow" },
  { kind: "Rect", icon: "rect", label: "tool.rect" },
  { kind: "Ellipse", icon: "ellipse", label: "tool.ellipse" },
  { kind: "Polygon", icon: "polygon", label: "tool.polygon" },
];

function ShapesButton(props: { tool: AnnotKind | null }): JSX.Element {
  const current = SHAPES.find((s) => s.kind === props.tool);
  return (
    <RibbonButton
      icon={current?.icon ?? "shapes"}
      tone="green"
      label={t("ribbon.shapes")}
      pressed={current !== undefined}
      onClick={() => undefined}
      menu={SHAPES.map((s) => ({
        label: t(s.label),
        icon: s.icon,
        tone: "green" as Tone,
        checked: props.tool === s.kind,
        onSelect: () => {
          useUi.getState().armMarkup(null);
          useDocument.getState().setTool(s.kind);
        },
      }))}
    />
  );
}

const MARKUPS: ReadonlyArray<{ kind: MarkupKind; icon: IconName; tone: Tone; label: StringKey }> = [
  { kind: "Highlight", icon: "highlight", tone: "amber", label: "annot.highlight" },
  { kind: "Underline", icon: "underline", tone: "blue", label: "annot.underline" },
  { kind: "StrikeOut", icon: "strike", tone: "rose", label: "annot.strikeout" },
];

function MarkupButton(props: { kind: MarkupKind; armed: MarkupKind | null }): JSX.Element {
  const spec = MARKUPS.find((m) => m.kind === props.kind) ?? MARKUPS[0];
  if (!spec) throw new Error("jenis markup tidak dikenal");
  return (
    <RibbonButton
      icon={spec.icon}
      tone={spec.tone}
      label={t(spec.label)}
      hint={t("ribbon.markupHint")}
      pressed={props.armed === props.kind}
      onClick={() => void markupSelection(props.kind)}
    />
  );
}

function UndoGroup(props: { canUndo: boolean; canRedo: boolean; selection: number }): JSX.Element {
  const store = useDocument.getState;
  return (
    <>
      <RibbonStack>
        <RibbonSmall icon="undo" label={t("annot.undo")} hint={`${t("annot.undo")} (Ctrl+Z)`} disabled={!props.canUndo} onClick={() => void store().undoAnnot()} />
        <RibbonSmall icon="redo" label={t("annot.redo")} hint={`${t("annot.redo")} (Ctrl+Y)`} disabled={!props.canRedo} onClick={() => void store().redoAnnot()} />
      </RibbonStack>
      <RibbonButton
        icon="delete"
        tone="rose"
        label={t("annot.delete")}
        hint={`${t("annot.delete")} (Delete)`}
        disabled={props.selection === 0}
        onClick={() => void store().deleteSelected()}
      />
    </>
  );
}

const VIEW_MODES: ReadonlyArray<{ mode: ViewMode; icon: IconName; label: StringKey }> = [
  { mode: "single", icon: "viewSingle", label: "view.single" },
  { mode: "dual", icon: "viewDual", label: "view.dual" },
  { mode: "dual_cover", icon: "viewCover", label: "view.dualCover" },
  { mode: "horizontal", icon: "viewHorizontal", label: "view.horizontal" },
];

const LAYOUTS: ReadonlyArray<{ layout: Layout; icon: IconName; label: StringKey }> = [
  { layout: "single", icon: "viewSingle", label: "panels.single" },
  { layout: "columns", icon: "layoutTwo", label: "panels.columns" },
  { layout: "rows", icon: "layoutRows", label: "panels.rows" },
  { layout: "grid", icon: "layoutFour", label: "panels.grid" },
];

/** Split view and compare mode (Phase 5), one dropdown as WPS's "Jendela". */
function WindowButton(): JSX.Element {
  const layout = useWorkspace((s) => s.panels.layout);
  const compare = useWorkspace((s) => s.compare);
  const tabs = useWorkspace((s) => s.tabs.length);
  const current = LAYOUTS.find((l) => l.layout === layout);
  return (
    <RibbonButton
      icon={compare ? "compare" : (current?.icon ?? "windows")}
      tone="blue"
      label={t("ribbon.windows")}
      onClick={() => undefined}
      menu={[
        ...LAYOUTS.map((l) => ({
          label: t(l.label),
          icon: l.icon,
          tone: "blue" as Tone,
          checked: !compare && l.layout === layout,
          onSelect: () => {
            useWorkspace.getState().setCompare(false);
            useWorkspace.getState().setLayout(l.layout);
          },
        })),
        {
          label: t("compare.title"),
          icon: "compare" as IconName,
          tone: "rose" as Tone,
          separator: true,
          checked: compare,
          disabled: tabs < 2 && !compare,
          onSelect: toggleCompare,
        },
      ]}
    />
  );
}

function HomePanel(): JSX.Element {
  const tool = useDocument((s) => s.tool);
  const zoomMode = useDocument((s) => s.zoomMode);
  const viewMode = useDocument((s) => s.viewMode);
  const page = useDocument((s) => s.page);
  const markup = useUi((s) => s.markup);
  const store = useDocument.getState;
  const current = VIEW_MODES.find((v) => v.mode === viewMode);

  return (
    <>
      <RibbonButton
        icon="select"
        tone="neutral"
        label={t("tool.select")}
        pressed={tool === null && markup === null}
        onClick={() => {
          useUi.getState().armMarkup(null);
          store().setTool(null);
        }}
      />
      <RibbonButton icon="open" tone="amber" label={t("ribbon.open")} hint={`${t("menu.open")} (Ctrl+O)`} onClick={() => void pickAndOpen()} />
      <RibbonDivider />
      <MarkupButton kind="Highlight" armed={markup} />
      <RibbonButton icon="note" tone="amber" label={t("tool.note")} {...toolProps("Note", tool)} />
      <RibbonButton icon="textbox" tone="violet" label={t("tool.text")} {...toolProps("FreeText", tool)} />
      <RibbonDivider />
      <RibbonButton icon="search" tone="violet" label={t("ribbon.find")} hint={`${t("search.label")} (Ctrl+F)`} onClick={() => store().toggleSearch(true)} />
      <RibbonDivider />
      <RibbonStack>
        <RibbonSmall icon="fitWidth" tone="blue" label={t("zoom.fitWidthLong")} pressed={zoomMode === "fitWidth"} onClick={() => store().setZoomMode("fitWidth")} />
        <RibbonSmall icon="fitPage" tone="blue" label={t("zoom.fitPageLong")} pressed={zoomMode === "fitPage"} onClick={() => store().setZoomMode("fitPage")} />
      </RibbonStack>
      <RibbonStack>
        <RibbonSmall icon="actualSize" tone="blue" label={t("zoom.actualLong")} hint="Ctrl+0" pressed={zoomMode === "actual"} onClick={() => store().setZoomMode("actual")} />
        <div className="flex">
          <RibbonSmall icon="zoomOut" tone="blue" label={t("zoom.out")} hint={`${t("zoom.out")} (Ctrl+−)`} hideLabel onClick={() => store().zoomOut()} />
          <RibbonSmall icon="zoomIn" tone="blue" label={t("zoom.in")} hint={`${t("zoom.in")} (Ctrl++)`} hideLabel onClick={() => store().zoomIn()} />
        </div>
      </RibbonStack>
      <RibbonDivider />
      <RibbonStack>
        <RibbonSmall icon="rotateLeft" tone="teal" label={t("rotate.left")} onClick={() => store().rotateDocument(-1)} />
        <RibbonSmall icon="rotateRight" tone="teal" label={t("rotate.right")} onClick={() => store().rotateDocument(1)} />
      </RibbonStack>
      <RibbonButton icon="rotatePage" tone="teal" label={t("rotate.page")} onClick={() => store().rotatePage(page, 1)} />
      <RibbonDivider />
      <RibbonButton
        icon={current?.icon ?? "viewSingle"}
        tone="blue"
        label={t("ribbon.view")}
        onClick={() => undefined}
        menu={VIEW_MODES.map((v) => ({
          label: t(v.label),
          icon: v.icon,
          tone: "blue" as Tone,
          checked: v.mode === viewMode,
          onSelect: () => store().setViewMode(v.mode),
        }))}
      />
      <WindowButton />
    </>
  );
}

function EditPanel(): JSX.Element {
  const tool = useDocument((s) => s.tool);
  const markup = useUi((s) => s.markup);
  const canUndo = useDocument((s) => s.canUndo);
  const canRedo = useDocument((s) => s.canRedo);
  const selection = useDocument((s) => s.selection.length);
  return (
    <>
      <RibbonButton
        icon="select"
        tone="neutral"
        label={t("tool.select")}
        pressed={tool === null && markup === null}
        onClick={() => {
          useUi.getState().armMarkup(null);
          useDocument.getState().setTool(null);
        }}
      />
      <RibbonDivider />
      <RibbonButton icon="textAdd" tone="violet" label={t("ribbon.addText")} {...toolProps("FreeText", tool)} />
      <RibbonButton icon="imageAdd" tone="teal" label={t("ribbon.addImage")} onClick={() => void insertImage()} />
      <ShapesButton tool={tool} />
      <RibbonButton icon="ink" tone="green" label={t("tool.ink")} {...toolProps("Ink", tool)} />
      <RibbonButton icon="stamp" tone="orange" label={t("tool.stamp")} {...toolProps("Stamp", tool)} />
      <RibbonDivider />
      <UndoGroup canUndo={canUndo} canRedo={canRedo} selection={selection} />
    </>
  );
}

function CommentPanel(): JSX.Element {
  const tool = useDocument((s) => s.tool);
  const markup = useUi((s) => s.markup);
  const canUndo = useDocument((s) => s.canUndo);
  const canRedo = useDocument((s) => s.canRedo);
  const selection = useDocument((s) => s.selection.length);
  const listOpen = useDocument((s) => s.sidebarOpen && s.sidebarTab === "annots");
  return (
    <>
      {MARKUPS.map((m) => (
        <MarkupButton key={m.kind} kind={m.kind} armed={markup} />
      ))}
      <RibbonDivider />
      <RibbonButton icon="note" tone="amber" label={t("tool.note")} {...toolProps("Note", tool)} />
      <RibbonButton icon="textbox" tone="violet" label={t("tool.text")} {...toolProps("FreeText", tool)} />
      <RibbonButton icon="ink" tone="green" label={t("tool.ink")} {...toolProps("Ink", tool)} />
      <ShapesButton tool={tool} />
      <RibbonButton icon="stamp" tone="orange" label={t("tool.stamp")} {...toolProps("Stamp", tool)} />
      <RibbonDivider />
      <RibbonButton
        icon="comments"
        tone="amber"
        label={t("ribbon.annotList")}
        pressed={listOpen}
        onClick={() => {
          const store = useDocument.getState();
          if (listOpen) store.toggleSidebar();
          else store.setSidebarTab("annots");
        }}
      />
      <RibbonDivider />
      <UndoGroup canUndo={canUndo} canRedo={canRedo} selection={selection} />
    </>
  );
}

/**
 * "Halaman" (Phase 5): the page operations of SPEC 11.3. Each acts on the
 * pages selected in the page panel, or on the page being read; each is one
 * undo step. Opening the page panel is part of the tab, because selecting
 * several pages and dragging them is done there.
 */
function PagesPanel(): JSX.Element {
  const canUndo = useDocument((s) => s.canUndo);
  const canRedo = useDocument((s) => s.canRedo);
  const selected = useDocument((s) => s.pageSelection.length);
  const pageCount = useDocument((s) => s.pageCount);
  const panelOpen = useDocument((s) => s.sidebarOpen && s.sidebarTab === "thumbnails");
  const store = useDocument.getState;
  const count = selected > 0 ? ` (${selected})` : "";
  return (
    <>
      <RibbonButton
        icon="thumbnails"
        tone="blue"
        label={t("ribbon.pagePanel")}
        pressed={panelOpen}
        onClick={() => {
          if (panelOpen) store().toggleSidebar();
          else store().setSidebarTab("thumbnails");
        }}
      />
      <RibbonDivider />
      <RibbonButton icon="pageBlank" tone="blue" label={t("pages.insertBlank")} hint={t("pages.insertBlankHint")} onClick={() => void insertBlankPage()} />
      <RibbonButton icon="merge" tone="teal" label={t("pages.merge")} hint={t("pages.mergeHint")} onClick={() => void mergeFile()} />
      <RibbonButton
        icon="pageDelete"
        tone="rose"
        label={`${t("pages.delete")}${count}`}
        hint={t("pages.deleteHint")}
        disabled={pageCount <= 1 || selected >= pageCount}
        onClick={() => void deletePages()}
      />
      <RibbonButton icon="pageDuplicate" tone="violet" label={`${t("pages.duplicate")}${count}`} onClick={() => void duplicatePages()} />
      <RibbonDivider />
      <RibbonStack>
        <RibbonSmall icon="rotateLeft" tone="teal" label={t("rotate.left")} onClick={() => void rotatePages(-1)} />
        <RibbonSmall icon="rotateRight" tone="teal" label={t("rotate.right")} onClick={() => void rotatePages(1)} />
      </RibbonStack>
      <RibbonDivider />
      <RibbonButton icon="extractPages" tone="blue" label={t("pages.extract")} hint={t("convert.pagesHint")} onClick={extractPages} />
      <RibbonButton icon="split" tone="orange" label={t("pages.split")} hint={t("pages.splitHint")} onClick={splitDocument} />
      <RibbonDivider />
      <RibbonStack>
        <RibbonSmall icon="undo" label={t("annot.undo")} hint={`${t("annot.undo")} (Ctrl+Z)`} disabled={!canUndo} onClick={() => void store().undoAnnot()} />
        <RibbonSmall icon="redo" label={t("annot.redo")} hint={`${t("annot.redo")} (Ctrl+Y)`} disabled={!canRedo} onClick={() => void store().redoAnnot()} />
      </RibbonStack>
    </>
  );
}

/**
 * "Konversi": the three conversions that need nothing but the PDF engine.
 * WPS's converters to Word, Excel and PowerPoint are absent — they are not
 * something PDFium can do, and SPEC 2 rules out sending the file anywhere
 * that could.
 */
function ConvertPanel(): JSX.Element {
  const ui = useUi.getState;
  return (
    <>
      <RibbonButton
        icon="toImages"
        tone="teal"
        label={t("convert.toImages")}
        hint={t("convert.toImagesHint")}
        onClick={() => ui().setExporting("images")}
      />
      <RibbonDivider />
      <RibbonButton
        icon="extractPages"
        tone="blue"
        label={t("convert.pages")}
        hint={t("convert.pagesHint")}
        onClick={() => ui().setExporting("pages")}
      />
      <RibbonButton icon="flatten" tone="violet" label={t("convert.flat")} hint={t("convert.flatHint")} onClick={() => void exportFlat()} />
      <RibbonDivider />
      <RibbonButton icon="saveAs" tone="blue" label={t("menu.saveAs")} hint={`${t("menu.saveAs")} (Ctrl+Shift+S)`} onClick={() => void saveDocument(undefined, "saveAs")} />
    </>
  );
}

export function Ribbon(): JSX.Element {
  const ribbon = useUi((s) => s.ribbon);
  const error = useDocument((s) => s.annotError);
  const markup = useUi((s) => s.markup);
  return (
    <div className="shrink-0 px-2 pb-1.5 bg-[var(--izul-chrome)]">
      <div
        id="ribbon-panel"
        role="tabpanel"
        aria-labelledby={`ribbon-tab-${ribbon}`}
        className="h-[70px] flex items-center gap-0.5 px-2 rounded-[8px] bg-[var(--izul-surface)] border border-[var(--izul-border)] overflow-x-auto overflow-y-hidden"
      >
        {ribbon === "home" ? (
          <HomePanel />
        ) : ribbon === "edit" ? (
          <EditPanel />
        ) : ribbon === "pages" ? (
          <PagesPanel />
        ) : ribbon === "comment" ? (
          <CommentPanel />
        ) : (
          <ConvertPanel />
        )}
      </div>
      {(error !== null || markup !== null) && (
        <p
          role={error !== null ? "alert" : "status"}
          className={[
            "px-2 pt-1 text-[12px]",
            error !== null ? "text-[var(--izul-danger)]" : "text-[var(--izul-text-dim)]",
          ].join(" ")}
        >
          {error ?? t("ribbon.markupArmed")}
        </p>
      )}
    </div>
  );
}
