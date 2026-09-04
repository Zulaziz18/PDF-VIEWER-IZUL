# Catatan untuk Claude — PDF Studio Izul v7

## Tentang pengguna

Pengguna proyek ini (Muhammad Izul) adalah **pemula** dalam hal development —
belum familiar dengan command line, git, atau proses build. Instruksi harus:

- Langkah demi langkah, urut, satu perintah per baris siap salin-tempel.
- Jelaskan istilah teknis singkat saat pertama disebut.
- Jangan asumsikan ia tahu perbedaan PowerShell biasa vs Administrator, atau
  kenapa restart komputer kadang diperlukan.
- Kalau ada error, minta beberapa baris terakhir dari pesan errornya sebelum
  menebak penyebabnya.

## Lingkungan development pengguna

- **OS:** Windows 11 Home Single Language, versi 25H2 (OS Build 26200.9168).
- **Direktori proyek di komputernya:** `C:\Users\muham\PDF-VIEWER-IZUL`
  (bukan di Documents — sudah di-clone langsung ke bawah folder user).
- **winget tidak tersedia** di komputernya (App Installer tidak muncul di
  Microsoft Store, tidak bisa dicari). Karena itu kelima alat berikut dipasang
  **manual lewat installer resmi dari browser**, bukan lewat winget:
  1. Git (git-scm.com)
  2. Node.js LTS (nodejs.org)
  3. Rust via rustup (rustup.rs)
  4. Microsoft Edge WebView2 Runtime (Evergreen Bootstrapper)
  5. Visual C++ Build Tools dengan workload "Desktop development with C++"
     (visualstudio.microsoft.com/downloads)
- **Status terakhir diketahui:**
  - Alat dasar terpasang dan terverifikasi: `git 2.55.0`, `node v24.20.0`,
    `cargo 1.98.1`.
  - Repo di `C:\Users\muham\PDF-VIEWER-IZUL` sudah di branch
    `claude/pdf-studio-izul-v7-fase-1-tki5pi` dan sudah `git pull` (sebelumnya
    sempat nyangkut di branch `atlas-r29mdh` dengan perubahan kosmetik
    LF/CRLF di `src-tauri/Cargo.toml` — sudah dibuang lewat `git restore`,
    aman, bukan isi sungguhan).
  - PDFium sudah ada di `vendor/pdfium/win-x64`, terverifikasi
    `MAJOR=151 MINOR=0 BUILD=7881 PATCH=0` — cocok dengan yang dipatok SPEC.
  - `npm ci` berhasil: 179 paket terpasang, 0 kerentanan. Ada peringatan
    `esbuild@0.28.2` soal install-scripts belum di-allowlist — ini normal,
    bukan error, tidak menghalangi apa pun.
  - **Aplikasi berhasil dijalankan** dengan `npm run tauri dev`, jendela
    terbuka, status bar menunjukkan "Pekerja 8/8". Sempat ada beberapa
    pekerja "dimatikan karena diam terlalu lama" lalu pulih sendiri saat
    startup pertama — kemungkinan besar cuma build `dev` (belum optimal)
    lambat memuat PDFium di 8 proses sekaligus; belum jadi masalah kalau
    tidak berulang terus-menerus.
  - **Dua bug ditemukan dan sudah diperbaiki** lewat pengujian langsung di
    Windows-nya (keduanya baru ketahuan sekarang karena sebelumnya belum ada
    yang menjalankan build sungguhan di Windows):
    1. Tombol "Buka Berkas" tidak merespons sama sekali — Tauri v2 butuh
       berkas `src-tauri/capabilities/default.json` eksplisit untuk plugin
       dialog, kalau tidak ada permintaan dialog ditolak diam-diam. Sudah
       ditambahkan.
    2. Setelah tombol diperbaiki dan PDF berhasil dibuka (diuji dengan PDF
       811 halaman), **semua halaman tampil putih kosong**. Percobaan
       perbaikan pertama (menulis URI ubin sebagai `https://izul.localhost/...`)
       **tidak cukup** — DevTools pengguna menunjukkan `TypeError: Failed to
       fetch` dengan "Response headers (0)", yaitu permintaan gagal total
       sebelum dapat balasan apa pun, bukan galat 400/404. Duduk perkaranya
       baru ketemu setelah membaca langsung kode sumber `wry` (mesin WebView2
       Tauri) yang terpasang di proyek: `wry` menerjemahkan
       `{http_or_https}://izul.localhost/x` kembali ke `izul://x` sebelum
       kode Rust melihatnya, tapi **hanya untuk skema yang benar-benar
       dipakai jendela itu** — `http` secara bawaan, kecuali opsi
       `useHttpsScheme` diaktifkan di `tauri.conf.json` (proyek ini tidak
       mengaktifkannya). Menulis `https://` meleset dari penerjemah itu dan
       jatuh sebagai pencarian DNS sungguhan ke host yang tidak ada — makanya
       gagal instan tanpa balasan. Perbaikan final: URI ubin ditulis sebagai
       `http://izul.localhost/...` (bukan `https://`). Kode Rust pengurainya
       tetap menerima ketiga bentuk (`izul://`, `http://izul.localhost/`,
       `https://izul.localhost/`) untuk jaga-jaga, dengan `http` di urutan
       pertama karena itu yang sungguhan dipakai.
  - **Langkah berikutnya:** pengguna perlu `git pull` lagi lalu jalankan
    ulang `npm run tauri dev`, buka PDF yang sama, dan pastikan halamannya
    kini benar-benar tergambar (bukan putih kosong). Kalau masih gagal,
    minta pengguna cek tab Network DevTools lagi dengan cara yang sama —
    kali ini diharapkan status seharusnya 200, bukan kosong/pending.

## Alur kerja proyek ini

- Branch aktif pengguna: `claude/pdf-studio-izul-v7-fase-1-tki5pi`.
- Dokumen rujukan: `SPEC.md` (jangan diubah tanpa dibahas). Progres per fase
  dicatat di `CHANGELOG.md`. Panduan pengguna di `PANDUAN.md`.
- Setiap akhir fase: laporkan hasil + angka benchmark nyata, tunggu
  persetujuan pengguna sebelum lanjut ke fase berikutnya (lihat SPEC.md
  Bagian 0 dan 18).
- Total 9 fase (0–8). Fase 0 dan 1 sudah selesai dan disetujui pengguna.
- Panduan menjalankan & menguji aplikasi di Windows (untuk pemula) ada di
  `TESTING.md`, bagian "Menjalankan sendiri di Windows (langkah demi
  langkah)" — termasuk cara memasang alat, mengambil PDFium, menjalankan
  `npm run tauri dev`, dan melihat frame rate lewat DevTools.
