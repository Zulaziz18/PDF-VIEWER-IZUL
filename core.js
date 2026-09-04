// ============================================================
// core.js - fungsi murni (tanpa DOM), dipakai app.js & bisa diuji
// ============================================================

/** Palet highlight: 5 warna kontras yang terbaca di atas teks gelap maupun latar gelap */
export const HIGHLIGHT_COLORS = [
  { name: "Kuning", hex: "#FFE066" },
  { name: "Hijau",  hex: "#8CE99A" },
  { name: "Biru",   hex: "#74C0FC" },
  { name: "Pink",   hex: "#FFA8C5" },
  { name: "Oranye", hex: "#FFB86B" },
];

/** Palet warna teks (termasuk putih untuk menulis di atas latar gelap) */
export const TEXT_COLORS = [
  { name: "Hitam",  hex: "#000000" },
  { name: "Putih",  hex: "#FFFFFF" },
  { name: "Merah",  hex: "#D32F2F" },
  { name: "Biru",   hex: "#1565C0" },
  { name: "Hijau",  hex: "#2E7D32" },
  { name: "Oranye", hex: "#E65100" },
  { name: "Ungu",   hex: "#6A1B9A" },
  { name: "Abu",    hex: "#616161" },
];

/** "#FFE066" -> {r:1, g:0.878, b:0.4} (skala 0..1 untuk pdf-lib) */
export function hexToRgb01(hex) {
  const h = hex.replace("#", "");
  const full = h.length === 3 ? h.split("").map((c) => c + c).join("") : h;
  return {
    r: parseInt(full.slice(0, 2), 16) / 255,
    g: parseInt(full.slice(2, 4), 16) / 255,
    b: parseInt(full.slice(4, 6), 16) / 255,
  };
}

/**
 * Pecah teks jadi baris-baris yang muat dalam maxWidth.
 * measure(str) harus mengembalikan lebar string tsb pada ukuran font yang dipakai.
 * Menghormati newline manual. Kata yang lebih panjang dari maxWidth dipecah paksa
 * per karakter supaya tidak meluber keluar kotak.
 */
export function wrapText(text, measure, maxWidth) {
  const out = [];
  const paragraphs = String(text).split("\n");
  for (const para of paragraphs) {
    if (para === "") { out.push(""); continue; }
    const words = para.split(/(\s+)/).filter((w) => w !== "");
    let cur = "";
    for (let token of words) {
      if (/^\s+$/.test(token)) {
        if (cur) cur += token;
        continue;
      }
      const trial = cur + token;
      if (measure(trial) <= maxWidth) { cur = trial; continue; }
      if (cur.trim()) { out.push(cur.trimEnd()); cur = ""; }
      // kata tunggal lebih lebar dari kotak -> pecah paksa per karakter
      if (measure(token) > maxWidth) {
        let chunk = "";
        for (const ch of token) {
          if (measure(chunk + ch) > maxWidth && chunk) {
            out.push(chunk);
            chunk = ch;
          } else {
            chunk += ch;
          }
        }
        cur = chunk;
      } else {
        cur = token;
      }
    }
    out.push(cur.trimEnd());
  }
  return out;
}

/**
 * Konversi kotak dari koordinat DOM (origin kiri-ATAS) ke koordinat PDF (origin kiri-BAWAH).
 * Semua satuan dalam PDF point.
 */
export function domRectToPdf(x, y, w, h, pageHeightPt) {
  return { x, y: pageHeightPt - y - h, width: w, height: h };
}

/** Kebalikan dari domRectToPdf */
export function pdfRectToDom(x, y, w, h, pageHeightPt) {
  return { x, y: pageHeightPt - y - h, width: w, height: h };
}

/**
 * Hitung posisi baseline tiap baris teks (dalam koordinat PDF, siap dipakai drawText).
 * boxTopY = tepi atas kotak dalam koordinat DOM (dari atas halaman).
 */
export function computeLineBaselines(lineCount, fontSize, lineHeightFactor, boxTopY, pageHeightPt, ascentRatio = 0.8) {
  const lineH = fontSize * lineHeightFactor;
  const out = [];
  for (let i = 0; i < lineCount; i++) {
    const topFromPageTop = boxTopY + i * lineH;
    const baselineFromPageTop = topFromPageTop + fontSize * ascentRatio;
    out.push(pageHeightPt - baselineFromPageTop);
  }
  return out;
}

/**
 * Terapkan resize dari sebuah handle.
 * handle: 'nw'|'ne'|'sw'|'se'|'n'|'s'|'e'|'w'
 * start: {x,y,w,h} kotak sebelum drag; dx,dy: pergeseran mouse (satuan sama).
 * lockAspect: pertahankan rasio (dipakai untuk handle sudut pada gambar).
 * minSize: batas minimum lebar/tinggi.
 */
export function applyResize(handle, start, dx, dy, lockAspect = false, minSize = 12) {
  let { x, y, w, h } = start;
  const aspect = start.w / start.h;

  if (handle.includes("e")) w = start.w + dx;
  if (handle.includes("w")) { w = start.w - dx; x = start.x + dx; }
  if (handle.includes("s")) h = start.h + dy;
  if (handle.includes("n")) { h = start.h - dy; y = start.y + dy; }

  if (lockAspect && handle.length === 2) {
    // handle sudut: samakan rasio, ikuti perubahan yang lebih dominan
    if (Math.abs(dx) > Math.abs(dy)) h = w / aspect;
    else w = h * aspect;
    if (handle.includes("w")) x = start.x + (start.w - w);
    if (handle.includes("n")) y = start.y + (start.h - h);
  }

  if (w < minSize) {
    if (handle.includes("w")) x = start.x + start.w - minSize;
    w = minSize;
  }
  if (h < minSize) {
    if (handle.includes("n")) y = start.y + start.h - minSize;
    h = minSize;
  }
  return { x, y, w, h };
}

/** Gabungkan kotak-kotak yang berada di baris sama & bersebelahan supaya highlight tidak terpotong-potong */
export function mergeRects(rects, tolerance = 2) {
  if (!rects.length) return [];
  const sorted = [...rects].sort((a, b) => (a.y - b.y) || (a.x - b.x));
  const out = [];
  for (const r of sorted) {
    const last = out[out.length - 1];
    const sameLine = last &&
      Math.abs(last.y - r.y) <= tolerance &&
      Math.abs(last.height - r.height) <= tolerance;
    const adjacent = last && r.x <= last.x + last.width + tolerance * 2;
    if (sameLine && adjacent) {
      const right = Math.max(last.x + last.width, r.x + r.width);
      last.width = right - last.x;
      last.height = Math.max(last.height, r.height);
    } else {
      out.push({ ...r });
    }
  }
  return out;
}

/** Batasi nilai ke rentang [lo, hi] */
export function clamp(v, lo, hi) { return Math.max(lo, Math.min(v, hi)); }
