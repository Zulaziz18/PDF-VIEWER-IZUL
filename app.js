// ============================================================
// PDF Studio Izul v6.2 - aplikasi utama (arsitektur anotasi hidup)
// ============================================================
console.log("%c[PDF Studio Izul v6.2] app.js versi terbaru berhasil dimuat.", "color:#0a0;font-weight:bold;font-size:13px");
import * as pdfjsLib from "./vendor/pdf.min.mjs";
import {
  HIGHLIGHT_COLORS, TEXT_COLORS, hexToRgb01, wrapText,
  applyResize, mergeRects, clamp,
} from "./core.js";
import {
  extractAnnots, buildPdfWithAnnots, fontKeyOf, LINE_HEIGHT,
} from "./annots.js";

pdfjsLib.GlobalWorkerOptions.workerSrc = "./vendor/pdf.worker.min.mjs";
const { TextLayer } = pdfjsLib;
const PDFLib = window.PDFLib;
const { PDFDocument, StandardFonts } = PDFLib;

// ---------- STATE ----------
const S = {
  baseBytes: null,       // PDF asli tanpa anotasi kita (tidak pernah berubah saat mengedit)
  pdfjsDoc: null,
  fileHandle: null,
  fileName: "dokumen.pdf",
  zoom: 1.15,
  dirty: false,
  annots: [],            // anotasi hidup
  selectedId: null,
  mode: null,            // null | 'text' | 'image' | 'highlight' | 'erase'
  undo: [], redo: [],
  pages: [],
  currentPage: 0,
  hlColor: HIGHLIGHT_COLORS[0].hex,
  hlFree: false,
  textColor: "#000000",
  textSize: 16,
  measureFonts: null,
  hasFS: "showOpenFilePicker" in window,
};

const el = (id) => document.getElementById(id);
const viewer = el("viewer-wrap");
const pagesCol = el("pages-column");
let uid = 0;
const newId = () => "a" + (Date.now().toString(36)) + (uid++);

// ============================================================
//  UI DASAR
// ============================================================
function toast(msg, ms = 2500) {
  const t = el("toast");
  t.textContent = msg;
  t.classList.add("show");
  clearTimeout(toast._t);
  toast._t = setTimeout(() => t.classList.remove("show"), ms);
}
const setStatus = (m) => (el("status-text").textContent = m);

function modal(title, body, buttons) {
  return new Promise((res) => {
    el("modal-title").textContent = title;
    el("modal-body").textContent = body;
    const a = el("modal-actions");
    a.innerHTML = "";
    buttons.forEach((b) => {
      const btn = document.createElement("button");
      btn.textContent = b.label;
      btn.className = b.cls || "btn-cancel";
      btn.onclick = () => { el("modal-overlay").classList.remove("show"); res(b.val); };
      a.appendChild(btn);
    });
    el("modal-overlay").classList.add("show");
  });
}
const ask = (title, body, label = "Ya", danger = false) =>
  modal(title, body, [
    { label: "Batal", val: false, cls: "btn-cancel" },
    { label, val: true, cls: danger ? "btn-danger" : "btn-confirm" },
  ]);

// ============================================================
//  FONT PENGUKUR (agar tampilan layar = hasil PDF)
// ============================================================
async function initFonts() {
  const d = await PDFDocument.create();
  S.measureFonts = {
    regular: await d.embedFont(StandardFonts.TimesRoman),
    bold: await d.embedFont(StandardFonts.TimesRomanBold),
    italic: await d.embedFont(StandardFonts.TimesRomanItalic),
    boldItalic: await d.embedFont(StandardFonts.TimesRomanBoldItalic),
  };
}
const measurer = (bold, italic, size) => {
  const f = S.measureFonts[fontKeyOf(bold, italic)];
  return (s) => f.widthOfTextAtSize(s, size);
};

// ============================================================
//  BUKA / TUTUP FILE
// ============================================================
async function openPdf() {
  if (S.dirty && !(await ask("Perubahan belum disimpan",
      "Buka file lain tanpa menyimpan?", "Lanjutkan", true))) return;
  try {
    if (S.hasFS) {
      const [h] = await window.showOpenFilePicker({
        types: [{ description: "PDF", accept: { "application/pdf": [".pdf"] } }] });
      const file = await h.getFile();
      await loadDoc(new Uint8Array(await file.arrayBuffer()), file.name, h);
    } else {
      el("file-input").value = "";
      el("file-input").click();
    }
  } catch (e) { if (e.name !== "AbortError") toast("Gagal membuka: " + e.message); }
}
el("file-input").addEventListener("change", async (ev) => {
  const f = ev.target.files[0];
  if (f) await loadDoc(new Uint8Array(await f.arrayBuffer()), f.name, null);
});

async function loadDoc(bytes, name, handle) {
  setStatus("Memuat PDF...");
  try {
    const { annots, cleanBytes } = await extractAnnots(PDFLib, bytes);
    S.baseBytes = cleanBytes;
    S.annots = annots;
    S.fileName = name;
    S.fileHandle = handle;
    S.dirty = false;
    S.undo = []; S.redo = [];
    S.selectedId = null;
    el("file-chip-text").textContent = name;
    setEnabled(true);
    await buildViewer();
    await buildOutline();
    setStatus(annots.length
      ? `PDF dimuat - ${S.pages.length} halaman, ${annots.length} anotasi dipulihkan.`
      : `PDF dimuat - ${S.pages.length} halaman.`);
    updateUndoBtns();
  } catch (e) { toast("Gagal memuat PDF: " + e.message); }
}

async function closePdf() {
  if (S.dirty && !(await ask("Perubahan belum disimpan",
      "Tutup PDF tanpa menyimpan?", "Tutup", true))) return;
  S.baseBytes = null; S.pdfjsDoc = null; S.annots = []; S.pages = [];
  S.selectedId = null; S.dirty = false; S.undo = []; S.redo = [];
  S.fileHandle = null; S.fileName = "dokumen.pdf";
  setMode(null);
  pagesCol.innerHTML = "";
  pagesCol.style.display = "none";
  el("empty-state").style.display = "flex";
  el("file-chip-text").textContent = "Belum ada PDF dibuka";
  el("dirty-dot").classList.remove("show");
  el("outline-list").innerHTML = '<div class="outline-empty">Belum ada dokumen</div>';
  el("outline-hint").textContent = "Buka PDF untuk melihat daftar judul.";
  el("page-indicator").textContent = "- / -";
  setEnabled(false);
  updateUndoBtns();
  setStatus("PDF ditutup.");
}
el("btn-close").addEventListener("click", closePdf);

