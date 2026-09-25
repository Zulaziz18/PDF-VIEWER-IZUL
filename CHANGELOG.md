# Changelog

Semua perubahan penting per fase. Format mengikuti [Keep a Changelog](https://keepachangelog.com/id/1.1.0/);
versi mengikuti `version.json` sebagai sumber tunggal.

## [7.0.0-alpha.6] — Fase 6: Redaksi Sejati

Bagian dokumen yang ditandai kini bisa **dihapus sungguhan** dari berkas —
bukan ditutup kotak hitam yang teksnya masih bisa disalin dari bawahnya.
Kriteria SPEC ("teks yang diredaksi tidak dapat ditemukan lagi oleh alat
ekstraksi mana pun") diuji dengan enam alat yang tidak berbagi kode:
`bench/results/phase6-redaction.txt`, **LULUS** di dua belas jenis halaman.

### Bagaimana redaksi bekerja

PDFium tidak punya API redaksi, jadi pekerjaannya dilakukan sendiri oleh
crate baru `izul-redact`: membaca berkas yang baru ditulis PDFium, menafsirkan
content stream setiap halaman yang ditandai (matriks, status teks, lebar
glyph dari font), membuang apa pun di dalam area, lalu **menulis ulang seluruh
berkas** — bukan pembaruan inkremental, yang akan menyisakan stream lama utuh
di dalam berkas. Hasilnya dibuka ulang oleh pekerja dengan PDFium dan
diperiksa dua kali (tidak ada karakter tersisa di area; tidak ada karakter di
luar area yang bergeser; pemeriksaan kedua lewat jalur konversi koordinat yang
berbeda), lalu sekali lagi pada berkas sementara sebelum menggantikan berkas
tujuan. Satu rutin (`izul_pdf::redaction::redact_document`) dipakai pekerja,
harness screenshot, dan alat bukti — yang dibuktikan adalah yang dikirim.

### Ditambahkan

- **Pita Lindungi**: Tandai Teks, Tandai Area, Cari & Tandai (tandai semua
  hasil pencarian sekaligus, termasuk pola), Terapkan Redaksi.
- **Tanda redaksi** sebagai anotasi `/Redact` standar: bisa dipindah, diubah
  ukurannya, diberi warna isi, dan di-undo selama belum diterapkan. Tanda
  yang disimpan tanpa diterapkan diperingatkan — isinya masih ada di berkas.
- **Dialog Terapkan Redaksi**: jumlah tanda dan halaman, anotasi yang ikut
  terhapus, peringatan tidak bisa dibatalkan, dan pilihan simpan sebagai
  berkas baru (bawaan, `nama (diredaksi).pdf`) atau timpa.
- Yang dihapus: glyph (termasuk teks tak terlihat lapisan OCR), gambar di
  dalam area, piksel gambar yang tertutup sebagian, path di dalam area, isi
  form XObject, `/ActualText`/`/Alt` di atas konten yang dibuang, anotasi dan
  field formulir di area, gambar mini halaman (`/Thumb`), dan metadata
  halaman. Sesudah menimpa berkas, sampul "berkas terbaru" dan indeks
  pencarian lamanya ikut dihapus — keduanya salinan isi yang diredaksi.
- `tools/redaction-proof/`: dua belas kasus (standard-14, TrueType tertanam,
  CID, TJ berkerning, teks miring, form, pindaian + OCR, gambar inline,
  Type 3, ActualText, anotasi/field, halaman /Rotate dengan MediaBox
  bergeser), dicari ulang oleh PDFium, poppler, MuPDF, pypdf, pdfminer, dan
  isi mentah berkas; dijalankan CI di ubuntu.
- `tools/redaction-proof/bench.py` → `bench/results/phase6-linux.txt`.
- IPC: `WorkRedact`, `VerifyRedacted` (+ balasannya) di ujung enum;
  `PROTOCOL_VERSION` 6 → 7. Anotasi jenis ke-14 `Redact` di `izul-model`.

### Diperbaiki (ditemukan oleh alat bukti sebelum rilis)

- **Huruf yang sebagian besar di bawah tanda hilang separuh di luar kotak.**
  Glyph yang ≥ 25 % tertutup dibuang utuh (sisanya akan tetap terbaca), tetapi
  bagiannya di luar kotak lenyap tanpa ditutup — paling terlihat di teks
  miring. Kini kotak glyph itu ikut diwarnai warna tanda, dan laporan redaksi
  mencatat setiap bagian yang diambil di luar area.
- **Pindaian membengkak 3× setelah diredaksi** (145 MB → 469 MB): gambar JPEG
  yang dinolkan sebagian ditulis ulang sebagai piksel mentah. Kini ditulis
  sebagai JPEG lagi dengan tabel kuantisasi dan subsampling aslinya: 104 MB,
  mutu di luar area praktis tak berubah (PSNR 70 dB), dan 500 halaman
  diredaksi dalam 31 detik, bukan 99.
- Catatan pengukuran di Fase 6 (2/n) keliru soal MuPDF: MuPDF **membulatkan**
  `/Widths` ke unit terdekat (terukur 0,0056 pt, diprediksi 0,00553 pt);
  poppler-lah yang memakai lebar persis.

### Diketahui

- Gambar JPEG 2000, JBIG2, dan faks yang tersentuh tanda dihapus **utuh**
  (tidak bisa dihapus sebagian di sini) dan dilaporkan di pesan hasil.
- Font tanpa informasi lebar yang bisa dibaca ditolak — redaksi dibatalkan
  dengan pesan, bukan ditebak.
- Redaksi dokumen terenkripsi belum didukung.
- Lisensi baru pihak ketiga: `jpeg-encoder` (MIT/Apache-2.0 + IJG); kalimat
  atribusi IJG ada di README dan kotak Tentang.

## [7.0.0-alpha.5] — Fase 5: Operasi Halaman, Split View & Banding

Halaman kini bisa disusun: dihapus, dipindah, diputar, diduplikat, disisip
kosong, digabung dari PDF lain, diekstrak, dan dipecah. Beberapa dokumen bisa
dilihat berdampingan dalam hingga empat panel, dan dua versi sebuah dokumen
bisa dibandingkan dengan perbedaannya ditandai.

### Bagaimana operasi halaman bekerja

Susunan halaman adalah **data**, bukan perubahan langsung pada PDF: sebuah
peta "halaman ke-*n* di layar adalah halaman ke-*m* berkas ini (atau berkas
lain), diputar sekian kali, atau halaman kosong". Peta itu hidup di tumpukan
undo yang sama dengan anotasi (`izul-model/pages.rs`), dan setiap operasi
membawa serta perpindahan anotasi di halamannya dalam satu langkah — tidak
ada keadaan di mana halaman sudah pindah tetapi stabilonya belum.

Saat disimpan, pekerja menerapkan peta itu **di tempat** pada salinan kerja
(`izul-pdf/arrange.rs`): halaman baru ditambah di ujung, yang tak dipakai
dihapus, sisanya dipindah ke urutan akhir dengan `FPDF_MovePages`. Membangun
dokumen baru lalu mengimpor halaman akan lebih sederhana — dan akan diam-diam
membuang bookmark, metadata, dan preferensi tampilan dokumen.

Halaman dari berkas lain dibaca dari **salinan** di folder data, dibuka
sebagai dokumen tersembunyi supaya dirender lewat jalur ubin yang sama.

### Ditambahkan

- **Pita Halaman**: Panel Halaman, Sisip Kosong, Gabung PDF, Hapus,
  Duplikat, Putar kiri/kanan, Ekstrak, Pecah, Batalkan/Ulangi. Tombol
  bertindak atas halaman yang dipilih di panel, atau halaman yang sedang
  dibaca.
- **Panel halaman**: klik, Ctrl+klik, Shift+klik untuk memilih; seret untuk
  memindah; Delete menghapus; Ctrl+A memilih semua. Seret ke dokumen lain di
  split view untuk menyalin (Shift untuk memindah) — anotasi yang belum
  disimpan ikut.
- **Pecah Dokumen**: per N halaman, per rentang ("1-3; 4-10; 11-"), atau satu
  berkas per bookmark utama, dengan pratinjau hasil sebelum menulis.
- **Split view**: satu, dua berdampingan, dua atas-bawah, atau empat panel
  (menu Jendela di pita Beranda). Tab diseret ke panel; pembatas diseret atau
  digeser dengan panah; tata letak dan ukuran diingat.
- **Mode Banding**: dua dokumen berdampingan, gulir bersamaan per posisi
  halaman, perbedaan kata ditandai merah muda di kedua sisi; halaman tanpa
  teks (pindaian) dibandingkan secara visual.
- "Putar halaman ini" kini suntingan dokumen (tersimpan, bisa di-undo), bukan
  hanya tampilan. Rotasi seluruh dokumen tetap pengaturan tampilan.
- IPC: `Request::WorkArrange` di ujung enum; `PROTOCOL_VERSION` 5 → 6.
- Perintah baru: `pages_state`, `pages_apply`, `pages_insert_file`,
  `pages_copy_from`, `compare_visual`, `pref_get`/`pref_set` (hanya kunci
  berawalan `ui.`).

### Diperbaiki

- **Gabung PDF membengkakkan berkas 11 kali.** Halaman diimpor satu per satu,
  dan PDFium menyalin sumber daya bersama (font tertanam) sekali per panggilan
  impor: dua berkas teks 1 MB jadi 22,1 MB. Sekarang satu panggilan per
  sumber: 1,9 MB. Test regresi dengan gambar 120 KB yang dipakai 20 halaman
  terbukti gagal pada cara lama (2,4 MB).
- **Mode banding sempat tidak menandai apa pun.** Cache teks sesi "mengklaim"
  halaman dengan daftar kosong sebelum jawabannya datang; pembanding yang
  membaca saat itu mengira halaman tanpa teks. Pembanding kini mengambil teks
  sendiri.
- Tanda tangan lapisan sorotan hanya memakai jumlah kotak, sehingga hasil
  pencarian baru dengan jumlah sama di tempat lain tidak digambar ulang.

### Angka

Kriteria SPEC 17 untuk Fase 5 tidak menyebut angka; yang diukur di sini
adalah kebenaran dan biaya. Build release, `bench/results/phase5-linux.txt`:

| Berkas | Operasi | Susun | Simpan | Hasil |
|---|---|---|---|---|
| text-500p (1,0 MB) | balik urutan 500 halaman | 37 ms | 10 ms | 1,0 MB |
| text-500p | hapus 250 halaman | 12 ms | 5 ms | 0,5 MB |
| text-500p | gabung 500 halaman lain | 8 ms | 27 ms | 1,9 MB |
| mixed-500p (50 MB) | balik urutan | 53 ms | 1 085 ms | 50,0 MB |
| mixed-500p | gabung 500 halaman lain | 4 ms | 1 127 ms | 51,0 MB |
| scan-50mb-500p (80 MB) | balik urutan | 42 ms | 142 ms | 79,8 MB |
| scan-50mb-500p | gabung 500 halaman lain | 5 ms | 147 ms | 80,8 MB |

Menyusun ulang sendiri paling lama 54 ms; biaya terbesar tetap PDFium menulis
berkas utuh, sama seperti simpan biasa di Fase 4.

**Kebenaran, dengan pekerja sungguhan** (`page_ops.rs`): lima operasi berurutan
(pindah, hapus, duplikat, sisip kosong, sisip dari berkas lain dengan rotasi)
disimpan lalu dibuka ulang: urutan teks halaman tepat, halaman asing berputar,
setiap anotasi di halaman tempat ia digambar, bookmark masih ada. Isi halaman
yang dihapus tidak ada lagi di berkas.

### Diketahui / belum

- Halaman ke-*n* dibandingkan dengan halaman ke-*n*; dokumen yang halamannya
  bergeser disejajarkan dulu dengan memindah halaman.
- Satu dokumen hanya bisa tampil di satu panel (lihat keputusan di CLAUDE.md).
- Setelah menyimpan perubahan susunan halaman, riwayat undo dikosongkan:
  langkah-langkahnya merujuk nomor halaman berkas lama.
- Bookmark yang menunjuk halaman yang dihapus tetap ada tetapi tidak menuju
  ke mana pun (perilaku PDFium, terlihat di test `page_ops.rs`).

## [7.0.0-alpha.4] — Langkah 0 (UI gaya WPS) + Fase 4: Tulis & Simpan

Anotasi sekarang sampai ke berkas. Yang disimpan adalah **anotasi PDF standar**
— pembaca lain menampilkannya apa adanya — ditambah metadata milik kita di satu
kunci tambahan, sehingga v7 bisa menyunting ulang semuanya setelah berkas
ditutup dan dibuka lagi. Kriteria lulus SPEC 17 ("tetap editable setelah
simpan-buka, tampil benar di Acrobat, Chrome, Edge") dijawab di bagian Angka.

### Langkah 0: kerangka UI gaya WPS Office (sebelum Fase 4)

Diminta pengguna; SPEC Bagian 12 ditulis ulang bertanggal 23 September 2026.
Bilah judul dengan tab dokumen dan tab Beranda tetap, pita bertab (Beranda,
Edit, Komentar, dan sejak Fase 4 Konversi) dengan tombol ikon besar, rel ikon
kiri, layar Beranda berbentuk tabel dengan panel Info Berkas, bilah bawah
dengan navigasi halaman dan slider zoom, mode gelap yang benar-benar
tersambung. Ikon: Fluent UI System Icons (MIT). Tab pita hanya muncul untuk
fitur yang sudah bekerja. Harness screenshot `npm run ui:shots` (Vite + mock
IPC + Chromium, ubin dirender PDFium sungguhan) — hasilnya di `docs/ui/`.

### Bagaimana menyimpan bekerja

1. Pekerja membuat **salinan kerja** dokumen, mengganti anotasi kita dengan
   penanda kosong bernama `izul-<id>`, lalu PDFium menulis berkas utuh.
2. Proses UI menambahkan **satu bagian pembaruan inkremental** di ujungnya
   (`crates/izul-write`): tiap penanda didefinisikan ulang sebagai anotasi
   standar lengkap dengan AP stream dari display list yang sama yang digambar
   kanvas (SPEC 3.2), font standard-14 WinAnsi, ExtGState untuk opasitas, dan
   gambar — JPEG diteruskan byte-per-byte, sisanya Flate RGB + SMask.
3. Hasilnya ditulis ke berkas sementara **di samping** target, di-`fsync`,
   **dibuka ulang oleh pekerja** untuk membuktikan ia PDF yang sehat dengan
   jumlah halaman dan anotasi yang benar, baru kemudian di-rename menimpa target
   (`MoveFileExW` + `WRITE_THROUGH` di Windows). Berkas lama tidak pernah
   setengah tertimpa: kalau apa pun gagal, target tidak tersentuh.

### Ditambahkan

- **Simpan** (Ctrl+S), **Simpan Sebagai** (Ctrl+Shift+S) — menu Berkas, tombol
  akses cepat, pita Konversi.
- **Pita Konversi**: PDF ke Gambar (PNG/JPG, 72–600 DPI, rentang halaman),
  Ekspor Halaman (PDF baru, anotasi tetap bisa disunting), Ekspor Rata (semua
  anotasi menyatu ke halaman).
- **Riwayat Ekspor** di Beranda, dengan ukuran dan berkas asal; hasil yang sudah
  dipindah/dihapus tampil pudar.
- **Tutup dengan pekerjaan belum disimpan** — tab maupun jendela (termasuk
  Alt+F4) menanyakan Simpan / Jangan Simpan / Batal. Hanya "Jangan Simpan" yang
  membuang pekerjaan.
- **Autosave draf** ke SQLite tiap 20 detik dan saat jendela kehilangan fokus
  (SPEC 8), ditawarkan untuk dipulihkan saat dokumennya dibuka lagi. Draf untuk
  berkas yang sudah diubah program lain mengatakannya dan hanya diterapkan bila
  diminta.
- **Berkas diubah program lain / hilang**: pita peringatan di atas halaman dengan
  Muat Ulang (anotasi yang belum disimpan dipasang kembali) atau Abaikan.
- **Status simpan dan ukuran berkas** di bilah bawah; titik oranye di tab kini
  mengikuti "belum disimpan" dari backend, bukan "bisa di-undo".
- **Impor anotasi** dari berkas yang disimpan v7: dilepas dari halaman tampilan
  saat pertama dimuat (supaya tidak tergambar dobel) dan dijadikan objek hidup.
- `cargo run -p izul-bench --bin fase4-uji` membuat PDF uji 14 anotasi dan
  versi ratanya untuk diperiksa di Acrobat, Chrome, Edge; `--bench` mengukur
  jalur simpan pada fixture 500 halaman.
- IPC: sebelas `Request` dan lima `Response` baru, semuanya di ujung enum;
  `PROTOCOL_VERSION` 4 → 5.
- Izin baru di `capabilities/default.json`: `dialog:allow-save`,
  `core:window:allow-destroy`. `dialog:allow-confirm` dilepas karena tidak lagi
  dipakai.

### Diperbaiki

- **Simpan kedua menghapus kotak teks.** Penulis membuang diam-diam objek yang
  metrik fontnya belum ada di cache — dan cache itu kosong untuk objek hasil
  impor. Sekarang metrik disiapkan untuk semua objek sebelum menulis, dan objek
  tanpa metrik adalah galat, bukan objek yang hilang. Test integrasi
  `save_round_trip` terbukti gagal pada kode lama (4 dari 5 anotasi kembali).
- **Ekspor atau Simpan Sebagai ke berkas yang terbuka di tab lain** kini
  ditolak dengan pesan. Di Windows rename-nya akan gagal di tengah jalan; di
  sistem lain berhasil dan diam-diam mengganti isi dokumen yang sedang dibaca.
- **"Jangan Simpan" bisa dibatalkan autosave.** Autosave yang kebetulan jalan
  di antara membuang draf dan menutup tab akan menulis ulang draf yang baru
  ditolak. `draft_discard` kini juga menandai perubahan itu sudah diputuskan.
- Tabel xref pembaruan dibuka dengan subbagian objek 0, sehingga pypdf tidak
  lagi memperingatkan "not zero-indexed".

### Angka

**Tetap editable setelah simpan-buka** — `open_annotate_save_reopen_and_edit_again` (test integrasi,
pekerja sungguhan): buka → anotasi → simpan → tutup → buka → sunting → simpan
lagi → buka; semua objek kembali dengan jenis, posisi, dan isi yang sama, gambar
piksel-per-piksel termasuk alfa, dan simpan kedua tidak menggandakan apa pun. Plus
`every_kind_survives_save_and_reopen_as_a_live_object` di `izul-pdf` untuk
ketiga belas jenis.

**Paritas layar vs berkas tersimpan** (SPEC 3.3, ambang 0,5 %): 22 kasus
(sebelas jenis tanpa teks, tegak dan diputar) **0,000 %**. Pemeriksaan
kewarasan: golden vs halaman putih polos 19,4 %, jadi nol itu identik, bukan
dua gambar kosong.

**Mesin lain:** poppler dan MuPDF menggambar keempat belas anotasi berkas uji di
tempatnya. Selisih keduanya 1,61 % dengan anotasi hidup dan 1,59 % pada versi
rata — perbedaan antialias antar mesin, bukan anotasi. Versi rata identik
0,00 % dengan versi beranotasi. Rinciannya di `bench/results/phase4-parity.txt`.

**Kecepatan jalur simpan** (build release, 280 anotasi di 20 halaman,
`bench/results/phase4-linux.txt`):

| Berkas | Ukuran | PDFium menulis | Anotasi (patch) | Verifikasi | Ratakan 20 hal. |
|---|---|---|---|---|---|
| text-500p | 1,0 MB | 22 ms | 13 ms | 13 ms | 27 ms |
| mixed-500p | 50,0 MB | 1 110 ms | 14 ms | 26 ms | 156 ms |
| scan-50mb-500p | 79,8 MB | 123 ms | 14 ms | 10 ms | 99 ms |

Biaya terbesar adalah PDFium menulis ulang berkas utuh; bagian anotasi kita
tetap belasan milidetik berapa pun besar berkasnya. SPEC tidak menetapkan
target waktu simpan, jadi tidak ada angka "lulus/gagal" di sini.

### Diketahui / belum

- Belum diuji di Adobe Acrobat dan Edge — tidak ada di lingkungan
  pengembangan. Pengganti yang dipakai: dua mesin PDF independen (poppler dan
  MuPDF) dan dua pemeriksa struktur (qpdf lewat pikepdf, pypdf mode ketat).
  Langkah memeriksanya sendiri ada di TESTING.md.
- Dokumen terenkripsi tidak bisa disimpan (pesan jelas, bukan kegagalan diam).
- Kotak teks, stempel, dan catatan memakai font standard-14 yang **tidak
  ditanam**; pembaca lain menggantinya dengan padanan terdekat. Menanam font
  datang bersama penyuntingan teks (Fase 7).
- `npm audit`: dua peringatan "moderate" pada `vitest` (alat test, tidak ikut ke
  aplikasi). Sudah ada sebelum fase ini; perbaikannya naik versi mayor.

## [7.0.0-alpha.3] — Fase 3: Mesin Anotasi & Paritas

Fase ini membuat anotasi ada, dan membuat paritas antara yang terlihat di layar
dan yang akan ditulis ke berkas menjadi **struktural**, bukan sesuatu yang
dikejar lewat laporan bug. Kriteria lulusnya golden image per jenis anotasi, dan
angkanya ada di `bench/results/phase3-parity.txt`.

### Bagaimana paritas dijamin

Model anotasi tidak menghasilkan piksel dan tidak menghasilkan AP stream. Ia
menghasilkan **display list** — urutan perintah gambar primitif dalam koordinat
PDF — dan dua backend membaca daftar yang sama: kanvas menggambar proksi
langsung saat objek diseret, AP stream menyerialkannya jadi operator PDF saat
disimpan. Geometri tidak mungkin menyimpang karena sumbernya hanya satu
(SPEC 3.2).

Yang membuat ini bekerja adalah disiplin di satu tempat: **semua** yang menggoda
untuk diserahkan ke backend diputuskan di `izul-model/build.rs`. Penghalusan
tinta jadi kurva Bézier eksplisit (bukan spline milik masing-masing), kepala
panah jadi jalur (bukan `/LE` yang hanya dimengerti sisi PDF), elips jadi empat
kurva (PDF tidak punya operator elips dan `ellipse()` kanvas adalah hampiran
lain), tata letak teks jadi glif berposisi, rotasi jadi satu transform.

### Ditambahkan

- **Tiga belas jenis anotasi SPEC 11.2** sebagai satu enum tertutup dengan
  payload per jenis, sehingga "semua jenis tertangani" adalah galat kompilasi.
- **Backend AP stream** dengan byte deterministik dan state seimbang. Stream
  yang membocorkan `q` merusak gambar anotasi *lain* di halaman yang sama, jadi
  penulisnya menutup apa pun yang diserahkan padanya alih-alih memercayainya.
- **Backend kanvas** di frontend, membaca daftar yang sama lewat perintah
  `annot_display_lists`. Frontend tidak pernah menghitung geometri sendiri.
- **Undo/redo** berbasis operasi yang tahu kebalikannya: satu gestur satu
  langkah, transaksi yang gagal di tengah dibatalkan seluruhnya, batas 200
  langkah (SPEC 8), id tidak pernah dipakai ulang.
- **Seleksi dan transformasi**: klik, Shift-klik, pita karet, delapan pegangan
  ubah ukuran, pegangan rotasi dengan snap 15 derajat, kunci objek. Uji tembak
  mengikuti geometri sebenarnya, bukan kotak pembatas — kotak sebuah garis
  diagonal sebagian besar ruang kosong.
- **Panel properti** (warna, opasitas, tebal garis, font, ukuran, teks, kunci,
  rotasi) dan **panel daftar anotasi** di sidebar, dikelompokkan per halaman,
  bisa disaring per jenis, klik untuk melompat.
- **Metrik font diukur dari PDFium**, bukan dari tabel yang ditulis dari ingatan
  — lewat permintaan IPC baru (`PROTOCOL_VERSION` naik 3 → 4), karena PDFium ada
  di dalam sandbox dan proses UI tidak boleh menautnya (SPEC 5). Asumsi bahwa
  kode karakter adalah indeks glif untuk standard-14 **diuji dengan render
  sungguhan**, bukan dipercaya.
- **Gambar** disisipkan dari berkas, disimpan di memori proses UI, dan diambil
  kanvas lewat rute protokol baru `izul://image/{doc}/{ref}`.
- **Golden image** untuk kelima belas kasus (tiga belas jenis + satu berputar
  dan tembus pandang), dan **harness paritas kanvas** yang menjalankan Chromium
  sungguhan.

### Angka

- **Golden image:** lima belas baseline, seluruhnya lulus pada ambang < 0,5
  persen piksel berbeda (toleransi 8/255 per kanal).
- **Kanvas vs PDFium:** dua belas jenis non-teks di bawah 0,53 persen. Tiga
  kasus berteks dikecualikan dari angka itu dan alasannya batasan, bukan
  kelulusan — baseline PDFium memakai font uji Type 3 sementara kanvas
  menggambar huruf sungguhan. Yang tetap berarti di sana: cakupan tintanya
  berdempetan (56,5 vs 56,6 persen), artinya teksnya mendarat di tempat sama.
- **Test:** 338 Rust (dari 254) dan 106 TypeScript (dari 71).

### Diketahui, dan tidak ditutup-tutupi

- **Baseline teks tidak boleh bergantung pada font mesin.** Versi pertama
  golden image memakai font sungguhan dan **gagal di CI Windows** — tiga
  baseline berteks berbeda 1,6–3,0 persen karena PDFium mengambil outline dari
  font sistem di sana, sementara dua belas lainnya lulus. Diperbaiki dengan
  memindahkan kedua sisinya ke dalam berkas test: metrik dari `FixedFont`, glif
  dari font Type 3 yang charproc-nya ditulis di situ. Baselinenya kini berupa
  blok, dan itu memang tujuannya — blok yang bergeser tetap regresi tata letak,
  tapi blok tidak bisa berubah bentuk karena mesinnya lain.
- **Cacat yang ditemukan harness paritas:** anotasi gambar semula berbeda 30,7
  persen karena kanvas menghaluskan gambar yang diperbesar dan PDFium tidak.
  Sudah diperbaiki (0,00 persen sesudahnya), dan itulah gunanya harness ini ada.
- **Menyimpan belum ada.** Seluruh anotasi hidup di memori proses UI sampai tab
  ditutup. Menulisnya ke PDF, autosave, dan pemulihan crash adalah Fase 4 —
  jangan menganggap pekerjaan di fase ini aman sebelum itu.
- **Teks disunting lewat panel properti, bukan langsung di halaman.** Kotak teks
  yang baru dibuat kosong sampai diisi dari panel. Caret di atas halaman perlu
  penyuntingan teks di tempat dan belum dikerjakan.
- **Paritas kanvas tidak dijalankan CI.** Ia butuh Chromium dan PDFium
  sungguhan; test yang diam-diam dilewati di lingkungan yang justru penting
  lebih buruk daripada test yang harus diminta.
- **UI Fase 3 belum pernah dijalankan di jendela sungguhan.** Kontainer
  pengembangan tidak punya layar. Yang terbukti di sini adalah lapisan model,
  backend, dan paritasnya; interaksinya menunggu pengujian di Windows.

## [7.0.0-alpha.2] — Fase 2: Multi-Dokumen & Pencarian

Fase ini membuat aplikasi bisa memegang banyak dokumen sekaligus dan mencari di
dalamnya. Kriteria lulusnya soal memori, dan angkanya ada di
`bench/results/phase2-linux.txt` — termasuk satu hasil yang **tidak** seperti
yang diharapkan dan dicatat apa adanya.

### Ditambahkan

- **Tab** (SPEC 10). Satu store per dokumen (`src/state/documentSession.ts`)
  dengan daftar tab di store ruang kerja (`src/state/workspaceStore.ts`), dan
  registri padanannya di sisi Rust (`src-tauri/src/workspace.rs`). Zoom, rotasi,
  posisi baca, daftar isi, teks, dan hasil pencarian milik *dokumen*, bukan
  jendela — jadi berpindah tab mengembalikan persis keadaan yang ditinggalkan.
  Strip tab menyembunyikan diri saat hanya ada satu dokumen, bisa diseret untuk
  diurutkan ulang, dan tombol tengah menutup.
- **Manajemen memori tab tidak aktif.** Tiga tab terbaru menyimpan bitmapnya;
  selebihnya di-`Trim` — di proses UI cache ubinnya dibuang, di pekerja
  pegangan halamannya dilepas. Tiga, bukan satu: pembaca yang membandingkan dua
  dokumen membolak-balik keduanya tiap beberapa detik, dan menyusutkan tiap
  pindah berarti merender ulang keduanya setiap kali. Aturannya murni dan
  diuji (`tabsToTrim`).
- **Session restore** (SPEC 11.3). Susunan tab ditulis ke `sessions`/
  `session_tabs` tiap kali berubah — bukan saat keluar, karena kasus yang
  membuat fitur ini ada justru kasus aplikasi tidak ditutup baik-baik. Berkas
  yang sudah pindah dilewati diam-diam saat dipulihkan.
- **Pencarian tiga tingkat** (SPEC 11.1):
  - *Dokumen ini* — indeks FTS5 dokumen yang terbuka, menjawab "halaman mana".
  - *Semua dokumen* — seluruh berkas yang pernah diindeks; satu klik membuka
    berkasnya di tab.
  - *Halaman* — PDFium menjawab "di sebelah mana", dengan kotak sorot yang
    digambar di atas halaman. Dibatalkan per ketukan lewat generasi, sama
    seperti ubin.
- **Regex sebagai jalur terpisah**, ditandai di UI sebagai hanya berlaku untuk
  dokumen yang sedang terbuka, satu halaman pada satu waktu. Ini bukan
  keterbatasan implementasi yang bisa ditambal nanti: FTS5 mengindeks token, dan
  tidak ada teks berurutan di sana untuk dijalankan sebuah pola. SPEC 11.1
  melarang menjanjikannya setara dengan dua tingkat lainnya, dan panel
  mengatakannya dengan kalimat yang bisa dibaca pengguna.
- **Pengindeksan teks latar** (SPEC 7). Berjalan per halaman lewat pekerja
  (permintaan yang pergi mengekstrak 500 halaman akan dibunuh supervisor di
  detik keenam), dengan jeda 15 ms antar halaman supaya viewport tetap
  didahulukan, komit tiap 16 halaman, dapat dilanjutkan dari tempat berhenti,
  dan dibatalkan saat tabnya ditutup. Kemajuannya terlihat di panel pencarian,
  karena hasil kosong dari indeks yang baru separuh jadi tidak bisa dibedakan
  dari dokumen yang memang tidak memuat katanya.
- **Berkas terakhir bergambar** (SPEC 11.3, SPEC 12). Sampul halaman pertama
  direkam pada saat dokumen dibuka — satu-satunya saat ia memang sudah terbuka
  dan sudah dirender — lalu disimpan sebagai PNG di folder data. Berkas yang
  belum pernah dibuka di aplikasi ini tidak punya sampul dan mendapat kartu
  polos; membuatkannya berarti membuka tiap berkas di daftar, yang justru biaya
  yang tidak boleh dimiliki panel ini. Bisa disematkan.
- **Seret & lepas** berkas ke jendela, lewat kanal drag-drop milik Tauri dan
  bukan milik webview: DOM menyerahkan objek `File` tanpa path, sedangkan
  pekerja membuka berkas *lewat path*.
- **Asosiasi berkas Windows**: PDF yang diklik ganda diteruskan sebagai argumen
  baris perintah dan dibuka saat jendela siap. Argumen yang bukan `.pdf` yang
  benar-benar ada diabaikan diam-diam.
- **Satu instance saja.** Klik ganda PDF kedua saat aplikasi sudah berjalan
  **menambah tab di jendela yang ada**, bukan membuka jendela kedua. Tanpa ini,
  tiap berkas yang dibuka dari Explorer akan menjalankan satu kolam delapan
  proses pekerja, satu cache ubin, dan satu baris sesi sendiri — dan strip tab
  tidak akan pernah terisi.
- **Pintasan**: Ctrl+F membuka pencarian, F3/Shift+F3 melompat antar hasil,
  Ctrl+Tab berpindah tab, dan Ctrl+W kini menutup **tab**, bukan jendela.
- **Benchmark Fase 2** (`cargo run --release -p izul-bench --bin multidoc`):
  50 dokumen terbuka, biaya per dokumen, efek `Trim`, dan 200 siklus buka-tutup
  dengan kurva RSS tiap 25 siklus.

### Angka

Linux x64, 8 pekerja, pekerja rilis, PDFium 151.0.7881.0. Bawaan harness adalah
**30 dokumen** atas permintaan pemilik proyek; SPEC Bagian 17 menuliskan
kriterianya sebagai 50 dokumen, jadi angka itu tetap dijalankan dan dicantumkan
di sebelahnya alih-alih dihapus:

| Yang diukur | 30 dokumen (bawaan) | 50 dokumen (kriteria SPEC) |
|---|---|---|
| Buka + render semuanya | 80 ms | 133 ms |
| Resident set 8 pekerja | 47,4 -> 80,6 MB | 47,2 -> 94,0 MB |
| Rata-rata per dokumen | 1,10 MB | 0,94 MB |

Rata-rata per dokumen turun saat jumlahnya naik karena ongkos tetap tiap
pekerja dibagi ke lebih banyak dokumen, bukan karena dokumennya jadi lebih
murah.

- 200 siklus buka-tutup: **1,7 ms per siklus**; RSS **mendatar** setelah ~25
  siklus (11,93 MB -> 11,99 MB selama 175 siklus berikutnya). Pertambahan
  1,58 MB yang terlihat dari ujung ke ujung seluruhnya terjadi saat pemanasan
  alokator.

### Diketahui, dan tidak ditutup-tutupi

- **`Trim` tidak menurunkan RSS pekerja.** Ia melepas seluruh pegangan halaman —
  dibuktikan test, bukan diasumsikan — tetapi PDFium menyimpan arena alokatornya,
  jadi memori itu menjadi *dapat dipakai ulang*, bukan dikembalikan ke sistem.
  Di proses UI ceritanya berbeda: di sana `Trim` membuang bitmap dari cache
  ubin, dan itu megabyte yang benar-benar kembali.
- **Satu berkas, satu tab.** Membuka berkas yang sudah terbuka memindahkan fokus
  ke tabnya alih-alih membuat salinan kedua. Dua tab untuk satu berkas menunggu
  split view di Fase 5.
- **Test integrasi masih hanya Unix.** Tiga test ujung-ke-ujung Fase 2 (banyak
  dokumen sekaligus, pencarian dengan kotak sorot, pengindeksan sampai FTS5)
  ikut digerbangi `#![cfg(unix)]` seperti Fase 1, jadi Windows — platform yang
  dikirim — masih dijaga test unit dan pengujian manual saja.

## [7.0.0-alpha.1] — Fase 1: Mesin Viewer

Fase ini membuat aplikasi bisa dibaca: pipeline render lengkap, scroll
tervirtualisasi, seluruh mode tampilan, zoom, rotasi, lapisan teks, dan sidebar
thumbnail serta daftar isi. Kriteria lulusnya angka, dan angkanya ada di
`bench/results/phase1-linux.txt` — termasuk satu klaim yang **belum** bisa
dibuktikan dan dicatat apa adanya.

### Ditambahkan

- **Pipeline render dua tingkat** (SPEC 9). Tingkat pertama adalah pratinjau
  seluruh halaman beresolusi thumbnail; tingkat kedua adalah ubin 512x512 pada
  skala saat ini yang digambar menimpanya begitu siap. Pratinjau itu **juga**
  thumbnail sidebar — satu bitmap, satu URI, satu render, dipakai keduanya.
- **Cache ubin LRU** beranggaran byte (bawaan 2 GiB, SPEC 9), hidup di proses
  UI: ia selamat dari matinya pekerja, dipakai bersama oleh dokumen yang
  kebetulan berada di pekerja berbeda, dan kuncinya adalah isi ubin — bukan
  generasi — sehingga zoom bolak-balik tidak membuang apa pun.
- **Penjadwal** dengan prioritas (terlihat > pratinjau > prefetch),
  penggabungan permintaan (dua peminta satu ubin = satu render), dan pembatalan
  di dua tempat: di proses UI sebelum permintaan dikirim, dan di pekerja untuk
  pekerjaan yang sudah diterima.
- **Prefetch prediktif** dari arah dan kecepatan scroll, menjangkau lebih jauh
  saat scroll cepat dan simetris saat pengguna berhenti.
- **Scroll tervirtualisasi**: tata letak seluruh dokumen dihitung sekali per
  perubahan zoom/rotasi/mode, dan pertanyaan per frame — halaman mana yang
  terlihat — dijawab dengan pencarian biner atas baris. Dokumen 500 halaman dan
  5 halaman berbiaya sama per frame. Placeholder berukuran benar sejak frame
  pertama, jadi scrollbar tidak pernah melompat.
- **Mode tampilan** satu halaman, dua halaman, dua halaman dengan sampul, dan
  scroll mendatar.
- **Zoom** 10–1600 % dengan tangga langkah tetap, fit width, fit page, ukuran
  asli, Ctrl+scroll dan pinch touchpad, **mempertahankan titik fokus kursor**.
- **Rotasi per dokumen dan per halaman**, diterapkan di dalam matriks ubin.
  Rotasi bawaan halaman (`/Rotate`) ikut dihitung — oleh PDFium, yang menyusun
  matriks tampilan halaman sebelum menerapkan matriks kita, jadi matriks ubin
  hanya membawa rotasi tambahan dari pengguna. Lihat "Diperbaiki setelah
  pengujian pertama di Windows": menganggapnya sebaliknya membuat setiap ubin
  di atas zoom 100 % tergambar putih.
- **Lapisan teks** dari kotak karakter PDFium, dikelompokkan menjadi baris,
  transparan di atas kanvas — jadi seleksi, salin, dan pembaca layar bekerja
  pada teks dokumen, bukan pada gambarnya. Kotaknya dipetakan ke ruang tampilan,
  sehingga seleksi tetap mendarat di glif pada halaman yang diputar.
- **Sidebar** thumbnail (tervirtualisasi: 500 halaman tidak berarti 500 kanvas)
  dan daftar isi dari bookmark dokumen, termasuk tujuan lewat aksi GoTo dan
  penjagaan terhadap outline yang siklik.
- **Posisi baca diingat per berkas** — halaman, scroll, zoom, rotasi, dan mode
  tampilan — disimpan ke `app.db` saat scroll berhenti dan dipulihkan saat
  dokumen dibuka lagi.
- **Protokol ubin sekali jalan**: `izul://tile/{doc}/{page}/{rot}/{skala}/{col}/{row}/{tingkat}`
  dilayani secara asinkron, dan satu permintaan itu berarti "kena cache" atau
  "jadwalkan render lalu jawab". Sebelumnya butuh dua perjalanan: `invoke` untuk
  merender, `fetch` untuk mengambil.
- **Test**: 166 test Rust (dari 111) dan 63 test TypeScript (dari nol), termasuk
  8 test integrasi Fase 1 yang menjalankan pekerja sungguhan untuk membuktikan
  ubin, pratinjau, rotasi, daftar isi, kotak teks, dan pembatalan.
- **`bench/viewport`**, harness pengukuran Fase 1 yang mendorong pipeline
  sungguhan lewat pekerja sungguhan.

### Angka (Linux, 4 core, profil release)

Rinciannya di `bench/results/phase1-linux.txt`; `mixed-500p.pdf` adalah berkas
50 MB / 500 halaman yang disebut SPEC 13.

| Metrik | Target | text-500p | mixed-500p |
|---|---|---|---|
| Buka → pratinjau halaman 1 (p95) | < 400 ms | 9,1 ms | 18,0 ms |
| Buka → layar pertama tajam (p95) | < 400 ms | 22,1 ms | 35,4 ms |
| Zoom → ada yang bisa digambar (p95) | < 16 ms | 0,0 ms | 0,0 ms |
| Zoom → versi tajam (p95) | < 150 ms | 25,6 ms | 16,9 ms |
| Layar penuh, cache panas (p95) | < 16 ms | 2,2 ms | 1,4 ms |
| RAM 10 dokumen (UI + pekerja) | < 1,5 GB | — | 428 MB |

Scroll 2500 px/detik selama 4 detik: satu frame dari 240 tanpa apa pun untuk
digambar (frame pertama, sebelum pratinjau pertama tiba), 21 frame menampilkan
pratinjau alih-alih ubin tajam, sisanya tajam.

### Belum terbukti

SPEC 13 meminta scroll "mengunci di refresh rate, nol frame drop". **Itu belum
dibuktikan.** Webview tidak bisa berjalan tanpa layar, jadi tidak ada kompositor
untuk diukur di sini. Yang bisa diukur sudah diukur — pada tiap frame 60 fps,
apakah pipeline punya sesuatu untuk digambar dan apakah versi tajamnya siap —
dan hasilnya ada di atas. Klaim frame pacing yang sesungguhnya harus diukur di
Windows dengan jendela nyata, dan sampai itu terjadi ia tetap ditulis sebagai
belum terbukti, bukan sebagai lulus.

Begitu pula **cold start < 1 detik**: yang terukur di sini hanya lantainya —
spawn pekerja, kanal, muat PDFium, ping pertama: 2,6 ms p95 di Linux dengan page
cache panas. Jendela dan frame pertama adalah sisa anggaran itu dan milik
pengukuran di Windows.

### Diperbaiki setelah pengujian pertama di Windows

Fase 1 dinyatakan selesai berdasarkan test dan benchmark di Linux. Menjalankan
build sungguhan di Windows menemukan tujuh cacat yang tak satu pun bisa
tertangkap oleh suite yang ada, karena semuanya hidup di lapisan yang tidak
dilewati test mana pun: jendela, webview, dan pompa kanal supervisor. Semuanya
kini punya test regresi yang **gagal pada kode lama**.

- **Tombol "Buka Berkas" tidak merespons.** Tauri v2 menolak permintaan plugin
  dialog secara diam-diam tanpa `src-tauri/capabilities/default.json`. Berkas
  itu ditambahkan.
- **URI ubin tidak pernah sampai ke Rust.** `wry` menerjemahkan
  `{http|https}://izul.localhost/x` kembali ke `izul://x`, tapi hanya untuk
  skema yang benar-benar dipakai jendela — `http` secara bawaan, karena proyek
  ini tidak menyalakan `useHttpsScheme`. Bentuk `https` meleset dari penerjemah
  itu dan jatuh sebagai pencarian DNS ke host yang tidak ada. Frontend sekarang
  menulis `http://izul.localhost/...`; pengurainya tetap menerima ketiga bentuk.
- **Pekerja diam lalu dibunuh, berulang-ulang.** `pump()` menaruh `read_frame`
  di dalam `tokio::select!`, dan `read_frame` tidak cancel-safe: ia membaca
  4 byte panjang lalu isinya, jadi perintah yang datang di antara keduanya
  membuang future itu berikut byte panjang yang sudah terbaca. Pembacaan
  berikutnya mulai dari tengah pesan dan menunggu selamanya. Pembacaan
  dipindah ke task tersendiri yang tidak pernah dibatalkan, menyalurkan frame
  utuh lewat `mpsc`.
- **`Ping` terbaca sebagai `Shutdown`.** `postcard` mengirim enum sebagai indeks
  varian, dan Fase 1 menyisipkan dua varian `Request` di tengah daftar, jadi
  binari pekerja lama menggeser semuanya. `npm run dev`/`build` sekarang
  membangun pekerja lebih dulu, pekerja mengirim salam `Response::Hello
  { protocol }`, dan supervisor menolak protokol yang tidak cocok dengan pesan
  yang menyuruh `cargo build --workspace`.
- **Balasan ubin dibuang browser.** Halaman dev berjalan di `localhost:5173`,
  ubin datang dari `izul.localhost` — lintas asal. Protokol yang didaftarkan
  tangan harus memasang `Access-Control-Allow-Origin` sendiri, dan tanpa
  `Access-Control-Expose-Headers` dimensi ubin terbaca `null` walau ubinnya
  sampai utuh. Keduanya kini dipasang pada **semua** balasan, termasuk yang
  galat, jadi status 409/410 pun sampai ke frontend.
- **Denyut jantung membunuh pekerja yang belum pernah disapa.** `sweep()`
  memeriksa lama diam sebelum mencoba ping, padahal diam hanya berarti "belum
  diajak bicara" — dan penyebabnya adalah supervisor sendiri, yang butuh ~2
  detik per pekerja untuk menghidupkan delapan. Sekarang ping dulu, vonis mati
  hanya bila ping gagal **dan** sudah lewat ambang; ping ke semua pekerja
  berjalan serentak agar satu pekerja macet tidak menahan kunci kolam.
- **Setiap ubin di atas zoom 100 % tergambar putih.** `FPDF_RenderPageBitmapWithMatrix`
  tidak menerima matriks ruang-pengguna: PDFium menyusun matriks tampilan
  halaman itu sendiri lebih dulu, lalu menerapkan matriks kita di atasnya.
  Matriks ubin lama membalik sumbu y, mengurangi kotak pembatas, dan memutar
  `/Rotate` untuk kedua kalinya. Pada zoom 100 % kebetulan masih ada yang
  mendarat di bitmap; begitu faktor skalanya meninggalkan 1,0 seluruh isi
  terdorong ke luar area klip. Matriksnya ditulis ulang di ruang yang benar —
  titik, origin kiri-atas, `/MediaBox` dan `/Rotate` sudah ditangani PDFium —
  sehingga yang tersisa hanyalah rotasi tambahan dari pengguna, offset rect
  sumber, dan skala. Header PDFium tidak menjelaskan ruang ini sama sekali, jadi
  ia dipatok dengan pengukuran: render bermatriks identitas kini wajib identik
  byte-per-byte dengan `FPDF_RenderPageBitmap`, dan halaman sintetis dengan
  `/MediaBox` bergeser serta `/Rotate` 90/180/270 memastikan tidak ada yang
  diterapkan dua kali.

- **`localhost` sisa penerjemahan `wry` ditolak sebagai jenis sumber daya
  tak dikenal — seluruh ubin ditolak di Windows, terbukti dari log lalu
  lintas ubin baru: 1400 permintaan, 0 terkirim.** `wry` menerjemahkan
  `http://izul.localhost/x` menjadi `izul://localhost/x`, bukan
  `izul://x` — kode sumbernya sendiri menyebut bentuk kanoniknya
  `{protocol}://localhost/abc`. Pengurai URI kita mengasumsikan tidak ada
  authority dan membaca `localhost` sebagai jenis sumber daya. Diperbaiki:
  authority dilucuti tepat di awal path, sebelum dibaca, dan hanya di
  posisi itu.
- **Log lalu lintas ubin.** Sebelumnya penolakan dicatat di `debug!` (di
  bawah saringan bawaan) atau tidak dicatat sama sekali, sehingga log
  terlihat sama baik saat viewport tidak meminta ubin maupun saat backend
  menolak semuanya. Sekarang tiap permintaan dihitung menurut hasilnya,
  dengan ringkasan berkala supaya scroll ribuan ubin tidak membanjiri log.

### Diputuskan selama Fase 1

- **Pipeline render menjadi crate sendiri, `izul-render`** — deviasi dari daftar
  crate di SPEC 5, dan disengaja. Di sinilah target SPEC 13 dipenuhi atau
  gagal, dan fase yang harus *membuktikannya* dengan benchmark perlu mendorong
  cache, antrean prioritas, dan pembatalan yang sebenarnya dari sebuah harness —
  bukan dari dalam aplikasi berjendela. Menaruhnya di `src-tauri` akan membuat
  benchmark jadi implementasi ulang dari hal yang diukurnya. Crate ini tidak
  tahu apa-apa tentang Tauri; ia mencapai pekerja lewat satu trait.
- **Ubin dialamatkan berdasarkan isi, bukan slot.** URI ubin menyebut dokumen,
  halaman, rotasi, skala, dan posisi grid, sehingga URI yang sama adalah kunci
  cache di webview, di backend, dan di log. Satu perjalanan menggantikan dua.
- **Generasi bukan bagian dari identitas ubin.** Ia mengatur penjadwalan, bukan
  isi. Memasukkannya ke kunci cache akan membuang seluruh cache tiap kali roda
  zoom diputar — persis saat cache paling berharga.
- **Pratinjau kebal terhadap pembatalan.** Ia tidak bergantung pada zoom maupun
  scroll, dan ia yang mencegah halaman putih; membuangnya karena pengguna masih
  menggulir akan meniadakan gunanya.
- **Satu permintaan menunggak per pekerja.** Pekerja merender di satu thread,
  jadi permintaan kedua hanya akan mengantre di dalamnya, di tempat penjadwal
  tidak bisa lagi mengubah prioritasnya.
- **Satu balasan untuk satu permintaan.** Protokol Fase 0 mengalirkan banyak
  balasan di bawah satu id untuk sapuan thumbnail; tabel balasan supervisor
  tidak bisa merutekannya dan diam-diam membuangnya. Fase 1 menariknya
  per halaman.
- **CSP diperbaiki.** `connect-src` Fase 0 tidak mengizinkan `izul:`, sehingga
  setiap pengambilan ubin akan diblokir di jendela sungguhan. Ditemukan saat
  membangun jalur ini, diperbaiki di sini.

### Ditinjau dari v6.2

Kode v6.2 di branch `v6.2-reference` dibaca pada fase ini (`app.js`, `core.js`,
`annots.js`). Perbandingan perilaku viewer-nya ada di `TESTING.md`. Dua hal
yang perlu dicatat:

- Repositori v6.2 **tidak berisi berkas test**. 21 test case yang disebut
  SPEC Bagian 16 tampaknya daftar pemeriksaan manual, bukan suite otomatis;
  membawanya ke v7 berarti menuliskannya ulang dari perilaku yang terbaca di
  kode, dan itu jatuh di Fase 3 dan 4 bersama anotasi dan simpan.
- v6.2 menyusun **daftar isi cadangan** dengan mendeteksi judul bab dari teks
  (BAB, BAGIAN, DAFTAR PUSTAKA, dan seterusnya) ketika PDF tidak membawa
  outline. v7 belum punya padanannya. Ia butuh sapuan teks seluruh dokumen —
  yang dibangun di Fase 2 bersama indeks FTS5 — jadi menambahkannya di sana
  nyaris tanpa biaya, dan menambahkannya di sini berarti membangun sapuan teks
  dua kali.

### Belum ada

Sengaja belum ada, dan bukan pekerjaan yang tertinggal: tab dan multi-dokumen,
split view, pencarian, seluruh mesin anotasi, penulisan, operasi halaman, mode
presentasi, dark mode dengan invert cerdas, dan command palette. Ekstraksi teks
sudah ada sebagai lapisan seleksi; indeks FTS5 dan pencarian tiga tingkat adalah
Fase 2.

[7.0.0-alpha.1]: https://github.com/Zulaziz18/PDF-VIEWER-IZUL/tree/claude/pdf-studio-izul-v7-fase-1-tki5pi

## [7.0.0-alpha.0] — Fase 0: Fondasi, Batas Proses, dan Spike Pengukuran

Fase fondasi. Tujuannya bukan fitur, melainkan menjawab satu pertanyaan sebelum
apa pun dibangun di atasnya: apakah PDFium sanggup memenuhi target Bagian 13,
dan apakah batas proses benar-benar melindungi UI.

### Ditambahkan

- **Workspace Rust** dengan lima crate produksi sesuai SPEC Bagian 5:
  `izul-model` (murni, tanpa PDFium dan tanpa I/O), `izul-pdf`, `izul-ipc`,
  `izul-store`, `izul-worker`, ditambah `izul-app` (proses UI) dan `izul-bench`.
- **Integrasi PDFium** dipatok ke `chromium/7881` (PDFium 151.0.7881.0), diambil
  oleh `vendor/pdfium/fetch.sh` untuk `win-x64` dan `linux-x64`.
- **Display list** (SPEC 3.2): kosakata `DisplayOp` lengkap dengan pemeriksaan
  keseimbangan push/pop dan serialisasi deterministik. Fungsi
  `display_list(obj, font_ctx)` sendiri adalah Fase 3 dan sengaja **tidak**
  di-stub.
- **Protokol IPC dua kanal** (SPEC 6): kanal perintah lewat named pipe (Windows)
  atau soket domain Unix, berbingkai panjang dengan `postcard`; kanal piksel
  lewat ring memori bersama berisi slot ubin 512×512 berukuran tetap.
- **Kolam pekerja tersandbox** (SPEC 3.4): `min(core/2, 8)` minimal 2, heartbeat
  2 detik, hang pada 6 detik, restart dengan backoff eksponensial berplafon,
  dan daftar dokumen beracun dua-strike.
- **Job Object Windows** dengan batas memori per proses dan per job,
  `KILL_ON_JOB_CLOSE`, dan pembatasan UI. Di Unix, `RLIMIT_AS` sebagai padanan
  pengembangan — bukan batas keamanan, dan dilaporkan apa adanya di status bar.
- **Skema SQLite** (SPEC 7): `app.db` dan `cache.db` terpisah, keduanya WAL,
  dengan FTS5 eksternal-konten berikut trigger sinkronisasinya.
- **Shell Tauri v2** dengan protokol URI `izul://tile/{doc}/{slot}/{epoch}` yang
  mengalirkan piksel langsung dari memori bersama — nol base64, nol `invoke`
  untuk bitmap.
- **Frontend** TypeScript `strict` + `noUncheckedIndexedAccess`, React 19,
  viewport kanvas imperatif tanpa React di dalamnya, seluruh string di modul
  i18n.
- **CI** menegakkan `cargo fmt`, `clippy -D warnings` dengan `unwrap`/`expect`/
  `panic` ditolak di kode produksi, `tsc`, ESLint, dan konsistensi
  `version.json` di seluruh manifest.

### Diputuskan selama Fase 0

Empat keputusan diambil karena pengukuran, bukan karena selera. Rinciannya ada
di `bench/results/phase0-linux-full.txt`.

- **Berkas dipetakan, bukan dibaca.** Sepuluh dokumen berat sebagai salinan
  penuh memakan 811–863 MB RSS; dengan `mmap` memakan 4–62 MB. Membuka juga
  5,6× lebih cepat. Target RAM Bagian 13 tidak tercapai tanpa perubahan ini.
- **Ukuran halaman dibaca dari pohon halaman.** `FPDF_GetPageSizeByIndexF`
  menyelesaikan 500 halaman dalam ~6 ms p95; memuat tiap halaman butuh ~495 ms.
  Scrollbar tidak bisa punya ukuran benar sebelum ini selesai.
- **Semua render berbasis ubin** — deviasi dari SPEC Bagian 9, yang menulis
  tile hanya di atas 200% zoom. Merender satu halaman penuh sebagai ubin
  512×512 berbiaya 0,79–1,00× dibanding sekali render, karena clipping membuat
  PDFium melewatkan pekerjaan di luar ubin: di bawah 200% pun ubin tidak lebih
  mahal, dan jalur piksel jadi satu bentuk (slot shm berukuran tetap) untuk
  seluruh rentang zoom, bukan dua jalur (halaman utuh vs ubin) yang berbeda
  kode dan berbeda perilaku pembatalannya. **Disetujui 2026-09-04.**
- **Handle PDFium dikelola sendiri.** Pembungkus aman `pdfium-render` tidak
  mengekspos `FPDF_RenderPageBitmapWithMatrix` maupun `FPDFBitmap_CreateEx`,
  sehingga ubin dan jalur nol-salinan mustahil lewat sana. Crate itu tetap
  dipakai untuk binding dinamis dan deklarasi ABI berversinya.

### Belum ada

Fase 0 tidak menjanjikan apa pun di luar daftar di atas. Yang berikut ini
sengaja belum ada, dan bukan pekerjaan yang tertinggal:
scroll tervirtualisasi, cache LRU, prefetch, tab, split view, pencarian,
seluruh mesin anotasi, penulisan, dan operasi halaman.

[7.0.0-alpha.0]: https://github.com/Zulaziz18/PDF-VIEWER-IZUL/tree/claude/pdf-studio-izul-v7-atlas-r29mdh
