#!/usr/bin/env bash
# Fetches ONNX Runtime and the background-removal model (Phase 7), pinned.
#
# ONNX Runtime 1.24.4 comes from Microsoft's own wheels on PyPI — the plain
# build for Linux, the DirectML build for Windows — and only the libraries are
# kept. The application loads them at run time (`ort`'s load-dynamic), so the
# build never downloads anything and the installer ships these files.
#
# Licences: ONNX Runtime MIT (Microsoft); DirectML.dll is Microsoft's
# redistributable under its own licence (see ThirdPartyNotices.txt kept here);
# u2netp.onnx is U²-Net by Xuebin Qin et al., Apache-2.0, as published by
# rembg (MIT).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PY="https://files.pythonhosted.org/packages"

check() { echo "$2  $1" | sha256sum --check --status; }

get() {
  local out="$1" url="$2" sum="$3"
  if [[ -f "$out" ]] && check "$out" "$sum"; then return; fi
  curl --fail --silent --show-error --location --retry 4 --retry-delay 2 -o "$out.part" "$url"
  if ! check "$out.part" "$sum"; then
    rm -f "$out.part"
    echo "$(basename "$out"): SHA-256 tidak cocok, berkas dibuang" >&2
    exit 1
  fi
  mv "$out.part" "$out"
}

# A wheel is a zip. Whatever can open one: unzip, Python, or — on Windows,
# where neither may be installed — PowerShell, which always is.
unpack() {
  local wheel="$1" dest="$2"; shift 2
  mkdir -p "$dest"
  local tmp; tmp="$(mktemp -d)"
  if command -v unzip >/dev/null 2>&1; then
    unzip -q -o "$wheel" "$@" -d "$tmp"
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c 'import sys, zipfile; z = zipfile.ZipFile(sys.argv[1]); [z.extract(n, sys.argv[2]) for n in sys.argv[3:]]' "$wheel" "$tmp" "$@"
  else
    cp "$wheel" "$tmp/w.zip"
    powershell.exe -NoProfile -Command "Expand-Archive -Force -Path '$(cygpath -w "$tmp/w.zip")' -DestinationPath '$(cygpath -w "$tmp")'"
  fi
  for n in "$@"; do cp "$tmp/$n" "$dest/"; done
  rm -rf "$tmp"
}

targets=("${@:-linux-x64 win-x64}")
for t in ${targets[@]}; do
  case "$t" in
    linux-x64)
      if [[ ! -f "$HERE/linux-x64/libonnxruntime.so" ]]; then
        echo "linux-x64: ONNX Runtime 1.24.4"
        get "$HERE/ort-linux.whl" \
          "$PY/6d/ab/5b68110e0460d73fad814d5bd11c7b1ddcce5c37b10177eb264d6a36e331/onnxruntime-1.24.4-cp312-cp312-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl" \
          0d640eb9f3782689b55cfa715094474cd5662f2f137be6a6f847a594b6e9705c
        unpack "$HERE/ort-linux.whl" "$HERE/linux-x64" \
          onnxruntime/capi/libonnxruntime.so.1.24.4 onnxruntime/LICENSE onnxruntime/ThirdPartyNotices.txt
        mv "$HERE/linux-x64/libonnxruntime.so.1.24.4" "$HERE/linux-x64/libonnxruntime.so"
        rm -f "$HERE/ort-linux.whl"
      fi
      echo "linux-x64: ok" ;;
    win-x64)
      if [[ ! -f "$HERE/win-x64/onnxruntime.dll" ]]; then
        echo "win-x64: ONNX Runtime 1.24.4 + DirectML"
        get "$HERE/ort-win.whl" \
          "$PY/88/ea/33814eb0ec96775eda4c1d30b0d86e91d7d2cd0d84c66d3915aef0e06fa3/onnxruntime_directml-1.24.4-cp312-cp312-win_amd64.whl" \
          f2ecb68b7b7b259d2ef3112ae760149f9b5a1e7c0fbb73d539da6250a648a614
        unpack "$HERE/ort-win.whl" "$HERE/win-x64" \
          onnxruntime/capi/onnxruntime.dll onnxruntime/capi/DirectML.dll \
          onnxruntime/LICENSE onnxruntime/ThirdPartyNotices.txt
        rm -f "$HERE/ort-win.whl"
      fi
      echo "win-x64: ok" ;;
    *) echo "target tidak dikenal: $t" >&2; exit 1 ;;
  esac
done

get "$HERE/u2netp.onnx" \
  "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx" \
  309c8469258dda742793dce0ebea8e6dd393174f89934733ecc8b14c76f4ddd8
echo "u2netp.onnx: ok"
