#!/usr/bin/env python3
"""The OCR proof (Phase 7, SPEC 17): scanned pages become searchable, the
words are where they are seen, and nothing on the page changes to the eye.

1. `make_scans.py` writes scanned-looking pages with known text and the box of
   every word; one page is stored on its side with /Rotate 90.
2. The application's own routine (`izul-bench --bin ocr-probe`, which runs
   `izul_ocr::ocr_page` as the worker does) reads every page and writes the
   invisible text layer.
3. Four extractors that are not the application read the text back — poppler,
   MuPDF, pdfminer.six, pypdf — and each is scored against the truth:
   character error rate (CER) and word error rate (WER).
4. Word positions from poppler are matched to the true boxes: of the words
   read correctly (misreadings are the CER's business), the share whose box
   centre falls inside the true box. Not IoU: the true boxes are ink boxes and
   ours are font boxes, so "an" and "di", with nothing above the x-height,
   would fail an overlap test while sitting exactly where they belong.
5. MuPDF renders every page before and after: the pictures must be identical,
   pixel for pixel — the layer is invisible.

    python3 tools/ocr-proof/run.py [--out bench/results/phase7-ocr.txt]

Needs poppler-utils, mupdf-tools, pdfminer.six, pypdf, pikepdf, Pillow.
Exits non-zero when a criterion fails (see LIMITS).
"""
import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent

# What counts as passing. Measured when the proof was written: CER 0.4-0.9 %
# for every extractor; limits leave room for a different font rasteriser,
# not for a broken layer (which reads as 100 %).
LIMITS = {"cer": 2.0, "placed": 95.0}


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, check=False, **kw)


def lev(a, b):
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]


def norm(text):
    return re.sub(r"\s+", " ", text).strip()


def poppler(path, page):
    return run(["pdftotext", "-q", "-f", str(page + 1), "-l", str(page + 1), str(path), "-"]).stdout


def mupdf(path, page):
    return run(["mutool", "draw", "-q", "-F", "txt", "-o", "-", str(path), str(page + 1)]).stdout


def pdfminer(path, page):
    from pdfminer.high_level import extract_text

    return extract_text(str(path), page_numbers=[page])


def pypdf_text(path, page):
    from pypdf import PdfReader

    return PdfReader(str(path)).pages[page].extract_text() or ""


TOOLS = [("poppler", poppler), ("MuPDF", mupdf), ("pdfminer", pdfminer), ("pypdf", pypdf_text)]


def poppler_words(path, page):
    r = run(["pdftotext", "-q", "-bbox", "-f", str(page + 1), "-l", str(page + 1), str(path), "-"])
    out = []
    for m in re.finditer(r'<word xMin="([\d.]+)" yMin="([\d.]+)" xMax="([\d.]+)" yMax="([\d.]+)">([^<]*)</word>', r.stdout):
        out.append([m.group(5)] + [float(m.group(i)) for i in range(1, 5)])
    return out


def centre_in(found, box):
    cx, cy = (found[0] + found[2]) / 2, (found[1] + found[3]) / 2
    return box[0] <= cx <= box[2] and box[1] <= cy <= box[3]


def placed(found, truth):
    """Of the true words read correctly somewhere on the page, the share with
    such a word centred inside the true box; and how many were compared."""
    hit = total = 0
    for text, *box in truth:
        same = [fb for t, *fb in found if t == text]
        if not same:
            continue
        total += 1
        if any(centre_in(fb, box) for fb in same):
            hit += 1
    return 100.0 * hit / max(1, total), total


