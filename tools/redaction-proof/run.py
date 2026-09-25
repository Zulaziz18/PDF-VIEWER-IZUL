#!/usr/bin/env python3
"""The redaction proof (Phase 6, SPEC 17): what is redacted cannot be found
again by any extraction tool.

For every case of `make_cases.py`:

1. The application's own redaction runs on it (`izul-bench --bin
   redact-proof`: the routine the worker runs, then PDFium's full save).
2. Six extractors that share no code with each other look for the secret,
   in the original and in the result:
     - PDFium (the engine the application uses — and Chrome, and Edge),
     - poppler `pdftotext`,
     - MuPDF `mutool`,
     - pypdf,
     - pdfminer.six,
     - pikepdf / qpdf reading the file's insides: every string object
       anywhere (annotation contents, field values, /ActualText, metadata),
       every string operand of every content stream, and the decoded bytes
       of every stream.
   A tool that cannot find the secret in the *original* proves nothing for
   that case and is reported as "t/b" (tidak berlaku), never as a pass.
3. The public token must still be found by every text extractor that found
   it before — redaction takes what was marked, not the page.
4. Where the secret is in pixels, the pixels under it must now be uniform.
5. MuPDF renders the page before and after; outside the redacted areas the
   two must be the same picture — nothing else moved or changed colour.

    python3 tools/redaction-proof/run.py [--out bench/results/phase6-redaction.txt]

Needs poppler-utils, mupdf-tools, and pypdf, pdfminer.six, pikepdf, Pillow.
Exits non-zero if anything fails.
"""
import argparse
import io
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import pikepdf
from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, check=False, **kw)


# ---------------------------------------------------------------------------
# Extractors
# ---------------------------------------------------------------------------

def poppler(path):
    r = run(["pdftotext", "-q", str(path), "-"])
    return r.stdout


def mupdf(path):
    r = run(["mutool", "draw", "-q", "-F", "txt", "-o", "-", str(path)])
    return r.stdout


def pypdf_text(path):
    from pypdf import PdfReader

    return "\n".join(p.extract_text() or "" for p in PdfReader(str(path)).pages)


def pdfminer_text(path):
    from pdfminer.high_level import extract_text

    return extract_text(str(path))


def insides(path):
    """Everything readable in the file without laying anything out."""
    out = []
    with pikepdf.open(str(path)) as pdf:
        def walk(obj, depth=0):
            if depth > 40:
                return
            if isinstance(obj, pikepdf.String):
                out.append(bytes(obj).decode("latin-1"))
                try:
                    out.append(str(obj))
                except Exception:
                    pass
            elif isinstance(obj, pikepdf.Array):
                for x in obj:
                    walk(x, depth + 1)
            elif isinstance(obj, (pikepdf.Dictionary, pikepdf.Stream)):
                for k in obj.keys():
                    v = obj.get(k)
                    if isinstance(v, (pikepdf.Dictionary, pikepdf.Stream)) and v.is_indirect:
                        continue  # visited on its own below
                    walk(v, depth + 1)
        for obj in pdf.objects:
            walk(obj)
            if isinstance(obj, pikepdf.Stream):
                try:
                    data = obj.read_bytes()
                except Exception:
                    continue
                out.append(data.decode("latin-1"))
                try:
                    for operands, _ in pikepdf.parse_content_stream(obj):
                        stack = list(operands)
                        while stack:
                            o = stack.pop()
                            if isinstance(o, pikepdf.String):
                                out.append(bytes(o).decode("latin-1"))
                            elif isinstance(o, pikepdf.Array):
                                stack.extend(o)
                except Exception:
                    pass
        for page in pdf.pages:
            try:
                for operands, op in pikepdf.parse_content_stream(page):
                    if str(op) == "INLINE IMAGE":
                        continue
                    for o in operands:
                        if isinstance(o, pikepdf.String):
                            out.append(bytes(o).decode("latin-1"))
            except Exception:
                pass
    return "\n".join(out)


def found(text, secret):
    """The secret, or any piece of it that could only be it."""
    flat = re.sub(r"\s+", "", text)
    n = secret.split("-")[0]  # QZX<n>
    return any(piece in flat for piece in (secret, n, "7391", "KJV"))


