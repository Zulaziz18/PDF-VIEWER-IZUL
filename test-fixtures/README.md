# test-fixtures

Berkas uji tidak masuk repositori. Ukurannya ratusan megabita, dan seluruhnya
dapat dibuat ulang secara deterministik dari seed tetap:

```bash
python3 -m pip install reportlab pikepdf pypdf pillow
python3 bench/make_fixtures.py test-fixtures         # ~10 menit
python3 bench/make_fixtures_extra.py test-fixtures   # varian 50 MB
```

`MANIFEST.json` mencatat ukuran hasil generasi Fase 0, sehingga pergeseran
ukuran akibat perubahan pembangkit dapat terlihat.

## Bentuk berkas

"50 MB / 500 halaman" dari SPEC Bagian 13 bukan satu bentuk dokumen melainkan
tiga, dengan profil biaya yang sangat berbeda. Ketiganya dibuat:

| Berkas | Isi | Biaya didominasi oleh |
|---|---|---|
| `scan-realistic-50mb-500p.pdf` | 500 citra JPEG 100 dpi | dekode gambar |
| `text-500p.pdf` | teks padat dengan font tertanam | penguraian |
| `mixed-raw-500p.pdf` | teks + vektor + foto | keduanya |
| `mixed-500p.pdf` | sama, dipadatkan ke 50 MB dengan balast | memisahkan "berkas besar" dari "berkas rumit" |
| `scan-500p.pdf` | 500 citra 150 dpi, 138 MB | kasus tekanan di luar target |

Tiap berkas punya kembaran `-lin` hasil linearisasi, karena SPEC meminta
keduanya diukur.

## Belum ada

`test-fixtures/` SPEC Bagian 16 juga meminta berkas terenkripsi, rusak,
berform, CJK, Arab (RTL), dan PDF beranotasi dari aplikasi lain. Semuanya
menyusul di fase yang membutuhkannya — belum ada kode yang bisa gagal terhadap
berkas-berkas itu, jadi menyediakannya sekarang hanya akan jadi berkas yang
tidak diuji siapa pun.
