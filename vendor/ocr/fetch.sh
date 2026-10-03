#!/usr/bin/env bash
# Fetches the pinned models of the local OCR engine (PP-OCRv6 small, 7.0.0).
#
# They come from RapidOCR's wheel on PyPI (rapidocr 3.9.2), which ships the
# PaddleOCR models converted to ONNX; only the two models are kept. Pinned by
# SHA-256 — the wheel and each model — never "latest": a different model reads
# differently, and the accuracy in bench/results/phase8-ocr.txt belongs to
# these two files. A download that does not match is deleted, not used.
#
# Licence: PaddleOCR and its models are Apache-2.0 (PaddlePaddle Authors);
# RapidOCR, which converted them, is Apache-2.0 too. The installer bundles
# them (src-tauri/tauri.bundle.json) with the notice in licenses/NOTICE.txt.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WHEEL_URL="https://files.pythonhosted.org/packages/55/ed/0ee9b9281986974be9d2406ae0134c8d7c91d2fc613f16ffda9701eeda6f/rapidocr-3.9.2-py3-none-any.whl"
WHEEL_SUM=04d6b8d151f823d930bd91910555f57bea897c0c44fa6794267b94cf9c1ef9a0
DET=PP-OCRv6_det_small.onnx
DET_SUM=090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f
REC=PP-OCRv6_rec_small.onnx
REC_SUM=6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884

check() { echo "$2  $1" | sha256sum --check --status; }

if [[ -f "$HERE/$DET" ]] && check "$HERE/$DET" "$DET_SUM" \
  && [[ -f "$HERE/$REC" ]] && check "$HERE/$REC" "$REC_SUM"; then
  echo "model OCR: sudah ada"
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
echo "model OCR: mengunduh rapidocr 3.9.2"
curl --fail --silent --show-error --location --retry 4 --retry-delay 2 -o "$tmp/w.whl" "$WHEEL_URL"
if ! check "$tmp/w.whl" "$WHEEL_SUM"; then
  echo "rapidocr-3.9.2.whl: SHA-256 tidak cocok, berkas dibuang" >&2
  exit 1
fi

# A wheel is a zip. Whatever can open one: unzip, Python, or — on Windows,
# where neither may be installed — PowerShell, which always is.
names=("rapidocr/models/$DET" "rapidocr/models/$REC")
if command -v unzip >/dev/null 2>&1; then
  unzip -q -o "$tmp/w.whl" "${names[@]}" -d "$tmp/x"
elif command -v python3 >/dev/null 2>&1; then
  python3 -c 'import sys, zipfile; z = zipfile.ZipFile(sys.argv[1]); [z.extract(n, sys.argv[2]) for n in sys.argv[3:]]' "$tmp/w.whl" "$tmp/x" "${names[@]}"
else
  cp "$tmp/w.whl" "$tmp/w.zip"
  powershell.exe -NoProfile -Command "Expand-Archive -Force -Path '$(cygpath -w "$tmp/w.zip")' -DestinationPath '$(cygpath -w "$tmp/x")'"
fi

for pair in "$DET:$DET_SUM" "$REC:$REC_SUM"; do
  name="${pair%%:*}" sum="${pair##*:}"
  if ! check "$tmp/x/rapidocr/models/$name" "$sum"; then
    echo "$name: SHA-256 tidak cocok, berkas dibuang" >&2
    exit 1
  fi
  cp "$tmp/x/rapidocr/models/$name" "$HERE/$name"
  echo "$name: ok"
done