// ============================================================
//  RENDER HALAMAN
// ============================================================
let io = null;
async function buildViewer() {
  el("empty-state").style.display = "none";
  pagesCol.style.display = "flex";
  pagesCol.innerHTML = "";
  S.pages = [];

  S.pdfjsDoc = await pdfjsLib.getDocument({ data: new Uint8Array(S.baseBytes) }).promise;

  for (let i = 0; i < S.pdfjsDoc.numPages; i++) {
    const proxy = await S.pdfjsDoc.getPage(i + 1);
    const vp = proxy.getViewport({ scale: 1 });
    const wrap = document.createElement("div");
    wrap.className = "page-wrap";
    wrap.dataset.pi = i;
    const canvas = document.createElement("canvas");
    canvas.className = "page-canvas";
    const tl = document.createElement("div");
    tl.className = "textLayer";
    const ov = document.createElement("div");
    ov.className = "annot-layer";
    const num = document.createElement("div");
    num.className = "page-num";
    num.textContent = `Halaman ${i + 1}`;
    wrap.append(canvas, tl, ov, num);
    pagesCol.appendChild(wrap);

    const p = { index: i, widthPt: vp.width, heightPt: vp.height, proxy,
                wrapEl: wrap, canvasEl: canvas, tlEl: tl, ovEl: ov,
                rendered: false, textDone: false };
    sizePage(p);
    S.pages.push(p);
  }

  if (io) io.disconnect();
  io = new IntersectionObserver((ents) => {
    ents.forEach((e) => {
      const p = S.pages[+e.target.dataset.pi];
      if (e.isIntersecting) renderPage(p); else clearPage(p);
    });
    updateIndicator();
  }, { root: viewer, rootMargin: "800px 0px", threshold: 0 });
  S.pages.forEach((p) => io.observe(p.wrapEl));

  renderAllAnnots();
  updateIndicator();
}

function sizePage(p) {
  const w = p.widthPt * S.zoom, h = p.heightPt * S.zoom;
  p.wrapEl.style.width = w + "px";
  p.wrapEl.style.height = h + "px";
  const dpr = window.devicePixelRatio || 1;
  p.canvasEl.style.width = w + "px";
  p.canvasEl.style.height = h + "px";
  p.canvasEl.width = Math.round(w * dpr);
  p.canvasEl.height = Math.round(h * dpr);
  p.tlEl.style.setProperty("--total-scale-factor", S.zoom);
}

async function renderPage(p) {
  if (p.rendered) return;
  const dpr = window.devicePixelRatio || 1;
  try {
    await p.proxy.render({
      canvasContext: p.canvasEl.getContext("2d"),
      viewport: p.proxy.getViewport({ scale: S.zoom * dpr }),
    }).promise;
    p.rendered = true;
  } catch { return; }
  if (!p.textDone) {
    try {
      p.tlEl.innerHTML = "";
      await new TextLayer({
        textContentSource: await p.proxy.getTextContent(),
        container: p.tlEl,
        viewport: p.proxy.getViewport({ scale: S.zoom }),
      }).render();
      p.textDone = true;
    } catch {}
  }
}
function clearPage(p) {
  if (S.annots.some((a) => a.pageIndex === p.index && a.id === S.selectedId)) return;
  p.rendered = false;
  p.canvasEl.getContext("2d").clearRect(0, 0, p.canvasEl.width, p.canvasEl.height);
}

function updateIndicator() {
  if (!S.pages.length) return;
  const wr = viewer.getBoundingClientRect();
  let best = 0, bd = Infinity;
  S.pages.forEach((p, i) => {
    const r = p.wrapEl.getBoundingClientRect();
    const d = Math.abs(r.top - wr.top);
    if (r.bottom > wr.top && r.top < wr.bottom && d < bd) { bd = d; best = i; }
  });
  S.currentPage = best;
  el("page-indicator").textContent = `Halaman ${best + 1} / ${S.pages.length}`;
}
viewer.addEventListener("scroll", () => {
  clearTimeout(viewer._t);
  viewer._t = setTimeout(() => { updateIndicator(); posToolbar(); }, 70);
});

async function setZoom(z) {
  if (!S.pages.length) return;
  S.zoom = clamp(z, 0.4, 3.0);
  el("zoom-label").textContent = Math.round(S.zoom * 100) + "%";
  S.pages.forEach((p) => {
    sizePage(p); p.rendered = false; p.textDone = false; p.tlEl.innerHTML = "";
  });
  renderAllAnnots();
  const wr = viewer.getBoundingClientRect();
  for (const p of S.pages) {
    const r = p.wrapEl.getBoundingClientRect();
    if (r.bottom > wr.top - 800 && r.top < wr.bottom + 800) await renderPage(p);
  }
  posToolbar();
}
el("btn-zoom-in").addEventListener("click", () => setZoom(S.zoom + 0.15));
el("btn-zoom-out").addEventListener("click", () => setZoom(S.zoom - 0.15));
viewer.addEventListener("wheel", (e) => {
  if (e.ctrlKey) { e.preventDefault(); setZoom(S.zoom - e.deltaY * 0.01); }
}, { passive: false });

const goPage = (i) => {
  const p = S.pages[clamp(i, 0, S.pages.length - 1)];
  if (p) p.wrapEl.scrollIntoView({ behavior: "smooth", block: "start" });
};

// ============================================================
//  DAFTAR JUDUL
// ============================================================
const CH = "^\\s*((BAB|Bab)\\s+[IVXLCDM0-9]+|(CHAPTER|Chapter)\\s+[IVXLCDM0-9]+|(BAGIAN|Bagian)\\s+[IVXLCDM0-9]+|(DAFTAR|KATA|ABSTRAK|ABSTRACT|PENDAHULUAN|KESIMPULAN|PENUTUP|LAMPIRAN|REFERENSI|DAFTAR PUSTAKA)\\b.*)";
export function pageText(items) {
  let t = "";
  for (const it of items) { t += it.str; t += it.hasEOL ? "\n" : " "; }
  return t;
}
export function findChapters(text) {
  const re = new RegExp(CH, "gm");
  const out = []; let m;
  while ((m = re.exec(text)) !== null) {
    let j = m[0].trim();
    if (j.length < 40) {
      const st = m.index + m[0].length;
      const L = text.slice(st).split("\n");
      let nx = "", used = 0;
      if (L[0]?.trim()) { nx = L[0].trim(); used = L[0].length; }
      else if (L.length > 1) { nx = L[1].trim(); used = L[0].length + 1 + L[1].length; }
      if (nx) { j += " - " + nx.slice(0, 50); const e = st + used; if (e > re.lastIndex) re.lastIndex = e; }
    }
    out.push(j.slice(0, 90));
    if (re.lastIndex === m.index) re.lastIndex++;
  }
  return out;
}
async function buildOutline() {
  let items = [];
  try {
    const o = await S.pdfjsDoc.getOutline();
    if (o?.length) for (const it of o.slice(0, 80)) {
      const pi = await destPage(it.dest);
      if (pi !== null) items.push({ title: it.title.trim(), page: pi });
    }
  } catch {}
  if (!items.length) {
    for (let i = 0; i < S.pdfjsDoc.numPages; i++) {
      const t = pageText((await (await S.pdfjsDoc.getPage(i + 1)).getTextContent()).items);
      for (const title of findChapters(t)) items.push({ title, page: i });
    }
  }
  if (!items.length) {
    for (let i = 0; i < S.pdfjsDoc.numPages; i++) {
      const tc = await (await S.pdfjsDoc.getPage(i + 1)).getTextContent();
      const f = tc.items.find((x) => x.str.trim().length > 3);
      if (f) items.push({ title: f.str.trim().slice(0, 70), page: i });
    }
  }
  const list = el("outline-list");
  list.innerHTML = "";
  if (!items.length) {
    el("outline-hint").textContent = "Tidak ada judul terdeteksi.";
    list.innerHTML = '<div class="outline-empty">Tidak ada judul</div>';
    return;
  }
  el("outline-hint").textContent = `${items.length} judul. Klik untuk lompat.`;
  items.forEach((it) => {
    const d = document.createElement("div");
    d.className = "outline-item";
    d.innerHTML = `<span>${esc(it.title)}</span><span class="pg">${it.page + 1}</span>`;
    d.onclick = () => goPage(it.page);
    list.appendChild(d);
  });
}
async function destPage(dest) {
  try {
    let d = dest;
    if (typeof d === "string") d = await S.pdfjsDoc.getDestination(d);
    return d ? await S.pdfjsDoc.getPageIndex(d[0]) : null;
  } catch { return null; }
}
const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

