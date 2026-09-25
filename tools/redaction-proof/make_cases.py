#!/usr/bin/env python3
"""The documents the redaction proof runs on (Phase 6, SPEC 17: "teks yang
diredaksi tidak dapat ditemukan lagi oleh alat ekstraksi mana pun").

Each case hides a *secret* — a token that appears nowhere else — in one of
the ways text and pictures reach a PDF, and shows a *public* token that must
survive. The cases are built by hand, object by object, so that each one is
exactly the construction it claims to be; a document from a word processor
would mix several and prove none of them in particular.

Every name and number is invented.

    python3 tools/redaction-proof/make_cases.py <out-dir>

Writes the PDFs and `cases.json`, which says what to mark and what to check.
"""
import io
import json
import struct
import sys
import zlib
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

OUT = Path(sys.argv[1] if len(sys.argv) > 1 else "tools/redaction-proof/.cases")
OUT.mkdir(parents=True, exist_ok=True)

DEJAVU = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
LIBERATION = "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf"
PUBLIC = "PUBLIKOK"


def secret(n):
    return f"QZX{n}-7391-KJV"


class Pdf:
    """A PDF written object by object."""

    def __init__(self):
        self.objects = []

    def add(self, body):
        self.objects.append(body if isinstance(body, bytes) else body.encode("latin-1"))
        return len(self.objects)

    def reserve(self):
        self.objects.append(b"null")
        return len(self.objects)

    def set(self, num, body):
        self.objects[num - 1] = body if isinstance(body, bytes) else body.encode("latin-1")

    def stream(self, data, extra="", compress=True):
        if isinstance(data, str):
            data = data.encode("latin-1")
        if compress:
            data = zlib.compress(data)
            extra += "/Filter/FlateDecode"
        return self.add(b"<<" + extra.encode("latin-1") + b"/Length %d>>\nstream\n" % len(data) + data + b"\nendstream")

    def write(self, path, pages, catalog_extra=""):
        """`pages`: list of page object numbers, reserved and filled by the caller."""
        pages_num = self.reserve()
        kids = " ".join(f"{p} 0 R" for p in pages)
        self.set(pages_num, f"<</Type/Pages/Kids[{kids}]/Count {len(pages)}>>")
        for p in pages:
            body = self.objects[p - 1].decode("latin-1").replace("/Parent PARENT", f"/Parent {pages_num} 0 R")
            self.set(p, body)
        root = self.add(f"<</Type/Catalog/Pages {pages_num} 0 R{catalog_extra}>>")
        out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        offsets = []
        for i, body in enumerate(self.objects):
            offsets.append(len(out))
            out += f"{i + 1} 0 obj\n".encode() + body + b"\nendobj\n"
        xref = len(out)
        out += f"xref\n0 {len(self.objects) + 1}\n0000000000 65535 f \n".encode()
        for o in offsets:
            out += f"{o:010} 00000 n \n".encode()
        out += f"trailer\n<</Size {len(self.objects) + 1}/Root {root} 0 R>>\nstartxref\n{xref}\n%%EOF\n".encode()
        Path(path).write_bytes(bytes(out))


def page(pdf, content, resources="", extra="", media="[0 0 595 842]"):
    c = pdf.stream(content)
    return pdf.add(f"<</Type/Page/Parent PARENT/MediaBox{media}/Resources<<{resources}>>/Contents {c} 0 R{extra}>>")


HELV = "/Font<</F1 {f} 0 R>>"


def helvetica(pdf):
    return pdf.add("<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>")


def public_line(y=500):
    return f"BT /F1 14 Tf 72 {y} Td ({PUBLIC} tetap terbaca) Tj ET\n"


CASES = []


def case(name, what, marks, **checks):
    CASES.append({"name": name, "what": what, "file": f"{name}.pdf", "secret": SECRETS[name], "marks": marks, **checks})


SECRETS = {}


def std14():
    n = "std14"
    SECRETS[n] = secret(1)
    pdf = Pdf()
    f = helvetica(pdf)
    p = page(pdf, f"BT /F1 14 Tf 72 700 Td (NIK: {SECRETS[n]}) Tj ET\n" + public_line(), HELV.format(f=f))
    pdf.write(OUT / f"{n}.pdf", [p])
    case(n, "Helvetica standar-14 tanpa /Widths", [{"page": 0, "find": SECRETS[n]}])


