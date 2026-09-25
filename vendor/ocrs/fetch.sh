#!/usr/bin/env bash
# Fetches the pinned models of the local OCR engine (`ocrs`, Phase 7).
#
# Pinned by SHA-256, never "latest": a different model reads differently, and
# the accuracy quoted in bench/results/phase7-ocr.txt belongs to these two
# files. A download that does not match is deleted, not used.
#
# Licence: the ocrs code is MIT/Apache-2.0. The models are trained on
# HierText (CC BY-SA 4.0), and the model repository states no licence for the
# weights themselves — see CLAUDE.md, "Keadaan Fase 7", before bundling them
# into an installer.
set -euo pipefail

BASE="https://ocrs-models.s3-accelerate.amazonaws.com"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

fetch() {
  local name="$1" sum="$2" out="$HERE/$1"
  if [[ -f "$out" ]] && echo "$sum  $out" | sha256sum --check --status; then
    echo "$name: already present"
    return
  fi
  echo "$name: downloading"
  curl --fail --silent --show-error --location --retry 4 --retry-delay 2 -o "$out.part" "$BASE/$name"
  if ! echo "$sum  $out.part" | sha256sum --check --status; then
    rm -f "$out.part"
    echo "$name: SHA-256 tidak cocok, berkas dibuang" >&2
    exit 1
  fi
  mv "$out.part" "$out"
  echo "$name: ok"
}

fetch text-detection.rten f15cfb56bd02c4bf478a20343986504a1f01e1665c2b3a0ad66340f054b1b5ca
fetch text-recognition.rten e484866d4cce403175bd8d00b128feb08ab42e208de30e42cd9889d8f1735a6e
