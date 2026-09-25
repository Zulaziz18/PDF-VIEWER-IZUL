#!/usr/bin/env python3
"""An AcroForm with one field of every kind the application fills (Phase 7).

Made with reportlab's own form support, so the fields are what a common PDF
producer writes: a text field, a multi-line text field, a checkbox whose "on"
state is not called /Yes, a radio group of three, a combo box, and a list box.
The content is invented.

    python3 tools/form-proof/make_form.py <out.pdf>
"""
import sys

from reportlab.lib.colors import black, white
from reportlab.pdfgen import canvas


def main():
    out = sys.argv[1]
    c = canvas.Canvas(out, pagesize=(595, 842))
    c.setFont("Helvetica-Bold", 16)
    c.drawString(60, 780, "Formulir Pendaftaran Kegiatan")
    c.setFont("Helvetica", 11)
    form = c.acroForm
    c.drawString(60, 730, "Nama lengkap")
    form.textfield(name="nama", x=180, y=722, width=300, height=22, borderColor=black, fillColor=white, fontSize=11)
    c.drawString(60, 690, "Alamat")
    form.textfield(name="alamat", x=180, y=630, width=300, height=70, borderColor=black, fillColor=white,
                   fontSize=10, fieldFlags="multiline")
    c.drawString(60, 600, "Setuju dengan ketentuan")
    form.checkbox(name="setuju", x=240, y=594, size=16, buttonStyle="check", borderColor=black, fillColor=white,
                  checked=False)
    c.drawString(60, 560, "Kategori")
    for i, (value, label) in enumerate([("umum", "Umum"), ("pelajar", "Pelajar"), ("pengajar", "Pengajar")]):
        form.radio(name="kategori", value=value, x=180 + i * 110, y=554, size=16, borderColor=black,
                   fillColor=white, selected=False)
        c.drawString(200 + i * 110, 558, label)
    c.drawString(60, 510, "Kota")
    form.choice(name="kota", x=180, y=502, width=200, height=22, options=["Bandung", "Jakarta", "Surabaya"],
                value="Bandung", fieldFlags="combo", borderColor=black, fillColor=white, fontSize=11)
    c.drawString(60, 470, "Sesi")
    form.listbox(name="sesi", x=180, y=400, width=200, height=66, options=["Pagi", "Siang", "Malam"],
                 value="Pagi", borderColor=black, fillColor=white, fontSize=11)
    c.showPage()
    c.save()
    # A checkbox whose "on" state is not /Yes, as many producers write it:
    # a filler that assumes /Yes leaves it looking unticked.
    import pikepdf

    with pikepdf.open(out, allow_overwriting_input=True) as pdf:
        for annot in pdf.pages[0].Annots:
            if annot.get("/T") == "setuju":
                n = annot.AP.N
                n[pikepdf.Name("/Ya")] = n["/Yes"]
                del n["/Yes"]
                if "/D" in annot.AP:
                    d = annot.AP.D
                    d[pikepdf.Name("/Ya")] = d["/Yes"]
                    del d["/Yes"]
        pdf.save(out)


if __name__ == "__main__":
    main()
