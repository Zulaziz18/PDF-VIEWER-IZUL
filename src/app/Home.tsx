/**
 * The home screen ("Beranda"), laid out after WPS Office's start page
 * (SPEC 12, revised 2026-09-23): navigation on the left, a table of files in
 * the middle, and the selected file's details on the right.
 *
 * Only local places appear in the navigation. WPS's cloud drives, account
 * storage and promotional cards are absent on purpose — SPEC 2 rules out
 * anything that touches the network, and a banner that sells something is not
 * something this application has to sell.
 *
 * A cover exists only for a file this application has rendered before. Making
 * one on demand would mean opening every file in the list in a worker, which is
 * precisely the cost a recent-files panel must not have.
 */

import { useEffect, useMemo, type JSX } from "react";
import { Icon, type IconName, type Tone } from "@/design/Icon";
import { IconButton } from "@/design/controls";
import { FileBadge } from "@/design/FileBadge";
import { t, type StringKey } from "@/i18n";
import { useHome, type ExportRecord, type HomeView } from "@/state/homeStore";
import {
  baseName,
  breakablePath,
  displayFolder,
  formatBytes,
  formatDate,
  formatDateTime,
  frequentFolders,
  groupRecent,
  parentFolder,
  type FolderEntry,
  type KnownFolder,
  type RecentFile,
} from "@/state/homeModel";
import { useWorkspace } from "@/state/workspaceStore";
import { pickAndOpen } from "./actions";

const KNOWN_ICON: Record<KnownFolder["kind"], { icon: IconName; tone: Tone; label: StringKey }> = {
  desktop: { icon: "desktop", tone: "teal", label: "home.desktop" },
  documents: { icon: "documents", tone: "blue", label: "home.documents" },
  downloads: { icon: "download", tone: "green", label: "home.downloads" },
  drive: { icon: "drive", tone: "neutral", label: "home.drive" },
};

function sameView(a: HomeView, b: HomeView): boolean {
  return a.kind === b.kind && (a.kind !== "folder" || (b.kind === "folder" && a.path === b.path));
}

function NavItem(props: {
  icon: IconName;
  tone: Tone;
  label: string;
  view: HomeView;
  current: HomeView;
  title?: string;
}): JSX.Element {
  const active = sameView(props.view, props.current);
  return (
    <button
      type="button"
      aria-current={active ? "page" : undefined}
      title={props.title ?? props.label}
      onClick={() => void useHome.getState().navigate(props.view)}
      className={[
        "w-full h-9 px-3 flex items-center gap-2.5 rounded-[8px] text-[13px] text-left",
        active
          ? "bg-[var(--izul-accent-soft)] text-[var(--izul-accent)] font-medium"
          : "hover:bg-[var(--izul-chrome-hover)]",
      ].join(" ")}
    >
      <Icon name={props.icon} size={20} tone={props.tone} />
      <span className="truncate">{props.label}</span>
    </button>
  );
}

function Navigation(props: { view: HomeView; recent: readonly RecentFile[]; known: readonly KnownFolder[] }): JSX.Element {
  const frequent = useMemo(
    () => frequentFolders(props.recent, 4, props.known.map((k) => k.path)),
    [props.recent, props.known],
  );
  const local = props.known.filter((k) => k.kind !== "drive");
  return (
    <nav aria-label={t("home.navigation")} className="w-[228px] shrink-0 flex flex-col gap-0.5 p-2 overflow-y-auto">
      <button
        type="button"
        onClick={() => void pickAndOpen()}
        className="mb-3 h-10 rounded-[8px] bg-[var(--izul-accent)] text-[var(--izul-on-accent)] font-medium flex items-center justify-center gap-2 hover:brightness-110"
      >
        <Icon name="open" size={20} />
        {t("home.open")}
      </button>
      <NavItem icon="recent" tone="blue" label={t("home.recent")} view={{ kind: "recent" }} current={props.view} />
      <NavItem icon="star" tone="amber" label={t("home.starred")} view={{ kind: "starred" }} current={props.view} />
      <NavItem icon="exportFile" tone="teal" label={t("home.exports")} view={{ kind: "exports" }} current={props.view} />

      <h2 className="mt-4 mb-1 px-3 text-[12px] font-semibold text-[var(--izul-text-dim)]">{t("home.local")}</h2>
      <NavItem icon="pc" tone="neutral" label={t("home.thisPc")} view={{ kind: "pc" }} current={props.view} />
      {local.map((k) => (
        <NavItem
          key={k.path}
          icon={KNOWN_ICON[k.kind].icon}
          tone={KNOWN_ICON[k.kind].tone}
          label={t(KNOWN_ICON[k.kind].label)}
          title={k.path}
          view={{ kind: "folder", path: k.path }}
          current={props.view}
        />
      ))}

      {frequent.length > 0 && (
        <>
          <h2 className="mt-4 mb-1 px-3 text-[12px] font-semibold text-[var(--izul-text-dim)]">
            {t("home.frequent")}
          </h2>
          {frequent.map((folder) => (
            <NavItem
              key={folder}
              icon="folder"
              tone="amber"
              label={baseName(folder)}
              title={folder}
              view={{ kind: "folder", path: folder }}
              current={props.view}
            />
          ))}
        </>
      )}
    </nav>
  );
}

