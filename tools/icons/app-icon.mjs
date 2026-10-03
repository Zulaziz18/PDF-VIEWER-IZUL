/**
 * The application icon (Phase 8): the same mark as `src/design/Logo.tsx`, the
 * one the title bar shows, at every size Windows asks for.
 *
 * The paths are read out of Logo.tsx rather than copied, so the icon cannot
 * drift from the mark. Each size is drawn by Chromium at that size — not
 * scaled down from a large one — so the small ones stay crisp. The .ico holds
 * PNG entries, which Windows has read since Vista.
 *
 *   npm run icons:app
 */

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const OUT = join(ROOT, "src-tauri/icons");
const BRAND = "#d7373f"; // --izul-brand, light theme

const logo = readFileSync(join(ROOT, "src/design/Logo.tsx"), "utf8");
const paths = [...logo.matchAll(/<path d="([^"]+)"([^/]*)\/>/g)].map(([, d, rest]) => {
  const fill = /fill="([^"]+)"/.exec(rest)?.[1] ?? BRAND;
  const opacity = /fillOpacity="([^"]+)"/.exec(rest)?.[1];
  return `<path d="${d}" fill="${fill}"${opacity ? ` fill-opacity="${opacity}"` : ""}/>`;
});
if (paths.length !== 3) throw new Error(`Logo.tsx: expected 3 paths, found ${paths.length}`);
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">${paths.join("")}</svg>`;

const browser = await chromium.launch({ executablePath: process.env.IZUL_CHROMIUM || undefined });
const page = await browser.newPage();
async function render(size) {
  await page.setViewportSize({ width: size, height: size });
  await page.setContent(
    `<html><body style="margin:0;background:transparent">` +
      svg.replace("<svg ", `<svg width="${size}" height="${size}" `) +
      `</body></html>`,
  );
  return page.screenshot({ omitBackground: true, clip: { x: 0, y: 0, width: size, height: size } });
}

mkdirSync(OUT, { recursive: true });
const files = { "32x32.png": 32, "128x128.png": 128, "128x128@2x.png": 256, "icon.png": 512 };
for (const [name, size] of Object.entries(files)) {
  writeFileSync(join(OUT, name), await render(size));
  console.log(`  ${name}`);
}

// ICONDIR + one ICONDIRENTRY per image, then the PNGs themselves.
const sizes = [16, 24, 32, 48, 64, 128, 256];
const images = [];
for (const s of sizes) images.push(await render(s));
const header = Buffer.alloc(6 + 16 * sizes.length);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2); // type: icon
header.writeUInt16LE(sizes.length, 4);
let offset = header.length;
sizes.forEach((s, i) => {
  const e = 6 + 16 * i;
  header.writeUInt8(s >= 256 ? 0 : s, e); // 0 means 256
  header.writeUInt8(s >= 256 ? 0 : s, e + 1);
  header.writeUInt8(0, e + 2); // no palette
  header.writeUInt8(0, e + 3);
  header.writeUInt16LE(1, e + 4); // colour planes
  header.writeUInt16LE(32, e + 6); // bits per pixel
  header.writeUInt32LE(images[i].length, e + 8);
  header.writeUInt32LE(offset, e + 12);
  offset += images[i].length;
});
writeFileSync(join(OUT, "icon.ico"), Buffer.concat([header, ...images]));
console.log(`  icon.ico (${sizes.join(", ")})`);
await browser.close();
