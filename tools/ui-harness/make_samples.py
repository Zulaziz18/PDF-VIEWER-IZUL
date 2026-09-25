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


def agreement(path, version):
    """Two versions of one made-up agreement, for compare mode: a few words
    changed, a sentence added, one figure in the table different."""
    c = canvas.Canvas(str(path))
    c.setTitle(f"Draf Perjanjian Kerja Sama {version}")
    fee = "Rp 12.500.000" if version == "v1" else "Rp 14.000.000"
    days = "30 (tiga puluh)" if version == "v1" else "14 (empat belas)"
    extra = "" if version == "v1" else " Keterlambatan pembayaran dikenakan denda satu persen per bulan."
    clauses = [
        [
            "Perjanjian ini dibuat antara Pihak Pertama, sebuah lembaga pelatihan contoh, dan Pihak Kedua, "
            "seorang penyedia jasa desain contoh. Kedua pihak sepakat bekerja sama dalam penyusunan materi pelatihan.",
            f"Nilai pekerjaan sebesar {fee} dibayar dalam dua tahap setelah hasil kerja diterima.{extra}",
            f"Pembayaran dilakukan paling lambat {days} hari kalender sejak tagihan diterima oleh Pihak Pertama.",
        ],
        [
            "Pihak Kedua menyerahkan rancangan awal dalam waktu tiga minggu sejak perjanjian ditandatangani.",
            "Revisi dilakukan paling banyak dua kali untuk setiap bagian materi."
            if version == "v1" else "Revisi dilakukan paling banyak tiga kali untuk setiap bagian materi.",
            "Hak cipta atas materi yang telah dibayar lunas beralih kepada Pihak Pertama.",
        ],
        [
            "Perselisihan diselesaikan secara musyawarah. Bila tidak tercapai kesepakatan, kedua pihak memilih "
            "penyelesaian melalui mediasi.",
            "Perjanjian ini dibuat dalam dua rangkap yang sama kuatnya.",
        ],
    ]
    table = [["Tahap", "Porsi", "Syarat"], ["Pertama", "50%", "Rancangan awal"],
             ["Kedua", "50%" if version == "v1" else "40%", "Materi final"]]
    if version == "v2":
        table.append(["Ketiga", "10%", "Masa garansi"])
    for i, paragraphs in enumerate(clauses):
        body_page(c, i + 1, f"Pasal {i + 1}", paragraphs, table if i == 0 else None)
    c.save()


def personnel(path):
    """Phase 6: a staff list with identity and phone numbers, the kind of page
    that gets redacted before it is shared. Every name and number is made up;
    the identity numbers follow no real registry's format beyond their length.
    Helvetica without embedding, so redaction measures it from the standard
    metrics."""
    c = canvas.Canvas(str(path), pagesize=A4)
    c.setTitle("Data Pegawai (contoh)")
    w, h = A4
    c.setFillColor(NAVY)
    c.setFont("Helvetica-Bold", 18)
    c.drawString(56, h - 72, "Data Pegawai Unit Pelatihan")
    c.setFillColor(HexColor("#555555"))
    c.setFont("Helvetica", 10)
    c.drawString(56, h - 90, "Lampiran surat tugas - untuk dibagikan setelah nomor pribadi disensor.")
    rows = [
        ("Nama", "Jabatan", "NIK", "Telepon"),
        ("Ayu Lestari", "Koordinator", "3174012345670001", "0812-3456-7801"),
        ("Bima Santoso", "Instruktur", "3174012345670002", "0812-3456-7802"),
        ("Citra Wulandari", "Instruktur", "3174012345670003", "0812-3456-7803"),
        ("Dimas Pratama", "Administrasi", "3174012345670004", "0812-3456-7804"),
        ("Eka Rahmawati", "Keuangan", "3174012345670005", "0812-3456-7805"),
        ("Fajar Nugroho", "Teknisi", "3174012345670006", "0812-3456-7806"),
    ]
    xs = [56, 186, 296, 436]
    y = h - 130
    for i, row in enumerate(rows):
        if i == 0:
            c.setFillColor(HexColor("#e8eef8"))
            c.rect(50, y - 6, w - 100, 22, stroke=0, fill=1)
        c.setFillColor(HexColor("#222222"))
        c.setFont("Helvetica-Bold" if i == 0 else "Helvetica", 10.5)
        for x, cell in zip(xs, row):
            c.drawString(x, y, cell)
        c.setStrokeColor(HexColor("#dddddd"))
        c.line(50, y - 8, w - 50, y - 8)
        y -= 26
    y -= 20
    y = wrap(c, "Nomor induk kependudukan dan nomor telepon di atas adalah data pribadi. "
             "Sebelum lampiran ini dikirim ke pihak lain, kedua kolom itu harus diredaksi, "
             "bukan sekadar ditutup kotak hitam: kotak yang digambar di atas teks masih "
             "menyisakan teksnya di dalam berkas.", 56, y, w - 112)
    c.setFillColor(HexColor("#888888"))
    c.setFont("Helvetica-Oblique", 9)
    c.drawString(56, 56, "Semua nama dan nomor di halaman ini karangan, dibuat untuk contoh tampilan.")
    c.showPage()
    c.save()