// ============================================================
//  MODE
// ============================================================
function setMode(m) {
  S.mode = m;
  ["btn-text", "btn-image", "btn-highlight", "btn-erase"]
    .forEach((id) => el(id).classList.remove("active"));
  if (m === "text") el("btn-text").classList.add("active");
  if (m === "image") el("btn-image").classList.add("active");
  if (m === "highlight") el("btn-highlight").classList.add("active");
  if (m === "erase") el("btn-erase").classList.add("active");

  el("hl-panel").classList.toggle("show", m === "highlight");
  viewer.className = "";
  viewer.id = "viewer-wrap";
  if (m === "text" || m === "image") viewer.classList.add("cur-place");
  if (m === "erase") viewer.classList.add("cur-erase");
  if (m === "highlight" && S.hlFree) viewer.classList.add("cur-cross");

  if (m) select(null);
  setStatus({
    text: "Klik di halaman untuk menaruh kotak teks.",
    image: "Pilih gambar terlebih dahulu.",
    highlight: S.hlFree ? "Seret untuk menyorot area bebas."
                        : "Sapu teks atau klik dua kali pada kata untuk menyorot.",
    erase: "Klik pada anotasi yang ingin dihapus.",
  }[m] || "Siap.");
}
el("btn-text").addEventListener("click", () => setMode(S.mode === "text" ? null : "text"));
el("btn-highlight").addEventListener("click", () => setMode(S.mode === "highlight" ? null : "highlight"));
el("btn-erase").addEventListener("click", () => setMode(S.mode === "erase" ? null : "erase"));
el("btn-image").addEventListener("click", () => { el("img-input").value = ""; el("img-input").click(); });

// ============================================================
//  RENDER ANOTASI KE OVERLAY
// ============================================================
function renderAllAnnots() {
  S.pages.forEach((p) => (p.ovEl.innerHTML = ""));
  S.annots.forEach(drawAnnot);
  if (S.selectedId) decorateSelected();
}

function drawAnnot(a) {
  const p = S.pages[a.pageIndex];
  if (!p) return;
  const z = S.zoom;

  if (a.type === "highlight") {
    const g = document.createElement("div");
    g.className = "an an-hl";
    g.dataset.id = a.id;
    (a.rects || []).forEach((r) => {
      const d = document.createElement("div");
      d.className = "hl-rect";
      d.style.left = r.x * z + "px";
      d.style.top = r.y * z + "px";
      d.style.width = r.width * z + "px";
      d.style.height = r.height * z + "px";
      d.style.background = a.color;
      g.appendChild(d);
    });
    p.ovEl.appendChild(g);
    a.el = g;
    return;
  }

  const n = document.createElement("div");
  n.className = "an an-" + a.type;
  n.dataset.id = a.id;
  n.style.left = a.x * z + "px";
  n.style.top = a.y * z + "px";
  n.style.width = a.w * z + "px";

  if (a.type === "text") {
    const b = document.createElement("div");
    b.className = "an-text-body";
    b.style.fontSize = a.fontSize * z + "px";
    b.style.lineHeight = LINE_HEIGHT;
    b.style.color = a.color;
    b.style.fontWeight = a.bold ? "700" : "400";
    b.style.fontStyle = a.italic ? "italic" : "normal";
    const lines = wrapText(a.text || "", measurer(a.bold, a.italic, a.fontSize), a.w);
    a.lines = lines;
    lines.forEach((ln) => {
      const s = document.createElement("div");
      s.className = "an-line";
      s.textContent = ln === "" ? "\u00A0" : ln;
      b.appendChild(s);
    });
    a.h = Math.max(lines.length, 1) * a.fontSize * LINE_HEIGHT;
    n.style.height = a.h * z + "px";
    n.appendChild(b);
    a.bodyEl = b;
  } else {
    n.style.height = a.h * z + "px";
    const im = document.createElement("img");
    im.className = "an-img";
    im.src = a.dataUrl;
    im.draggable = false;
    if (a.cropSrc) applyCropStyle(im, a.cropSrc);
    n.appendChild(im);
  }
  p.ovEl.appendChild(n);
  a.el = n;
}

function redrawAnnot(a) {
  a.el?.remove();
  drawAnnot(a);
  if (S.selectedId === a.id) decorateSelected();
}

// ============================================================
//  SELEKSI ANOTASI
// ============================================================
function select(id) {
  if (S.selectedId === id) { if (id) posToolbar(); return; }
  const prev = getA(S.selectedId);
  if (prev?.el) {
    prev.el.classList.remove("selected");
    prev.el.querySelectorAll(".handle").forEach((h) => h.remove());
  }
  S.selectedId = id;
  if (!id) { el("an-toolbar").classList.remove("show"); return; }
  decorateSelected();
  showToolbar();
}
const getA = (id) => S.annots.find((a) => a.id === id);

function decorateSelected() {
  const a = getA(S.selectedId);
  if (!a?.el) return;
  a.el.classList.add("selected");
  a.el.querySelectorAll(".handle").forEach((h) => h.remove());
  if (a.type === "highlight") return;  // highlight tidak di-resize
  const hs = a.type === "text" ? ["nw","ne","se","sw","e","w"]
                               : ["nw","n","ne","e","se","s","sw","w"];
  hs.forEach((h) => {
    const d = document.createElement("div");
    d.className = "handle handle-" + h;
    d.dataset.handle = h;
    a.el.appendChild(d);
  });
}

// klik di halaman
viewer.addEventListener("mousedown", (e) => {
  const anEl = e.target.closest(".an");
  if (!anEl) {
    if (!e.target.closest("#an-toolbar")) select(null);
    return;
  }
  const a = getA(anEl.dataset.id);
  if (!a) return;

  if (S.mode === "erase") { e.preventDefault(); removeAnnot(a.id); return; }
  if (S.mode === "text" || S.mode === "image") return;

  e.preventDefault();
  select(a.id);
  if (a.type === "highlight") return;
  startDrag(e, a);
});

