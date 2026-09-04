// ============================================================
// annots.js - membaca & menulis anotasi ke dalam file PDF
// Anotasi disimpan sebagai objek PDF standar (Highlight/FreeText/Stamp)
// sehingga tetap terlihat di Adobe/Chrome, DAN bisa dibuka-edit lagi di sini.
// ============================================================
import { hexToRgb01, wrapText, computeLineBaselines } from "./core.js";

const TAG = "PDFStudio";          // penanda: anotasi ini dibuat oleh aplikasi kita
export const LINE_HEIGHT = 1.2;
export const ASCENT_RATIO = 0.8;

const STD = {
  regular: "TimesRoman", bold: "TimesRomanBold",
  italic: "TimesRomanItalic", boldItalic: "TimesRomanBoldItalic",
};
export function fontKeyOf(bold, italic) {
  if (bold && italic) return "boldItalic";
  if (bold) return "bold";
  if (italic) return "italic";
  return "regular";
}

// ------------------------------------------------------------
//  MEMBACA anotasi dari PDF
// ------------------------------------------------------------
/**
 * Ambil semua anotasi buatan aplikasi ini, lalu hapus dari dokumen.
 * Mengembalikan { annots, cleanBytes } — cleanBytes = PDF tanpa anotasi kita
 * (anotasi dari aplikasi lain tetap dipertahankan).
 */
export async function extractAnnots(PDFLib, bytes) {
  const { PDFDocument, PDFName } = PDFLib;
  const doc = await PDFDocument.load(bytes, { ignoreEncryption: true });
  const out = [];
  let found = false;

  const pages = doc.getPages();
  for (let pi = 0; pi < pages.length; pi++) {
    const page = pages[pi];
    const arr = page.node.get(PDFName.of("Annots"));
    if (!arr || typeof arr.size !== "function") continue;

    const keepRefs = [];
    for (let i = 0; i < arr.size(); i++) {
      const ref = arr.get(i);
      let d;
      try { d = doc.context.lookup(ref); } catch { keepRefs.push(ref); continue; }
      if (!d || typeof d.get !== "function") { keepRefs.push(ref); continue; }

      const T = d.get(PDFName.of("T"));
      const isOurs = T && typeof T.decodeText === "function" && T.decodeText() === TAG;
      if (!isOurs) { keepRefs.push(ref); continue; }

      const RC = d.get(PDFName.of("RC"));
      let meta = {};
      try { meta = JSON.parse(RC.decodeText()); } catch { keepRefs.push(ref); continue; }

      const pageH = page.getSize().height;
      const a = annotFromMeta(meta, d, PDFName, pageH, pi);
      if (a) { out.push(a); found = true; } else { keepRefs.push(ref); }
    }

    if (keepRefs.length !== arr.size()) {
      while (arr.size() > 0) arr.remove(0);
      keepRefs.forEach((r) => arr.push(r));
    }
  }

  const cleanBytes = found ? await doc.save() : bytes;
  return { annots: out, cleanBytes };
}

function rectOf(d, PDFName) {
  const r = d.get(PDFName.of("Rect"));
  if (!r || typeof r.asArray !== "function") return null;
  return r.asArray().map((n) => n.asNumber());
}

function annotFromMeta(meta, d, PDFName, pageH, pageIndex) {
  const rect = rectOf(d, PDFName);
  if (!rect) return null;
  const [x0, y0, x1, y1] = rect;
  const base = {
    id: meta.id || "a" + Math.random().toString(36).slice(2, 9),
    pageIndex,
    x: x0,
    y: pageH - y1,          // konversi PDF (bawah) -> DOM (atas)
    w: x1 - x0,
    h: y1 - y0,
  };

  if (meta.kind === "highlight") {
    let rects = meta.rects;
    if (!Array.isArray(rects) || !rects.length) {
      rects = [{ x: base.x, y: base.y, width: base.w, height: base.h }];
    }
    return { ...base, type: "highlight", color: meta.color || "#FFE066", rects };
  }
  if (meta.kind === "text") {
    return {
      ...base, type: "text",
      text: meta.text || "",
      fontSize: meta.size || 16,
      color: meta.color || "#000000",
      bold: !!meta.bold, italic: !!meta.italic,
    };
  }
  if (meta.kind === "image" && meta.dataUrl) {
    return { ...base, type: "image", dataUrl: meta.dataUrl, mime: meta.mime || "image/png",
             crop: meta.crop || null, cropSrc: meta.cropSrc || null };
  }
  return null;
}

