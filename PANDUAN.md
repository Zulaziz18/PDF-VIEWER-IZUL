# PANDUAN.md — Panduan Pengguna

> Panduan ini ditulis bertahap seiring fitur tersedia. Menuliskan petunjuk untuk
> fitur yang belum ada hanya akan menyesatkan, jadi bagian yang belum bisa
> dipakai sengaja dikosongkan sampai fasenya selesai.

## Status: Fase 0

Yang sudah dapat dilakukan:

- Menjalankan aplikasi dan melihat jendela utama.
- Membuka satu berkas PDF dan melihat halaman pertamanya.
- Melihat versi aplikasi di title bar dan jumlah pekerja di status bar.

Yang **belum** dapat dilakukan: scroll antar halaman, zoom, pencarian, anotasi,
menyimpan, dan tab. Semua itu datang di Fase 1 sampai Fase 4.

## Persyaratan sistem

- Windows 10 versi 2004 (build 19041) atau lebih baru, 64-bit.
- RAM 8 GB (16 GB disarankan untuk dokumen besar).
- Tidak memerlukan koneksi internet, sekarang maupun nanti. Aplikasi ini tidak
  pernah menghubungi jaringan: tanpa telemetri, tanpa pemeriksaan pembaruan,
  tanpa font daring.

## Di mana data disimpan

| Isi | Lokasi |
|---|---|
| Sesi, berkas terakhir, posisi baca, draf | `%APPDATA%\PDF Studio Izul\app.db` |
| Thumbnail dan cache halaman | `%APPDATA%\PDF Studio Izul\cache.db` |
| Log | `%APPDATA%\PDF Studio Izul\logs\` |

`cache.db` aman dihapus kapan saja; isinya dapat dibuat ulang. `app.db` tidak —
di situlah draf anotasi yang belum disimpan berada.

Versi portabel menyimpan ketiganya di folder `data` di samping berkas `.exe`,
sehingga seluruh aplikasi beserta datanya dapat dibawa di flash disk.

## Jika sebuah tab menampilkan pesan galat

Aplikasi menjalankan pengurai PDF di proses terpisah yang dikurung. Berkas rusak
dapat menjatuhkan proses itu, tetapi tidak dapat menjatuhkan aplikasi. Tab yang
terdampak akan memuat ulang sendiri, dan draf yang belum disimpan diambil
kembali dari basis data.

Dokumen yang menjatuhkan pekerja dua kali akan dibuka sendirian dalam mode
terbatas hanya-baca, agar tidak mengganggu dokumen lain.

## Melaporkan masalah

Tombol **Buka folder log** di kotak Tentang membuka folder log. Isinya tidak
pernah dikirim ke mana pun; lampirkan sendiri bila ingin melaporkan masalah.