// ============================================================
//  DRAG & RESIZE
// ============================================================
let drag = null;
function startDrag(e, a) {
  const handle = e.target.dataset.handle || null;
  drag = { a, handle, mx: e.clientX, my: e.clientY,
           box: { x: a.x, y: a.y, w: a.w, h: a.h }, fs: a.fontSize };
  document.addEventListener("mousemove", onDrag);
  document.addEventListener("mouseup", endDrag);
}
function onDrag(e) {
  if (!drag) return;
  const { a, handle, box } = drag;
  const dx = (e.clientX - drag.mx) / S.zoom;
  const dy = (e.clientY - drag.my) / S.zoom;

  if (!handle) { a.x = box.x + dx; a.y = box.y + dy; }
  else if (a.type === "text") {
    if (handle === "e" || handle === "w") {
      const r = applyResize(handle, box, dx, dy, false, 30);
      a.x = r.x; a.w = r.w;
    } else {
      const r = applyResize(handle, box, dx, dy, true, 16);
      a.fontSize = clamp(drag.fs * (r.h / box.h), 5, 200);
      a.w = r.w; a.x = r.x;
      if (handle.includes("n")) a.y = r.y;
      el("an-size").value = Math.round(a.fontSize);
    }
  } else {
    const r = applyResize(handle, box, dx, dy, handle.length === 2, 16);
    Object.assign(a, r);
  }
  clampToPage(a);
  redrawAnnot(a);
  posToolbar();
}
function endDrag() {
  if (drag) { markDirty(); pushUndo(); }
  drag = null;
  document.removeEventListener("mousemove", onDrag);
  document.removeEventListener("mouseup", endDrag);
}
function clampToPage(a) {
  const p = S.pages[a.pageIndex];
  a.x = clamp(a.x, -a.w * 0.6, p.widthPt - a.w * 0.4);
  a.y = clamp(a.y, -a.h * 0.6, p.heightPt - a.h * 0.4);
}

// ============================================================
//  TOOLBAR ANOTASI TERPILIH
// ============================================================
function showToolbar() {
  const a = getA(S.selectedId);
  if (!a) return;
  const tb = el("an-toolbar");
  tb.classList.add("show");
  el("an-text-tools").style.display = a.type === "text" ? "flex" : "none";
  el("an-hl-tools").style.display = a.type === "highlight" ? "flex" : "none";
  el("an-img-tools").style.display = a.type === "image" ? "flex" : "none";

  if (a.type === "text") {
    el("an-size").value = Math.round(a.fontSize);
    el("an-bold").classList.toggle("on", a.bold);
    el("an-italic").classList.toggle("on", a.italic);
    buildColorDropdown(el("an-color-dd"), TEXT_COLORS, a.color, (hex) => {
      a.color = hex; S.textColor = hex; redrawAnnot(a); markDirty(); pushUndo();
    });
  }
  if (a.type === "highlight") {
    buildColorDropdown(el("an-hl-color-dd"), HIGHLIGHT_COLORS, a.color, (hex) => {
      a.color = hex; redrawAnnot(a); markDirty(); pushUndo();
    });
  }
  posToolbar();
}
function posToolbar() {
  const a = getA(S.selectedId);
  const tb = el("an-toolbar");
  if (!a?.el) { tb.classList.remove("show"); return; }
  const r = a.el.getBoundingClientRect();
  const vr = viewer.getBoundingClientRect();
  if (r.bottom < vr.top || r.top > vr.bottom) { tb.style.opacity = "0"; return; }
  tb.style.opacity = "1";
  const h = tb.offsetHeight || 42;
  let top = r.top - h - 10;
  if (top < vr.top + 6) top = r.bottom + 10;
  tb.style.top = top + "px";
  tb.style.left = clamp(r.left, 12, window.innerWidth - tb.offsetWidth - 12) + "px";
}
window.addEventListener("resize", posToolbar);

// ---- dropdown warna ----
function buildColorDropdown(host, colors, active, onPick) {
  host.innerHTML = "";
  const cur = colors.find((c) => c.hex.toLowerCase() === String(active).toLowerCase()) || colors[0];
  const btn = document.createElement("button");
  btn.className = "cdd-btn";
  btn.innerHTML = `<span class="cdd-dot" style="background:${cur.hex}"></span><span>${cur.name}</span><span class="cdd-arrow">▾</span>`;
  const menu = document.createElement("div");
  menu.className = "cdd-menu";
  colors.forEach((c) => {
    const it = document.createElement("button");
    it.className = "cdd-item" + (c.hex === cur.hex ? " on" : "");
    it.innerHTML = `<span class="cdd-dot" style="background:${c.hex}"></span><span>${c.name}</span>`;
    it.onclick = (ev) => { ev.stopPropagation(); menu.classList.remove("open"); onPick(c.hex); };
    menu.appendChild(it);
  });
  btn.onclick = (ev) => {
    ev.stopPropagation();
    document.querySelectorAll(".cdd-menu.open").forEach((m) => m !== menu && m.classList.remove("open"));
    menu.classList.toggle("open");
  };
  host.append(btn, menu);
}
document.addEventListener("click", () =>
  document.querySelectorAll(".cdd-menu.open").forEach((m) => m.classList.remove("open")));

// ---- kontrol toolbar ----
el("an-size").addEventListener("input", (e) => {
  const a = getA(S.selectedId);
  if (a?.type !== "text") return;
  a.fontSize = clamp(parseFloat(e.target.value) || 16, 5, 200);
  S.textSize = a.fontSize;
  redrawAnnot(a); posToolbar(); markDirty();
});
el("an-size").addEventListener("change", pushUndo);
el("an-bold").addEventListener("click", () => {
  const a = getA(S.selectedId);
  if (a?.type !== "text") return;
  a.bold = !a.bold;
  el("an-bold").classList.toggle("on", a.bold);
  redrawAnnot(a); markDirty(); pushUndo();
});
el("an-italic").addEventListener("click", () => {
  const a = getA(S.selectedId);
  if (a?.type !== "text") return;
  a.italic = !a.italic;
  el("an-italic").classList.toggle("on", a.italic);
  redrawAnnot(a); markDirty(); pushUndo();
});
el("an-edit").addEventListener("click", () => editText(getA(S.selectedId)));
el("an-delete").addEventListener("click", () => removeAnnot(S.selectedId));

// ============================================================
//  EDIT TEKS
// ============================================================
function editText(a) {
  if (!a || a.type !== "text" || !a.bodyEl) return;
  const b = a.bodyEl;
  a.editing = true;
  b.contentEditable = "true";
  b.classList.add("editing");
  b.innerHTML = "";
  String(a.text || "").split("\n").forEach((ln) => {
    const d = document.createElement("div");
    d.textContent = ln === "" ? "\u00A0" : ln;
    b.appendChild(d);
  });
  b.focus();
  const rg = document.createRange();
  rg.selectNodeContents(b);
  rg.collapse(false);
  const sel = window.getSelection();
  sel.removeAllRanges(); sel.addRange(rg);

  const done = () => {
    a.text = (b.innerText || "").replace(/\u00A0/g, " ").trimEnd();
    a.editing = false;
    b.contentEditable = "false";
    b.classList.remove("editing");
    b.removeEventListener("blur", done);
    if (!a.text.trim()) { removeAnnot(a.id); return; }
    redrawAnnot(a); posToolbar(); markDirty(); pushUndo();
  };
  b.addEventListener("blur", done);
}