# ---------------------------------------------------------------------------
# Pixels
# ---------------------------------------------------------------------------

def images_of(path):
    """Every image drawn on page 1, decoded — XObjects and inline ones."""
    out = []
    with pikepdf.open(str(path)) as pdf:
        page = pdf.pages[0]
        for _, raw in page.images.items():
            try:
                out.append(pikepdf.PdfImage(raw).as_pil_image())
            except Exception:
                pass
        for operands, op in pikepdf.parse_content_stream(page):
            if str(op) == "INLINE IMAGE":
                try:
                    out.append(operands[0].as_pil_image())
                except Exception:
                    pass
    return out


def region_uniform(img, box):
    crop = img.convert("L").crop(tuple(box))
    lo, hi = crop.getextrema()
    return hi - lo <= 2


RENDER_DPI = 288


def render(path, dpi=RENDER_DPI):
    with tempfile.TemporaryDirectory() as d:
        out = Path(d) / "p.png"
        run(["mutool", "draw", "-q", "-r", str(dpi), "-o", str(out), str(path), "1"])
        return Image.open(out).convert("RGB").copy()


def page_box(path):
    with pikepdf.open(str(path)) as pdf:
        p = pdf.pages[0]
        box = [float(x) for x in p.obj.get("/CropBox", p.obj.MediaBox)]
        rot = int(p.obj.get("/Rotate", 0)) % 360
    return box, rot


def to_pixels(area, box, rot, scale):
    """A user-space rectangle to pixel bounds of the rendered page."""
    l, b, r, t = box
    bw, bh = r - l, t - b
    corners = [(area[0], area[1]), (area[2], area[3])]
    pts = []
    for x, y in corners:
        u, v = x - l, y - b
        if rot == 0:
            dx, dy, dh = u, v, bh
        elif rot == 90:
            dx, dy, dh = v, bw - u, bw
        elif rot == 180:
            dx, dy, dh = bw - u, bh - v, bh
        else:
            dx, dy, dh = bh - v, u, bw
        pts.append((dx * scale, (dh - dy) * scale))
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    return [min(xs), min(ys), max(xs), max(ys)]


def outside_unchanged(before, after, areas, box, rot, beyond=(), grow_pt=1.0):
    """Renders compared outside the areas, one pixel of slack each way.

    Not a plain difference: MuPDF snaps glyph origins to a subpixel grid, and
    a shift far below anything visible can flip a glyph into the next bucket
    (measured: moving a line 0.0056 pt, with no redaction at all, changes
    pixels by 65 at 72 dpi; 0.002 pt changes nothing). So a pixel passes when
    it lies within the range of its 3x3 neighbourhood in the other render —
    at 288 dpi that is 0.25 pt, far less than any visible move, and the text
    positions are checked much tighter by `positions_kept`. A colour change or
    anything drawn or removed still fails.
    """
    from PIL import ImageDraw, ImageFilter

    if before.size != after.size:
        return False, "ukuran render berbeda"
    scale = RENDER_DPI / 72
    mask = Image.new("L", before.size, 255)
    d = ImageDraw.Draw(mask)
    grow = grow_pt * scale
    for a in areas:
        x0, y0, x1, y1 = to_pixels(a, box, rot, scale)
        d.rectangle([x0 - grow, y0 - grow, x1 + grow, y1 + grow], fill=0)
    # What the redaction itself reports it took past an area (a glyph mostly
    # inside one, covered where it was) is not "something else".
    for q in beyond:
        pts = [tuple(to_pixels([x, y, x, y], box, rot, scale)[:2]) for x, y in q]
        d.polygon(pts, fill=0)
        # Pillow draws a polygon's outline inside it; a line along the edges
        # grows it outwards as the rectangles are grown.
        d.line(pts + [pts[0]], fill=0, width=max(1, round(2 * grow)), joint="curve")
    worst = 0
    for one, other in ((before, after), (after, before)):
        lo, hi = one.filter(ImageFilter.MinFilter(3)), one.filter(ImageFilter.MaxFilter(3))
        out = ImageChops.lighter(ImageChops.subtract(lo, other), ImageChops.subtract(other, hi))
        out = ImageChops.multiply(out.convert("L"), mask)
        worst = max(worst, out.getextrema()[1])
    return worst <= 8, f"selisih maks {worst}"


