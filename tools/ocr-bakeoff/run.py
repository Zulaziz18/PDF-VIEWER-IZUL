#!/usr/bin/env python3
"""Which OCR engine reads better. Written to choose between `ocrs` (what the
application shipped until 7.0.0) and PaddleOCR; since 7.0.0 the application's
own engine *is* PP-OCRv6, and this checks that its Rust port reads as well as
RapidOCR's Python with the same models, next to Tesseract as a long-standing
reference.

All of them read the same five sets of scanned-looking pages (`make_sets.py`)
and are scored against the true text.

- The application's engine runs through its own routine (`ocr-probe`, which runs
  `izul_ocr::ocr_page` as the worker does), and its text is read back from the
  PDF it writes with poppler. That is what a user would get.
- PaddleOCR and Tesseract read PNGs rendered by poppler. RapidOCR is used with
  its defaults, apart from lifting `max_side_len` for the 300 dpi runs — the
  default 2000 px would shrink a 300 dpi A4 page and throw away what the extra
  resolution was for.

Scores, all against the true text:

- CER / WER — edit distance per character / per word. They punish a line read
  in the wrong place, and a missing space, as hard as a wrong letter.
- CER without spaces — the same with all whitespace removed on both sides,
  so it measures reading the letters, not placing the gaps.
- Whole words found — the share of true words present, as whole words, in
  what the engine returned (case and edge punctuation ignored, order ignored).
  This is what decides whether a search for a word finds the page.

What this does NOT show: real scans (everything here is synthetic), tables,
multi-column pages, handwriting, accented or non-Latin text. Times are
wall-clock on whatever machine runs it. ocrs is Rust + rten, PaddleOCR is
Python + ONNX Runtime, Tesseract is C++ in a separate process, so small speed
differences say little about a Rust integration (`ort`, which the application
already bundles for background removal).

    python3 tools/ocr-bakeoff/run.py [--out bench/results/ocr-bakeoff.txt]

Needs: poppler-utils, tesseract-ocr (+ `ind`, `eng` data), Pillow, numpy, and
`pip install rapidocr-onnxruntime rapidocr`.
"""
import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile
import time
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
SETS = ["bersih", "foto_hp", "pudar", "kecil", "angka"]


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


def tokens(text):
    return [t for t in (w.strip(".,;:!?()\"'").lower() for w in text.split()) if t]


class Score:
    """Errors and truth sizes summed over pages, so a rate weighs every
    character the same however it is split into pages."""

    def __init__(self):
        self.c_err = self.c_len = self.n_err = self.n_len = 0
        self.w_err = self.w_len = self.found = self.words = 0
        self.ms = []
        self.pages = []

    def add(self, got, truth, ms):
        got, truth = norm(got), norm(truth)
        ce = lev(got, truth)
        self.c_err += ce
        self.c_len += len(truth)
        g0, t0 = got.replace(" ", ""), truth.replace(" ", "")
        self.n_err += lev(g0, t0)
        self.n_len += len(t0)
        self.w_err += lev(got.split(" "), truth.split(" "))
        self.w_len += len(truth.split(" "))
        want, have = Counter(tokens(truth)), Counter(tokens(got))
        self.found += sum(min(n, have[w]) for w, n in want.items())
        self.words += sum(want.values())
        self.ms.append(ms)
        self.pages.append(100.0 * ce / max(1, len(truth)))

    def merge(self, other):
        for k in ("c_err", "c_len", "n_err", "n_len", "w_err", "w_len", "found", "words"):
            setattr(self, k, getattr(self, k) + getattr(other, k))
        self.ms += other.ms
        self.pages += other.pages

    cer = property(lambda s: 100.0 * s.c_err / max(1, s.c_len))
    cer_ns = property(lambda s: 100.0 * s.n_err / max(1, s.n_len))
    wer = property(lambda s: 100.0 * s.w_err / max(1, s.w_len))
    whole = property(lambda s: 100.0 * s.found / max(1, s.words))


# --- engines: each takes (pdf, pages, work) and returns [(text, ms)] ----------