// ============================================================
//  BUAT ANOTASI BARU
// ============================================================
viewer.addEventListener("click", (e) => {
  if (S.mode !== "text") return;
  if (e.target.closest(".an") || e.target.closest("#an-toolbar")) return;
  const wEl = e.target.closest(".page-wrap");
  if (!wEl) return;
  const p = S.pages[+wEl.dataset.pi];
  const r = p.canvasEl.getBoundingClientRect();
  const x = (e.clientX - r.left) / S.zoom;
  const y = (e.clientY - r.top) / S.zoom;
  const a = {
    id: newId(), type: "text", pageIndex: p.index,
    x, y, w: Math.max(Math.min(280, p.widthPt - x - 12), 90),
    h: S.textSize * LINE_HEIGHT,
    text: "Ketik di sini", fontSize: S.textSize, color: S.textColor,
    bold: false, italic: false,
  };
  S.annots.push(a);
  drawAnnot(a);
  setMode(null);
  select(a.id);
  editText(a);
  markDirty();
});

el("img-input").addEventListener("change", async (ev) => {
  const file = ev.target.files[0];
  if (!file) return;
  if (!/image\/(png|jpeg)/.test(file.type)) { toast("Hanya PNG atau JPG."); return; }
  const dataUrl = await fileToDataUrl(file);
  await createImageAnnot(dataUrl, file.type);
});

async function fileToDataUrl(file) {
  return new Promise((res) => {
    const fr = new FileReader();
    fr.onload = () => res(fr.result);
    fr.readAsDataURL(file);
  });
}
function loadImageDim(dataUrl) {
  return new Promise((res) => {
    const im = new Image();
    im.onload = () => res({ w: im.naturalWidth, h: im.naturalHeight });
    im.src = dataUrl;
  });
}
async function createImageAnnot(dataUrl, mime) {
  if (!S.pages.length) { toast("Buka PDF terlebih dahulu."); return; }
  const dim = await loadImageDim(dataUrl);
  const p = S.pages[S.currentPage] || S.pages[0];
  const sc = Math.min((p.widthPt * 0.45) / dim.w, 1);
  const w = dim.w * sc, h = dim.h * sc;
  const a = {
    id: newId(), type: "image", pageIndex: p.index,
    x: (p.widthPt - w) / 2, y: (p.heightPt - h) / 3,
    w, h, dataUrl, mime, crop: null,
  };
  S.annots.push(a);
  drawAnnot(a);
  setMode(null);
  goPage(p.index);
  select(a.id);
  markDirty(); pushUndo();
  setStatus("Geser gambar, tarik titik sudut untuk mengubah ukuran.");
  return a;
}

// ============================================================
//  CROP GAMBAR (setelah ditempel, tetap bisa diubah lagi)
// ============================================================
function applyCropStyle(imgEl, crop) {
  // trik: perbesar gambar sesuai kebalikan crop, lalu geser -
  // area di luar tetap "ada" tapi disembunyikan oleh overflow:hidden induknya.
  const sx = 1 / crop.w, sy = 1 / crop.h;
  imgEl.style.width = sx * 100 + "%";
  imgEl.style.height = sy * 100 + "%";
  imgEl.style.maxWidth = "none";
  imgEl.style.position = "absolute";
  imgEl.style.left = -crop.x * sx * 100 + "%";
  imgEl.style.top = -crop.y * sy * 100 + "%";
}

let cropCtx = null;
function startCrop() {
  const a = getA(S.selectedId);
  if (!a || a.type !== "image") return;
  el("an-toolbar").classList.remove("show");
  const box = document.createElement("div");
  box.className = "crop-box";
  const c0 = a.crop || { x: 0, y: 0, w: 1, h: 1 };
  const setBoxStyle = () => {
    box.style.left = c0.x * a.w * S.zoom + "px";
    box.style.top = c0.y * a.h * S.zoom + "px";
    box.style.width = c0.w * a.w * S.zoom + "px";
    box.style.height = c0.h * a.h * S.zoom + "px";
  };
  setBoxStyle();
  ["nw","n","ne","e","se","s","sw","w"].forEach((h) => {
    const hd = document.createElement("div");
    hd.className = "handle handle-" + h;
    hd.dataset.handle = h;
    box.appendChild(hd);
  });
  a.el.appendChild(box);
  a.el.classList.add("cropping");

  const bar = document.createElement("div");
  bar.className = "crop-bar";
  bar.innerHTML = `<button class="mini-btn apply" id="crop-ok">✓ Terapkan Crop</button>
                    <button class="mini-btn" id="crop-cancel">✕ Batal</button>`;
  a.el.appendChild(bar);
  posCropBar(a, bar);

  cropCtx = { a, box, c0: { ...c0 }, dragging: null };
  box.addEventListener("mousedown", onCropMouseDown);
  bar.querySelector("#crop-ok").onclick = () => finishCrop(true);
  bar.querySelector("#crop-cancel").onclick = () => finishCrop(false);
}
function posCropBar(a, bar) {
  const r = a.el.getBoundingClientRect();
  bar.style.top = (r.bottom + 8) + "px";
  bar.style.left = clamp(r.left, 12, window.innerWidth - 260) + "px";
}
function onCropMouseDown(e) {
  e.stopPropagation(); e.preventDefault();
  cropCtx.dragging = {
    handle: e.target.dataset.handle || null,
    mx: e.clientX, my: e.clientY, start: { ...cropCtx.c0 },
  };
  document.addEventListener("mousemove", onCropMouseMove);
  document.addEventListener("mouseup", onCropMouseUp);
}
function onCropMouseMove(e) {
  if (!cropCtx?.dragging) return;
  const { a, box } = cropCtx;
  const { handle, mx, my, start } = cropCtx.dragging;
  const dx = (e.clientX - mx) / (a.w * S.zoom);
  const dy = (e.clientY - my) / (a.h * S.zoom);
  let { x, y, w, h } = start;

  if (!handle) {
    x = clamp(start.x + dx, 0, 1 - start.w);
    y = clamp(start.y + dy, 0, 1 - start.h);
  } else {
    if (handle.includes("e")) w = clamp(start.w + dx, 0.05, 1 - start.x);
    if (handle.includes("s")) h = clamp(start.h + dy, 0.05, 1 - start.y);
    if (handle.includes("w")) { const nx = clamp(start.x + dx, 0, start.x + start.w - 0.05); w = start.w + (start.x - nx); x = nx; }
    if (handle.includes("n")) { const ny = clamp(start.y + dy, 0, start.y + start.h - 0.05); h = start.h + (start.y - ny); y = ny; }
  }
  cropCtx.c0 = { x, y, w, h };
  box.style.left = x * a.w * S.zoom + "px";
  box.style.top = y * a.h * S.zoom + "px";
  box.style.width = w * a.w * S.zoom + "px";
  box.style.height = h * a.h * S.zoom + "px";
}
function onCropMouseUp() {
  if (cropCtx) cropCtx.dragging = null;
  document.removeEventListener("mousemove", onCropMouseMove);
  document.removeEventListener("mouseup", onCropMouseUp);
}
function finishCrop(apply) {
  if (!cropCtx) return;
  const { a } = cropCtx;
  if (apply) {
    const c = cropCtx.c0;
    // Simpan dimensi & posisi ASLI (sebelum crop ini) sekali saja, supaya
    // "Reset Crop" bisa mengembalikan bingkai persis seperti semula —
    // walau crop diterapkan berkali-kali secara berurutan.
    if (!a.cropOrig) {
      a.cropOrig = { x: a.x, y: a.y, w: a.w, h: a.h };
    }
    // Perkecil BINGKAI (frame) mengikuti area yang dipotong, supaya yang
    // tampil di kanvas langsung berupa hasil crop saja (bukan jendela
    // kecil di dalam bingkai besar yang lama).
    const newW = a.w * c.w, newH = a.h * c.h;
    a.x = a.x + c.x * a.w;
    a.y = a.y + c.y * a.h;
    a.w = newW;
    a.h = newH;
    // crop sekarang menutupi 100% bingkai baru; offset relatif terhadap
    // gambar SUMBER asli tetap disimpan di cropSrc untuk keperluan reset/edit ulang.
    a.cropSrc = a.cropSrc
      ? { x: a.cropSrc.x + c.x * a.cropSrc.w, y: a.cropSrc.y + c.y * a.cropSrc.h,
          w: a.cropSrc.w * c.w, h: a.cropSrc.h * c.h }
      : { ...c };
    // crop = null supaya applyCropStyle tidak melakukan scaling lagi —
    // bingkai (a.w/a.h) sudah pas dengan area crop, gambar cukup ditampilkan penuh.
    a.crop = null;
    markDirty(); pushUndo();
  }
  a.el.classList.remove("cropping");
  document.removeEventListener("mousemove", onCropMouseMove);
  document.removeEventListener("mouseup", onCropMouseUp);
  cropCtx = null;
  redrawAnnot(a);
  select(a.id);
}
el("an-crop").addEventListener("click", startCrop);
el("an-crop-reset").addEventListener("click", () => {
  const a = getA(S.selectedId);
  if (!a || a.type !== "image") return;
  if (a.cropOrig) {
    a.x = a.cropOrig.x; a.y = a.cropOrig.y;
    a.w = a.cropOrig.w; a.h = a.cropOrig.h;
    a.cropOrig = null;
  }
  a.crop = null;
  a.cropSrc = null;
  redrawAnnot(a);
  markDirty(); pushUndo();
  setStatus("Crop dibatalkan, gambar kembali ke ukuran & posisi semula.");
});

