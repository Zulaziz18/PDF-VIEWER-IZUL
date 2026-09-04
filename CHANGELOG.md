# Changelog

Semua perubahan penting per fase. Format mengikuti [Keep a Changelog](https://keepachangelog.com/id/1.1.0/);
versi mengikuti `version.json` sebagai sumber tunggal.

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
