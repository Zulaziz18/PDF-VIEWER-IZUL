#!/usr/bin/env python3
"""Sample documents for the UI screenshot harness.

The harness shows the real application against real pages, rendered by the
same PDFium the application ships, so the screenshots look like the product and
not like a wireframe. These are the documents it opens.

Every word here is invented. The harness exists to be compared against
reference screenshots of other software, and those references carry their
owner's private file names; nothing from them is reproduced.

Only reportlab's standard-14 fonts are used, so the output is identical on any
machine that runs this and no font file is needed.

    python3 tools/ui-harness/make_samples.py [out-dir]
"""
import sys
from pathlib import Path

from reportlab.lib.colors import HexColor, white
from reportlab.lib.pagesizes import A4, landscape
from reportlab.pdfgen import canvas

OUT = Path(sys.argv[1] if len(sys.argv) > 1 else "tools/ui-harness/.samples")
OUT.mkdir(parents=True, exist_ok=True)

NAVY = HexColor("#16325c")
BANDS = ["#d6336c", "#b0527a", "#8e3b6a", "#3f8f3a", "#2f6b2a", "#46b6c9", "#c9a227", "#d9622b"]
LOREM = (
    "Dokumen ini disusun sebagai bahan uji tampilan. Setiap paragraf berisi "
    "kalimat biasa dengan panjang yang beragam supaya lapisan teks, pencarian, "
    "dan sorotan punya sesuatu untuk dikerjakan. Tidak ada isi yang merujuk ke "
    "orang atau lembaga sungguhan."
)


def wrap(c, text, x, y, width, font="Helvetica", size=11, leading=15):
    c.setFont(font, size)
    words = text.split()
    line = ""
    for word in words:
        trial = f"{line} {word}".strip()
        if c.stringWidth(trial, font, size) > width and line:
            c.drawString(x, y, line)
            y -= leading
            line = word
        else:
            line = trial
    if line:
        c.drawString(x, y, line)
        y -= leading
    return y


def cover(c, title_lines, subtitle, labels):
    """A slide-shaped cover: a navy spine with coloured index tabs."""
    w, h = landscape(A4)
    c.setPageSize((w, h))
    c.setFillColor(HexColor("#fbfbfa"))
    c.rect(0, 0, w, h, stroke=0, fill=1)
    # Faint geometric background.
    c.setStrokeColor(HexColor("#ecebe8"))
    c.setLineWidth(0.6)
    for i in range(-10, 40):
        c.line(i * 40, 0, i * 40 + h, h)
        c.line(i * 40 + h, 0, i * 40, h)
    c.setFillColor(NAVY)
    c.roundRect(18, 18, 150, h - 36, 18, stroke=0, fill=1)
    c.setFillColor(white)
    c.setFont("Helvetica-Bold", 26)
    c.saveState()
    c.translate(70, 120)
    c.rotate(90)
    c.drawString(0, 0, "2026")
    c.restoreState()
    c.setFont("Helvetica-Bold", 13)
    for i, word in enumerate(["PANDUAN", "STUDI", "MAHASISWA"]):
        c.drawString(40, 240 - i * 18, word)
    band_h = (h - 60) / len(labels)
    for i, (label, colour) in enumerate(zip(labels, BANDS)):
        y = h - 30 - (i + 1) * band_h
        c.setFillColor(HexColor(colour))
        c.roundRect(150, y + 3, 120, band_h - 6, 8, stroke=0, fill=1)
        c.setFillColor(white)
        c.setFont("Helvetica-Bold", 17)
        c.drawRightString(262, y + band_h / 2 - 6, label)
    c.setFillColor(NAVY)
    c.setFont("Helvetica-Bold", 40)
    y = h - 200
    for line in title_lines:
        c.drawString(310, y, line)
        y -= 46
    c.setFillColor(HexColor("#222222"))
    c.setFont("Helvetica-Bold", 18)
    c.drawString(316, 110, "Disusun oleh:")
    c.drawString(316, 84, subtitle)
    c.showPage()


