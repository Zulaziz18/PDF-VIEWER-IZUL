#!/usr/bin/env python3
"""Scanned-looking test sets for comparing OCR engines (Phase 8 follow-up).

`tools/ocr-proof/make_scans.py` makes one kind of page: a clean flatbed scan.
That measures whether the text layer is written correctly, but it says little
about which engine reads better, because every engine does well on clean
print. This writes five sets, from clean to bad, each four image-only pages
with the true text beside them:

    bersih    flatbed scan, like the proof's
    foto_hp   phone photo: tilted, perspective, uneven light, soft, low-res, JPEG 35
    pudar     faded ink on grey paper, heavy speckle
    kecil     8-9 pt print
    angka     amounts, dates, invoice and phone numbers

All text is invented (no real people, accounts or invoices). Everything is
synthetic: it is a stand-in for real scans, not a replacement for them.

    python3 tools/ocr-bakeoff/make_sets.py <out-dir>

Writes <out-dir>/<set>.pdf and <out-dir>/truth.json: {"<set>": [page text...]}.
"""
import importlib.util
import json
import random
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("make_scans", HERE.parent / "ocr-proof" / "make_scans.py")
_scans = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_scans)
INDONESIAN, ENGLISH, FONTS, DPI, W, H = (
    _scans.INDONESIAN, _scans.ENGLISH, _scans.FONTS, _scans.DPI, _scans.W, _scans.H,
)

NUMERIC = [
    "Total pembayaran Rp 12.480.000 jatuh tempo pada 15/10/2026.",
    "Faktur No. 2026/IX/0457 tertanggal 03-09-2026 belum dilunasi.",
    "Telepon kantor (021) 555-0142, faks (021) 555-0143, lokal 217.",
    "Luas tanah 245 m2 dengan harga Rp 1.150.000 per meter persegi.",
    "Kode pos 40115; rekening 123-456-7890 a.n. PT Contoh Abadi.",
    "Suhu rata-rata 27,5 derajat Celsius dengan kelembapan 82 persen.",
    "Pengiriman 48 koli seberat 1.236,5 kg tiba pada pukul 14.45 WIB.",
    "Diskon 12,5% berlaku untuk pembelian minimal 3 unit sebelum 31/12.",
    "Nomor seri: SN-4471-0928-AB, garansi hingga 07 Maret 2028.",
    "Saldo akhir Rp 3.904.250 setelah dikurangi biaya administrasi.",
]

# Each profile is a recipe for how a perfect page gets spoiled.
PROFILES = {
    "bersih": dict(paper=246, ink=(20, 45), rot=0.6, persp=0.0, light=0.0, blur=0.7, speck=400, scale=1.0, jpeg=70),
    "foto_hp": dict(paper=236, ink=(30, 70), rot=2.5, persp=0.025, light=0.35, blur=1.4, speck=900, scale=0.45, jpeg=35),
    "pudar": dict(paper=214, ink=(110, 150), rot=1.0, persp=0.0, light=0.1, blur=1.0, speck=120, scale=0.6, jpeg=50),
    "kecil": dict(paper=246, ink=(20, 45), rot=0.6, persp=0.0, light=0.0, blur=0.7, speck=400, scale=1.0, jpeg=70),
    "angka": dict(paper=246, ink=(20, 45), rot=0.6, persp=0.0, light=0.0, blur=0.7, speck=400, scale=1.0, jpeg=70),
}

# (lines, font, size in pt) for the four pages of each set.
PAGES = {
    "bersih": [(INDONESIAN, 0, 12), (ENGLISH, 1, 12), (INDONESIAN[::-1], 2, 11), (ENGLISH[::-1], 3, 10)],
    "foto_hp": [(INDONESIAN, 1, 12), (ENGLISH, 2, 12), (INDONESIAN[::-1], 0, 11), (ENGLISH[::-1], 3, 12)],
    "pudar": [(INDONESIAN, 2, 12), (ENGLISH, 0, 12), (INDONESIAN[::-1], 1, 12), (ENGLISH[::-1], 2, 11)],
    "kecil": [(INDONESIAN, 1, 8), (ENGLISH, 2, 9), (INDONESIAN[::-1], 3, 8), (ENGLISH[::-1], 0, 9)],
    "angka": [(NUMERIC, 1, 12), (NUMERIC[::-1], 2, 12), (NUMERIC[:5] + INDONESIAN[:3], 3, 11), (ENGLISH[:4] + NUMERIC[5:], 0, 11)],
}