/** One row of the table, for a recent file or a folder entry alike. */
interface Row {
  readonly path: string;
  readonly name: string;
  readonly isDir: boolean;
  readonly folder: string;
  readonly modified: number | null;
  readonly size: number | null;
  readonly pinned: boolean;
  readonly available: boolean;
  readonly icon?: IconName;
  readonly tone?: Tone;
}

function openRow(row: Row): void {
  if (row.isDir) {
    void useHome.getState().navigate({ kind: "folder", path: row.path });
    return;
  }
  // An exported image is listed so it can be found again, not opened here.
  if (!row.available || !/\.pdf$/i.test(row.path)) return;
  void useWorkspace.getState().openFile(row.path);
}

function FileRow(props: { row: Row; selected: boolean; pinnable: boolean }): JSX.Element {
  const { row } = props;
  return (
    <tr
      tabIndex={0}
      aria-selected={props.selected}
      onClick={() => useHome.getState().select(row.path)}
      onDoubleClick={() => openRow(row)}
      onKeyDown={(e) => {
        if (e.key === "Enter") openRow(row);
      }}
      title={row.available ? row.path : `${row.path} — ${t("empty.missing")}`}
      className={[
        "group h-11 cursor-default outline-none",
        props.selected
          ? "bg-[var(--izul-accent-soft)]"
          : "hover:bg-[var(--izul-surface-raised)] focus-visible:bg-[var(--izul-surface-raised)]",
        row.available ? "" : "opacity-50",
      ].join(" ")}
    >
      <td className="pl-4 pr-2">
        <div className="flex items-center gap-3 min-w-0">
          {row.isDir || row.icon ? (
            <Icon name={row.icon ?? "folder"} size={24} tone={row.tone ?? "amber"} />
          ) : (
            <FileBadge size={26} />
          )}
          <span className="truncate text-[13px]">{row.name}</span>
          {props.pinnable && (
            <button
              type="button"
              aria-label={row.pinned ? t("empty.unpin") : t("empty.pin")}
              aria-pressed={row.pinned}
              onClick={(e) => {
                e.stopPropagation();
                void useHome.getState().togglePin(row.path);
              }}
              className={[
                "shrink-0 w-6 h-6 grid place-items-center rounded-[6px] hover:bg-[var(--izul-chrome-hover)]",
                row.pinned ? "" : "opacity-0 group-hover:opacity-100 focus:opacity-100",
              ].join(" ")}
            >
              <Icon name="star" size={16} tone={row.pinned ? "amber" : "plain"} />
            </button>
          )}
        </div>
      </td>
      <td className="px-2 text-[13px] text-[var(--izul-text-dim)] truncate max-w-0">{row.folder}</td>
      <td className="px-2 text-[13px] text-[var(--izul-text-dim)] tabular-nums whitespace-nowrap">
        {formatDate(row.modified)}
      </td>
      <td className="pl-2 pr-4 text-[13px] text-[var(--izul-text-dim)] tabular-nums text-right whitespace-nowrap">
        {formatBytes(row.size)}
      </td>
    </tr>
  );
}

function GroupRow(props: { label: string }): JSX.Element {
  return (
    <tr>
      <th colSpan={4} scope="colgroup" className="pt-4 pb-1.5 pl-4 text-left text-[13px] font-semibold">
        {props.label}
      </th>
    </tr>
  );
}

