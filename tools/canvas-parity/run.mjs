/**
 * Draws every golden display list with the canvas backend in a real Chromium
 * and compares the result with PDFium's render of the same list.
 *
 * Deliberately dependency-free beyond what the repository already has: esbuild
 * (a Vite dependency) bundles the backend, and Chromium is driven through its
 * own `--headless --screenshot` rather than through a browser-automation
 * package. Adding one to `package.json` would make CI download a browser for a
 * test CI does not run.
 */

import { execFileSync } from "node:child_process";
import { build } from "esbuild";
import { mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../..");
const goldenDir = join(repo, "crates/izul-pdf/golden");

/** Where Chromium lives. Playwright's copy is what this container has. */
function chromiumPath() {
  const candidates = [
    process.env.CHROMIUM_PATH,
    "/opt/pw-browsers/chromium-1194/chrome-linux/chrome",
    "/opt/pw-browsers/chromium/chrome-linux/chrome",
    "/usr/bin/chromium",
    "/usr/bin/google-chrome",
  ].filter(Boolean);
  for (const candidate of candidates) {
    try {
      execFileSync(candidate, ["--version"], { stdio: "ignore" });
      return candidate;
    } catch {
      // Try the next one.
    }
  }
  throw new Error("Chromium tidak ditemukan; set CHROMIUM_PATH");
}

/** Bundles the canvas backend into something a plain <script> can load. */
async function bundle(outDir) {
  await build({
    entryPoints: [join(repo, "tools/canvas-parity/entry.ts")],
    bundle: true,
    format: "iife",
    globalName: "IZUL_CANVAS",
    outfile: join(outDir, "bundle.js"),
    target: "chrome110",
    logLevel: "silent",
    alias: { "@": join(repo, "src") },
  });
}

/** Per-channel tolerance, 0..255. Two rasterisers never agree to the level of
 * a single code value on an antialiased edge, and demanding they did would be
 * demanding PDFium and Skia be one program. */
const TOLERANCE = 24;

/** Runs one case in Chromium and reads the comparison out of the DOM. */
function runCase(chromium, workDir, name, data) {
  const html = readFileSync(join(here, "page.html"), "utf8").replace(
    "<!-- IZUL_INJECT -->",
    `<script>window.IZUL_CASE = ${JSON.stringify(data)};<\/script>\n<script src="./bundle.js"><\/script>`,
  );
  const page = join(workDir, `${name}.html`);
  writeFileSync(page, html);
  const dom = execFileSync(
    chromium,
    [
      "--headless",
      "--no-sandbox",
      "--disable-gpu",
      // Without this a file:// image taints the canvas and getImageData throws.
      "--allow-file-access-from-files",
      "--virtual-time-budget=5000",
      "--dump-dom",
      `file://${page}`,
    ],
    { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "ignore"] },
  );
  const match = dom.match(/<pre id="out">([\s\S]*?)<\/pre>/);
  if (!match || match[1] === "pending") {
    throw new Error(`${name}: halaman tidak melaporkan hasil`);
  }
  return JSON.parse(match[1].replace(/&quot;/g, '"').replace(/&amp;/g, "&"));
}

async function main() {
  const chromium = chromiumPath();
  const workDir = mkdtempSync(join(tmpdir(), "izul-parity-"));
  await bundle(workDir);

  const cases = readdirSync(goldenDir).filter((f) => f.endsWith(".json"));
  if (cases.length === 0) {
    throw new Error("belum ada display list; jalankan cargo test -p izul-pdf --lib golden");
  }

  const rows = [];
  for (const file of cases.sort()) {
    const name = file.replace(/\.json$/, "");
    const data = JSON.parse(readFileSync(join(goldenDir, file), "utf8"));
    data.baseline = `file://${join(goldenDir, `${name}.png`)}`;
    data.tolerance = TOLERANCE;
    const result = runCase(chromium, workDir, name, data);
    if (result.error) throw new Error(`${name}: ${result.error}`);
    // The canvas render is kept so a surprising number can be looked at rather
    // than argued about.
    writeFileSync(
      join(workDir, `${name}.png`),
      Buffer.from(String(result.png).split(",")[1] ?? "", "base64"),
    );
    rows.push({ name, textual: data.textual === true, ...result });
  }

  let worst = 0;
  console.log(`kanvas vs PDFium — piksel berbeda (toleransi ${TOLERANCE}/255 per kanal)\n`);
  console.log(
    `  ${"jenis".padEnd(20)} halus    kasar    tinta kanvas / PDFium`,
  );
  let worstCoarse = 0;
  for (const row of rows) {
    if (!row.textual) {
      worst = Math.max(worst, row.differing);
      worstCoarse = Math.max(worstCoarse, row.coarse);
    }
    console.log(
      `  ${row.name.padEnd(20)} ${(row.differing * 100).toFixed(2).padStart(6)} % ` +
        `${(row.coarse * 100).toFixed(2).padStart(7)} %   ` +
        `${(row.inkCanvas * 100).toFixed(1)} % / ${(row.inkPdfium * 100).toFixed(1)} %` +
        (row.textual ? "   (teks: bentuk glif memang beda)" : ""),
    );
  }
  console.log(
    `\nterburuk halus: ${(worst * 100).toFixed(2)} % · terburuk kasar: ${(worstCoarse * 100).toFixed(2)} %` +
      "  (di luar kasus berteks)",
  );
  console.log(
    "halus = tiap piksel (termasuk antialias dan penghalusan gambar); " +
      "kasar = keduanya diperkecil 4x lebih dulu, jadi yang tersisa perbedaan bentuk.",
  );
  console.log(
    "Kasus berteks dikecualikan dari angka terburuk: baseline PDFium memakai font uji Type 3 " +
      "(glifnya blok, supaya deterministik di semua platform) sedangkan kanvas menggambar huruf " +
      "sungguhan. Yang masih berarti di baris itu adalah cakupan tintanya — posisi teksnya sama.",
  );
  console.log(`gambar kanvas ada di ${workDir}`);
}

main().catch((e) => {
  console.error(e.message ?? e);
  process.exit(1);
});