def words_poppler(path):
    """(text, x, y) of every word, from poppler's own layout."""
    r = run(["pdftotext", "-q", "-bbox", str(path), "-"])
    out = []
    for m in re.finditer(r'<word xMin="([\d.]+)" yMin="([\d.]+)" xMax="[\d.]+" yMax="[\d.]+">([^<]*)</word>', r.stdout):
        out.append((m.group(3), float(m.group(1)), float(m.group(2))))
    return out, 0.0


def words_mupdf(path):
    """(text, x, y) of every word from MuPDF's structured text, and the
    largest font size on the page."""
    r = run(["mutool", "draw", "-q", "-F", "stext", "-o", "-", str(path), "1"])
    out, size = [], 0.0
    for line in re.finditer(r"<line [^>]*>(.*?)</line>", r.stdout, re.S):
        word, at = "", None
        for fm in re.finditer(r'<font [^>]*size="([\d.]+)"|<char [^>]*x="([-\d.]+)" y="([-\d.]+)"[^>]*c="([^"]*)"', line.group(1)):
            if fm.group(1):
                size = max(size, float(fm.group(1)))
                continue
            c = fm.group(4)
            if c.strip() == "":
                if word:
                    out.append((word, *at))
                word, at = "", None
                continue
            if not word:
                at = (float(fm.group(2)), float(fm.group(3)))
            word += c
        if word:
            out.append((word, *at))
    return out, size


def positions_kept(before, after):
    """How far the words still on the page moved: every word of the result
    is matched to the nearest word with the same text in the original.
    Returns (largest move in points, words not found before)."""
    worst, unmatched = 0.0, 0
    for text, x, y in after:
        dists = [((x - bx) ** 2 + (y - by) ** 2) ** 0.5 for t, bx, by in before if t == text]
        if not dists:
            unmatched += 1
            continue
        worst = max(worst, min(dists))
    return worst, unmatched


# Poppler lays glyphs out with the exact /Widths, as the specification does,
# and so does the redaction's TJ gap: nothing may move at all.
POPPLER_TOLERANCE = 0.01
# MuPDF rounds each /Widths entry to a whole 1/1000 em (measured on the
# truetype case: 7336.4255 units exact, 7336 rounded, predicted shift
# 0.00553 pt, measured 0.0056 pt). A removed glyph can therefore move the
# rest of its line by up to half a unit in MuPDF — per glyph removed.
MUPDF_PER_GLYPH_EM = 0.0005


# ---------------------------------------------------------------------------

TOOLS = [
    ("PDFium", None),
    ("poppler", poppler),
    ("MuPDF", mupdf),
    ("pypdf", pypdf_text),
    ("pdfminer", pdfminer_text),
    ("isi mentah", insides),
]


def mutated(src, dst, prefix=b"", suffix=b""):
    """A copy of a one-page file with bytes put around its content."""
    with pikepdf.open(str(src)) as pdf:
        page = pdf.pages[0]
        body = b"".join(c.read_bytes() for c in page.obj.Contents) if isinstance(page.obj.Contents, pikepdf.Array) else page.obj.Contents.read_bytes()
        page.obj.Contents = pdf.make_stream(prefix + b"\nq\n" + body + b"\nQ\n" + suffix)
        pdf.save(str(dst))


