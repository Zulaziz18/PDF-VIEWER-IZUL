#!/usr/bin/env python3
"""PDFs for the Word-conversion proof (7.1.0), written with reportlab.

Each case is shaped after a kind of document people convert, with made-up
content only:

- jurnal: a two-column article — centred title, italic authors, bold
  headings, justified body, a superscript, a bulleted and a numbered list, a
  figure with its caption, a ruled table, running header and footer with page
  numbers, and a sideways "diunduh dari" line in the margin.
- surat: a one-column letter — right-aligned date, centred subject, justified
  paragraphs, a signature block.
- foto: a page that is mostly a photograph (JPEG) with a caption.
- pindaian: a page that is only a picture — a scan with no text at all.

    python3 tools/docx-proof/make_cases.py <out-dir>
"""
import io
import random
import sys
from pathlib import Path

from PIL import Image, ImageDraw
from reportlab.lib import colors
from reportlab.lib.enums import TA_CENTER, TA_JUSTIFY, TA_RIGHT
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle
from reportlab.lib.units import cm
from reportlab.platypus import (
    BaseDocTemplate,
    Frame,
    Image as RLImage,
    ListFlowable,
    ListItem,
    PageTemplate,
    Paragraph,
    SimpleDocTemplate,
    Spacer,
    Table,
    TableStyle,
)

BODY = ParagraphStyle("body", fontName="Times-Roman", fontSize=10, leading=13, alignment=TA_JUSTIFY)
TITLE = ParagraphStyle("title", fontName="Helvetica-Bold", fontSize=17, leading=21, alignment=TA_CENTER)
AUTHORS = ParagraphStyle("authors", fontName="Times-Italic", fontSize=10, leading=13, alignment=TA_CENTER)
HEAD = ParagraphStyle("head", fontName="Helvetica-Bold", fontSize=12, leading=15, spaceBefore=8, spaceAfter=3)
CAPTION = ParagraphStyle("caption", fontName="Times-Italic", fontSize=9, leading=11, alignment=TA_CENTER)

random.seed(7)
WORDS = (
    "kajian ini membahas cara kerja layanan akademik kampus dengan data yang dikumpulkan "
    "dari survei mahasiswa selama dua semester hasilnya menunjukkan bahwa waktu tunggu "
    "berkurang ketika jadwal konsultasi diumumkan lebih awal dan formulir dapat diisi daring"
).split()


def sentences(n):
    out = []
    for _ in range(n):
        k = random.randint(9, 18)
        words = [random.choice(WORDS) for _ in range(k)]
        out.append(" ".join(words).capitalize() + ".")
    return " ".join(out)


def figure_png():
    img = Image.new("RGB", (480, 300), "white")
    d = ImageDraw.Draw(img)
    for i, h in enumerate([120, 190, 150, 240, 210]):
        d.rectangle([40 + i * 85, 280 - h, 100 + i * 85, 280], fill=(40 + i * 40, 90, 160))
    d.line([20, 280, 460, 280], fill="black", width=2)
    buf = io.BytesIO()
    img.save(buf, "PNG")
    buf.seek(0)
    return buf