// ------------------------------------------------------------
//  MENULIS anotasi ke PDF
// ------------------------------------------------------------
export async function buildPdfWithAnnots(PDFLib, baseBytes, annots, fontsNeeded) {
  const { PDFDocument, PDFName, PDFString, StandardFonts } = PDFLib;
  const doc = await PDFDocument.load(baseBytes, { ignoreEncryption: true });
  const ctx = doc.context;
  const pages = doc.getPages();

  // siapkan font yang dibutuhkan (hanya yang terpakai)
  const fontCache = {};
  for (const key of fontsNeeded) {
    fontCache[key] = await doc.embedFont(StandardFonts[STD[key]]);
  }
  const imgCache = new Map();

  const ensureAnnots = (page) => {
    let a = page.node.get(PDFName.of("Annots"));
    if (!a) { a = ctx.obj([]); page.node.set(PDFName.of("Annots"), a); }
    return a;
  };

  for (const an of annots) {
    const page = pages[an.pageIndex];
    if (!page) continue;
    const pageH = page.getSize().height;
    const arr = ensureAnnots(page);

    if (an.type === "highlight") {
      arr.push(ctx.register(makeHighlight(ctx, PDFString, PDFName, an, pageH)));
    } else if (an.type === "text") {
      const ref = await makeFreeText(ctx, PDFString, an, pageH, fontCache);
      if (ref) arr.push(ctx.register(ref));
    } else if (an.type === "image") {
      const ref = await makeStamp(doc, ctx, PDFString, an, pageH, imgCache);
      if (ref) arr.push(ctx.register(ref));
    }
  }
  return await doc.save();
}

function makeHighlight(ctx, PDFString, PDFName, an, pageH) {
  const rects = an.rects && an.rects.length
    ? an.rects
    : [{ x: an.x, y: an.y, width: an.w, height: an.h }];

  const quads = [];
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  const pdfRects = rects.map((r) => {
    const x0 = r.x, x1 = r.x + r.width;
    const yTop = pageH - r.y;
    const yBot = pageH - (r.y + r.height);
    quads.push(x0, yTop, x1, yTop, x0, yBot, x1, yBot);
    minX = Math.min(minX, x0); maxX = Math.max(maxX, x1);
    minY = Math.min(minY, yBot); maxY = Math.max(maxY, yTop);
    return { x0, yTop, x1, yBot };
  });
  const c = hexToRgb01(an.color);
  const bw = maxX - minX, bh = maxY - minY;

  // Appearance stream eksplisit dengan ExtGState Multiply + alpha 0.55,
  // supaya tampilan di SEMUA viewer (Adobe, Chrome, HP) identik persis
  // dengan editor (lihat opacity:.55 pada .hl-rect di style.css): teks
  // tetap tajam, warna hanya "menggelapkan" seperti stabilo asli - bukan menutup.
  const gs = ctx.obj({ Type: "ExtGState", BM: "Multiply", ca: 0.55 });
  const gsRef = ctx.register(gs);
  let body = `q /GS1 gs ${f(c.r)} ${f(c.g)} ${f(c.b)} rg\n`;
  for (const r of pdfRects) {
    body += `${f(r.x0 - minX)} ${f(r.yBot - minY)} ${f(r.x1 - r.x0)} ${f(r.yTop - r.yBot)} re f\n`;
  }
  body += "Q";
  const apStream = ctx.stream(body, {
    Type: "XObject", Subtype: "Form", BBox: [0, 0, bw, bh],
    Resources: { ExtGState: { GS1: gsRef } },
  });
  const apRef = ctx.register(apStream);

  return ctx.obj({
    Type: "Annot", Subtype: "Highlight",
    Rect: [minX, minY, maxX, maxY],
    QuadPoints: quads,
    C: [c.r, c.g, c.b],
    CA: 0.55,
    F: 4,
    T: PDFString.of(TAG),
    RC: PDFString.of(JSON.stringify({ id: an.id, kind: "highlight", color: an.color, rects })),
    AP: { N: apRef },
  });
}

