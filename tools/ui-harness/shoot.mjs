#!/usr/bin/env node
/**
 * UI screenshot harness: `npm run ui:shots`.
 *
 * The development container has no display, and a user interface nobody has
 * looked at cannot be made to resemble anything. This runs the real frontend in
 * a real Chromium with Tauri's IPC mocked (`mock.ts`), feeds it pages rendered
 * by the application's own PDFium (`izul-bench --bin ui-harness`), and writes a
 * screenshot per scene, size and theme to `docs/ui/`.
 *
 *   npm run ui:shots                         # every scene, 1366x768 + 1920x1080, light + dark
 *   npm run ui:shots -- --scene=home         # one scene
 *   npm run ui:shots -- --scale=1.5          # Windows display scaling (device pixel ratio)
 *   npm run ui:shots -- --out=/tmp/shots     # somewhere other than docs/ui
 *   npm run ui:shots -- --serve              # leave the harness running for a human
 *
 * Nothing here is part of the application: the mock lives in
 * `tools/ui-harness/`, which the production build never references.
 */

import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";
import { chromium } from "playwright-core";
import { createServer } from "vite";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "../..");
const SAMPLES = join(HERE, ".samples");
const args = Object.fromEntries(
  process.argv.slice(2).map((a) => {
    const [k, v] = a.replace(/^--/, "").split("=");
    return [k, v ?? "true"];
  }),
);
const OUT = resolve(ROOT, args.out ?? "docs/ui");
const PORT = Number(args.port ?? 5199);

// ---------------------------------------------------------------------------
// Prerequisites
// ---------------------------------------------------------------------------

function run(cmd, argv) {
  const r = spawnSync(cmd, argv, { cwd: ROOT, stdio: "inherit", shell: process.platform === "win32" });
  if (r.status !== 0) throw new Error(`${cmd} ${argv.join(" ")} gagal`);
}

// Made again whenever one is missing — a phase that adds a sample adds it here.
if (["Panduan Studi 2026.pdf", "Data Pegawai.pdf", "stempel.png", "Formulir Pendaftaran.pdf"].some((f) => !existsSync(join(SAMPLES, f)))) {
  run(process.platform === "win32" ? "python" : "python3", ["tools/ui-harness/make_samples.py"]);
}
run("cargo", ["build", "-q", "-p", "izul-bench", "--bin", "ui-harness"]);

// ---------------------------------------------------------------------------
// The PDFium side: one long-lived process, one request at a time.
// ---------------------------------------------------------------------------

const exe = join(ROOT, "target/debug", process.platform === "win32" ? "ui-harness.exe" : "ui-harness");
const helper = spawn(exe, [], { cwd: ROOT, stdio: ["pipe", "pipe", "inherit"] });
let buffered = Buffer.alloc(0);
const waiting = [];
helper.stdout.on("data", (chunk) => {
  buffered = Buffer.concat([buffered, chunk]);
  for (;;) {
    const head = waiting[0];
    if (!head) return;
    const nl = buffered.indexOf(10);
    if (nl < 0) return;
    const header = JSON.parse(buffered.subarray(0, nl).toString("utf8"));
    if (buffered.length < nl + 1 + header.len) return;
    const body = buffered.subarray(nl + 1, nl + 1 + header.len);
    buffered = buffered.subarray(nl + 1 + header.len);
    waiting.shift();
    head.resolve({ header, body: Buffer.from(body) });
  }
});
let chain = Promise.resolve();
function ask(req) {
  const p = chain.then(
    () =>
      new Promise((res) => {
        waiting.push({ resolve: res });
        helper.stdin.write(JSON.stringify(req) + "\n");
      }),
  );
  chain = p.then(() => undefined);
  return p;
}

// ---------------------------------------------------------------------------
// Sample data
// ---------------------------------------------------------------------------

