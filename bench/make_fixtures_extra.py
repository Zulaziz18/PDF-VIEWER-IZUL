#!/usr/bin/env python3
"""Second-pass fixtures: a scan at the SPEC's stated 50 MB, and an unpadded
mixed document so we can separate 'file is big' from 'file is complex'."""
import sys, io, json
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))
import make_fixtures as mf
import pikepdf

OUT = Path(sys.argv[1] if len(sys.argv) > 1 else "test-fixtures")

# ~50 MB of scan = ~100 KB/page. 120 dpi at JPEG q35 lands close.
mf.make_scan(OUT / "scan-50mb-500p.pdf", quality=35, dpi=120)
mf.report(OUT / "scan-50mb-500p.pdf")
mf.linearize(OUT / "scan-50mb-500p.pdf", OUT / "scan-50mb-500p-lin.pdf")
mf.report(OUT / "scan-50mb-500p-lin.pdf")

mf.make_mixed(OUT / "mixed-raw-500p.pdf")          # no ballast padding
mf.report(OUT / "mixed-raw-500p.pdf")

sizes = {p.name: p.stat().st_size for p in sorted(OUT.glob("*.pdf"))}
(OUT / "MANIFEST.json").write_text(json.dumps(sizes, indent=2) + "\n")
print(json.dumps(sizes, indent=2))