def truetype():
    """A TrueType subset with fractional widths, as reportlab writes it."""
    from reportlab.pdfbase import pdfmetrics
    from reportlab.pdfbase.ttfonts import TTFont
    from reportlab.pdfgen import canvas

    n = "truetype"
    SECRETS[n] = secret(2)
    pdfmetrics.registerFont(TTFont("LibSans", LIBERATION))
    c = canvas.Canvas(str(OUT / f"{n}.pdf"), pagesize=(595, 842))
    c.setFont("LibSans", 13)
    c.drawString(72, 700, f"Nomor rekening {SECRETS[n]} milik contoh")
    c.drawString(72, 500, f"{PUBLIC} tetap terbaca")
    c.showPage()
    c.save()
    case(n, "TrueType tertanam (subset, lebar pecahan)", [{"page": 0, "find": SECRETS[n]}])


def ttf_tables(path):
    data = open(path, "rb").read()
    num = struct.unpack(">H", data[4:6])[0]
    tables = {}
    for i in range(num):
        tag, _, off, length = struct.unpack(">4sIII", data[12 + 16 * i: 28 + 16 * i])
        tables[tag.decode()] = (off, length)
    return data, tables


def ttf_metrics(path):
    data, t = ttf_tables(path)
    head = t["head"][0]
    upm = struct.unpack(">H", data[head + 18: head + 20])[0]
    hhea = t["hhea"][0]
    nhm = struct.unpack(">H", data[hhea + 34: hhea + 36])[0]
    hmtx = t["hmtx"][0]
    widths = [struct.unpack(">H", data[hmtx + 4 * i: hmtx + 4 * i + 2])[0] for i in range(nhm)]
    cmap = t["cmap"][0]
    count = struct.unpack(">H", data[cmap + 2: cmap + 4])[0]
    sub = None
    for i in range(count):
        pid, eid, off = struct.unpack(">HHI", data[cmap + 4 + 8 * i: cmap + 12 + 8 * i])
        if pid == 3 and eid == 1:
            sub = cmap + off
    assert sub is not None and struct.unpack(">H", data[sub: sub + 2])[0] == 4
    segx2 = struct.unpack(">H", data[sub + 6: sub + 8])[0]
    seg = segx2 // 2
    ends = struct.unpack(f">{seg}H", data[sub + 14: sub + 14 + segx2])
    starts = struct.unpack(f">{seg}H", data[sub + 16 + segx2: sub + 16 + 2 * segx2])
    deltas = struct.unpack(f">{seg}h", data[sub + 16 + 2 * segx2: sub + 16 + 3 * segx2])
    ro_at = sub + 16 + 3 * segx2
    ranges = struct.unpack(f">{seg}H", data[ro_at: ro_at + segx2])

    def gid(ch):
        c = ord(ch)
        for i in range(seg):
            if starts[i] <= c <= ends[i]:
                if ranges[i] == 0:
                    return (c + deltas[i]) & 0xFFFF
                at = ro_at + 2 * i + ranges[i] + 2 * (c - starts[i])
                g = struct.unpack(">H", data[at: at + 2])[0]
                return (g + deltas[i]) & 0xFFFF if g else 0
        return 0

    def width(g):
        return widths[min(g, len(widths) - 1)] * 1000 / upm

    return data, gid, width