def checks_can_fail(work, cases, results):
    """The outside checks, run on an unredacted case changed on purpose.

    A check that cannot fail proves nothing (Phase 5's lesson), so each of
    these must be caught by the check it is meant for."""
    case = next(c for c in cases if c["name"] == "std14")
    src = work / case["file"]
    areas = [a for p in results["std14"]["result"]["pages"] for a in p["areas"]]
    box, rot = page_box(src)
    out = []
    probes = [
        ("halaman digeser 0,5 pt", dict(prefix=b"1 0 0 1 0.5 0 cm"), "render"),
        ("kotak abu-abu 3x3 pt digambar", dict(suffix=b"q 0.8 g 40 40 3 3 re f Q"), "render"),
        ("warna teks jadi abu gelap", dict(prefix=b"0.3 g"), "render"),
        ("halaman digeser 0,05 pt", dict(prefix=b"1 0 0 1 0.05 0 cm"), "posisi"),
    ]
    for label, how, check in probes:
        dst = work / "mutasi.pdf"
        # The colour probe needs the text itself recoloured: std14 sets no
        # colour, so a fill colour set before it is what the text uses.
        mutated(src, dst, **how)
        if check == "render":
            same, why = outside_unchanged(render(src), render(dst), areas, box, rot)
            caught = not same
        else:
            moved, _ = positions_kept(words_poppler(src)[0], words_poppler(dst)[0])
            caught, why = moved > POPPLER_TOLERANCE, f"geser {moved:.4f} pt"
        out.append(("ok " if caught else "TIDAK TERTANGKAP ") + f"{label}: {why}")
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(ROOT / "bench/results/phase6-redaction.txt"))
    ap.add_argument("--keep", default=None, help="folder untuk kasus dan hasilnya")
    args = ap.parse_args()

    work = Path(args.keep) if args.keep else Path(tempfile.mkdtemp(prefix="izul-redaksi-"))
    work.mkdir(parents=True, exist_ok=True)
    subprocess.run([sys.executable, str(HERE / "make_cases.py"), str(work)], check=True, stdout=subprocess.DEVNULL)
    spec = json.loads((work / "cases.json").read_text())
    public = spec["public"]
    cases = spec["cases"]
    job = {
        "cases": [
            {"name": c["name"], "in": str(work / c["file"]), "out": str(work / f"{c['name']}.redaksi.pdf"), "marks": c["marks"]}
            for c in cases
        ]
    }
    (work / "job.json").write_text(json.dumps(job))
    subprocess.run(["cargo", "build", "-q", "-p", "izul-bench", "--bin", "redact-proof"], cwd=ROOT, check=True)
    exe = ROOT / "target/debug" / ("redact-proof.exe" if sys.platform == "win32" else "redact-proof")
    r = run([str(exe), str(work / "job.json")], cwd=ROOT)
    if r.returncode != 0:
        print(r.stderr, file=sys.stderr)
        sys.exit(2)
    results = {json.loads(l)["name"]: json.loads(l) for l in r.stdout.splitlines() if l.strip()}

    failures = []
    rows = []
    for c in cases:
        name = c["name"]
        res = results.get(name)
        src = work / c["file"]
        dst = work / f"{name}.redaksi.pdf"
        row = {"case": name, "what": c["what"]}
        if not res or not res["ok"]:
            failures.append(f"{name}: redaksi gagal: {res and res.get('error')}")
            row["error"] = res and res.get("error")
            rows.append(row)
            continue
        r = res["result"]
        page = r["pages"][0] if r["pages"] else {}
        row["glyphs"] = page.get("glyphs", 0)
        row["images"] = page.get("images_cleared", 0) + page.get("images_removed", 0)
        row["annots"] = page.get("annotations", 0)
        row["marked"] = page.get("marked_content", 0)
        cells = {}
        for tool, fn in TOOLS:
            if fn is None:
                before, after = "\n".join(r["pdfium_before"]), "\n".join(r["pdfium_text"])
            else:
                before, after = fn(src), fn(dst)
            had, has = found(before, c["secret"]), found(after, c["secret"])
            keeps = public in re.sub(r"\s+", " ", after) or tool == "isi mentah"
            if not had:
                cells[tool] = "t/b"
            elif has:
                cells[tool] = "BOCOR"
                failures.append(f"{name}: {tool} masih menemukan rahasia")
            else:
                cells[tool] = "hilang"
            if tool != "isi mentah" and public in re.sub(r"\s+", " ", before) and not keeps:
                cells[tool] += " (publik hilang!)"
                failures.append(f"{name}: {tool} tidak lagi menemukan teks publik")
        row["tools"] = cells
        if "pixels" in c:
            px = c["pixels"]
            want = tuple(px["image"])
            pick = lambda imgs: next((i for i in imgs if i.size == want), None)
            ib, ia = pick(images_of(src)), pick(images_of(dst))
            if ib is None or ia is None:
                row["pixels"] = "gambar tidak ditemukan"
                failures.append(f"{name}: gambar sebelum/sesudah tidak ditemukan")
            else:
                sane = not region_uniform(ib, px["box"])
                ok = region_uniform(ia, px["box"])
                row["pixels"] = ("seragam" if ok else "MASIH ADA") if sane else "t/b"
                if sane and not ok:
                    failures.append(f"{name}: piksel rahasia masih ada")
        box, rot = page_box(src)
        areas = [a for p in r["pages"] for a in p["areas"]]
        beyond = [q for p in r["pages"] for q in p["beyond"]]
        row["beyond"] = len(beyond)
        same, why = outside_unchanged(render(src), render(dst), areas, box, rot, beyond)
        row["outside"] = "sama" if same else "BERUBAH"
        if not same:
            failures.append(f"{name}: render di luar area berubah ({why})")
        (pb, _), (pa, _) = words_poppler(src), words_poppler(dst)
        (mb, size), (ma, _) = words_mupdf(src), words_mupdf(dst)
        moved_p, _ = positions_kept(pb, pa)
        moved_m, _ = positions_kept(mb, ma)
        allowed_m = row["glyphs"] * MUPDF_PER_GLYPH_EM * max(size, 1.0) + POPPLER_TOLERANCE
        row["moved"] = f"{moved_p:.4f}/{moved_m:.4f}"
        if moved_p > POPPLER_TOLERANCE:
            failures.append(f"{name}: kata di luar area bergeser {moved_p:.4f} pt menurut poppler (batas {POPPLER_TOLERANCE})")
        if moved_m > allowed_m:
            failures.append(f"{name}: kata di luar area bergeser {moved_m:.4f} pt menurut MuPDF (batas {allowed_m:.4f})")
        rows.append(row)

    sanity = checks_can_fail(work, cases, results)
    failures += [f"pemeriksaan tidak bisa gagal: {x}" for x in sanity if not x.startswith("ok ")]

    lines = [
        "Fase 6 — bukti redaksi: rahasia dicari ulang oleh enam alat ekstraksi",
        "",
        "hilang = alat menemukan rahasia di berkas asli dan tidak lagi di hasil redaksi",
        "t/b    = alat tidak menemukan rahasia bahkan di berkas asli (tidak membuktikan apa pun)",
        "Piksel = wilayah gambar di bawah rahasia kini seragam; Luar area = render MuPDF 288 dpi sebelum/sesudah sama di luar area",
        "Luar   = bagian yang ikut diambil di luar area (glyph yang sebagian besar di dalam area — ditutup warna area —,",
        "         gambar atau anotasi yang dibuang utuh); dilaporkan redaksi sendiri dan dikecualikan dari 'luar area'",
        f"Geser  = pergeseran terbesar kata di luar area, poppler/MuPDF, dalam pt (batas poppler {POPPLER_TOLERANCE};",
        f"         MuPDF membulatkan /Widths, batasnya {MUPDF_PER_GLYPH_EM} em per glyph yang dibuang)",
        "",
    ]
    head = f"{'kasus':<13}{'glyph':>6}{'gbr':>5}{'anot':>5}{'luar':>5}  " + "".join(f"{t:<12}" for t, _ in TOOLS) + f"{'piksel':<10}{'luar area':<11}{'geser':<14}"
    lines.append(head)
    lines.append("-" * len(head))
    for row in rows:
        if "error" in row:
            lines.append(f"{row['case']:<13} GAGAL: {row['error']}")
            continue
        lines.append(
            f"{row['case']:<13}{row['glyphs']:>6}{row['images']:>5}{row['annots']:>5}{row['beyond']:>5}  "
            + "".join(f"{row['tools'][t]:<12}" for t, _ in TOOLS)
            + f"{row.get('pixels', '-'):<10}{row['outside']:<11}{row['moved']:<14}"
        )
    lines.append("")
    lines.append("Kasus:")
    for c in cases:
        lines.append(f"  {c['name']:<13} {c['what']}")
    lines.append("")
    lines.append("Kewarasan pemeriksaan (tiap perubahan kecil di luar area harus tertangkap):")
    for x in sanity:
        lines.append(f"  {x}")
    lines.append("")
    lines.append("HASIL: " + ("LULUS — tidak ada alat yang menemukan rahasia di hasil redaksi" if not failures else "GAGAL"))
    for f in failures:
        lines.append(f"  - {f}")
    text = "\n".join(lines) + "\n"
    Path(args.out).write_text(text)
    print(text)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