// ============================================================
//  HAPUS BACKGROUND (AI, lokal di browser)
// ============================================================
let bgRemovalMod = null;
async function ensureBgRemoval() {
  if (bgRemovalMod) return bgRemovalMod;
  try {
    bgRemovalMod = await import("./vendor/bg-removal.min.mjs");
  } catch (e) {
    throw new Error(
      "Fitur ini butuh paket tambahan 'Model AI' (folder vendor/bgremoval). " +
      "Download paket itu terpisah, lalu gabungkan ke folder aplikasi ini."
    );
  }
  return bgRemovalMod;
}
el("an-rmbg").addEventListener("click", async () => {
  const a = getA(S.selectedId);
  if (!a || a.type !== "image") return;
  const btn = el("an-rmbg");
  const oldLabel = btn.textContent;
  btn.disabled = true;
  try {
    setStatus("Memuat model AI (pertama kali bisa ~1 menit, ~52MB, hanya sekali)...");
    btn.textContent = "⏳ Memproses...";
    const mod = await ensureBgRemoval();
    const res = await fetch(a.dataUrl);
    const srcBlob = await res.blob();
    const outBlob = await mod.removeBackground(srcBlob, {
      publicPath: new URL("./vendor/bgremoval/", document.baseURI).href,
      model: "isnet_quint8",
      output: { format: "image/png", quality: 0.9 },
      progress: (key, cur, total) => {
        if (total) setStatus(`Mengunduh model AI: ${Math.round(cur / total * 100)}%`);
      },
    });
    const dataUrl = await new Promise((res2) => {
      const fr = new FileReader();
      fr.onload = () => res2(fr.result);
      fr.readAsDataURL(outBlob);
    });
    a.dataUrl = dataUrl;
    a.mime = "image/png";
    redrawAnnot(a);
    select(a.id);
    markDirty(); pushUndo();
    setStatus("Latar belakang berhasil dihapus.");
    toast("Latar belakang dihapus.");
  } catch (e) {
    toast("Gagal menghapus latar belakang: " + e.message);
    setStatus("Gagal menghapus latar belakang.");
  } finally {
    btn.disabled = false;
    btn.textContent = oldLabel;
  }
});

// ============================================================
//  HIGHLIGHT
// ============================================================
function initHl() {
  const host = el("hl-colors");
  host.innerHTML = "";
  HIGHLIGHT_COLORS.forEach((c) => {
    const b = document.createElement("button");
    b.className = "swatch" + (c.hex === S.hlColor ? " active" : "");
    b.style.background = c.hex;
    b.title = c.name;
    b.onclick = () => { S.hlColor = c.hex; initHl(); };
    host.appendChild(b);
  });
}
el("hl-free").addEventListener("change", (e) => {
  S.hlFree = e.target.checked;
  setMode("highlight");
});

document.addEventListener("mouseup", () => {
  if (S.mode !== "highlight" || S.hlFree || drag) return;
  setTimeout(applySelectionHighlight, 0);
});
function applySelectionHighlight() {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || !sel.rangeCount) return;
  const rg = sel.getRangeAt(0);
  const start = rg.startContainer.nodeType === 3 ? rg.startContainer.parentElement : rg.startContainer;
  const wEl = start?.closest?.(".page-wrap");
  if (!wEl) return;
  const p = S.pages[+wEl.dataset.pi];
  const cr = p.canvasEl.getBoundingClientRect();
  const z = S.zoom;
  const rects = [...rg.getClientRects()]
    .filter((r) => r.width > 0.5 && r.height > 0.5)
    .map((r) => ({ x: (r.left - cr.left) / z, y: (r.top - cr.top) / z,
                   width: r.width / z, height: r.height / z }))
    .filter((r) => r.x >= -2 && r.y >= -2 && r.x < p.widthPt + 2);
  sel.removeAllRanges();
  if (!rects.length) return;
  addHighlight(p.index, mergeRects(rects));
}

