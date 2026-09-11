#!/usr/bin/env python3
"""Generate benchmark fixtures for the PDF Studio Izul v7 measurement spike.

The SPEC targets "PDF 50 MB / 500 halaman sampai halaman pertama tampil < 400 ms".
A 50 MB / 500 page PDF is not one document shape but three, with very different
cost profiles, so we build all three plus linearized twins:

  scan-500p     500 JPEG page images at 150 dpi  -> ~50 MB, image decode bound
  text-500p     dense embedded-font text          -> small file, parse bound
  mixed-500p    text + vector art + photos        -> ~50 MB, worst realistic case

Deterministic: fixed RNG seed, so numbers are comparable across runs.
"""
import os, sys, io, math, random, zlib
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont, ImageFilter
from reportlab.pdfgen import canvas as rl_canvas
from reportlab.lib.pagesizes import letter
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
import pikepdf

OUT = Path(sys.argv[1] if len(sys.argv) > 1 else "test-fixtures")
OUT.mkdir(parents=True, exist_ok=True)
SEED = 20260904
PAGES = 500
PW, PH = letter                      # 612 x 792 pt



def _font(*candidates):
    """First candidate that exists on this machine.

    The fixtures are generated on Linux, Windows and a developer's laptop, and
    no font ships on all three. Liberation stays first so the documents this
    produces on Linux keep the same text as the ones the numbers in
    `bench/results/` were measured against; the rest are fallbacks in
    descending order of how close they are to it metrically.

    (Not byte-identical: reportlab stamps `CreationDate` and `ModDate`, so two
    runs of this script have always differed by those few bytes. Only the
    rendered content is stable.)

    The tests that read these fixtures assert structure — a tile grid that
    lines up, a page that carries ink, a text box over the glyph it describes —
    not exact pixels, so a substituted face changes nothing they check.
    """
    for path in candidates:
        if Path(path).exists():
            return path
    raise SystemExit(
        "tidak ada font serif yang bisa dipakai. Dicari:\n  "
        + "\n  ".join(candidates)
        + "\nDi Debian/Ubuntu: sudo apt-get install fonts-liberation"
    )


SERIF = _font(
    "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
    "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
    "C:/Windows/Fonts/times.ttf",
)
SERIF_B = _font(
    "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
    "/System/Library/Fonts/Supplemental/Times New Roman Bold.ttf",
    "C:/Windows/Fonts/timesbd.ttf",
)

WORDS = ("dokumen halaman anotasi sorotan gambar tanda tangan lampiran ekspor "
         "keterangan pendahuluan metodologi hasil pembahasan kesimpulan lampiran "
         "the quick brown fox jumps over a lazy dog while parsing structured "
         "content streams and cross reference tables inside a portable document"
         ).split()


def paragraph(rng, n):
    return " ".join(rng.choice(WORDS) for _ in range(n))