def cid():
    """Type0 / CIDFontType2 / Identity-H: two-byte codes that are glyph ids,
    the way browsers and office suites write text."""
    n = "cid"
    SECRETS[n] = secret(3)
    data, gid, width = ttf_metrics(DEJAVU)
    pdf = Pdf()
    text = f"Kode akses {SECRETS[n]} jangan dibagikan"
    pub = f"{PUBLIC} tetap terbaca"
    used = sorted({gid(ch) for ch in text + pub})
    ff = pdf.stream(data, f"/Length1 {len(data)}")
    desc = pdf.add(f"<</Type/FontDescriptor/FontName/DejaVuSans/Flags 32/FontBBox[-1021 -463 1793 1232]/ItalicAngle 0/Ascent 928/Descent -236/CapHeight 729/StemV 80/FontFile2 {ff} 0 R>>")
    w = " ".join(f"{g}[{width(g):.3f}]" for g in used)
    cidfont = pdf.add(f"<</Type/Font/Subtype/CIDFontType2/BaseFont/DejaVuSans/CIDSystemInfo<</Registry(Adobe)/Ordering(Identity)/Supplement 0>>/FontDescriptor {desc} 0 R/CIDToGIDMap/Identity/DW 600/W[{w}]>>")
    rev = {}
    for ch in text + pub:
        rev[gid(ch)] = ch
    bf = "\n".join(f"<{g:04X}> <{ord(ch):04X}>" for g, ch in sorted(rev.items()))
    tu = pdf.stream(f"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Izul def 1 begincodespacerange <0000> <FFFF> endcodespacerange {len(rev)} beginbfchar\n{bf}\nendbfchar endcmap CMapName currentdict /CMap defineresource pop end end")
    f = pdf.add(f"<</Type/Font/Subtype/Type0/BaseFont/DejaVuSans/Encoding/Identity-H/DescendantFonts[{cidfont} 0 R]/ToUnicode {tu} 0 R>>")

    def hexs(s):
        return "<" + "".join(f"{gid(ch):04X}" for ch in s) + ">"

    content = f"BT /F1 13 Tf 72 700 Td {hexs(text)} Tj ET\nBT /F1 14 Tf 72 500 Td {hexs(pub)} Tj ET\n"
    p = page(pdf, content, HELV.format(f=f))
    pdf.write(OUT / f"{n}.pdf", [p])
    case(n, "Type0 CID Identity-H (DejaVu Sans tertanam)", [{"page": 0, "find": SECRETS[n]}])


def tj_spacing():
    """Kerning inside the secret, character and word spacing, horizontal
    scaling: every term of the advance a removed glyph must give back."""
    n = "tj-spacing"
    SECRETS[n] = secret(4)
    s = SECRETS[n]
    pdf = Pdf()
    f = helvetica(pdf)
    content = (
        f"BT /F1 13 Tf 0.4 Tc 3 Tw 92 Tz 72 700 Td [(Isi: ) -40 ({s[:4]}) 60 ({s[4:9]}) -25 ({s[9:]}) (  lalu teks sesudahnya)] TJ ET\n"
        + public_line()
    )
    p = page(pdf, content, HELV.format(f=f))
    pdf.write(OUT / f"{n}.pdf", [p])
    case(n, "TJ dengan kerning, Tc, Tw, Tz", [{"page": 0, "find": s}])


def rotated():
    n = "rotated"
    SECRETS[n] = secret(5)
    pdf = Pdf()
    f = helvetica(pdf)
    content = f"BT /F1 14 Tf 0.866 0.5 -0.5 0.866 150 380 Tm (Diagonal {SECRETS[n]} miring) Tj ET\n" + public_line(700)
    p = page(pdf, content, HELV.format(f=f))
    pdf.write(OUT / f"{n}.pdf", [p])
    case(n, "Teks diputar 30 derajat", [{"page": 0, "find": SECRETS[n]}])


def form():
    n = "form"
    SECRETS[n] = secret(6)
    pdf = Pdf()
    f = helvetica(pdf)
    fx = pdf.stream(
        f"BT /F1 14 Tf 52 730 Td (Dalam form: {SECRETS[n]}) Tj ET",
        f"/Type/XObject/Subtype/Form/BBox[0 0 595 842]/Matrix[1 0 0 1 20 -30]/Resources<</Font<</F1 {f} 0 R>>>>",
    )
    p = page(pdf, "q /Fm1 Do Q\n" + public_line(), f"/Font<</F1 {f} 0 R>>/XObject<</Fm1 {fx} 0 R>>")
    pdf.write(OUT / f"{n}.pdf", [p])
    case(n, "Teks di dalam Form XObject bermatriks", [{"page": 0, "find": SECRETS[n]}])


def scan_image(text, size, font_px, at):
    """A grey 'scan' with `text` drawn at pixel `at`; returns (image, text box)."""
    img = Image.new("RGB", size, (250, 250, 246))
    d = ImageDraw.Draw(img)
    font = ImageFont.truetype(DEJAVU, font_px)
    d.text(at, text, fill=(20, 20, 20), font=font)
    box = d.textbbox(at, text, font=font)
    return img, box