def photo_jpeg(seed):
    rnd = random.Random(seed)
    img = Image.new("RGB", (900, 600))
    px = img.load()
    for y in range(600):
        for x in range(900):
            px[x, y] = (
                (x * 255 // 900 + rnd.randint(0, 20)) % 256,
                (y * 255 // 600 + rnd.randint(0, 20)) % 256,
                140,
            )
    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=85)
    buf.seek(0)
    return buf


def jurnal(path):
    doc = BaseDocTemplate(str(path), pagesize=A4, leftMargin=2 * cm, rightMargin=2 * cm, topMargin=2.5 * cm, bottomMargin=2.5 * cm)
    w, h = A4
    gap = 0.8 * cm
    col = (w - 4 * cm - gap) / 2
    full = Frame(2 * cm, h - 2.5 * cm - 3.2 * cm, w - 4 * cm, 3.2 * cm, id="head")
    left = Frame(2 * cm, 2.5 * cm, col, h - 5 * cm - 3.2 * cm, id="l")
    right = Frame(2 * cm + col + gap, 2.5 * cm, col, h - 5 * cm - 3.2 * cm, id="r")
    left2 = Frame(2 * cm, 2.5 * cm, col, h - 5 * cm, id="l2")
    right2 = Frame(2 * cm + col + gap, 2.5 * cm, col, h - 5 * cm, id="r2")

    def decorate(canvas, d):
        canvas.saveState()
        canvas.setFont("Helvetica", 8)
        canvas.setFillGray(0.4)
        canvas.drawString(2 * cm, h - 1.5 * cm, "Jurnal Layanan Akademik 2026")
        canvas.drawCentredString(w / 2, 1.5 * cm, f"Halaman {d.page}")
        canvas.translate(1 * cm, h / 2)
        canvas.rotate(90)
        canvas.drawCentredString(0, 0, "Diunduh dari contoh.invalid pada 1 Oktober 2026")
        canvas.restoreState()

    doc.addPageTemplates(
        [
            PageTemplate(id="first", frames=[full, left, right], onPage=decorate),
            PageTemplate(id="rest", frames=[left2, right2], onPage=decorate),
        ]
    )
    from reportlab.platypus import FrameBreak, NextPageTemplate

    story = [
        Paragraph("Waktu Tunggu Layanan Akademik Setelah Pendaftaran Daring", TITLE),
        Spacer(1, 6),
        Paragraph("Rina Contoh, Budi Sampel, dan Tim Penulis Fiktif", AUTHORS),
        NextPageTemplate("rest"),
        FrameBreak(),
        Paragraph("Ringkasan", HEAD),
        Paragraph(sentences(5) + ' Angka ini naik<super>1</super> pada semester kedua.', BODY),
        Paragraph("Pendahuluan", HEAD),
        Paragraph(sentences(7), BODY),
        Spacer(1, 4),
        ListFlowable(
            [ListItem(Paragraph(sentences(1), BODY)) for _ in range(3)],
            bulletType="bullet",
            start="•",
        ),
        Paragraph("Metode", HEAD),
        Paragraph(sentences(6), BODY),
        ListFlowable(
            [ListItem(Paragraph(sentences(1), BODY)) for _ in range(3)],
            bulletType="1",
            bulletFormat="%s.",
        ),
        RLImage(figure_png(), width=col, height=col * 300 / 480),
        Paragraph("Gambar 1. Rata-rata waktu tunggu per bulan.", CAPTION),
        Paragraph("Hasil", HEAD),
        Paragraph(sentences(8), BODY),
        Table(
            [["Layanan", "Sebelum", "Sesudah"], ["Konsultasi", "12 hari", "4 hari"], ["Legalisir", "7 hari", "2 hari"], ["Transkrip", "5 hari", "1 hari"]],
            colWidths=[col * 0.44, col * 0.28, col * 0.28],
            style=TableStyle(
                [
                    ("FONT", (0, 0), (-1, 0), "Helvetica-Bold", 9),
                    ("FONT", (0, 1), (-1, -1), "Helvetica", 9),
                    ("GRID", (0, 0), (-1, -1), 0.5, colors.grey),
                    ("BACKGROUND", (0, 0), (-1, 0), colors.HexColor("#1f3864")),
                    ("TEXTCOLOR", (0, 0), (-1, 0), colors.white),
                ]
            ),
        ),
        Paragraph("Pembahasan", HEAD),
        Paragraph(sentences(9), BODY),
        Paragraph(sentences(8), BODY),
        Paragraph("Kesimpulan", HEAD),
        Paragraph(sentences(6), BODY),
    ]
    doc.build(story)


def surat(path):
    doc = SimpleDocTemplate(str(path), pagesize=A4, leftMargin=2.5 * cm, rightMargin=2.5 * cm, topMargin=3 * cm)
    body = ParagraphStyle("b", parent=BODY, fontName="Helvetica", fontSize=11, leading=16)
    story = [
        Paragraph("Kota Contoh, 1 Oktober 2026", ParagraphStyle("d", parent=body, alignment=TA_RIGHT)),
        Spacer(1, 18),
        Paragraph("Perihal: Undangan Rapat Evaluasi", ParagraphStyle("s", parent=body, fontName="Helvetica-Bold", alignment=TA_CENTER)),
        Spacer(1, 18),
        Paragraph("Dengan hormat,", body),
        Spacer(1, 8),
        Paragraph(sentences(5), body),
        Spacer(1, 8),
        Paragraph(sentences(6), body),
        Spacer(1, 8),
        Paragraph(sentences(3), body),
        Spacer(1, 30),
        Paragraph("Hormat kami,", ParagraphStyle("r", parent=body, alignment=TA_RIGHT)),
        Spacer(1, 40),
        Paragraph("<b>Kepala Bagian Fiktif</b>", ParagraphStyle("r2", parent=body, alignment=TA_RIGHT)),
    ]
    doc.build(story)


def foto(path):
    doc = SimpleDocTemplate(str(path), pagesize=A4)
    w = A4[0] - 5 * cm
    story = [
        Paragraph("Dokumentasi Kegiatan", TITLE),
        Spacer(1, 12),
        RLImage(photo_jpeg(1), width=w, height=w * 600 / 900),
        Spacer(1, 6),
        Paragraph("Foto 1. Suasana kegiatan (gambar buatan untuk uji).", CAPTION),
    ]
    doc.build(story)


def pindaian(path):
    from reportlab.pdfgen import canvas

    c = canvas.Canvas(str(path), pagesize=A4)
    from reportlab.lib.utils import ImageReader

    c.drawImage(ImageReader(photo_jpeg(2)), 0, 0, A4[0], A4[1])
    c.showPage()
    c.save()


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/docx-cases")
    out.mkdir(parents=True, exist_ok=True)
    jurnal(out / "jurnal.pdf")
    surat(out / "surat.pdf")
    foto(out / "foto.pdf")
    pindaian(out / "pindaian.pdf")
    print(out)


if __name__ == "__main__":
    main()