# --------------------------------------------------------------------------- #
# 1. scan-500p : each page is one JPEG, like a flatbed scan
# --------------------------------------------------------------------------- #
def make_scan(path, quality=55, dpi=150):
    rng = random.Random(SEED)
    w, h = int(PW / 72 * dpi), int(PH / 72 * dpi)
    font = ImageFont.truetype(SERIF, int(dpi / 6))
    head = ImageFont.truetype(SERIF_B, int(dpi / 4))
    c = rl_canvas.Canvas(str(path), pagesize=letter)
    for p in range(PAGES):
        img = Image.new("RGB", (w, h), (250, 249, 245))
        d = ImageDraw.Draw(img)
        d.text((int(dpi * 0.9), int(dpi * 0.8)), f"Bab {p // 20 + 1} — Halaman {p + 1}",
               font=head, fill=(20, 20, 25))
        y = int(dpi * 1.3)
        while y < h - dpi:
            d.text((int(dpi * 0.9), y), paragraph(rng, 11)[:78], font=font, fill=(30, 30, 35))
            y += int(dpi / 4.5)
        # scanner artefacts: speckle + very slight blur + page skew shading
        px = img.load()
        for _ in range(w * h // 900):
            x0, y0 = rng.randrange(w), rng.randrange(h)
            v = rng.randrange(150, 235)
            px[x0, y0] = (v, v, v - 4)
        img = img.filter(ImageFilter.GaussianBlur(0.4))
        buf = io.BytesIO()
        img.save(buf, "JPEG", quality=quality, optimize=False)
        buf.seek(0)
        c.drawImage(__import__("reportlab.lib.utils", fromlist=["ImageReader"]).ImageReader(buf),
                    0, 0, width=PW, height=PH)
        c.showPage()
        if p % 100 == 0:
            print(f"  scan page {p}/{PAGES}", flush=True)
    c.save()


# --------------------------------------------------------------------------- #
# 2. text-500p : dense real text with an embedded TrueType font
# --------------------------------------------------------------------------- #
def make_text(path):
    rng = random.Random(SEED + 1)
    pdfmetrics.registerFont(TTFont("LibSerif", SERIF))
    pdfmetrics.registerFont(TTFont("LibSerifB", SERIF_B))
    c = rl_canvas.Canvas(str(path), pagesize=letter)
    for p in range(PAGES):
        c.setFont("LibSerifB", 14)
        c.drawString(72, PH - 72, f"Bab {p // 20 + 1} — Halaman {p + 1}")
        c.setFont("LibSerif", 10.5)
        t = c.beginText(72, PH - 100)
        t.setLeading(13.5)
        for _ in range(48):
            t.textLine(paragraph(rng, 13)[:95])
        c.drawText(t)
        c.showPage()
        if p % 200 == 0:
            print(f"  text page {p}/{PAGES}", flush=True)
    c.save()


# --------------------------------------------------------------------------- #
# 3. mixed-500p : text + vector art + a photo, the realistic worst case
# --------------------------------------------------------------------------- #
def photo(rng, w=900, h=650):
    img = Image.new("RGB", (w, h))
    px = img.load()
    ox, oy = rng.random() * 8, rng.random() * 8
    for y in range(h):
        for x in range(0, w, 2):
            v = math.sin(x / 40 + ox) * math.cos(y / 55 + oy)
            r = int(128 + 90 * v)
            g = int(120 + 80 * math.sin(v * 2.3 + oy))
            b = int(140 + 70 * math.cos(v * 1.7 + ox))
            px[x, y] = (r & 255, g & 255, b & 255)
            if x + 1 < w:
                px[x + 1, y] = (r & 255, g & 255, b & 255)
    return img.filter(ImageFilter.GaussianBlur(0.6))


def make_mixed(path):
    rng = random.Random(SEED + 2)
    pdfmetrics.registerFont(TTFont("LibSerifM", SERIF))
    pdfmetrics.registerFont(TTFont("LibSerifMB", SERIF_B))
    from reportlab.lib.utils import ImageReader
    photos = []
    for i in range(25):                       # reused pool, like a real report
        b = io.BytesIO()
        photo(rng).save(b, "JPEG", quality=72)
        b.seek(0)
        photos.append(ImageReader(b))
    c = rl_canvas.Canvas(str(path), pagesize=letter)
    for p in range(PAGES):
        c.setFont("LibSerifMB", 15)
        c.drawString(56, PH - 56, f"Laporan Bagian {p // 25 + 1}.{p % 25 + 1}")
        c.drawImage(photos[p % len(photos)], 56, PH - 300, width=PW - 112, height=220)
        c.setFont("LibSerifM", 10)
        t = c.beginText(56, PH - 325)
        t.setLeading(12.5)
        for _ in range(20):
            t.textLine(paragraph(rng, 14)[:100])
        c.drawText(t)
        # vector art: a chart-like scatter of stroked and filled paths
        c.saveState()
        for _ in range(240):
            c.setStrokeColorRGB(rng.random(), rng.random() * .6, rng.random() * .7)
            c.setLineWidth(rng.random() * 1.6 + .2)
            x0, y0 = 56 + rng.random() * (PW - 112), 60 + rng.random() * 150
            c.line(x0, y0, x0 + rng.random() * 40 - 20, y0 + rng.random() * 40 - 20)
        for _ in range(60):
            c.setFillColorRGB(rng.random(), rng.random(), rng.random(), alpha=0.55)
            c.circle(56 + rng.random() * (PW - 112), 60 + rng.random() * 150,
                     rng.random() * 9 + 1, stroke=0, fill=1)
        c.restoreState()
        c.showPage()
        if p % 100 == 0:
            print(f"  mixed page {p}/{PAGES}", flush=True)
    c.save()


def make_viewer(path):
    """A small document for the viewer tests: a bookmark tree, pages that are
    not all the same size, and one page carrying its own /Rotate.

    Phase 1 needs a fixture whose *structure* is interesting rather than whose
    size is: an outline to walk, a rotated page to prove the tile matrix, and
    differing page sizes to prove the layout does not assume a uniform grid."""
    rng = random.Random(SEED + 3)
    pdfmetrics.registerFont(TTFont("LibSerifV", SERIF))
    c = rl_canvas.Canvas(str(path), pagesize=letter)
    sizes = [(PW, PH), (PW, PH), (PH, PW), (PW, PH * 0.75), (PW, PH)] * 2
    for i, (w, h) in enumerate(sizes):
        c.setPageSize((w, h))
        c.setFont("LibSerifV", 24)
        c.drawString(56, h - 80, f"Bagian {i // 2 + 1} — halaman {i + 1}")
        c.setFont("LibSerifV", 11)
        y = h - 120
        for _ in range(18):
            c.drawString(56, y, paragraph(rng, 11))
            y -= 16
        if i % 2 == 0:
            c.bookmarkPage(f"p{i}")
            c.addOutlineEntry(f"Bagian {i // 2 + 1}", f"p{i}", level=0)
            c.addOutlineEntry(f"Sub {i // 2 + 1}.1", f"p{i}", level=1)
        c.showPage()
    c.save()

    # One page rotated by /Rotate, which PDFium applies for a plain render but
    # not for the matrix-based tile path — so this page is what catches a tile
    # matrix that ignores the page's own rotation.
    with pikepdf.open(path, allow_overwriting_input=True) as pdf:
        pdf.pages[2].Rotate = 90
        pdf.save(path)


def pad_to(path, target_mb):
    """Grow a PDF to ~target_mb by attaching incompressible ballast as an
    unreferenced stream. Parsing cost of the ballast is nil, which is exactly
    what we want: it isolates 'file is big' from 'file is complex'."""
    size = path.stat().st_size
    need = int(target_mb * 1024 * 1024) - size
    if need <= 0:
        return
    with pikepdf.open(path, allow_overwriting_input=True) as pdf:
        st = pikepdf.Stream(pdf, os.urandom(need))
        st.stream_dict["/Type"] = pikepdf.Name("/IzulBallast")
        pdf.Root["/IzulBallast"] = pdf.make_indirect(st)
        pdf.save(path, compress_streams=False, stream_decode_level=pikepdf.StreamDecodeLevel.none)


def linearize(src, dst):
    with pikepdf.open(src) as pdf:
        pdf.save(dst, linearize=True)


def report(p):
    with pikepdf.open(p) as pdf:
        n = len(pdf.pages)
    print(f"  {p.name:<34} {p.stat().st_size / 1048576:7.2f} MB  {n} pages", flush=True)


if __name__ == "__main__":
    jobs = sys.argv[2:] or ["scan", "text", "mixed", "viewer"]
    if "scan" in jobs:
        print("scan-500p ..."); make_scan(OUT / "scan-500p.pdf"); report(OUT / "scan-500p.pdf")
    if "text" in jobs:
        print("text-500p ..."); make_text(OUT / "text-500p.pdf"); report(OUT / "text-500p.pdf")
    if "viewer" in jobs:
        print("viewer-10p ..."); make_viewer(OUT / "viewer-10p.pdf"); report(OUT / "viewer-10p.pdf")
    if "mixed" in jobs:
        print("mixed-500p ..."); make_mixed(OUT / "mixed-500p.pdf")
        pad_to(OUT / "mixed-500p.pdf", 50); report(OUT / "mixed-500p.pdf")
    for name in ("scan-500p", "text-500p", "mixed-500p"):
        src = OUT / f"{name}.pdf"
        if src.exists():
            linearize(src, OUT / f"{name}-lin.pdf"); report(OUT / f"{name}-lin.pdf")
    print("done")