let hlDrag = null;
viewer.addEventListener("mousedown", (e) => {
  if (S.mode !== "highlight" || !S.hlFree) return;
  const wEl = e.target.closest(".page-wrap");
  if (!wEl) return;
  e.preventDefault();
  const p = S.pages[+wEl.dataset.pi];
  const r = p.canvasEl.getBoundingClientRect();
  const box = document.createElement("div");
  box.className = "hl-drag";
  box.style.background = S.hlColor;
  p.ovEl.appendChild(box);
  hlDrag = { p, x0: e.clientX - r.left, y0: e.clientY - r.top, box };
});
document.addEventListener("mousemove", (e) => {
  if (!hlDrag) return;
  const r = hlDrag.p.canvasEl.getBoundingClientRect();
  const cx = e.clientX - r.left, cy = e.clientY - r.top;
  Object.assign(hlDrag.box.style, {
    left: Math.min(hlDrag.x0, cx) + "px", top: Math.min(hlDrag.y0, cy) + "px",
    width: Math.abs(cx - hlDrag.x0) + "px", height: Math.abs(cy - hlDrag.y0) + "px",
  });
});
document.addEventListener("mouseup", (e) => {
  if (!hlDrag) return;
  const { p, x0, y0, box } = hlDrag;
  const r = p.canvasEl.getBoundingClientRect();
  const cx = e.clientX - r.left, cy = e.clientY - r.top;
  box.remove(); hlDrag = null;
  const z = S.zoom;
  const rect = { x: Math.min(x0, cx) / z, y: Math.min(y0, cy) / z,
                 width: Math.abs(cx - x0) / z, height: Math.abs(cy - y0) / z };
  if (rect.width < 3 || rect.height < 3) return;
  addHighlight(p.index, [rect]);
});

function addHighlight(pageIndex, rects) {
  const xs = rects.map((r) => r.x), ys = rects.map((r) => r.y);
  const a = {
    id: newId(), type: "highlight", pageIndex, color: S.hlColor, rects,
    x: Math.min(...xs), y: Math.min(...ys),
    w: Math.max(...rects.map((r) => r.x + r.width)) - Math.min(...xs),
    h: Math.max(...rects.map((r) => r.y + r.height)) - Math.min(...ys),
  };
  S.annots.push(a);
  drawAnnot(a);
  markDirty(); pushUndo();
  setStatus("Sorotan ditambahkan. Klik sorotan untuk mengubah warna atau menghapus.");
}

// ============================================================
//  HAPUS / UNDO / REDO
// ============================================================
function removeAnnot(id) {
  const i = S.annots.findIndex((a) => a.id === id);
  if (i < 0) return;
  S.annots[i].el?.remove();
  S.annots.splice(i, 1);
  if (S.selectedId === id) select(null);
  markDirty(); pushUndo();
  setStatus("Anotasi dihapus.");
}

const snap = () => JSON.stringify(S.annots.map((a) => {
  const { el: _e, bodyEl: _b, lines: _l, editing: _ed, ...rest } = a;
  return rest;
}));
function pushUndo() {
  const s = snap();
  if (S.undo[S.undo.length - 1] === s) return;
  S.undo.push(s);
  if (S.undo.length > 60) S.undo.shift();
  S.redo = [];
  updateUndoBtns();
}
function restore(json) {
  S.annots = JSON.parse(json);
  S.selectedId = null;
  el("an-toolbar").classList.remove("show");
  renderAllAnnots();
  markDirty();
  updateUndoBtns();
}
el("btn-undo").addEventListener("click", () => {
  if (S.undo.length < 2) {
    if (S.undo.length === 1) { S.redo.push(snap()); restore(S.undo[0]); S.undo = [S.undo[0]]; }
    return;
  }
  S.redo.push(S.undo.pop());
  restore(S.undo[S.undo.length - 1]);
});
el("btn-redo").addEventListener("click", () => {
  if (!S.redo.length) return;
  const j = S.redo.pop();
  S.undo.push(j);
  restore(j);
});
function updateUndoBtns() {
  el("btn-undo").disabled = S.undo.length < 1 || !S.baseBytes;
  el("btn-redo").disabled = !S.redo.length || !S.baseBytes;
}

function markDirty() { S.dirty = true; el("dirty-dot").classList.add("show"); }