guide(OUT / "Panduan Studi 2026.pdf")
personnel(OUT / "Data Pegawai.pdf")
agreement(OUT / "Draf Perjanjian v1.pdf", "v1")
agreement(OUT / "Draf Perjanjian v2.pdf", "v2")
report(OUT / "Laporan Kegiatan Semester.pdf", "Laporan Kegiatan", 9)
report(OUT / "Catatan Rapat Organisasi.pdf", "Catatan Rapat", 5)
report(OUT / "Proposal Penelitian.pdf", "Proposal Penelitian", 14)


def stamp_photo(path):
    """A stamp photographed on paper, for the "Hapus Latar" scenes."""
    from PIL import Image, ImageDraw

    w, h = 360, 360
    img = Image.new("RGB", (w, h), (246, 243, 236))
    px = img.load()
    for y in range(h):
        for x in range(w):
            f = 1 - 0.2 * ((x / w) ** 2 + (y / h) * 0.5)
            r, g, b = px[x, y]
            px[x, y] = (int(r * f), int(g * f), int(b * f))
    d = ImageDraw.Draw(img)
    ink = (170, 30, 40)
    d.ellipse((40, 40, 320, 320), outline=ink, width=12)
    d.ellipse((90, 90, 270, 270), outline=ink, width=6)
    d.rectangle((105, 160, 255, 200), fill=ink)
    img.save(path)


stamp_photo(OUT / "stempel.png")


def registration_form(path):
    """An AcroForm, for the "Formulir" scenes (Phase 7): one field of each
    kind the panel draws. Invented content."""
    c = canvas.Canvas(str(path), pagesize=A4)
    w, h = A4
    c.setTitle("Formulir Pendaftaran (contoh)")
    c.setFont("Helvetica-Bold", 18)
    c.drawString(56, h - 72, "Formulir Pendaftaran Pelatihan")
    c.setFont("Helvetica", 10)
    c.setFillColor(HexColor("#555555"))
    c.drawString(56, h - 92, "Isi semua bagian, lalu simpan berkas ini.")
    c.setFillColor(HexColor("#000000"))
    c.setFont("Helvetica", 11)
    form = c.acroForm
    black, white = HexColor("#000000"), HexColor("#ffffff")
    c.drawString(56, h - 140, "Nama lengkap")
    form.textfield(name="nama_lengkap", x=190, y=h - 148, width=320, height=22, borderColor=black,
                   fillColor=white, fontSize=11)
    c.drawString(56, h - 180, "Alamat")
    form.textfield(name="alamat", x=190, y=h - 250, width=320, height=80, borderColor=black, fillColor=white,
                   fontSize=10, fieldFlags="multiline")
    c.drawString(56, h - 280, "Kategori peserta")
    for i, (value, label) in enumerate([("Umum", "Umum"), ("Pelajar", "Pelajar"), ("Pengajar", "Pengajar")]):
        form.radio(name="kategori", value=value, x=190 + i * 110, y=h - 286, size=16, borderColor=black,
                   fillColor=white, selected=False)
        c.drawString(212 + i * 110, h - 282, label)
    c.drawString(56, h - 320, "Kota")
    form.choice(name="kota", x=190, y=h - 328, width=200, height=22,
                options=["Bandung", "Jakarta", "Surabaya", "Yogyakarta"], value="Bandung", fieldFlags="combo",
                borderColor=black, fillColor=white, fontSize=11)
    c.drawString(56, h - 364, "Setuju dengan tata tertib")
    form.checkbox(name="setuju", x=240, y=h - 370, size=16, buttonStyle="check", borderColor=black,
                  fillColor=white, checked=False)
    c.setFillColor(HexColor("#888888"))
    c.setFont("Helvetica-Oblique", 9)
    c.drawString(56, 56, "Formulir karangan, dibuat untuk contoh tampilan.")
    c.showPage()
    c.save()


registration_form(OUT / "Formulir Pendaftaran.pdf")
print(f"sampel ditulis ke {OUT}")
