#!/usr/bin/env bash
# Fetches the pinned PDFium build for every target we ship or test on.
#
# The version is pinned, never "latest": pdfium-render generates its FFI
# bindings per PDFium ABI revision, and a mismatch shows up as a missing-symbol
# failure at library load, or worse, silently at the first call. PDFIUM_TAG here
# must stay in step with the pdfium_* feature selected in the workspace manifest.
set -euo pipefail

PDFIUM_TAG="chromium/7881"
BASE="https://github.com/bblanchon/pdfium-binaries/releases/download/${PDFIUM_TAG//\//%2F}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

targets=("${@:-linux-x64 win-x64}")
for t in ${targets[@]}; do
  out="$HERE/$t"
  if [[ -f "$out/VERSION" ]]; then echo "$t: already present"; continue; fi
  echo "$t: downloading $PDFIUM_TAG"
  mkdir -p "$out"
  curl --fail --silent --show-error --location --retry 4 --retry-delay 2 \
       "$BASE/pdfium-$t.tgz" | tar xz -C "$out"
  echo "$t: $(tr '\n' ' ' < "$out/VERSION")"
done
