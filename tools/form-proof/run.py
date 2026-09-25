#!/usr/bin/env python3
"""The form-filling proof (Phase 7): a filled form reads as filled in other
readers — the values, and what they draw.

1. `make_form.py` writes a form with a text field, a multi-line text field,
   a checkbox whose "on" state is /Ya (not /Yes), a radio group, a combo box
   and a list box.
2. The application's own routine (`izul-bench --bin form-probe`, which runs
   `izul_pdf::forms::Document::fill_form` as the worker does) fills it.
3. pypdf reads the field values and widget states; poppler and MuPDF
   extract the text their renderers draw from the widgets' appearances; and
   MuPDF renders the checkbox and radio widgets before and after.
4. A control: the same values written the naive way — /V only, with pikepdf —
   must fail the appearance checks, or they prove nothing.

    python3 tools/form-proof/run.py [--out bench/results/phase7-forms.txt]

Needs poppler-utils, mupdf-tools, pypdf, pikepdf, reportlab, Pillow.
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

VALUES = [
    ["nama", {"Text": "Siti Rahayu"}],
    ["alamat", {"Text": "Jl. Contoh No. 12\nBandung"}],
    ["setuju", {"Checked": True}],
    ["kategori", {"Radio": "pelajar"}],
    ["kota", {"Choice": [2]}],
    ["sesi", {"Choice": [1]}],
]
# What each field should read as, and the text that must be drawn for it.
WANT = {
    "nama": ("Siti Rahayu", "Siti Rahayu"),
    # Not "Bandung": the combo box draws that as its first option anyway.
    "alamat": ("Jl. Contoh No. 12", "Jl. Contoh No. 12"),
    "setuju": ("/Ya", None),
    "kategori": ("/pelajar", None),
    "kota": ("Surabaya", "Surabaya"),
    # A list box draws every option, selected or not; its selection is
    # checked in the render instead (the highlighted row).
    "sesi": ("Siang", None),
}


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, check=False, **kw)


def norm(t):
    return re.sub(r"\s+", " ", t)


def render(path):
    with tempfile.TemporaryDirectory() as d:
        out = Path(d) / "p.png"
        run(["mutool", "draw", "-q", "-r", "72", "-o", str(out), str(path), "1"])
        return Image.open(out).convert("L").copy()


def widget_boxes(path):
    import pikepdf

    boxes = {}
    with pikepdf.open(str(path)) as pdf:
        h = float(pdf.pages[0].MediaBox[3])
        for a in pdf.pages[0].Annots:
            name = str(a.get("/T") or a.Parent.get("/T"))
            on = [k for k in a.AP.N.keys() if k != "/Off"] if "/AP" in a and hasattr(a.AP.N, "keys") else []
            x0, y0, x1, y1 = [float(v) for v in a.Rect]
            boxes.setdefault(name, []).append(((round(x0), round(h - y1), round(x1), round(h - y0)), on[:1]))
    return boxes


def check(path, label, failures, before_img):
    from pypdf import PdfReader

    r = PdfReader(str(path))
    fields = {k: v.get("/V") for k, v in r.get_fields().items()}
    states = {}
    for a in r.pages[0]["/Annots"]:
        a = a.get_object()
        name = a.get("/T") or a["/Parent"].get("/T")
        states.setdefault(str(name), []).append(str(a.get("/AS")))
    poppler = norm(run(["pdftotext", "-q", str(path), "-"]).stdout)
    mupdf = norm(run(["mutool", "draw", "-q", "-F", "txt", "-o", "-", str(path), "1"]).stdout)
    after_img = render(path)
    rows = []
    for name, (value, drawn) in WANT.items():
        v = str(fields.get(name))
        ok_value = value in v
        ok_pop = drawn is None or drawn in poppler
        ok_mu = drawn is None or drawn in mupdf
        ink = "-"
        if name == "sesi":
            (box, _), = widget_boxes(path)[name]
            d = ImageChops.difference(before_img.crop(box), after_img.crop(box)).getextrema()[1]
            ok_ink = d > 60
            ink = "ya" if ok_ink else "TIDAK"
        elif name in ("setuju", "kategori"):
            # The widget that should be on must look different now; for the
            # radio group, only that one.
            changed = []
            for (box, on) in widget_boxes(path)[name]:
                d = ImageChops.difference(before_img.crop(box), after_img.crop(box)).getextrema()[1]
                changed.append((on, d > 60))
            want_on = "/Ya" if name == "setuju" else "/pelajar"
            ok_ink = all(c == (on == [want_on]) for on, c in changed)
            ink = "ya" if ok_ink else "TIDAK"
        else:
            ok_ink = True
        ok_state = True
        if name == "setuju":
            ok_state = states[name] == ["/Ya"]
        if name == "kategori":
            ok_state = sorted(states[name]) == ["/Off", "/Off", "/pelajar"]
        rows.append((name, v, ok_value and ok_state, ok_pop, ok_mu, ink))
        if not (ok_value and ok_state and ok_pop and ok_mu and ok_ink):
            failures.append(f"{label}: {name}")
    return rows


def naive(src, dst):
    """The control: /V (and /AS /Yes) written without new appearances."""
    import pikepdf

    with pikepdf.open(str(src)) as pdf:
        for a in pdf.pages[0].Annots:
            field = a if "/T" in a else a.Parent
            name = str(field.T)
            if name in ("nama", "alamat", "kota", "sesi"):
                field.V = {"nama": "Siti Rahayu", "alamat": "Jl. Contoh No. 12\nBandung", "kota": "Surabaya", "sesi": "Siang"}[name]
            elif name == "setuju":
                field.V = pikepdf.Name("/Yes")
                a.AS = pikepdf.Name("/Yes")
            elif name == "kategori":
                field.V = pikepdf.Name("/pelajar")
        pdf.save(str(dst))


def table(rows):
    out = [f"{'isian':<10}{'nilai (pypdf)':<30}{'nilai':>7}{'poppler':>9}{'MuPDF':>7}{'gambar':>8}"]
    for name, v, a, b, c, ink in rows:
        yn = lambda x: "ya" if x else "TIDAK"
        out.append(f"{name:<10}{v.replace(chr(13), ' ').replace(chr(10), ' ')[:28]:<30}{yn(a):>7}{yn(b):>9}{yn(c):>7}{ink:>8}")
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(ROOT / "bench/results/phase7-forms.txt"))
    args = ap.parse_args()
    work = Path(tempfile.mkdtemp(prefix="izul-form-"))
    src, dst, ctrl = work / "form.pdf", work / "form.filled.pdf", work / "form.naive.pdf"
    subprocess.run([sys.executable, str(HERE / "make_form.py"), str(src)], check=True)
    subprocess.run(["cargo", "build", "-q", "-p", "izul-bench", "--bin", "form-probe"], cwd=ROOT, check=True)
    exe = ROOT / "target/debug" / ("form-probe.exe" if sys.platform == "win32" else "form-probe")
    r = run([str(exe), str(src), str(dst), json.dumps(VALUES)], cwd=ROOT)
    if r.returncode != 0:
        print(r.stderr, file=sys.stderr)
        sys.exit(2)
    before = render(src)
    failures = []
    rows = check(dst, "aplikasi", failures, before)
    naive(src, ctrl)
    control_failures = []
    control = check(ctrl, "pembanding", control_failures, before)

    lines = [
        "Fase 7 — bukti pengisian formulir: isian terbaca dan tergambar di pembaca lain",
        "",
        "nilai   = /V (dan /AS untuk kotak centang/radio) menurut pypdf sesuai yang diisi",
        "poppler, MuPDF = teks isian ikut terbaca dari tampilan (appearance) widget yang digambar pembaca itu",
        "gambar  = render MuPDF: widget yang seharusnya menyala berubah, yang lain tidak",
        "Kotak centang memakai keadaan /Ya (bukan /Yes); radio memakai /umum, /pelajar, /pengajar.",
        "",
        "Aplikasi (izul_pdf::forms, lewat lingkungan form-fill PDFium):",
        *table(rows),
        "",
        "Pembanding: nilai yang sama ditulis cara naif (hanya /V, /AS /Yes) dengan pikepdf — harus gagal:",
        *table(control),
        "",
    ]
    if not control_failures:
        failures.append("pembanding lolos — pemeriksaannya tidak bisa gagal")
    lines.append("HASIL: " + ("LULUS" if not failures else "GAGAL"))
    for f in failures:
        lines.append(f"  - {f}")
    text = "\n".join(lines) + "\n"
    Path(args.out).write_text(text)
    print(text)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