const DAY = 86_400_000;
const NOW = Date.parse("2026-09-23T10:00:00+07:00");
const USER = "C:\\Users\\contoh";
const docsFolder = `${USER}\\Documents`;
const SAMPLE_FILES = [
  { name: "Panduan Studi 2026.pdf", folder: docsFolder, opened: NOW - 2 * 3600_000, modified: NOW - 3 * DAY },
  { name: "Laporan Kegiatan Semester.pdf", folder: `${USER}\\Desktop`, opened: NOW - 1 * DAY, modified: NOW - 9 * DAY },
  { name: "Proposal Penelitian.pdf", folder: `${docsFolder}\\Kuliah`, opened: NOW - 6 * DAY, modified: NOW - 40 * DAY },
  { name: "Catatan Rapat Organisasi.pdf", folder: `${USER}\\Downloads`, opened: NOW - 45 * DAY, modified: NOW - 45 * DAY },
  // Phase 5: two versions of one document, for compare mode.
  { name: "Draf Perjanjian v1.pdf", folder: docsFolder, opened: NOW - 70 * DAY, modified: NOW - 70 * DAY },
  { name: "Draf Perjanjian v2.pdf", folder: docsFolder, opened: NOW - 69 * DAY, modified: NOW - 69 * DAY },
  // Phase 6: a page to redact, and the same page redacted by the real pipeline.
  { name: "Data Pegawai.pdf", folder: docsFolder, opened: NOW - 80 * DAY, modified: NOW - 80 * DAY },
  { name: "Data Pegawai (diredaksi).pdf", folder: docsFolder, opened: NOW - 81 * DAY, modified: NOW - 81 * DAY },
  // Phase 7: a form to fill.
  { name: "Formulir Pendaftaran.pdf", folder: `${USER}\\Downloads`, opened: NOW - 90 * DAY, modified: NOW - 90 * DAY },
];

const realPath = new Map();
const docs = [];
const recent = [];