def render(pdf, page, dpi, work):
    png = work / f"{pdf.stem}-{dpi}-{page}"
    run(["pdftoppm", "-r", str(dpi), "-f", str(page + 1), "-l", str(page + 1), "-singlefile", "-png", str(pdf), str(png)])
    return str(png) + ".png"


def blank(work):
    from PIL import Image

    p = work / "warm.png"
    Image.new("L", (400, 100), 255).save(p)
    return str(p)


def app_engine(probe, reader):
    """The application's OCR, read back from the PDF it writes by `reader`
    ("poppler" or "MuPDF"). Run once per set; the second reader reuses it.

    Unlike the other columns this is what an extractor makes of the text
    layer, not the engine's own output: on a tilted page (foto_hp) poppler
    orders the words of neighbouring lines by height and interleaves them,
    which costs CER without losing a word; MuPDF keeps lines together."""
    runs = {}

    def go(pdf, n, work):
        out = work / f"{pdf.stem}.ocr.pdf"
        if pdf not in runs:
            r = run([str(probe), str(pdf), str(out)], cwd=ROOT)
            if r.returncode != 0:
                sys.exit(f"ocr-probe gagal: {r.stderr[-400:]}")
            runs[pdf] = [json.loads(l)["ms"] for l in r.stdout.splitlines() if l.startswith("{")]
        if reader == "poppler":
            texts = [run(["pdftotext", "-q", "-f", str(i + 1), "-l", str(i + 1), str(out), "-"]).stdout for i in range(n)]
        else:
            texts = [run(["mutool", "draw", "-q", "-F", "txt", "-o", "-", str(out), str(i + 1)]).stdout for i in range(n)]
        return list(zip(texts, runs[pdf]))

    return go


def paddle_v4(dpi, max_side):
    from rapidocr_onnxruntime import RapidOCR

    eng = RapidOCR(max_side_len=max_side)
    eng(blank(Path(tempfile.gettempdir())))

    def go(pdf, n, work):
        out = []
        for i in range(n):
            png = render(pdf, i, dpi, work)
            t = time.perf_counter()
            res, _ = eng(png)
            out.append((" ".join(r[1] for r in (res or [])), (time.perf_counter() - t) * 1000))
        return out

    return go


def paddle_v6(dpi, max_side):
    from rapidocr import RapidOCR

    eng = RapidOCR(params={"Global.max_side_len": max_side, "Global.log_level": "warning"})
    eng(blank(Path(tempfile.gettempdir())))

    def go(pdf, n, work):
        out = []
        for i in range(n):
            png = render(pdf, i, dpi, work)
            t = time.perf_counter()
            res = eng(png)
            out.append((" ".join(res.txts or []), (time.perf_counter() - t) * 1000))
        return out

    return go


def tesseract(dpi):
    def go(pdf, n, work):
        out = []
        for i in range(n):
            png = render(pdf, i, dpi, work)
            t = time.perf_counter()
            r = run(["tesseract", png, "stdout", "-l", "ind+eng"])
            out.append((r.stdout, (time.perf_counter() - t) * 1000))
        return out

    return go