def body_page(c, number, heading, paragraphs, table=None, chart=None):
    w, h = A4
    c.setPageSize((w, h))
    c.setFillColor(NAVY)
    c.rect(0, h - 56, w, 56, stroke=0, fill=1)
    c.setFillColor(white)
    c.setFont("Helvetica-Bold", 16)
    c.drawString(56, h - 36, heading)
    c.setFillColor(HexColor("#222222"))
    y = h - 96
    for p in paragraphs:
        y = wrap(c, p, 56, y, w - 112) - 8
    if table:
        c.setFont("Helvetica-Bold", 10)
        col = (w - 112) / len(table[0])
        for r, row in enumerate(table):
            c.setFillColor(HexColor("#e9eef7") if r == 0 else (HexColor("#f7f7f5") if r % 2 else white))
            c.rect(56, y - 18, w - 112, 20, stroke=0, fill=1)
            c.setFillColor(HexColor("#222222"))
            c.setFont("Helvetica-Bold" if r == 0 else "Helvetica", 10)
            for k, cell in enumerate(row):
                c.drawString(62 + k * col, y - 12, cell)
            y -= 20
        y -= 16
    if chart:
        base = y - 160
        c.setStrokeColor(HexColor("#999999"))
        c.line(70, base, w - 70, base)
        bw = (w - 160) / len(chart)
        for i, (label, value) in enumerate(chart):
            c.setFillColor(HexColor(BANDS[i % len(BANDS)]))
            c.rect(80 + i * bw, base, bw * 0.6, value * 1.4, stroke=0, fill=1)
            c.setFillColor(HexColor("#333333"))
            c.setFont("Helvetica", 9)
            c.drawString(80 + i * bw, base - 12, label)
    c.setFont("Helvetica", 9)
    c.setFillColor(HexColor("#888888"))
    c.drawCentredString(w / 2, 28, f"Halaman {number}")
    c.showPage()


def guide(path):
    c = canvas.Canvas(str(path))
    c.setTitle("Panduan Studi Mahasiswa 2026")
    cover(
        c,
        ["PANDUAN", "STUDI DAN LAYANAN", "AKADEMIK KAMPUS", "TAHUN 2026"],
        "BAGIAN AKADEMIK FAKULTAS CONTOH",
        ["JADWAL", "KURIKULUM", "KRS", "UJIAN", "SKRIPSI", "BEASISWA", "LAYANAN", "KONTAK"],
    )
    sections = [
        "Pendahuluan", "Kalender Akademik", "Struktur Kurikulum", "Pengisian KRS",
        "Evaluasi Belajar", "Tugas Akhir", "Beasiswa", "Layanan Mahasiswa",
    ]
    n = 2
    for i, s in enumerate(sections * 4):
        table = None
        chart = None
        if i % 3 == 1:
            table = [["Kegiatan", "Mulai", "Selesai", "Keterangan"],
                     ["Pengisian KRS", "1 Agustus", "14 Agustus", "Daring"],
                     ["Perkuliahan", "25 Agustus", "12 Desember", "Tatap muka"],
                     ["Ujian Tengah", "13 Oktober", "24 Oktober", "Terjadwal"],
                     ["Ujian Akhir", "15 Desember", "2 Januari", "Terjadwal"]]
        if i % 4 == 2:
            chart = [("2021", 60), ("2022", 74), ("2023", 81), ("2024", 90), ("2025", 97), ("2026", 104)]
        body_page(c, n, f"{(i % 8) + 1}. {s}", [LOREM, LOREM + " " + LOREM, LOREM], table, chart)
        n += 1
    c.save()


def report(path, title, pages):
    c = canvas.Canvas(str(path))
    c.setTitle(title)
    for p in range(1, pages + 1):
        body_page(c, p, f"{title} — Bagian {p}", [LOREM, LOREM, LOREM + " " + LOREM],
                  table=[["Pos", "Anggaran", "Realisasi"], ["Operasional", "120", "112"],
                         ["Kegiatan", "80", "77"], ["Cadangan", "20", "4"]] if p % 2 else None,
                  chart=[("Jan", 40), ("Feb", 55), ("Mar", 48), ("Apr", 70)] if p % 3 == 0 else None)
    c.save()


guide(OUT / "Panduan Studi 2026.pdf")
report(OUT / "Laporan Kegiatan Semester.pdf", "Laporan Kegiatan", 9)
report(OUT / "Catatan Rapat Organisasi.pdf", "Catatan Rapat", 5)
report(OUT / "Proposal Penelitian.pdf", "Proposal Penelitian", 14)
print(f"sampel ditulis ke {OUT}")