function crc32(buf) {
  let c;
  const table = (crc32.t ??= Array.from({ length: 256 }, (_, n) => {
    c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  }));
  let crc = 0xffffffff;
  for (const b of buf) crc = table[(crc ^ b) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function png(width, height, stride, bgra) {
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (width * 4 + 1)] = 0;
    for (let x = 0; x < width; x++) {
      const s = y * stride + x * 4;
      const d = y * (width * 4 + 1) + 1 + x * 4;
      raw[d] = bgra[s + 2];
      raw[d + 1] = bgra[s + 1];
      raw[d + 2] = bgra[s];
      raw[d + 3] = bgra[s + 3];
    }
  }
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const td = Buffer.concat([Buffer.from(type), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(td));
    return Buffer.concat([len, td, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// The redacted sample is made fresh each run, by izul-redact itself, so the
// "after" screenshot always shows what the current code produces.
{
  const r = await ask({
    op: "redact",
    path: join(SAMPLES, "Data Pegawai.pdf"),
    page: 0,
    out: join(SAMPLES, "Data Pegawai (diredaksi).pdf"),
  });
  if (!r.header.ok) throw new Error(`sampel redaksi gagal: ${JSON.stringify(r.header)}`);
}

let nextDoc = 1;
for (const f of SAMPLE_FILES) {
  const real = join(SAMPLES, f.name);
  const shown = `${f.folder}\\${f.name}`;
  realPath.set(shown, real);
  const { header } = await ask({ op: "open", path: real });
  const doc = nextDoc++;
  docs.push({ doc, path: shown, page_count: header.page_count, page_sizes: header.sizes });
  const cover = await ask({ op: "tile", path: real, page: 0, scale: 256, tier: "preview" });
  recent.push({
    path: shown,
    name: f.name,
    // Seconds, as the store writes them (`as_secs()`), not milliseconds.
    last_opened: Math.floor(f.opened / 1000),
    modified: Math.floor(f.modified / 1000),
    size: readFileSync(real).length,
    page_count: header.page_count,
    pinned: f.name.startsWith("Panduan"),
    available: true,
    cover: `data:image/png;base64,${png(cover.header.width, cover.header.height, cover.header.stride, cover.body).toString("base64")}`,
  });
}
const docById = new Map(docs.map((d) => [d.doc, realPath.get(d.path)]));

// The form panel's fields: the real widgets (izul_pdf::forms), grouped into
// fields as src-tauri/src/forms.rs does, with the values set in this run.
const formValues = new Map();
// A form filled in one shot is not filled in the next: the harness's copy of
// the document is dropped and opened fresh.
async function forgetForms() {
  const paths = new Set([...formValues.keys()].map((k) => k.split("\u0000")[0]));
  for (const path of paths) await ask({ op: "forget", path });
  formValues.clear();
}
async function formFieldsOf(path) {
  const { header } = await ask({ op: "forms", path });
  const fields = new Map();
  for (const w of header.widgets) {
    if (w.kind === "Other") continue;
    const f = fields.get(w.name) ?? { name: w.name, kind: w.kind, read_only: w.read_only, widgets: [], all: [] };
    f.widgets.push([w.page, w.rect]);
    f.all.push(w);
    fields.set(w.name, f);
  }
  return [...fields.values()].map((f) => {
    const first = f.all[0];
    const kind = typeof f.kind === "string" ? f.kind : Object.keys(f.kind)[0];
    const fromFile =
      kind === "CheckBox"
        ? { Checked: f.all.some((w) => w.checked) }
        : kind === "Radio"
          ? { Radio: f.all.find((w) => w.checked)?.export ?? "" }
          : kind === "ComboBox" || kind === "ListBox"
            ? { Choice: first.selected }
            : { Text: first.value };
    return {
      name: f.name,
      kind: f.kind,
      read_only: f.read_only,
      widgets: f.widgets,
      value: formValues.get(`${path}\u0000${f.name}`) ?? fromFile,
      options: first.options,
      exports: kind === "Radio" ? f.all.map((w) => w.export) : [],
    };
  });
}

const version = JSON.parse(readFileSync(join(ROOT, "version.json"), "utf8"));
const folders = {
  known: [
    { kind: "desktop", path: `${USER}\\Desktop` },
    { kind: "documents", path: docsFolder },
    { kind: "downloads", path: `${USER}\\Downloads` },
    { kind: "drive", path: "C:\\" },
    { kind: "drive", path: "D:\\" },
  ],
};
for (const f of SAMPLE_FILES) {
  const list = (folders[f.folder] ??= []);
  const r = recent.find((x) => x.name === f.name);
  list.push({ name: f.name, path: `${f.folder}\\${f.name}`, is_dir: false, size: r.size, modified: r.modified });
}
folders[docsFolder]?.unshift({ name: "Kuliah", path: `${docsFolder}\\Kuliah`, is_dir: true, size: null, modified: Math.floor((NOW - 12 * DAY) / 1000) });

// ---------------------------------------------------------------------------
// Scenes. Later phases add theirs here, one per new piece of UI.
// ---------------------------------------------------------------------------

const OPEN_ALL = SAMPLE_FILES.slice(0, 3).map((f) => `${f.folder}\\${f.name}`);

/** Resolves once the viewport has stopped asking for tiles for a while. */
async function settle(page, quietMs = 700) {
  const start = Date.now();
  for (;;) {
    await page.waitForTimeout(150);
    if (Date.now() - page.__lastTile > quietMs && Date.now() - start > quietMs) return;
    if (Date.now() - start > 15_000) return;
  }
}

async function izul(page, fn, arg) {
  return page.evaluate(
    ([src, a]) => {
      const f = new Function("izul", "arg", `return (${src})(izul, arg);`);
      return f(window.__izul, a);
    },
    [fn.toString(), arg],
  );
}

const SCENES = {
  home: {
    session: [],
    async steps(page) {
      await page.getByRole("row", { name: /Panduan Studi 2026/ }).first().click();
    },
  },
  document: { session: OPEN_ALL, async steps() {} },
  edit: {
    session: OPEN_ALL,
    async steps(page) {
      await page.getByRole("tab", { name: "Edit", exact: true }).click();
      await izul(page, (z) => z.goToPage(2));
      await settle(page);
      await izul(page, (z) => z.select([205]));
    },
  },
  comment: {
    session: OPEN_ALL,
    async steps(page) {
      await page.getByRole("tab", { name: "Komentar", exact: true }).click();
      await izul(page, (z) => z.goToPage(2));
      await izul(page, (z) => z.sidebar("annots"));
    },
  },
  // Phase 4 ------------------------------------------------------------------
  convert: {
    session: OPEN_ALL,
    flags: { dirty: true },
    async steps(page) {
      await page.getByRole("tab", { name: "Konversi", exact: true }).click();
    },
  },
  export: {
    session: OPEN_ALL,
    async steps(page) {
      await page.getByRole("tab", { name: "Konversi", exact: true }).click();
      await page.getByRole("button", { name: "PDF ke Gambar", exact: true }).click();
      await page.getByRole("radio", { name: "JPG" }).click();
      await page.getByRole("radio", { name: /Pilih halaman/ }).click();
      await page.getByRole("textbox", { name: "Pilih halaman" }).fill("1-3, 7");
    },
  },
  close: {
    session: OPEN_ALL,
    flags: { dirty: true },
    async steps(page) {
      await page.getByRole("button", { name: /^Tutup tab — / }).first().click();
      await page.getByRole("dialog").waitFor();
    },
  },
  draft: {
    session: OPEN_ALL.slice(0, 1),
    flags: { draft: true },
    async steps(page) {
      await page.getByRole("dialog").waitFor();
    },
  },
  changed: {
    session: OPEN_ALL,
    flags: { dirty: true, diskChanged: true },
    async steps(page) {
      await page.getByRole("alert").first().waitFor();
    },
  },
  exports: {
    session: [],
    flags: { exports: true },
    async steps(page) {
      await page.getByRole("button", { name: "Riwayat Ekspor" }).click();
      await page.getByRole("row", { name: /-rata\.pdf/ }).first().click();
    },
  },
  // Phase 5 ------------------------------------------------------------------
  pages: {
    session: OPEN_ALL.slice(0, 1),
    async steps(page) {
      await page.getByRole("tab", { name: "Halaman", exact: true }).click();
      await page.getByRole("button", { name: "Sisip Kosong", exact: true }).click();
      await settle(page);
      await izul(page, (z) => z.pages([3, 4]));
    },
  },
  splitdialog: {
    session: OPEN_ALL.slice(0, 1),
    async steps(page) {
      await page.getByRole("tab", { name: "Halaman", exact: true }).click();
      await page.getByRole("button", { name: "Pecah", exact: true }).click();
      await page.getByRole("dialog").waitFor();
    },
  },
  split: {
    session: OPEN_ALL,
    async steps(page) {
      await izul(page, (z) => z.layout("grid"));
      await izul(page, (z) => z.sidebar("thumbnails"));
    },
  },
  // Phase 6 ------------------------------------------------------------------
  protect: {
    session: [`${docsFolder}\\Data Pegawai.pdf`],
    async steps(page) {
      await page.getByRole("tab", { name: "Lindungi", exact: true }).click();
      await settle(page);
      await izul(page, (z) => z.select([900]));
    },
  },
  redactdialog: {
    session: [`${docsFolder}\\Data Pegawai.pdf`],
    async steps(page) {
      await page.getByRole("tab", { name: "Lindungi", exact: true }).click();
      await page.getByRole("button", { name: "Terapkan Redaksi", exact: true }).click();
      await page.getByRole("dialog").waitFor();
      await page.getByText(/tanda di 1 halaman/).waitFor();
    },
  },
  redacted: {
    session: [`${docsFolder}\\Data Pegawai (diredaksi).pdf`],
    async steps(page) {
      await page.getByRole("tab", { name: "Lindungi", exact: true }).click();
      await settle(page);
    },
  },
  // Phase 7 ------------------------------------------------------------------
  bgpanel: {
    session: OPEN_ALL,
    async steps(page) {
      await page.getByRole("tab", { name: "Edit", exact: true }).click();
      await izul(page, (z) => z.goToPage(2));
      await settle(page);
      await izul(page, (z) => z.select([208]));
      await page.getByText("Hapus latar", { exact: true }).waitFor();
    },
  },
  bgdone: {
    session: OPEN_ALL,
    async steps(page) {
      await page.getByRole("tab", { name: "Edit", exact: true }).click();
      await izul(page, (z) => z.goToPage(2));
      await settle(page);
      await izul(page, (z) => z.select([208]));
      await page.getByRole("button", { name: "Tanda tangan / stempel", exact: true }).click();
      await page.getByText(/Latar dihapus/).waitFor();
      await settle(page, 800);
    },
  },
  ocrdialog: {
    session: [`${docsFolder}\\Data Pegawai.pdf`],
    async steps(page) {
      await page.getByRole("tab", { name: "Konversi", exact: true }).click();
      await page.getByRole("button", { name: "Kenali Teks (OCR)", exact: true }).click();
      await page.getByRole("dialog").waitFor();
      await page.getByRole("button", { name: /Kenali \d+ halaman/ }).waitFor();
    },
  },
  ocrrunning: {
    session: [`${docsFolder}\\Data Pegawai.pdf`],
    async steps(page) {
      await page.getByRole("tab", { name: "Konversi", exact: true }).click();
      await page.getByRole("button", { name: "Kenali Teks (OCR)", exact: true }).click();
      await page.getByRole("dialog").waitFor();
      await page.getByRole("radio", { name: "Timpa berkas ini" }).check();
      await page.getByRole("button", { name: /Kenali \d+ halaman/ }).click();
      await page.getByText(/Mengenali halaman 3 dari 5/).waitFor();
    },
  },
  ocrmissing: {
    session: [`${docsFolder}\\Data Pegawai.pdf`],
    flags: { noOcr: true },
    async steps(page) {
      await page.getByRole("tab", { name: "Konversi", exact: true }).click();
      await page.getByRole("button", { name: "Kenali Teks (OCR)", exact: true }).click();
      await page.getByText(/Model OCR belum terpasang/).waitFor();
    },
  },
  textedit: {
    session: [`${docsFolder}\\Panduan Studi 2026.pdf`],
    async steps(page) {
      await page.getByRole("tab", { name: "Edit", exact: true }).click();
      await settle(page);
      const found = await izul(page, (z) => z.selectText("PANDUAN"));
      if (!found) throw new Error("teks contoh tidak ada di lapisan teks");
      await page.getByRole("button", { name: "Edit Teks", exact: true }).click();
      await page.getByRole("dialog").waitFor();
      await page.getByRole("textbox", { name: "Ganti menjadi" }).fill("PEDOMAN");
    },
  },
  texteditrefused: {
    session: [`${docsFolder}\\Panduan Studi 2026.pdf`],
    async steps(page) {
      await page.getByRole("tab", { name: "Edit", exact: true }).click();
      await settle(page);
      await izul(page, (z) => z.selectText("PANDUAN"));
      await page.getByRole("button", { name: "Edit Teks", exact: true }).click();
      await page.getByRole("textbox", { name: "Ganti menjadi" }).fill("PEDOMAN");
      await page.getByText("Timpa berkas ini", { exact: true }).click();
      await page.getByRole("button", { name: "Ganti & Simpan", exact: true }).click();
      await page.getByRole("alert").filter({ hasText: "tidak tertanam" }).waitFor();
    },
  },
  formbanner: {
    session: [`${USER}\\Downloads\\Formulir Pendaftaran.pdf`],
    async steps(page) {
      await page.getByText(/Dokumen ini berisi formulir/).waitFor();
      await settle(page);
    },
  },
  forms: {
    session: [`${USER}\\Downloads\\Formulir Pendaftaran.pdf`],
    async steps(page) {
      await page.getByRole("button", { name: "Isi Formulir", exact: true }).click();
      const name = page.getByRole("textbox", { name: "Nama lengkap", exact: true });
      await name.fill("Rahmawati Putri");
      await name.press("Enter");
      const address = page.getByRole("textbox", { name: "Alamat", exact: true });
      await address.fill("Jl. Melati No. 12\nKecamatan Coblong\nBandung 40132");
      await address.blur();
      // The controls show the backend's value, so a click shows once it answers.
      const radio = page.getByRole("radio", { name: "Pelajar", exact: true });
      await radio.click();
      await page.waitForFunction(() => document.querySelector('input[type=radio][name="form-kategori"]:checked') !== null);
      await page.getByRole("combobox", { name: "Kota", exact: true }).selectOption({ label: "Yogyakarta" });
      await page.getByRole("checkbox").click();
      await page.getByText("Dicentang", { exact: true }).waitFor();
      // Back to the top, where the filled fields are.
      await izul(page, (z) => z.goToPage(0));
      await settle(page, 1200);
    },
  },
  compare: {
    session: [`${docsFolder}\\Draf Perjanjian v1.pdf`, `${docsFolder}\\Draf Perjanjian v2.pdf`],
    async steps(page) {
      await izul(page, (z) => z.compare(true));
      await settle(page, 1200);
    },
  },
};

/** Export history rows for the "exports" scene, from the sample files. */
const EXPORTS = (() => {
  const [a, b] = SAMPLE_FILES;
  const at = (hoursAgo) => Math.floor((NOW - hoursAgo * 3600 * 1000) / 1000);
  const stem = (f) => f.name.replace(/\.pdf$/i, "");
  const rows = [
    { source: `${a.folder}\\${a.name}`, out_path: `${a.folder}\\${stem(a)}-rata.pdf`, kind: "flat", created_at: at(2), exists: true, size: 412_338 },
    { source: `${a.folder}\\${a.name}`, out_path: `${a.folder}\\${stem(a)}-hal 1-3.pdf`, kind: "pages", created_at: at(5), exists: true, size: 96_120 },
  ];
  for (let n = 1; n <= 3; n++) {
    rows.push({ source: `${b.folder}\\${b.name}`, out_path: `${b.folder}\\${stem(b)}-${n}.jpg`, kind: "jpg", created_at: at(26), exists: true, size: 180_000 + n * 7_311 });
  }
  rows.push({ source: `${b.folder}\\${b.name}`, out_path: `${b.folder}\\lama-${stem(b)}-rata.pdf`, kind: "flat", created_at: at(24 * 40), exists: false, size: null });
  return rows;
})();

// ---------------------------------------------------------------------------
// Run
// ---------------------------------------------------------------------------

const server = await createServer({
  root: ROOT,
  configFile: join(ROOT, "vite.config.ts"),
  server: { port: PORT, strictPort: true },
  logLevel: "warn",
});
await server.listen();

const browser = await chromium.launch({
  executablePath: process.env.IZUL_CHROMIUM || undefined,
});

async function newPage({ width, height, theme, scale, scene }) {
  const context = await browser.newContext({
    viewport: { width, height },
    deviceScaleFactor: scale,
    colorScheme: theme,
    locale: "id-ID",
    timezoneId: "Asia/Jakarta",
  });
  const page = await context.newPage();
  page.__lastTile = Date.now();
  page.on("pageerror", (e) => console.error(`  [halaman] ${e.message}`));
  page.on("console", (m) => {
    if (m.type() === "error") console.error(`  [konsol] ${m.text()}`);
  });
  await page.clock.setFixedTime(new Date(NOW));
  const data = {
    version,
    docs,
    recent,
    session: SCENES[scene].session,
    active: SCENES[scene].session[0] ?? null,
    folders,
    flags: SCENES[scene].flags ?? {},
    exports: EXPORTS,
    now: Math.floor(NOW / 1000),
  };
  await page.addInitScript((d) => {
    window.__HARNESS__ = d;
  }, data);
  await page.exposeFunction("__harnessBackend", async (cmd, a) => {
    const path = docById.get(a.doc);
    if (cmd === "annot_remove_background") {
      // The picture comes back as image 2, which the route below makes with
      // the real model; the object is the sample's own, pointed at it.
      const { header } = await ask({ op: "annots", path, page: 2 });
      const obj = structuredClone(header.objects.find((o) => o.id === a.id));
      obj.payload.Image.image = 2;
      page.__backgroundDone = a.id;
      return {
        edit: { objects: [obj], can_undo: true, can_redo: false, dirty: true, map_revision: 0 },
        device: "CPU",
        paper: a.kind === "OnPaper",
      };
    }
    if (cmd === "form_fields") return formFieldsOf(path);
    if (cmd === "text_replace") {
      // The real routine on the sample: its Helvetica is not embedded, so
      // this is the refusal a user would get, in its own words.
      const { header } = await ask({ op: "textedit", path, page: a.page, rect: a.rect, text: a.text });
      // `ok` is the harness transport's own field; the answer is `accepted`.
      if (!header.accepted) throw `Teks tidak diganti, berkas tidak diubah: ${header.refused}`;
      return { path, bytes: 1, annotations: 0, restructured: null, redaction: null, text: { before: header.before, glyphs: header.glyphs } };
    }
    if (cmd === "form_set") {
      // The real form-fill routine on the harness's copy of the document, so
      // the page shows what PDFium makes of the value.
      const { header } = await ask({ op: "formfill", path, values: [[a.name, a.value]] });
      formValues.set(`${path}\u0000${a.name}`, a.value);
      return { objects: [], can_undo: true, can_redo: false, dirty: true, map_revision: 0, repaint: header.pages };
    }
    if (cmd === "document_outline") return (await ask({ op: "outline", path })).header.outline;
    if (cmd === "page_text") return (await ask({ op: "text", path, page: a.page, rotation: a.rotation ?? 0 })).header;
    const redactable = String(path).endsWith("Data Pegawai.pdf");
    if (cmd === "redact_preview") {
      const { header } = await ask({ op: "annots", path, page: 0 });
      return { marks: redactable ? header.objects.length : 0, pages: redactable ? [0] : [], annotations: 0 };
    }
    const annotated = (a.doc === 1 && a.page === 2) || (redactable && a.page === 0);
    if (!annotated) return [];
    const { header } = await ask({ op: "annots", path, page: a.page });
    if (cmd === "annot_list") return header.objects;
    // After "Hapus Latar", the picture's display list draws image 2.
    const done = page.__backgroundDone;
    return done === undefined
      ? header.lists
      : header.lists.map((l) => (l.id === done ? JSON.parse(JSON.stringify(l).replace(/"image":1\b/g, '"image":2')) : l));
  });
  await page.route("http://izul.localhost/**", async (route) => {
    page.__lastTile = Date.now();
    const url = new URL(route.request().url());
    const cors = {
      "Access-Control-Allow-Origin": "*",
      "Access-Control-Expose-Headers": "X-Izul-Width, X-Izul-Height, X-Izul-Stride",
    };
    const [, kind, doc, pg, rot, scale, col, row, tier] = url.pathname.split("/");
    if (kind === "image") {
      // `/image/{doc}/{image}`: 1 is the sample stamp photo, 2 the model's
      // result on it (the "bgdone" scene).
      const stampPath = join(SAMPLES, "stempel.png");
      const body = pg === "2" ? (await ask({ op: "background", path: stampPath })).body : readFileSync(stampPath);
      return route.fulfill({ status: 200, headers: { ...cors, "Content-Type": "image/png" }, body });
    }
    if (kind !== "tile") return route.fulfill({ status: 404, headers: cors });
    const { header, body } = await ask({
      op: "tile",
      path: docById.get(Number(doc)),
      page: Number(pg),
      rotation: Number(rot),
      scale: Number(scale),
      col: Number(col),
      row: Number(row),
      tier,
    });
    page.__lastTile = Date.now();
    if (!header.ok) return route.fulfill({ status: 404, headers: cors });
    return route.fulfill({
      status: 200,
      headers: {
        ...cors,
        "Content-Type": "application/octet-stream",
        "X-Izul-Width": String(header.width),
        "X-Izul-Height": String(header.height),
        "X-Izul-Stride": String(header.stride),
      },
      body,
    });
  });
  await page.goto(`http://localhost:${PORT}/tools/ui-harness/index.html?scene=${scene}`);
  return page;
}

const sizes = (args.size ? [args.size] : ["1366x768", "1920x1080"]).map((s) => s.split("x").map(Number));
const themes = args.theme ? [args.theme] : ["light", "dark"];
const scenes = args.scene ? args.scene.split(",") : Object.keys(SCENES);
const scale = Number(args.scale ?? 1);
mkdirSync(OUT, { recursive: true });

if (args.serve === "true") {
  const page = await newPage({ width: 1366, height: 768, theme: "light", scale: 1, scene: scenes[0] });
  console.log(`harness berjalan di http://localhost:${PORT}/tools/ui-harness/index.html (Ctrl+C untuk berhenti)`);
  void page;
} else {
  for (const scene of scenes) {
    for (const [width, height] of sizes) {
      for (const theme of themes) {
        await forgetForms();
        const page = await newPage({ width, height, theme, scale, scene });
        await page.waitForFunction(() => window.__izul !== undefined, null, { timeout: 15_000 });
        await settle(page);
        await SCENES[scene].steps(page);
        await settle(page);
        const suffix = scale === 1 ? "" : `@${scale}x`;
        const file = join(OUT, `${scene}-${width}x${height}-${theme}${suffix}.png`);
        await page.screenshot({ path: file });
        console.log(`  ${file}`);
        await page.context().close();
      }
    }
  }
  await browser.close();
  await server.close();
  helper.stdin.end();
}