def versions():
    import importlib.metadata as md
    import importlib.util

    tess = run(["tesseract", "--version"]).stdout.splitlines()[0]
    v4 = md.version("rapidocr-onnxruntime") if importlib.util.find_spec("rapidocr_onnxruntime") else "-"
    return f"RapidOCR {md.version('rapidocr')} (PP-OCRv6 small), {v4} (PP-OCRv4); {tess}"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=None)
    ap.add_argument("--sets", default=None, help="folder from make_sets.py (made if absent)")
    ap.add_argument("--all", action="store_true", help="also PP-OCRv4 (RapidOCR 1.4.4)")
    a = ap.parse_args()

    work = Path(tempfile.mkdtemp(prefix="ocr-bakeoff-"))
    sets = Path(a.sets) if a.sets else work / "sets"
    if not (sets / "truth.json").exists():
        subprocess.run([sys.executable, str(HERE / "make_sets.py"), str(sets)], check=True)
    truth = json.loads((sets / "truth.json").read_text())

    subprocess.run(["cargo", "build", "-q", "--release", "-p", "izul-bench", "--bin", "ocr-probe"], cwd=ROOT, check=True)
    probe = ROOT / "target/release" / ("ocr-probe.exe" if sys.platform == "win32" else "ocr-probe")

    engines = {
        "izul/poppler": app_engine(probe, "poppler"),
        "izul/MuPDF": app_engine(probe, "MuPDF"),
        "PPv6 150": paddle_v6(150, 2000),
        "PPv6 300": paddle_v6(300, 4000),
        "Tess 300": tesseract(300),
    }
    if a.all:
        engines["PPv4 150"] = paddle_v4(150, 2000)
        engines["PPv4 300"] = paddle_v4(300, 4000)
    names = list(engines)
    scores = {e: {s: Score() for s in SETS} for e in names}
    sample = {}
    for s in SETS:
        pdf = sets / f"{s}.pdf"
        n = len(truth[s])
        for e, fn in engines.items():
            pages = fn(pdf, n, work)
            for (text, ms), t in zip(pages, truth[s]):
                scores[e][s].add(text, t, ms)
            if s == "foto_hp":
                sample[e] = norm(pages[0][0])
        print(f"  {s} ok", file=sys.stderr)

    tot = {e: Score() for e in names}
    for e in names:
        for s in SETS:
            tot[e].merge(scores[e][s])

    L = []
    w = L.append
    w("Adu OCR — mesin aplikasi (izul: PP-OCRv6 lewat ONNX Runtime, pengolahan gambar ditulis di Rust)")
    w("lawan RapidOCR Python dengan model yang sama (rujukan) dan Tesseract")
    w("")
    w(versions())
    w("Sebelum 7.0.0 aplikasi memakai ocrs 0.13.1: CER gabungan 11,01 %, foto_hp 42,48 %, kata utuh 91,3 %,")
    w("0,5 dtk/halaman (diukur dengan alat ini pada set yang sama, 2 Oktober 2026).")
    w("Semua halaman buatan (sintetis): 5 set x 4 halaman, teks karangan sendiri. Bukan pindaian nyata.")
    w(f"Mesin: {os.cpu_count()} inti. Angka di belakang nama = dpi render. ms = rata-rata per halaman.")
    w("izul = aplikasi pada 150 dpi (render + baca + tulis lapisan teks), teksnya dibaca kembali dengan poppler / MuPDF;")
    w("lainnya = keluaran mesinnya sendiri, baca saja, tanpa render. Di foto_hp poppler mencampur urutan baris halaman miring:")
    w("CER-nya naik tanpa ada kata yang hilang (lihat 'kata utuh').")
    w("")
    col = 14
    head = f"{'':<9}" + "".join(f"{e:>{col}}" for e in names)
    w("CER (%) per set — salah per karakter, makin kecil makin baik:")
    w(head)
    for s in SETS:
        w(f"{s:<9}" + "".join(f"{scores[e][s].cer:>{col}.2f}" for e in names))
    w("")
    w("Kata utuh ditemukan (%) per set — makin besar makin baik, tak peduli urutan:")
    w(head)
    for s in SETS:
        w(f"{s:<9}" + "".join(f"{scores[e][s].whole:>{col}.1f}" for e in names))
    w("")
    w("Seluruh set digabung:")
    w(head)
    rows = [
        ("CER %", lambda t: f"{t.cer:.2f}"),
        ("CER tanpa spasi", lambda t: f"{t.cer_ns:.2f}"),
        ("WER %", lambda t: f"{t.wer:.1f}"),
        ("kata utuh %", lambda t: f"{t.whole:.1f}"),
        ("hal. terburuk", lambda t: f"{max(t.pages):.1f}"),
        ("ms/halaman", lambda t: f"{statistics.mean(t.ms):.0f}"),
    ]
    for label, f in rows:
        w(f"{label:<9}" + "".join(f"{f(tot[e]):>{col}}" for e in names))
    w("")
    w("Cuplikan halaman 1 set foto_hp (150 karakter pertama):")
    w(f"  [BENAR] {norm(truth['foto_hp'][0])[:150]}")
    for e in names:
        w(f"  [{e}] {sample[e][:150]}")
    text = "\n".join(L)
    print(text)
    if a.out:
        Path(a.out).write_text(text + "\n")


if __name__ == "__main__":
    main()