async function makeFreeText(ctx, PDFString, an, pageH, fontCache) {
  const text = (an.text || "").trim();
  if (!text) return null;
  const key = fontKeyOf(an.bold, an.italic);
  const font = fontCache[key];
  if (!font) return null;

  const size = an.fontSize;
  const lines = wrapText(text, (s) => font.widthOfTextAtSize(s, size), an.w);
  const c = hexToRgb01(an.color);
  const boxH = Math.max(lines.length, 1) * size * LINE_HEIGHT;

  // appearance stream: koordinat lokal, origin kiri-bawah BBox
  const parts = [`q BT /F1 ${size} Tf ${f(c.r)} ${f(c.g)} ${f(c.b)} rg`];
  lines.forEach((ln, i) => {
    const yFromTop = i * size * LINE_HEIGHT + size * ASCENT_RATIO;
    const yLocal = boxH - yFromTop;
    const hex = font.encodeText(ln === "" ? " " : ln).toString();
    parts.push(`1 0 0 1 0 ${f(yLocal)} Tm ${hex} Tj`);
  });
  parts.push("ET Q");

  const ap = ctx.stream(parts.join("\n"), {
    Type: "XObject", Subtype: "Form",
    BBox: [0, 0, an.w, boxH],
    Resources: { Font: { F1: font.ref } },
  });
  const apRef = ctx.register(ap);

  const yBottom = pageH - an.y - boxH;
  return ctx.obj({
    Type: "Annot", Subtype: "FreeText",
    Rect: [an.x, yBottom, an.x + an.w, yBottom + boxH],
    Contents: PDFString.of(text),
    DA: PDFString.of(`/F1 ${size} Tf ${f(c.r)} ${f(c.g)} ${f(c.b)} rg`),
    F: 4,
    T: PDFString.of(TAG),
    RC: PDFString.of(JSON.stringify({
      id: an.id, kind: "text", text, size,
      color: an.color, bold: !!an.bold, italic: !!an.italic,
    })),
    AP: { N: apRef },
  });
}

async function makeStamp(doc, ctx, PDFString, an, pageH, imgCache) {
  if (!an.dataUrl) return null;
  let img = imgCache.get(an.dataUrl);
  if (!img) {
    const isPng = /^data:image\/png/i.test(an.dataUrl);
    img = isPng ? await doc.embedPng(an.dataUrl) : await doc.embedJpg(an.dataUrl);
    imgCache.set(an.dataUrl, img);
  }
  // an.crop (jika ada): {x,y,w,h} dalam satuan 0..1 relatif ke gambar asli -
  // diterapkan lewat clipping box pada content stream, gambar aslinya tidak
  // disentuh sehingga crop bisa diubah lagi kapan saja tanpa kehilangan kualitas.
  // crop.x/y/w/h dalam skala 0..1, konvensi DOM (y diukur dari ATAS gambar).
  // Ruang gambar PDF sendiri berkebalikan (y=0 di BAWAH), jadi translasi-Y
  // harus dikonversi lewat (1 - crop.y - crop.h), bukan crop.y langsung.
  const crop = an.cropSrc || an.crop || { x: 0, y: 0, w: 1, h: 1 };
  const sx = an.w / crop.w, sy = an.h / crop.h;
  const tx = -an.w * crop.x / crop.w;
  const ty = -sy * (1 - crop.y - crop.h);
  const ap = ctx.stream(
    `q 0 0 ${f(an.w)} ${f(an.h)} re W n ` +
    `${f(sx)} 0 0 ${f(sy)} ${f(tx)} ${f(ty)} cm /Im0 Do Q`,
    { Type: "XObject", Subtype: "Form",
      BBox: [0, 0, an.w, an.h],
      Resources: { XObject: { Im0: img.ref } } }
  );
  const apRef = ctx.register(ap);
  const yBottom = pageH - an.y - an.h;
  return ctx.obj({
    Type: "Annot", Subtype: "Stamp",
    Rect: [an.x, yBottom, an.x + an.w, yBottom + an.h],
    F: 4,
    T: PDFString.of(TAG),
    RC: PDFString.of(JSON.stringify({
      id: an.id, kind: "image", mime: an.mime, dataUrl: an.dataUrl,
      crop: an.crop || null, cropSrc: an.cropSrc || null,
    })),
    AP: { N: apRef },
  });
}

const f = (n) => (Math.round(n * 1000) / 1000).toString();
