#!/usr/bin/env python3
"""The background-removal proof (Phase 7): how well "Hapus Latar" separates a
picture from its background, measured against masks drawn with the pictures.

1. `make_pictures.py` draws five pictures and their exact masks: two objects
   on textured backgrounds, a signature and a stamp photographed on paper, and
   a white object on white — the case the paper mode gets wrong.
2. The application's own routine (`izul-bench --bin bg-probe`, which runs
   `izul_ocr::background` as the worker does) removes the background in both
   modes the interface offers: "Foto" and "Tanda tangan / stempel".
3. Each result's alpha is compared with the truth: IoU (intersection over
   union of subject pixels), how much of the subject was lost, and how much
   of the background was kept.

    python3 tools/background-proof/run.py [--out bench/results/phase7-background.txt]

Needs vendor/onnx/fetch.sh and Pillow. Exits non-zero when a picture misses
its limit in the mode meant for it (see EXPECT).
"""
import argparse
import json
import os
import platform
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent

# The mode each picture is meant for, and the IoU it must reach there.
# Measured when this was written: 0.99 / 0.99 / 1.00 / 1.00 / 0.90.
EXPECT = {
    "cangkir": ("photo", 0.95),
    "buah": ("photo", 0.95),
    "tanda-tangan": ("paper", 0.95),
    "stempel": ("paper", 0.95),
    "produk-putih": ("photo", 0.85),
}


def score(truth, got):
    t = Image.open(truth).convert("L").tobytes()
    g = Image.open(got).convert("L").tobytes()
    tp = fp = fn = bg = 0
    for a, b in zip(t, g):
        subject, kept = a > 127, b > 127
        tp += subject and kept
        fp += (not subject) and kept
        fn += subject and not kept
        bg += not subject
    return tp / max(1, tp + fp + fn), fn / max(1, tp + fn), fp / max(1, bg)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(ROOT / "bench/results/phase7-background.txt"))
    args = ap.parse_args()
    work = Path(tempfile.mkdtemp(prefix="izul-latar-"))
    subprocess.run([sys.executable, str(HERE / "make_pictures.py"), str(work)], check=True)
    cases = json.loads((work / "cases.json").read_text())
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "izul-bench", "--bin", "bg-probe"], cwd=ROOT, check=True)
    win = sys.platform == "win32"
    exe = ROOT / "target/release" / ("bg-probe.exe" if win else "bg-probe")
    lib = ROOT / "vendor/onnx" / ("win-x64/onnxruntime.dll" if win else "linux-x64/libonnxruntime.so")
    model = ROOT / "vendor/onnx/u2netp.onnx"
    results = {}
    device = "?"
    for mode in ("photo", "paper"):
        pairs = []
        for c in cases:
            pairs += [str(work / f"{c['name']}.png"), str(work / f"{c['name']}.{mode}.png")]
        env = dict(os.environ, BG_KIND=mode)
        r = subprocess.run([str(exe), str(lib), str(model), *pairs], capture_output=True, text=True, env=env)
        if r.returncode != 0:
            print(r.stderr, file=sys.stderr)
            sys.exit(2)
        lines = [json.loads(l) for l in r.stdout.splitlines() if l.strip()]
        device = lines[0]["device"]
        for l in lines[1:]:
            name = Path(l["in"]).stem
            results[(name, mode)] = (l["ms"], l.get("paper", False))

    failures = []
    rows = []
    for c in cases:
        n = c["name"]
        row = [n]
        for mode in ("photo", "paper"):
            iou, lost, kept = score(work / f"{n}.truth.png", work / f"{n}.{mode}.png")
            row.append((iou, lost, kept, results[(n, mode)][0]))
        want_mode, want = EXPECT[n]
        got = row[1][0] if want_mode == "photo" else row[2][0]
        if got < want:
            failures.append(f"{n}: IoU {got:.3f} di mode {want_mode}, batas {want}")
        rows.append((row, c["what"], want_mode))

    lines = [
        f"Fase 7 — bukti hapus latar ({platform.system().lower()} {platform.machine()}, perangkat: {device})",
        "",
        "Model: U²-Net kecil (u2netp, Apache-2.0) lewat ONNX Runtime 1.24.4. Dua mode seperti di aplikasi:",
        "Foto = mask model apa adanya; TTD/stempel = mask model + kertas polos ikut dibuang.",
        "IoU = irisan/gabungan piksel subjek terhadap mask sebenarnya; hilang = subjek yang terbuang; sisa = latar yang tertinggal.",
        "",
        f"{'gambar':<14}{'Foto IoU':>10}{'hilang':>8}{'sisa':>8}   {'TTD/stempel IoU':>16}{'hilang':>8}{'sisa':>8}{'ms':>7}  mode yang tepat",
    ]
    for row, what, want_mode in rows:
        n, (a, b, c, ms), (d, e, f, _) = row
        lines.append(
            f"{n:<14}{a:>10.3f}{b:>8.1%}{c:>8.1%}   {d:>16.3f}{e:>8.1%}{f:>8.1%}{ms:>7.0f}  "
            + ("Foto" if want_mode == "photo" else "TTD/stempel")
        )
    lines.append("")
    for row, what, _ in rows:
        lines.append(f"  {row[0]:<14} {what}")
    lines.append("")
    lines.append("Mode tidak bisa ditebak dari gambar: stempel di kertas dan benda putih di latar putih sama-sama bertepi")
    lines.append("polos dan terang. Mode yang salah terlihat di tabel (benda putih di mode TTD/stempel), dan Ctrl+Z membatalkannya.")
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