def render(path, page):
    with tempfile.TemporaryDirectory() as d:
        out = Path(d) / "p.png"
        run(["mutool", "draw", "-q", "-r", "72", "-o", str(out), str(path), str(page + 1)])
        return Image.open(out).convert("RGB").copy()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(ROOT / "bench/results/phase7-ocr.txt"))
    args = ap.parse_args()
    work = Path(tempfile.mkdtemp(prefix="izul-ocr-"))
    subprocess.run([sys.executable, str(HERE / "make_scans.py"), str(work)], check=True, stdout=subprocess.DEVNULL)
    truth = json.loads((work / "truth.json").read_text())["pages"]
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "izul-bench", "--bin", "ocr-probe"], cwd=ROOT, check=True)
    exe = ROOT / "target/release" / ("ocr-probe.exe" if sys.platform == "win32" else "ocr-probe")
    src, dst = work / "scans.pdf", work / "scans.ocr.pdf"
    r = run([str(exe), str(src), str(dst)], cwd=ROOT)
    if r.returncode != 0:
        print(r.stderr, file=sys.stderr)
        sys.exit(2)
    per_page = [json.loads(l) for l in r.stdout.splitlines() if l.strip()]

    failures = []
    rows = []
    totals = {t: [0, 0, 0, 0] for t, _ in TOOLS}
    for page, t in enumerate(truth):
        want = norm(" ".join(t["lines"]))
        row = {"page": page, "ms": per_page[page]["ms"], "words": per_page[page].get("words", 0)}
        before_text = norm(poppler(src, page))
        if before_text:
            failures.append(f"halaman {page + 1}: berkas asli sudah punya teks ({before_text[:30]}…) — kasus tidak menguji apa pun")
        for tool, fn in TOOLS:
            got = norm(fn(dst, page))
            e = lev(got, want)
            we = lev(got.split(), want.split())
            tot = totals[tool]
            tot[0] += e
            tot[1] += len(want)
            tot[2] += we
            tot[3] += len(want.split())
            row[tool] = 100.0 * e / max(1, len(want))
        row["placed"], row["compared"] = placed(poppler_words(dst, page), t["words"])
        diff = ImageChops.difference(render(src, page), render(dst, page)).getextrema()
        row["pixels"] = max(hi for _, hi in diff)
        if row["placed"] < LIMITS["placed"]:
            failures.append(f"halaman {page + 1}: hanya {row['placed']:.1f} % kata di tempatnya")
        if row["pixels"] != 0:
            failures.append(f"halaman {page + 1}: tampilan halaman berubah (selisih {row['pixels']})")
        rows.append(row)
    summary = {}
    for tool, (e, n, we, wn) in totals.items():
        cer, wer = 100.0 * e / max(1, n), 100.0 * we / max(1, wn)
        summary[tool] = (cer, wer)
        if cer > LIMITS["cer"]:
            failures.append(f"{tool}: CER {cer:.2f} % di atas batas {LIMITS['cer']} %")

    lines = [
        "Fase 7 — bukti OCR: pindaian jadi bisa dicari, kata di tempatnya, tampilan tidak berubah",
        "",
        "CER/WER = tingkat kesalahan karakter/kata teks yang dibaca alat itu dari hasil OCR, terhadap teks sebenarnya",
        "Tempat  = dari kata yang terbaca benar, persen yang pusat kotaknya (menurut poppler) ada di dalam kotak kata sebenarnya",
        "Piksel  = selisih terbesar render MuPDF sebelum/sesudah (harus 0: lapisannya tak terlihat)",
        "Halaman 5 disimpan miring (gambar tidur, /Rotate 90).",
        "",
        f"{'halaman':<9}{'kata':>6}{'ms':>8}  " + "".join(f"{t + ' CER':>14}" for t, _ in TOOLS) + f"{'tempat':>9}{'piksel':>8}",
    ]
    for row in rows:
        lines.append(
            f"{row['page'] + 1:<9}{row['words']:>6}{row['ms']:>8.0f}  "
            + "".join(f"{row[t]:>13.2f}%" for t, _ in TOOLS)
            + f"{row['placed']:>8.1f}%{row['pixels']:>8}"
        )
    lines.append("")
    lines.append("Seluruh halaman: " + "; ".join(f"{t} CER {c:.2f} %, WER {w:.1f} %" for t, (c, w) in summary.items()))
    lines.append(f"Batas: CER <= {LIMITS['cer']} % tiap alat, tempat >= {LIMITS['placed']} %, piksel = 0.")
    lines.append("")
    lines.append("HASIL: " + ("LULUS" if not failures else "GAGAL"))
    for f in failures:
        lines.append(f"  - {f}")
    text = "\n".join(lines) + "\n"
    Path(args.out).write_text(text)
    print(text)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
