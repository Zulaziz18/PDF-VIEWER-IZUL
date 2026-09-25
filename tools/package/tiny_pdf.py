"""A one-page PDF for the packaging job's self-test, with a correct xref.

Standard library only: the Windows runner has Python but none of the PDF
packages the other proofs install.
"""

import sys

objects = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R"
    b" /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> >>",
]
content = b"BT /F1 24 Tf 72 760 Td (PDF Studio Izul - uji paket) Tj ET"
objects.append(b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream")

out = bytearray(b"%PDF-1.7\n")
offsets = []
for i, body in enumerate(objects, start=1):
    offsets.append(len(out))
    out += b"%d 0 obj\n" % i + body + b"\nendobj\n"
xref = len(out)
out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
for off in offsets:
    out += b"%010d 00000 n \n" % off
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, xref)

with open(sys.argv[1], "wb") as f:
    f.write(out)
