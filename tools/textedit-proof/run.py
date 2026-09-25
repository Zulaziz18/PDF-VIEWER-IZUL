#!/usr/bin/env python3
"""The text-editing proof (Phase 7): a replacement reads as replaced in
readers that are not PDFium, and nothing else on the page changed.

1. `make_doc.py` writes a page with lines in an embedded font subset, one
   line in Helvetica (not embedded), a shape and colour blocks.
2. The application's own routine (`izul-bench --bin textedit-probe`, which
   runs `izul_pdf::textedit::replace_text_document` as the worker does)
   replaces text: a name at the end of a line (longer), and a word in the
   middle of a line (shorter, the rest of the line after it).
3. poppler, MuPDF and pypdf extract the text: the new text is there, the old
   is not, and every other line reads as it did.
4. poppler and MuPDF render before and after at 100 dpi: every changed pixel
   lies in the replaced span of its line — nothing else on the page moved by
   a pixel — and the span did change (a check that cannot fail proves
   nothing).
5. Refusals, each with its reason: a font that is not embedded, a letter the
   subset does not have, a replacement that runs into the next word.

    python3 tools/textedit-proof/run.py [--out bench/results/phase7-textedit.txt]

Needs poppler-utils, mupdf-tools, pypdf, reportlab, Pillow, DejaVu Sans.
"""
import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[2]
DPI = 100
PAGE_H = 841.8898  # A4, points


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw).stdout


def probe(exe, src, out, needle, text):
    return json.loads(run([str(exe), str(src), str(out), "0", needle, text]))


def poppler_text(pdf):
    return run(["pdftotext", "-layout", str(pdf), "-"])


def mupdf_text(pdf):
    return run(["mutool", "draw", "-q", "-F", "txt", str(pdf), "1"])


def pypdf_text(pdf):
    from pypdf import PdfReader

    return PdfReader(str(pdf), strict=True).pages[0].extract_text()


def flat(text):
    return " ".join(text.split())


def lines(text):
    return [" ".join(l.split()) for l in text.splitlines() if l.strip()]


def render(pdf, out, engine):
    if engine == "poppler":
        run(["pdftoppm", "-r", str(DPI), "-png", "-singlefile", str(pdf), str(out)])
        return Image.open(f"{out}.png").convert("L")
    run(["mutool", "draw", "-q", "-r", str(DPI), "-o", f"{out}.png", str(pdf), "1"])
    return Image.open(f"{out}.png").convert("L")


def changed_outside(a, b, box):
    """Changed pixels (difference > 40) outside `box` (pixels), and inside."""
    d = ImageChops.difference(a, b).point(lambda v: 255 if v > 40 else 0)
    x0, y0, x1, y1 = box
    outside = inside = 0
    px = d.load()
    for y in range(d.height):
        for x in range(d.width):
            if px[x, y]:
                if x0 <= x <= x1 and y0 <= y <= y1:
                    inside += 1
                else:
                    outside += 1
    return outside, inside


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(ROOT / "bench/results/phase7-textedit.txt"))
    args = ap.parse_args()
    subprocess.run(["cargo", "build", "-q", "-p", "izul-bench", "--bin", "textedit-probe"], cwd=ROOT, check=True)
    exe = ROOT / "target/debug" / ("textedit-probe.exe" if sys.platform == "win32" else "textedit-probe")
    report = ["Bukti penyuntingan teks asli (Fase 7)", "=" * 40, ""]
    ok = True

    def verdict(cond, what):
        nonlocal ok
        ok &= bool(cond)
        report.append(f"  {'ya   ' if cond else 'TIDAK'}  {what}")

    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        src = tmp / "surat.pdf"
        run([sys.executable, str(ROOT / "tools/textedit-proof/make_doc.py"), str(src)])
        before_text = {e: f(src) for e, f in (("poppler", poppler_text), ("MuPDF", mupdf_text), ("pypdf", pypdf_text))}

        cases = [
            ("nama di ujung baris, lebih panjang", "Budi Santoso", "Rina Wulandari"),
            ("kata di tengah baris, lebih pendek", "Serbaguna", "Utama"),
        ]
        for name, needle, text in cases:
            out = tmp / f"hasil-{len(report)}.pdf"
            r = probe(exe, src, out, needle, text)
            report.append(f"Kasus: {name}: \"{needle}\" -> \"{text}\"")
            verdict(r.get("ok"), f"diterima oleh rutin aplikasi ({r.get('refused', 'glyph diganti: %s' % r.get('glyphs'))})")
            if not r.get("ok"):
                report.append("")
                continue
            left, bottom, right, top = r["area"]
            for engine, extract in (("poppler", poppler_text), ("MuPDF", mupdf_text), ("pypdf", pypdf_text)):
                raw = extract(out)
                got = lines(raw)
                new_line = [l for l in got if text in l]
                old_gone = not any(needle in l for l in got)
                verdict(new_line and old_gone, f"{engine:<7} membaca teks baru, teks lama hilang: {new_line[:1]}")
                # The whole page, in order, with the one change and no other.
                # Line breaks are the extractor's guess and not compared: a
                # shorter word leaves a gap (no reflow, SPEC 17), and MuPDF
                # starts a new line at a gap that wide.
                same = flat(raw) == flat(before_text[engine]).replace(needle, text, 1)
                split = len(got) != len(lines(before_text[engine]))
                verdict(same, f"{engine:<7} seluruh halaman terbaca sama selain penggantian"
                        + (" (baris dipecah di celah)" if split else ""))
            # The replaced span, in pixels, with a margin for antialiasing;
            # a longer text reaches past the old one to the right.
            s = DPI / 72.0
            widest = right + (len(text) - len(needle)) * 7.5 if len(text) > len(needle) else right
            box = (int(left * s) - 3, int((PAGE_H - top) * s) - 4, int(widest * s) + 3, int((PAGE_H - bottom) * s) + 4)
            for engine in ("poppler", "MuPDF"):
                a = render(src, tmp / f"a-{engine}", engine)
                b = render(out, tmp / f"b-{engine}", engine)
                outside, inside = changed_outside(a, b, box)
                verdict(outside == 0, f"{engine:<7} piksel berubah di luar bagian yang diganti: {outside}")
                verdict(inside > 0, f"{engine:<7} piksel berubah di bagian yang diganti: {inside} (harus > 0)")
            report.append("")

        report.append("Penolakan (berkas tidak ditulis):")
        refusals = [
            ("dibawa", "DIBAWA", "tidak tertanam", "font Helvetica tidak tertanam"),
            ("Santoso", "Santosé", "\"é\"", "huruf yang tidak ada di subset"),
            ("Tanggal", "Tanggal pelaksanaan resmi", "menabrak", "teks pengganti menabrak kata berikutnya"),
        ]
        for needle, text, key, what in refusals:
            out = tmp / "tolak.pdf"
            r = probe(exe, src, out, needle, text)
            refused = (not r.get("ok")) and key in r.get("refused", "")
            verdict(refused and not out.exists(), f"{what}: {r.get('refused', 'DITERIMA')}")

    report.append("")
    report.append(f"HASIL: {'LULUS' if ok else 'GAGAL'}")
    Path(args.out).write_text("\n".join(report) + "\n", encoding="utf-8")
    print("\n".join(report))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