// ============================================================
//  SIMPAN
// ============================================================
async function composePdf() {
  const needed = new Set();
  S.annots.forEach((a) => { if (a.type === "text") needed.add(fontKeyOf(a.bold, a.italic)); });
  return await buildPdfWithAnnots(PDFLib, S.baseBytes, S.annots, [...needed]);
}
async function save() {
  if (!S.baseBytes) return;
  if (!S.fileHandle) return saveAs();
  if (!(await ask("Simpan", `Timpa file "${S.fileName}"?`, "Simpan"))) return;
  try {
    setStatus("Menyimpan...");
    const bytes = await composePdf();
    const w = await S.fileHandle.createWritable();
    await w.write(bytes); await w.close();
    S.dirty = false; el("dirty-dot").classList.remove("show");
    recordHistory(S.fileName, bytes.length, null);
    toast("Tersimpan. Anotasi tetap bisa diedit saat dibuka lagi.");
    setStatus("Tersimpan.");
  } catch (e) { toast("Gagal menyimpan: " + e.message); }
}
async function saveAs() {
  if (!S.baseBytes) return;
  const name = S.fileName.replace(/\.pdf$/i, "") + "_edit.pdf";
  try {
    setStatus("Menyimpan...");
    const bytes = await composePdf();
    if (S.hasFS) {
      const h = await window.showSaveFilePicker({
        suggestedName: name,
        types: [{ description: "PDF", accept: { "application/pdf": [".pdf"] } }] });
      const w = await h.createWritable();
      await w.write(bytes); await w.close();
      S.fileHandle = h; S.fileName = h.name;
      el("file-chip-text").textContent = h.name;
      recordHistory(h.name, bytes.length, null);
    } else {
      const blob = new Blob([bytes], { type: "application/pdf" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url; a.download = name; a.click();
      recordHistory(name, bytes.length, url);
    }
    S.dirty = false; el("dirty-dot").classList.remove("show");
    toast("Tersimpan.");
    setStatus("Tersimpan.");
  } catch (e) { if (e.name !== "AbortError") toast("Gagal menyimpan: " + e.message); }
}
el("btn-save").addEventListener("click", save);
el("btn-saveas").addEventListener("click", saveAs);

// ============================================================
//  RIWAYAT SIMPAN / UNDUHAN (persisten lintas sesi via localStorage)
// ============================================================
// Catatan: blob URL (untuk tombol "Buka") hanya hidup selama sesi
// browser berjalan, jadi tidak ikut disimpan ke localStorage - yang
// disimpan cuma nama file, waktu, dan ukuran. Kalau baris riwayat
// dari sesi sebelumnya diklik "Buka", tautannya sudah tidak berlaku
// lagi (wajar, sesuai keterbatasan blob URL browser).
const DL_HISTORY_KEY = "pdfStudioIzul.downloadHistory.v1";
function loadDownloadHistory() {
  try {
    const raw = localStorage.getItem(DL_HISTORY_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch { return []; }
}
function saveDownloadHistoryToDisk() {
  try {
    const persistable = S.downloadHistory.map(({ url: _u, ...rest }) => rest);
    localStorage.setItem(DL_HISTORY_KEY, JSON.stringify(persistable.slice(0, 50)));
  } catch { /* localStorage penuh/diblokir - abaikan, riwayat tetap ada di sesi ini */ }
}
S.downloadHistory = loadDownloadHistory();
function recordHistory(name, bytesLength, url) {
  S.downloadHistory.unshift({ name, time: new Date().toISOString(), size: bytesLength, url: url || null });
  if (S.downloadHistory.length > 50) S.downloadHistory.pop();
  saveDownloadHistoryToDisk();
  renderDownloadHistory();
}
function renderDownloadHistory() {
  const host = el("dl-list");
  host.innerHTML = "";
  if (!S.downloadHistory.length) {
    host.innerHTML = '<div class="dl-empty">Belum ada riwayat.</div>';
    return;
  }
  S.downloadHistory.forEach((d) => {
    const row = document.createElement("div");
    row.className = "dl-item";
    const time = new Date(d.time).toLocaleString("id-ID",
      { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" });
    const kb = (d.size / 1024).toFixed(0);
    row.innerHTML = `<div class="dl-info"><span class="dl-name">${esc(d.name)}</span>
      <span class="dl-meta">${time} · ${kb} KB</span></div>
      ${d.url ? '<button class="mini-btn dl-open">↗ Buka</button>' : '<span class="dl-meta">(sesi lampau)</span>'}`;
    if (d.url) row.querySelector(".dl-open").onclick = () => window.open(d.url, "_blank");
    host.appendChild(row);
  });
}
renderDownloadHistory();
el("btn-dl-history").addEventListener("click", () => el("dl-panel").classList.toggle("show"));
el("dl-close").addEventListener("click", () => el("dl-panel").classList.remove("show"));
el("dl-clear-all").addEventListener("click", async () => {
  if (!(await ask("Bersihkan Riwayat", "Hapus semua riwayat simpan/unduhan?", "Hapus"))) return;
  S.downloadHistory.forEach((d) => { if (d.url) URL.revokeObjectURL(d.url); });
  S.downloadHistory = [];
  saveDownloadHistoryToDisk();
  renderDownloadHistory();
});

el("btn-open").addEventListener("click", openPdf);
el("btn-open-empty").addEventListener("click", openPdf);

// ============================================================
//  PANEL & INFO
// ============================================================
function togglePanel(force) {
  const p = el("outline-panel");
  const closed = force !== undefined ? !force : !p.classList.contains("closed");
  p.classList.toggle("closed", closed);
  el("btn-panel").classList.toggle("active", !closed);
  el("panel-reopen").classList.toggle("show", closed);
}
el("btn-panel").addEventListener("click", () => togglePanel());
el("panel-reopen").addEventListener("click", () => togglePanel(true));
el("outline-close").addEventListener("click", () => togglePanel(false));

el("btn-info").addEventListener("click", async () => {
  if (!S.pdfjsDoc) { toast("Belum ada PDF."); return; }
  const m = await S.pdfjsDoc.getMetadata().catch(() => null);
  const i = m?.info || {};
  const n = S.annots.reduce((o, a) => ({ ...o, [a.type]: (o[a.type] || 0) + 1 }), {});
  modal("Informasi Dokumen",
    `Nama file : ${S.fileName}\nHalaman   : ${S.pages.length}\n` +
    `Judul     : ${i.Title || "(kosong)"}\nPenulis   : ${i.Author || "(kosong)"}\n\n` +
    `Anotasi   : ${S.annots.length} total\n` +
    `  sorotan : ${n.highlight || 0}\n  teks    : ${n.text || 0}\n  gambar  : ${n.image || 0}`,
    [{ label: "Tutup", val: true, cls: "btn-confirm" }]);
});

// ============================================================
//  PASTE (Ctrl+V) - teks maupun gambar/screenshot dari clipboard
// ============================================================
document.addEventListener("paste", async (e) => {
  const editingText = getA(S.selectedId)?.editing;
  if (editingText) return;   // biarkan browser tempel teks normal ke dalam kotak yang sedang diedit
  if (!S.pages.length) return;

  const items = [...(e.clipboardData?.items || [])];
  const imgItem = items.find((it) => it.type.startsWith("image/"));
  if (imgItem) {
    e.preventDefault();
    const file = imgItem.getAsFile();
    if (!file) return;
    const dataUrl = await fileToDataUrl(file);
    await createImageAnnot(dataUrl, file.type || "image/png");
    setStatus("Gambar dari clipboard ditempel.");
    return;
  }

  const text = e.clipboardData?.getData("text/plain");
  if (text && text.trim()) {
    e.preventDefault();
    const p = S.pages[S.currentPage] || S.pages[0];
    const wr = viewer.getBoundingClientRect();
    const pr = p.wrapEl.getBoundingClientRect();
    const cx = clamp((wr.left + wr.width / 2 - pr.left) / S.zoom, 20, p.widthPt - 40);
    const cy = clamp((wr.top + 80 - pr.top) / S.zoom, 20, p.heightPt - 40);
    const a = {
      id: newId(), type: "text", pageIndex: p.index,
      x: cx, y: cy, w: Math.min(320, p.widthPt - cx - 12),
      h: S.textSize * LINE_HEIGHT,
      text: text.trim(), fontSize: S.textSize, color: S.textColor,
      bold: false, italic: false,
    };
    S.annots.push(a);
    drawAnnot(a);
    select(a.id);
    markDirty(); pushUndo();
    setStatus("Teks dari clipboard ditempel.");
  }
});

// ============================================================
//  KEYBOARD
// ============================================================
document.addEventListener("keydown", (e) => {
  const a = getA(S.selectedId);
  if (a?.editing) { if (e.key === "Escape") a.bodyEl.blur(); return; }
  const mod = e.ctrlKey || e.metaKey;
  if (e.key === "Escape") { select(null); setMode(null); return; }
  if (mod && e.key.toLowerCase() === "z" && !e.shiftKey) { e.preventDefault(); el("btn-undo").click(); }
  if (mod && (e.key.toLowerCase() === "y" || (e.shiftKey && e.key.toLowerCase() === "z"))) {
    e.preventDefault(); el("btn-redo").click();
  }
  if (mod && e.key.toLowerCase() === "s") { e.preventDefault(); save(); }
  if ((e.key === "Delete" || e.key === "Backspace") && S.selectedId) {
    e.preventDefault(); removeAnnot(S.selectedId);
  }
  if (e.key === "Enter" && a?.type === "text" && !a.editing) { e.preventDefault(); editText(a); }
});

function setEnabled(on) {
  ["btn-save","btn-saveas","btn-close","btn-text",
   "btn-image","btn-highlight","btn-erase","btn-zoom-in","btn-zoom-out","btn-info"]
    .forEach((id) => (el(id).disabled = !on));
}
window.addEventListener("beforeunload", (e) => {
  if (S.dirty) { e.preventDefault(); e.returnValue = ""; }
});

(async function init() {
  await initFonts();
  initHl();
  setEnabled(false);
  if (!S.hasFS) toast("Untuk fitur Save penuh, gunakan Chrome atau Edge.", 4000);
})();