function recentRow(f: RecentFile, home: string | null): Row {
  return {
    path: f.path,
    name: f.name,
    isDir: false,
    folder: displayFolder(f.path, home),
    modified: f.modified,
    size: f.size,
    pinned: f.pinned,
    available: f.available,
  };
}

function entryRow(e: FolderEntry, pinned: ReadonlySet<string>): Row {
  return {
    path: e.path,
    name: e.name,
    isDir: e.is_dir,
    folder: parentFolder(e.path),
    modified: e.modified,
    size: e.size,
    pinned: pinned.has(e.path),
    available: true,
  };
}

const EXPORT_KIND: Record<string, { icon: IconName; tone: Tone; label: StringKey }> = {
  flat: { icon: "flatten", tone: "violet", label: "convert.flat" },
  pages: { icon: "extractPages", tone: "blue", label: "convert.pages" },
  png: { icon: "toImages", tone: "teal", label: "export.png" },
  jpg: { icon: "toImages", tone: "teal", label: "export.jpg" },
  split: { icon: "split", tone: "orange", label: "pages.split" },
};

function exportRow(e: ExportRecord, home: string | null): Row {
  const kind = EXPORT_KIND[e.kind];
  return {
    path: e.out_path,
    name: baseName(e.out_path),
    isDir: false,
    folder: displayFolder(e.out_path, home),
    modified: e.created_at,
    size: e.size,
    pinned: false,
    available: e.exists,
    ...(kind && kind.icon !== "extractPages" && kind.icon !== "flatten" ? { icon: kind.icon, tone: kind.tone } : {}),
  };
}

function Details(props: {
  file: RecentFile | null;
  entry: FolderEntry | null;
  exported: ExportRecord | null;
  home: string | null;
}): JSX.Element {
  const { file, entry, exported } = props;
  if (!file && !entry) {
    return (
      <div className="h-full flex flex-col items-center justify-center gap-3 p-6 text-center text-[var(--izul-text-dim)]">
        <Icon name="info" size={28} tone="blue" />
        <p className="text-[13px]">{t("home.selectHint")}</p>
      </div>
    );
  }
  const name = file?.name ?? entry?.name ?? "";
  const path = file?.path ?? entry?.path ?? "";
  const size = file?.size ?? entry?.size ?? null;
  const modified = file?.modified ?? entry?.modified ?? null;
  const facts: Array<[StringKey, string]> = [
    ["home.col.location", parentFolder(path)],
    ["home.col.size", formatBytes(size)],
    [exported ? "home.col.exported" : "home.col.modified", formatDateTime(modified)],
  ];
  if (file?.last_opened) facts.push(["home.lastOpened", formatDateTime(file.last_opened)]);
  if (exported) {
    const kind = EXPORT_KIND[exported.kind];
    if (kind) facts.push(["home.exportKind", t(kind.label)]);
    facts.push(["home.exportSource", exported.source]);
  }
  const available = file ? file.available : exported ? exported.exists : true;
  // Only a PDF opens here; an exported picture is listed so it can be found.
  const openable = available && /\.pdf$/i.test(path);
  return (
    <div className="h-full flex flex-col gap-4 p-4 overflow-y-auto">
      <div className="aspect-[4/3] rounded-[8px] bg-[var(--izul-canvas)] border border-[var(--izul-border)] grid place-items-center overflow-hidden">
        {file?.cover ? (
          <img src={file.cover} alt="" className="max-w-full max-h-full object-contain shadow-sm" />
        ) : (
          <FileBadge size={40} />
        )}
      </div>
      <h3 className="text-[15px] font-semibold break-words">{name}</h3>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-2 text-[13px]">
        {facts.map(([key, value]) => (
          <div key={key} className="contents">
            <dt className="text-[var(--izul-text-dim)]">{t(key)}</dt>
            <dd className="m-0 break-words">{breakablePath(value)}</dd>
          </div>
        ))}
      </dl>
      {!available && <p className="text-[12px] text-[var(--izul-danger)]">{t("empty.missing")}</p>}
      <div className="flex gap-2">
        <button
          type="button"
          disabled={!openable}
          onClick={() => void useWorkspace.getState().openFile(path)}
          className="flex-1 h-9 rounded-[8px] bg-[var(--izul-accent)] text-[var(--izul-on-accent)] font-medium disabled:opacity-40"
        >
          {t("home.openFile")}
        </button>
        {file && (
          <button
            type="button"
            aria-pressed={file.pinned}
            onClick={() => void useHome.getState().togglePin(file.path)}
            className="h-9 px-3 rounded-[8px] border border-[var(--izul-border)] flex items-center gap-1.5 hover:bg-[var(--izul-surface-raised)]"
          >
            <Icon name="star" size={16} tone={file.pinned ? "amber" : "plain"} />
            {file.pinned ? t("empty.unpin") : t("empty.pin")}
          </button>
        )}
      </div>
    </div>
  );
}

