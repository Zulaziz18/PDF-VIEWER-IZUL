#!/usr/bin/env python3
"""How long redaction takes and what it does to the file size (Phase 6).

One area (72,600)-(400,700) on the first page, then on every page, of the
500-page benchmark fixtures: text, mixed (text + JPEG photos) and scanned
(one JPEG per page). The time is the application's routine — redaction, the
PDFium verification, and the save — measured inside `redact-proof`, in a
release build.

    python3 bench/make_fixtures.py test-fixtures text mixed scan
    python3 tools/redaction-proof/bench.py [--out bench/results/phase6-linux.txt]
"""
import argparse
import json
import platform
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(ROOT / "bench/results/phase6-linux.txt"))
    args = ap.parse_args()
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "izul-bench", "--bin", "redact-proof"], cwd=ROOT, check=True)
    exe = ROOT / "target/release" / ("redact-proof.exe" if sys.platform == "win32" else "redact-proof")
    rows = []
    with tempfile.TemporaryDirectory() as work:
        for name in ("text-500p", "mixed-500p", "scan-500p"):
            src = ROOT / "test-fixtures" / f"{name}.pdf"
            if not src.exists():
                sys.exit(f"{src} tidak ada — jalankan bench/make_fixtures.py dulu")
            for pages in (1, 500):
                job = Path(work) / "job.json"
                out = Path(work) / "out.pdf"
                marks = [{"page": p, "rect": [72, 600, 400, 700]} for p in range(pages)]
                job.write_text(json.dumps({"cases": [{"name": name, "in": str(src), "out": str(out), "marks": marks}]}))
                r = subprocess.run([str(exe), str(job)], capture_output=True, text=True, check=True, cwd=ROOT)
                d = json.loads(r.stdout)
                if not d["ok"]:
                    sys.exit(f"{name}: {d['error']}")
                res = d["result"]
                ps = res["pages"]
                rows.append((name, pages, res["ms"], src.stat().st_size / 1e6, res["bytes"] / 1e6,
                             sum(p["glyphs"] for p in ps), sum(p["images_cleared"] + p["images_removed"] for p in ps)))
    lines = [
        f"Fase 6 — redaksi: waktu dan ukuran berkas ({platform.system().lower()} {platform.machine()})",
        "Satu area (72,600)-(400,700) di halaman pertama, lalu di setiap halaman. Waktu = redaksi +",
        "verifikasi PDFium + simpan, build release. Tidak ada target di SPEC; angka ini rujukan.",
        "",
        f"{'berkas':<16}{'halaman':>8}{'waktu s':>10}{'asli MB':>10}{'hasil MB':>10}{'glyph':>9}{'gambar':>8}",
    ]
    for name, pages, ms, a, b, g, i in rows:
        lines.append(f"{name + '.pdf':<16}{pages:>8}{ms / 1000:>10.2f}{a:>10.1f}{b:>10.1f}{g:>9}{i:>8}")
    text = "\n".join(lines) + "\n"
    Path(args.out).write_text(text)
    print(text)


if __name__ == "__main__":
    main()
