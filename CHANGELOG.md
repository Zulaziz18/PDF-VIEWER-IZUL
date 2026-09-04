# Changelog

Semua perubahan penting per fase. Format mengikuti [Keep a Changelog](https://keepachangelog.com/id/1.1.0/);
versi mengikuti `version.json` sebagai sumber tunggal.

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
  Rotasi bawaan halaman (`/Rotate`) ikut dihitung: PDFium menerapkannya sendiri
  pada render biasa tapi **tidak** pada render bermatriks, dan render bermatriks
  itulah yang membuat ubin mungkin.
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
