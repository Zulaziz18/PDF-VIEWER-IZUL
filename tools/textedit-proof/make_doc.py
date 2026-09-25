#!/usr/bin/env python3
"""A page to edit text on (Phase 7): lines in an embedded font, one line in a
font that is not embedded, a line whose glyphs are only a subset, and a
picture and a shape around them, so that a rewrite that disturbs anything
else on the page shows. The content is invented.

    python3 tools/textedit-proof/make_doc.py <out.pdf>
"""
import sys

from reportlab.lib.colors import HexColor
from reportlab.lib.pagesizes import A4
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas

FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"


def main():
    out = sys.argv[1]
    pdfmetrics.registerFont(TTFont("DejaVu", FONT))
    c = canvas.Canvas(out, pagesize=A4)
    w, h = A4
    c.setTitle("Surat Keterangan (contoh)")
    c.setFillColor(HexColor("#1d4e89"))
    c.rect(56, h - 110, w - 112, 40, fill=1, stroke=0)
    c.setFillColor(HexColor("#ffffff"))
    c.setFont("DejaVu", 16)
    c.drawString(70, h - 96, "SURAT KETERANGAN KEGIATAN")
    c.setFillColor(HexColor("#000000"))
    c.setFont("DejaVu", 12)
    c.drawString(56, h - 150, "Nama peserta: Budi Santoso")
    c.drawString(56, h - 172, "Tanggal kegiatan: 14 Agustus 2026")
    c.drawString(56, h - 194, "Tempat: Aula Serbaguna Kampus Timur")
    # Standard 14, not embedded: an edit here must be refused.
    c.setFont("Helvetica", 12)
    c.drawString(56, h - 230, "Catatan: dibawa saat registrasi ulang.")
    # A shape and a gradient-ish block the rewrite must leave alone.
    c.setStrokeColor(HexColor("#b03a2e"))
    c.setLineWidth(2)
    c.circle(470, h - 180, 40, stroke=1, fill=0)
    for i in range(20):
        c.setFillColor(HexColor("#%02x%02x%02x" % (240 - i * 8, 200, 120 + i * 6)))
        c.rect(56 + i * 20, h - 320, 20, 50, fill=1, stroke=0)
    c.setFillColor(HexColor("#555555"))
    c.setFont("DejaVu", 9)
    c.drawString(56, 60, "Dokumen karangan untuk uji penyuntingan teks.")
    c.showPage()
    c.save()


if __name__ == "__main__":
    main()