def ocr_scan():
    """A scanned page: a JPEG with the secret in its pixels, and an invisible
    OCR text layer over it — the case redaction exists for."""
    n = "ocr-scan"
    SECRETS[n] = secret(7)
    s = SECRETS[n]
    W, H = 1400, 500
    img, box = scan_image(f"Rekening: {s}", (W, H), 56, (60, 200))
    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=92)
    pdf = Pdf()
    f = helvetica(pdf)
    im = pdf.stream(buf.getvalue(), f"/Type/XObject/Subtype/Image/Width {W}/Height {H}/ColorSpace/DeviceRGB/BitsPerComponent 8/Filter/DCTDecode", compress=False)
    # The image is drawn 475 x 170 pt at (60, 560): 1 px = 475/1400 pt.
    sx, sy, x0, y0 = 475 / W, 170 / H, 60, 560
    px = lambda x: x0 + x * sx
    py = lambda y: y0 + (H - y) * sy
    # The secret's pixel box in page space, a little wider: the area a user
    # would drag over the picture.
    area = [px(box[0]) - 3, py(box[3]) - 3, px(box[2]) + 3, py(box[1]) + 3]
    # Invisible text over the words, as an OCR engine lays it: Helvetica
    # scaled so its width matches the picture's.
    text_w = (box[2] - box[0]) * sx
    size = 18
    from reportlab.pdfbase.pdfmetrics import stringWidth

    helv_w = stringWidth(f"Rekening: {s}", "Helvetica", size)
    tz = 100 * text_w / helv_w
    content = (
        f"q 475 0 0 170 {x0} {y0} cm /Im1 Do Q\n"
        f"BT 3 Tr /F1 {size} Tf {tz:.2f} Tz {px(box[0]):.2f} {py(box[3]) + 4:.2f} Td (Rekening: {s}) Tj ET\n"
        + public_line(400)
    )
    p = page(pdf, content, f"/Font<</F1 {f} 0 R>>/XObject<</Im1 {im} 0 R>>")
    pdf.write(OUT / f"{n}.pdf", [p])
    case(
        n,
        "Pindaian JPEG + lapisan OCR tak terlihat (Tr 3)",
        [{"page": 0, "rect": area}, {"page": 0, "find": s}],
        pixels={"image": [W, H], "box": [box[0], box[1], box[2], box[3]]},
    )


def inline_image():
    n = "inline-image"
    SECRETS[n] = secret(12)
    W, H = 300, 60
    img, box = scan_image(SECRETS[n], (W, H), 24, (10, 14))
    grey = img.convert("L")
    pdf = Pdf()
    f = helvetica(pdf)
    data = grey.tobytes().hex().upper() + ">"
    content = (
        f"q 300 0 0 60 72 650 cm BI /W {W} /H {H} /CS /G /BPC 8 /F /AHx ID {data}\nEI Q\n" + public_line()
    )
    area = [72 + box[0] - 2, 650 + (H - box[3]) - 2, 72 + box[2] + 2, 650 + (H - box[1]) + 2]
    p = page(pdf, content, HELV.format(f=f))
    pdf.write(OUT / f"{n}.pdf", [p])
    case(n, "Gambar inline berisi teks (piksel)", [{"page": 0, "rect": area}], pixels={"image": [W, H], "box": list(box), "inline": True})


def type3():
    n = "type3"
    SECRETS[n] = secret(8)
    pdf = Pdf()
    proc = pdf.stream("600 0 0 0 500 700 d1 60 0 440 700 re f")
    codes = range(32, 123)
    diffs = " ".join("/g" for _ in codes)
    tu = pdf.stream(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T3 def 1 begincodespacerange <00> <FF> endcodespacerange "
        f"1 beginbfrange <20> <7A> <0020> endbfrange endcmap CMapName currentdict /CMap defineresource pop end end"
    )
    widths = " ".join("600" for _ in codes)
    f = pdf.add(
        f"<</Type/Font/Subtype/Type3/FontBBox[0 0 600 700]/FontMatrix[0.001 0 0 0.001 0 0]/CharProcs<</g {proc} 0 R>>"
        f"/Encoding<</Type/Encoding/Differences[32 {diffs}]>>/FirstChar 32/LastChar 122/Widths[{widths}]/ToUnicode {tu} 0 R/Resources<<>>>>"
    )
    h = helvetica(pdf)
    content = f"BT /F2 14 Tf 72 700 Td (Sandi {SECRETS[n]} tipe tiga) Tj ET\n" + public_line()
    p = page(pdf, content, f"/Font<</F1 {h} 0 R/F2 {f} 0 R>>")
    pdf.write(OUT / f"{n}.pdf", [p])
    case(n, "Font Type 3 dengan ToUnicode", [{"page": 0, "find": SECRETS[n]}])