def perspective_coeffs(src, dst):
    """The 8 numbers PIL's PERSPECTIVE transform wants, from four point pairs."""
    rows, rhs = [], []
    for (x, y), (u, v) in zip(src, dst):
        rows += [[x, y, 1, 0, 0, 0, -u * x, -u * y], [0, 0, 0, x, y, 1, -v * x, -v * y]]
        rhs += [u, v]
    return np.linalg.solve(np.array(rows, float), np.array(rhs, float)).tolist()


def spoil(lines, font_path, size_pt, seed, p):
    rnd = random.Random(seed)
    img = Image.new("L", (W, H), p["paper"])
    d = ImageDraw.Draw(img)
    font = ImageFont.truetype(font_path, int(size_pt * DPI / 72))
    y = int(1.2 * DPI)
    step = int(size_pt * 1.6 * DPI / 72)
    for line in lines:
        d.text((int(1.0 * DPI), y), line, font=font, fill=rnd.randint(*p["ink"]))
        y += step
    img = img.rotate(rnd.uniform(-p["rot"], p["rot"]), resample=Image.BICUBIC, fillcolor=p["paper"])
    if p["persp"]:
        k = p["persp"]
        jitter = lambda: rnd.uniform(-k, k)
        quad = [(W * jitter() * 4, H * jitter() * 4), (W * (1 + jitter() * 4), H * jitter() * 4),
                (W * (1 + jitter() * 4), H * (1 + jitter() * 4)), (W * jitter() * 4, H * (1 + jitter() * 4))]
        flat = [(0, 0), (W, 0), (W, H), (0, H)]
        img = img.transform((W, H), Image.PERSPECTIVE, perspective_coeffs(quad, flat), Image.BICUBIC, fillcolor=p["paper"])
    a = np.asarray(img, dtype=np.float32)
    if p["light"]:
        # A lamp off to one side: the page is darker at one corner.
        gx = np.linspace(0, 1, W)[None, :]
        gy = np.linspace(0, 1, H)[:, None]
        corner = rnd.choice([(0, 0), (1, 0), (0, 1), (1, 1)])
        dist = np.hypot(gx - corner[0], gy - corner[1]) / 1.41
        a *= 1.0 - p["light"] * dist
    img = Image.fromarray(np.clip(a, 0, 255).astype(np.uint8))
    img = img.filter(ImageFilter.GaussianBlur(p["blur"]))
    px = img.load()
    for _ in range(W * H // p["speck"]):
        px[rnd.randrange(W), rnd.randrange(H)] = rnd.choice((90, 140, 200))
    if p["scale"] != 1.0:
        img = img.resize((int(W * p["scale"]), int(H * p["scale"])), Image.LANCZOS)
    return img


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    truth = {}
    for name, pages in PAGES.items():
        images, texts = [], []
        for i, (lines, font, size) in enumerate(pages):
            images.append(spoil(lines, FONTS[font], size, seed=100 + i, p=PROFILES[name]))
            texts.append(" ".join(lines))
        # The page keeps its A4 size however few pixels the photo has.
        res = images[0].width / 8.27
        images[0].save(out / f"{name}.pdf", "PDF", resolution=res, save_all=True,
                       append_images=images[1:], quality=PROFILES[name]["jpeg"])
        truth[name] = texts
        print(f"{name}: {len(images)} halaman, {images[0].width}x{images[0].height} px")
    (out / "truth.json").write_text(json.dumps(truth, ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
