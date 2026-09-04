# PANDUAN.md — Panduan Pengguna

> Panduan ini ditulis bertahap seiring fitur tersedia. Menuliskan petunjuk untuk
> fitur yang belum ada hanya akan menyesatkan, jadi bagian yang belum bisa
> dipakai sengaja dikosongkan sampai fasenya selesai.

## Status: Fase 1

Aplikasi ini sekarang bisa dipakai membaca.

### Membuka dan menggulir

Buka berkas lewat tombol **Buka Berkas** atau `Ctrl+O`. Dokumen tampil sebagai
gulungan berkelanjutan: gulirkan dengan roda tetikus, panah, `Page Up`/
`Page Down`, `Home` dan `End`. Nomor halaman di toolbar bisa diisi langsung
untuk melompat.

Halaman yang belum sempat dirender tajam tampil sebagai versi buram lebih dulu,
lalu berganti tajam. Itu disengaja: yang penting halaman tidak pernah kosong.

### Perbesaran

| Perintah | Cara |
|---|---|
| Perbesar / perkecil | `Ctrl` + `+` / `Ctrl` + `−`, atau tombol di toolbar |
| Ukuran asli (100 %) | `Ctrl` + `0` |
| Muat lebar / muat halaman | Tombol **Lebar** dan **Muat** di toolbar |
| Perbesar di titik tertentu | `Ctrl` + roda tetikus, atau cubit di touchpad |

`Ctrl` + roda mempertahankan titik di bawah kursor: yang Anda tunjuk tidak
bergeser saat diperbesar.

### Tata letak halaman

Empat mode di toolbar: **Satu** halaman, **Dua** halaman berdampingan, **Dua +
sampul** (halaman pertama sendirian, seperti buku yang dibuka), dan **Mendatar**
(menggulir ke samping).

### Memutar

**Putar kiri** dan **Putar kanan** memutar seluruh dokumen; **Putar halaman ini**
hanya halaman yang sedang dibaca — berguna untuk satu halaman lanskap di tengah
dokumen potret.

### Memilih dan menyalin teks

Teks dokumen dapat diseleksi dan disalin seperti di halaman web, termasuk pada
halaman yang diputar. Pembaca layar juga membaca teks ini, bukan gambarnya.

### Panel samping

Tombol ☰ membuka panel samping. **Halaman** menampilkan thumbnail — klik untuk
melompat. **Daftar Isi** menampilkan bookmark bawaan dokumen, bila ada.

### Posisi baca diingat

Menutup lalu membuka kembali berkas yang sama akan mengembalikan halaman,
posisi gulir, perbesaran, rotasi, dan mode tampilan seperti saat ditinggalkan.

Yang **belum** dapat dilakukan: membuka beberapa dokumen sekaligus dalam tab,
mencari, menganotasi, dan menyimpan. Semua itu datang di Fase 2 sampai Fase 4.

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