def actualtext():
    """Glyphs that say one thing and an /ActualText that says another: an
    extractor that honours ActualText reads the secret without it being drawn."""
    n = "actualtext"
    SECRETS[n] = secret(9)
    pdf = Pdf()
    f = helvetica(pdf)
    content = (
        f"/Span <</ActualText ({SECRETS[n]})>> BDC BT /F1 14 Tf 72 700 Td (xxxxxxxxxxxxxxx) Tj ET EMC\n" + public_line()
    )
    p = page(pdf, content, HELV.format(f=f))
    pdf.write(OUT / f"{n}.pdf", [p])
    # 15 x's of Helvetica at 14 pt (500/1000 em) from x = 72.
    case(n, "Teks pengganti /ActualText di marked content", [{"page": 0, "rect": [70, 695, 72 + 15 * 7 + 2, 715]}])


def annotations():
    n = "annotations"
    SECRETS[n] = secret(10)
    s = SECRETS[n]
    pdf = Pdf()
    f = helvetica(pdf)
    pg = pdf.reserve()
    ap = pdf.stream(f"BT /F1 10 Tf 2 6 Td ({s}) Tj ET", f"/Type/XObject/Subtype/Form/BBox[0 0 200 24]/Resources<</Font<</F1 {f} 0 R>>>>")
    note = pdf.add(f"<</Type/Annot/Subtype/FreeText/Rect[72 690 272 714]/Contents({s})/DA(/F1 10 Tf 0 g)/AP<</N {ap} 0 R>>/P {pg} 0 R>>")
    field_ap = pdf.stream(f"/Tx BMC BT /F1 10 Tf 2 6 Td ({s}) Tj ET EMC", f"/Type/XObject/Subtype/Form/BBox[0 0 200 24]/Resources<</Font<</F1 {f} 0 R>>>>")
    widget = pdf.add(f"<</Type/Annot/Subtype/Widget/FT/Tx/T(nomor)/V({s})/DA(/F1 10 Tf 0 g)/Rect[72 640 272 664]/AP<</N {field_ap} 0 R>>/P {pg} 0 R>>")
    c = pdf.stream("BT /F1 12 Tf 72 720 Td (Catatan dan isian:) Tj ET\n" + public_line())
    pdf.set(pg, f"<</Type/Page/Parent PARENT/MediaBox[0 0 595 842]/Resources<</Font<</F1 {f} 0 R>>>>/Contents {c} 0 R/Annots[{note} 0 R {widget} 0 R]>>")
    pdf.write(OUT / f"{n}.pdf", [pg], catalog_extra=f"/AcroForm<</Fields[{widget} 0 R]/DA(/F1 10 Tf 0 g)/DR<</Font<</F1 {f} 0 R>>>>>>")
    case(n, "Isi anotasi FreeText dan nilai field formulir", [{"page": 0, "rect": [60, 630, 290, 716]}])


def page_rotate():
    n = "page-rotate"
    SECRETS[n] = secret(11)
    pdf = Pdf()
    f = helvetica(pdf)
    content = f"BT /F1 14 Tf 102 740 Td (Alamat {SECRETS[n]} rahasia) Tj ET\nBT /F1 14 Tf 102 540 Td ({PUBLIC} tetap terbaca) Tj ET\n"
    p = page(pdf, content, HELV.format(f=f), extra="/Rotate 90", media="[30 40 625 882]")
    pdf.write(OUT / f"{n}.pdf", [p])
    case(n, "Halaman /Rotate 90 dengan MediaBox tidak di titik nol", [{"page": 0, "find": SECRETS[n]}])


if __name__ == "__main__":
    for make in (std14, truetype, cid, tj_spacing, rotated, form, ocr_scan, inline_image, type3, actualtext, annotations, page_rotate):
        make()
    (OUT / "cases.json").write_text(json.dumps({"public": PUBLIC, "cases": CASES}, indent=1))
    print(f"{len(CASES)} kasus ditulis ke {OUT}")
