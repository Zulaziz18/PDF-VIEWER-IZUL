# PANDUAN.md — Panduan Pengguna

> Panduan ini ditulis bertahap seiring fitur tersedia. Menuliskan petunjuk untuk
> fitur yang belum ada hanya akan menyesatkan, jadi bagian yang belum bisa
> dipakai sengaja dikosongkan sampai fasenya selesai.

## Status: Fase 4

Aplikasi ini sekarang bisa dipakai membaca beberapa dokumen sekaligus, mencari
di dalamnya, menganotasi, dan **menyimpan** anotasi itu ke berkas PDF —
sebagai anotasi biasa yang terlihat di pembaca PDF lain, dan tetap bisa
disunting ulang di sini setelah berkasnya dibuka lagi.

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

### Jendela aplikasi

Jendelanya memakai bilah judul sendiri, bukan milik Windows. Tombol perkecil,
perbesar, dan **tutup (✕)** ada di ujung kanan bilah judul itu. Bilah judulnya
juga bisa diseret untuk memindahkan jendela, dan diklik ganda untuk
memperbesar.

Kalau ada anotasi di dokumen yang terbuka, menutup aplikasi akan bertanya lebih
dulu — anotasi belum bisa disimpan ke berkas PDF sampai Fase 4, jadi menutup
berarti kehilangannya.

### Membuka berkas dengan cara lain

- **Tombol 📂 di toolbar** (paling kiri) atau `Ctrl+O` — bisa memilih beberapa
  berkas sekaligus, masing-masing jadi satu tab. Tidak perlu menutup aplikasi
  dulu.
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

### Menganotasi

Baris alat di bawah toolbar berisi alat gambar. Pilih satu, lalu seret di atas
halaman.

| Alat | Cara pakai |
|---|---|
| Pena bebas, garis, panah | seret dari titik awal ke titik akhir |
| Kotak, elips | seret untuk membentuk kotak pembatasnya |
| Poligon | seret; bentuknya mengikuti jalur yang dilalui |
| Kotak teks | seret untuk membuat kotaknya, lalu ketik isinya di panel properti |
| Catatan tempel | klik di tempat catatan ingin ditempelkan |
| Stempel | seret untuk membentuk badgenya, lalu ubah tulisannya di panel properti |
| Gambar | tombol gambar membuka pemilih berkas; gambarnya muncul di tengah halaman |

Untuk stabilo, garis bawah, dan coret: **tandai dulu teksnya** dengan menyeret
kursor di atas halaman, lalu tekan tombolnya. Ketiganya mengikuti teks yang
ditandai, jadi tombolnya tidak melakukan apa-apa kalau tidak ada yang ditandai.

### Mengubah anotasi yang sudah ada

Dengan alat panah (tombol pertama, atau tekan `Esc`):

- **Pilih**: klik objeknya. Shift+klik menambah ke pilihan. Menyeret di ruang
  kosong membuat kotak pilihan.
- **Geser**: seret objeknya.
- **Ubah ukuran**: seret salah satu dari delapan pegangan di tepi kotaknya.
- **Putar**: seret pegangan bulat di atas kotaknya. Tahan Shift untuk mengunci
  ke kelipatan 15 derajat.
- **Hapus**: tombol `Delete`.
- **Batalkan / ulangi**: `Ctrl+Z` dan `Ctrl+Y`. Satu geseran adalah satu
  langkah, bukan seratus langkah kecil. Riwayatnya menyimpan 200 langkah.

Panel di sebelah kanan mengatur warna, opasitas, tebal garis, font, ukuran
huruf, isi teks, rotasi, dan kunci. Objek yang dikunci tidak ikut terseret dan
bisa diklik tembus — berguna untuk stempel latar.

Tab **Anotasi** di panel samping mendaftar semua anotasi dokumen, dikelompokkan
per halaman dan bisa disaring per jenis. Mengkliknya melompat ke tempatnya.

### Menyimpan

- **Ctrl+S** atau ikon disket di kiri atas menyimpan ke berkas yang sama.
  **Ctrl+Shift+S** (Simpan Sebagai) menyimpan ke nama atau folder lain; tab
  ikut pindah ke berkas baru, berkas lama tidak berubah.
- Titik oranye di tab dan tulisan "Belum disimpan" di bilah bawah berarti ada
  perubahan yang belum ada di berkas.
- Menyimpan tidak pernah merusak berkas lama. Aplikasi menulis salinan baru di
  samping berkas Anda, memeriksanya dengan membukanya ulang, dan baru
  menggantinya kalau pemeriksaan lulus. Kalau apa pun gagal — disk penuh,
  listrik padam — berkas lama tetap utuh.
- Menutup tab atau aplikasi dengan perubahan yang belum disimpan selalu
  bertanya dulu: **Simpan**, **Jangan Simpan**, atau **Batal**.

### Kalau aplikasi tertutup mendadak

Setiap 20 detik, dan setiap kali Anda berpindah ke jendela lain, perubahan yang
belum disimpan dicatat sebagai **draf** (bukan ke berkas PDF Anda). Kalau
aplikasi tertutup paksa, saat berkas itu dibuka lagi muncul pertanyaan
"Pulihkan pekerjaan yang belum disimpan?". **Pulihkan** mengembalikan
anotasinya; simpan setelah itu untuk menuliskannya ke berkas. **Nanti**
menyimpan drafnya untuk kesempatan berikutnya.

Kalau berkasnya sudah diubah program lain sejak draf dibuat, pertanyaannya
mengatakan itu — anotasi bisa berada di tempat yang salah bila halamannya
bergeser, jadi periksa dulu sebelum menyimpan.

### Kalau berkas diubah program lain

Pita kuning di atas halaman muncul bila berkas yang sedang terbuka diubah atau
dihapus oleh program lain. **Muat Ulang** membuka versi terbarunya dan
memasang kembali anotasi yang belum disimpan. **Abaikan** tetap memakai yang
terbuka — tetapi menyimpan nanti akan menimpa perubahan dari program lain itu.

### Konversi dan ekspor

Tab pita **Konversi**:

- **PDF ke Gambar** — halaman jadi berkas PNG atau JPG di folder pilihan Anda.
  Pilih semua halaman, halaman yang sedang dibuka, atau rentang seperti
  `1-3, 5, 8-` (`8-` berarti halaman 8 sampai akhir). 150 DPI cukup untuk
  layar dan dokumen biasa; 300 DPI untuk dicetak.
- **Ekspor Halaman** — halaman tertentu jadi PDF baru. Anotasinya ikut dan
  masih bisa disunting.
- **Ekspor Rata** — PDF baru dengan semua anotasi menyatu ke halaman. Cocok
  untuk dikirim ke orang lain bila anotasinya tidak boleh diubah atau
  dihapus.

Ekspor tidak pernah mengubah berkas yang sedang Anda buka, dan menolak menimpa
berkas yang sedang terbuka di tab lain. Semua hasil ekspor tercatat di
**Beranda → Riwayat Ekspor**.

### Yang perlu diketahui soal anotasi di fase ini

- Kotak teks, stempel, dan catatan memakai font standar PDF (Helvetica, Times,
  Courier) yang tidak ditanam ke berkas; pembaca lain menampilkannya dengan
  font padanan terdekat, jadi bentuk hurufnya bisa sedikit berbeda.
- Dokumen yang dilindungi kata sandi belum bisa disimpan.
- Yang **belum** dapat dilakukan: menyunting teks asli dokumen langsung di atas
  halaman (isi kotak teks diketik lewat panel properti).

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