export function Home(props: { dropping: boolean }): JSX.Element {
  const view = useHome((s) => s.view);
  const recent = useHome((s) => s.recent);
  const exports = useHome((s) => s.exports);
  const known = useHome((s) => s.known);
  const entries = useHome((s) => s.entries);
  const selected = useHome((s) => s.selected);
  const loading = useHome((s) => s.loading);
  const error = useHome((s) => s.error);

  useEffect(() => {
    void useHome.getState().load();
  }, []);

  // The user's home folder, for shortening the location column: the parent of
  // the Desktop the platform reports.
  const homeDir = useMemo(() => {
    const desktop = known.find((k) => k.kind === "desktop");
    return desktop ? parentFolder(desktop.path) : null;
  }, [known]);
  const pinned = useMemo(() => new Set(recent.filter((f) => f.pinned).map((f) => f.path)), [recent]);

  let title: string;
  let body: JSX.Element;
  if (view.kind === "recent" || view.kind === "starred") {
    const files = view.kind === "starred" ? recent.filter((f) => f.pinned) : recent;
    title = view.kind === "starred" ? t("home.starred") : t("home.recent");
    const groups = view.kind === "recent" ? groupRecent(files, Date.now()) : [{ key: "all" as const, files }];
    body =
      files.length === 0 ? (
        <Empty text={view.kind === "starred" ? t("home.noStarred") : t("empty.noRecent")} />
      ) : (
        <Table>
          {groups.map((g) => (
            <GroupBlock key={g.key} label={g.key === "all" ? null : t(g.key === "last30" ? "home.last30" : "home.earlier")}>
              {g.files.map((f) => (
                <FileRow key={f.path} row={recentRow(f, homeDir)} selected={selected === f.path} pinnable />
              ))}
            </GroupBlock>
          ))}
        </Table>
      );
  } else if (view.kind === "exports") {
    title = t("home.exports");
    body =
      exports.length === 0 ? (
        <Empty text={t("home.noExports")} />
      ) : (
        <Table dateLabel="home.col.exported">
          {groupRecent(
            exports.map((e) => ({ ...e, last_opened: e.created_at })),
            Date.now(),
          ).map((g) => (
            <GroupBlock key={g.key} label={t(g.key === "last30" ? "home.last30" : "home.earlier")}>
              {g.files.map((e) => (
                <FileRow key={e.out_path} row={exportRow(e, homeDir)} selected={selected === e.out_path} pinnable={false} />
              ))}
            </GroupBlock>
          ))}
        </Table>
      );
  } else if (view.kind === "pc") {
    title = t("home.thisPc");
    const rows: Row[] = known.map((k) => ({
      path: k.path,
      name: k.kind === "drive" ? `${t("home.drive")} (${k.path.replace(/\\$/, "")})` : t(KNOWN_ICON[k.kind].label),
      isDir: true,
      folder: k.kind === "drive" ? "" : parentFolder(k.path),
      modified: null,
      size: null,
      pinned: false,
      available: true,
      icon: KNOWN_ICON[k.kind].icon,
      tone: KNOWN_ICON[k.kind].tone,
    }));
    body = (
      <Table>
        <GroupBlock label={null}>
          {rows.map((r) => (
            <FileRow key={r.path} row={r} selected={selected === r.path} pinnable={false} />
          ))}
        </GroupBlock>
      </Table>
    );
  } else {
    title = baseName(view.path);
    body = error ? (
      <Empty text={error} />
    ) : loading ? (
      <Empty text={t("home.loading")} />
    ) : entries.length === 0 ? (
      <Empty text={t("home.emptyFolder")} />
    ) : (
      <Table>
        <GroupBlock label={null}>
          {entries.map((e) => (
            <FileRow key={e.path} row={entryRow(e, pinned)} selected={selected === e.path} pinnable={false} />
          ))}
        </GroupBlock>
      </Table>
    );
  }

  const selectedFile = recent.find((f) => f.path === selected) ?? null;
  const exported = (view.kind === "exports" ? exports.find((e) => e.out_path === selected) : undefined) ?? null;
  const selectedEntry = selectedFile
    ? null
    : exported
      ? { name: baseName(exported.out_path), path: exported.out_path, is_dir: false, size: exported.size, modified: exported.created_at }
      : (entries.find((e) => e.path === selected && !e.is_dir) ?? null);
  const parent = view.kind === "folder" ? parentFolder(view.path) : null;

  return (
    <div className="flex-1 min-h-0 flex bg-[var(--izul-chrome)]">
      <Navigation view={view} recent={recent} known={known} />
      <main
        className={[
          "flex-1 min-w-0 flex my-2 mr-2 rounded-[12px] bg-[var(--izul-surface)] border border-[var(--izul-border)] overflow-hidden",
          props.dropping ? "outline outline-2 -outline-offset-2 outline-[var(--izul-accent)]" : "",
        ].join(" ")}
      >
        <section className="flex-1 min-w-0 flex flex-col">
          <div className="px-6 pt-5 pb-4 border-b border-[var(--izul-border)]">
            <h1 className="text-[20px] font-semibold">{t("home.welcome")}</h1>
            <p className="mt-1 text-[13px] text-[var(--izul-text-dim)]">{t("empty.subtitle")}</p>
          </div>
          <header className="h-14 shrink-0 px-6 flex items-center gap-2">
            {parent !== null && parent !== (view.kind === "folder" ? view.path : "") && (
              <IconButton
                icon="previous"
                label={t("home.up")}
                onClick={() => void useHome.getState().navigate({ kind: "folder", path: parent })}
              />
            )}
            <h2 className="text-[17px] font-semibold truncate" title={view.kind === "folder" ? view.path : undefined}>
              {title}
            </h2>
            <IconButton icon="refresh" label={t("home.refresh")} onClick={() => void refresh(view)} />
          </header>
          <div className="flex-1 min-h-0 overflow-y-auto">{body}</div>
        </section>
        <aside
          aria-label={t("home.fileInfo")}
          className="w-[280px] shrink-0 border-l border-[var(--izul-border)] flex flex-col"
        >
          <h2 className="h-14 shrink-0 px-4 flex items-center text-[15px] font-semibold">{t("home.fileInfo")}</h2>
          <div className="flex-1 min-h-0">
            <Details file={selectedFile} entry={selectedEntry} exported={exported} home={homeDir} />
          </div>
        </aside>
      </main>
    </div>
  );
}

