# PANDUAN.md — Panduan Pengguna

> Panduan ini ditulis bertahap seiring fitur tersedia. Menuliskan petunjuk untuk
> fitur yang belum ada hanya akan menyesatkan, jadi bagian yang belum bisa
> dipakai sengaja dikosongkan sampai fasenya selesai.

## Status: Fase 2

Aplikasi ini sekarang bisa dipakai membaca beberapa dokumen sekaligus, dan
mencari di dalamnya.

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

### Beberapa dokumen sekaligus

Setiap dokumen yang dibuka mendapat tabnya sendiri. Strip tab muncul begitu ada
dua dokumen — dengan satu dokumen ia hanya akan memakan ruang halaman.

- Berpindah tab: klik, atau `Ctrl+Tab` dan `Ctrl+Shift+Tab`.
- Menutup tab: tombol `×` pada tabnya, klik tombol tengah tetikus, atau
  `Ctrl+W`. `Ctrl+W` menutup **tab**, bukan jendela.
- Mengurutkan ulang: seret tabnya ke tempat lain.

Setiap tab mengingat perbesaran, rotasi, posisi baca, dan hasil pencariannya
sendiri, jadi berpindah bolak-balik tidak menghilangkan apa pun.

Tab yang lama tidak dilihat melepaskan gambar halamannya untuk menghemat memori,
dan mengambilnya kembali saat dibuka lagi. Tiga tab terakhir yang dilihat selalu
siap seketika. Membuka berkas yang sudah terbuka akan berpindah ke tabnya, bukan
membuat salinan kedua.

Susunan tab disimpan otomatis dan dipulihkan saat aplikasi dijalankan lagi.
Berkas yang sudah dipindah atau dihapus dilewati tanpa pesan galat.

### Membuka berkas dengan cara lain

- **Seret & lepas** berkas PDF ke jendela.
- **Klik ganda** berkas PDF di Windows Explorer, jika asosiasi berkas dipasang
  saat instalasi. Kalau aplikasi sudah berjalan, berkasnya menjadi tab baru di
  jendela yang sedang terbuka — bukan jendela kedua.
- **Daftar berkas terakhir** di layar awal, kini bergambar sampul halaman
  pertama. Sampul hanya ada untuk berkas yang pernah dibuka di aplikasi ini —
  membuatkannya untuk berkas yang belum pernah dibuka berarti membuka semuanya,
  dan itu akan membuat layar awal lambat. Tombol bintang menyematkan berkas ke
  atas daftar.

### Mencari

Buka panel pencarian dengan `Ctrl+F`. Ada tiga cakupan:

| Cakupan | Mencari di | Hasilnya |
|---|---|---|
| **Dokumen ini** | seluruh halaman dokumen yang terbuka | daftar halaman beserta cuplikan kalimatnya |
| **Semua dokumen** | setiap berkas yang pernah dibuka di aplikasi ini | daftar halaman beserta nama berkasnya; satu klik membukanya |
| **Regex** | halaman yang sedang dibuka saja | kecocokan disorot langsung di halaman |

`F3` melompat ke hasil berikutnya, `Shift+F3` ke sebelumnya, `Esc` menutup
panel. Kecocokan pada halaman yang sedang tampak selalu disorot kuning.

Dua catatan yang jujur soal batasannya:

- **Teks dokumen diindeks di latar belakang** saat dokumen dibuka. Untuk
  dokumen besar ini perlu beberapa detik, dan panel menampilkan kemajuannya.
  Mencari sebelum selesai akan menemukan halaman yang sudah terindeks saja.
- **Regex hanya berlaku untuk dokumen yang sedang terbuka, satu halaman pada
  satu waktu.** Ini bukan kekurangan yang akan ditambal: indeks pencarian
  menyimpan kata, bukan teks berurutan, sehingga pola tidak punya apa pun untuk
  dijalankan di sana. Untuk mencari di seluruh pustaka, pakai cakupan
  **Semua dokumen**.

Yang **belum** dapat dilakukan: menganotasi dan menyimpan. Keduanya datang di
Fase 3 dan Fase 4.

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
