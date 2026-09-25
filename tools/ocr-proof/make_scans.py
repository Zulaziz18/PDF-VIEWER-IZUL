#!/usr/bin/env python3
"""Scanned-looking pages with known text, for measuring OCR (Phase 7).

Every page is a picture only — no text layer — made the way a flatbed scan
looks: paper tint, speckle, slight blur, a small skew, JPEG. The text is
written here (invented, no real people or records), half Indonesian and half
English, and saved beside the PDF as the ground truth.

    python3 tools/ocr-proof/make_scans.py <out-dir>

Writes <out-dir>/scans.pdf and <out-dir>/truth.json:
{"pages": [{"lines": [...], "words": [[text, x0, top, x1, bottom], ...]}]},
word boxes in points from the top-left of the page as shown. The last page is
stored turned (the picture on its side, `/Rotate 90`), as scanners do.
"""
import io
import json
import random
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

FONTS = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
]

INDONESIAN = [
    "Laporan kegiatan bulanan ini disusun untuk memberikan gambaran",
    "tentang pelaksanaan program kerja selama bulan September.",
    "Rapat koordinasi diadakan pada hari Senin pukul 09.30 di ruang",
    "pertemuan lantai dua dan dihadiri oleh dua belas peserta.",
    "Anggaran yang digunakan sebesar Rp 4.750.000 dari total dana",
    "yang tersedia, sehingga sisa anggaran masih cukup untuk kegiatan",
    "berikutnya. Beberapa kendala yang ditemui antara lain keterlambatan",
    "pengiriman barang dan perubahan jadwal narasumber.",
    "Rekomendasi: jadwal diperbarui setiap minggu dan dibagikan",
    "kepada semua pihak paling lambat hari Jumat sore.",
]

ENGLISH = [
    "The quarterly review covers progress on the library project,",
    "including the new reading room and the digital catalogue.",
    "Visitors increased by 18 percent compared with last year, and",
    "the average stay was about forty-five minutes per visit.",
    "Three workshops were held in July: bookbinding, map reading,",
    "and an introduction to archives for secondary school students.",
    "Next steps are to extend opening hours on Saturday, to replace",
    "the old shelving on the ground floor, and to publish a short",
    "guide for new members. Questions may be sent to the front desk",
    "or left in the suggestion box near the main entrance.",
]

DPI = 300
W, H = int(8.27 * DPI), int(11.69 * DPI)


def page(lines, font_path, size_pt, seed):
    rnd = random.Random(seed)
    img = Image.new("L", (W, H), 246)
    d = ImageDraw.Draw(img)
    font = ImageFont.truetype(font_path, int(size_pt * DPI / 72))
    y = int(1.2 * DPI)
    step = int(size_pt * 1.6 * DPI / 72)
    words = []
    for line in lines:
        x = int(1.0 * DPI)
        d.text((x, y), line, font=font, fill=rnd.randint(20, 45))
        # Each word's box where it was drawn, in points.
        at = 0
        for word in line.split(" "):
            start = line.index(word, at)
            left = x + d.textlength(line[:start], font=font)
            box = d.textbbox((left, y), word, font=font)
            words.append([word] + [v * 72 / DPI for v in box])
            at = start + len(word)
        y += step
    img = img.rotate(rnd.uniform(-0.6, 0.6), resample=Image.BICUBIC, fillcolor=246)
    img = img.filter(ImageFilter.GaussianBlur(0.7))
    px = img.load()
    for _ in range(W * H // 400):
        x, yy = rnd.randrange(W), rnd.randrange(H)
        px[x, yy] = rnd.choice((90, 140, 200))
    return img, words


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    pages = [
        (INDONESIAN, FONTS[0], 12),
        (ENGLISH, FONTS[1], 12),
        (INDONESIAN[::-1], FONTS[2], 11),
        (ENGLISH[::-1], FONTS[3], 10),
        (ENGLISH[:6] + INDONESIAN[:4], FONTS[1], 11),
    ]
    images = []
    truth = []
    for i, (lines, font, size) in enumerate(pages):
        img, words = page(lines, font, size, seed=i)
        images.append(img)
        truth.append({"lines": lines, "words": words})
    # The last page lies on its side in the file; /Rotate 90 stands it up.
    images[-1] = images[-1].transpose(Image.ROTATE_90)
    buf = io.BytesIO()
    images[0].save(buf, "PDF", resolution=DPI, save_all=True, append_images=images[1:], quality=70)
    import pikepdf

    pdf = pikepdf.open(io.BytesIO(buf.getvalue()))
    pdf.pages[-1].obj.Rotate = 90
    pdf.save(out / "scans.pdf")
    (out / "truth.json").write_text(json.dumps({"pages": truth}, ensure_ascii=False, indent=1))
    print(f"{len(images)} halaman -> {out / 'scans.pdf'}")


if __name__ == "__main__":
    main()