async function refresh(view: HomeView): Promise<void> {
  await useHome.getState().load();
  if (view.kind === "folder") await useHome.getState().navigate(view);
}

function Table(props: { children: React.ReactNode; dateLabel?: StringKey }): JSX.Element {
  return (
    <table className="w-full table-fixed border-collapse">
      <colgroup>
        <col />
        <col className="w-[30%]" />
        <col className="w-[132px]" />
        <col className="w-[96px]" />
      </colgroup>
      <thead className="sticky top-0 z-10 bg-[var(--izul-surface)]">
        <tr className="h-9 text-left text-[12px] text-[var(--izul-text-dim)]">
          <th scope="col" className="pl-4 pr-2 font-normal">{t("home.col.name")}</th>
          <th scope="col" className="px-2 font-normal">{t("home.col.location")}</th>
          <th scope="col" className="px-2 font-normal">{t(props.dateLabel ?? "home.col.modified")}</th>
          <th scope="col" className="pl-2 pr-4 font-normal text-right">{t("home.col.size")}</th>
        </tr>
      </thead>
      {props.children}
    </table>
  );
}

function GroupBlock(props: { label: string | null; children: React.ReactNode }): JSX.Element {
  return (
    <tbody>
      {props.label !== null && <GroupRow label={props.label} />}
      {props.children}
    </tbody>
  );
}

function Empty(props: { text: string }): JSX.Element {
  return <p className="px-6 py-8 text-[13px] text-[var(--izul-text-dim)]">{props.text}</p>;
}
