# PDF Studio Izul v7 "Atlas"

Editor dan pembaca PDF offline untuk Windows. Satu mesin PDF untuk render dan
tulis, anotasi sebagai objek hidup, dan nol koneksi jaringan.

Dokumen rujukan proyek adalah [`SPEC.md`](SPEC.md). Berkas ini hanya menjelaskan
cara membangun.

**Status: Fase 1 selesai.** Mesin viewer: pipeline render dua tingkat dengan
ubin, cache, prefetch dan pembatalan; scroll tervirtualisasi; seluruh mode
tampilan, zoom dan rotasi; lapisan teks; sidebar thumbnail dan daftar isi.
Belum ada tab, pencarian, maupun anotasi — lihat [`CHANGELOG.md`](CHANGELOG.md)
untuk apa yang ada, apa yang belum, dan angka pengukurannya.

## Membangun

```bash
./vendor/pdfium/fetch.sh win-x64 linux-x64   # PDFium terpatok chromium/7881
npm ci
cargo build --workspace
npm run tauri dev                            # atau: npm run tauri build
```

PDFium tidak masuk repositori; `fetch.sh` mengambil versi yang dipatok. Versi itu
harus sejalan dengan fitur `pdfium_7881` di `Cargo.toml` — `pdfium-render`
membangkitkan binding per revisi ABI, dan ketidakcocokan muncul sebagai simbol
hilang saat pustaka dimuat.

## Tata letak

| Direktori | Isi |
|---|---|
| `crates/izul-model` | Objek anotasi dan display list. Murni: tanpa PDFium, tanpa I/O. |
| `crates/izul-pdf` | Pembungkus PDFium: buka, render, ekstrak teks, daftar isi. |
| `crates/izul-render` | Cache ubin, prioritas, penggabungan, pembatalan. Tanpa Tauri. |
| `crates/izul-ipc` | Pesan, framing, memori bersama. Dipakai kedua sisi. |
| `crates/izul-worker` | Binari proses pekerja tersandbox. |
| `crates/izul-store` | SQLite: sesi, recent, indeks, cache. |
| `src-tauri` | Proses UI: jendela, supervisor, protokol `izul://`. |
| `src/viewport` | Kanvas imperatif, tata letak, lapisan teks. **Tanpa React** (ditegakkan ESLint). |
| `bench` | Harness pengukuran dan pembangkit berkas uji. |

`izul-model` tidak boleh menyentuh PDFium atau berkas. Aturan itu bukan gaya:
display list adalah yang menjamin tampilan layar dan hasil ekspor sama, dan
jaminan itu hanya berarti bila bisa diuji tanpa merender apa pun.

## Menjalankan pengukuran

```bash
python3 bench/make_fixtures.py test-fixtures   # ~10 menit
cargo build --release
sudo ./target/release/spike --json bench/results/hasil.json      # Fase 0: PDFium
./target/release/viewport --json bench/results/hasil-viewer.json # Fase 1: pipeline
```

`sudo` diperlukan pada `spike` agar page cache bisa dibuang sebelum pengukuran
"cold"; tanpa itu angka cold akan menyerupai angka warm.

`viewport` mendorong pipeline render yang sebenarnya lewat proses pekerja
sungguhan. Yang tidak bisa diukurnya — frame pacing, yang butuh kompositor —
dicatat sebagai belum terbukti, bukan dilewatkan diam-diam.

## Lisensi pihak ketiga

PDFium: BSD-3-Clause (`vendor/pdfium/*/LICENSE`). Tauri, React, dan seluruh
crate: MIT atau Apache-2.0. Tidak ada beban copyleft.

Satu pengecualian yang wajib disebut: `jpeg-encoder` (dipakai redaksi untuk
menulis ulang gambar JPEG) berlisensi (MIT atau Apache-2.0) **dan IJG**.
Lisensi IJG meminta dokumentasi program menyatakan: *perangkat lunak ini
sebagian didasarkan pada karya Independent JPEG Group* (this software is based
in part on the work of the Independent JPEG Group). Kalimat itu juga ada di
kotak Tentang aplikasi.

Fase 7 membawa komponen dengan lisensinya sendiri, **belum dibundel ke
installer** (itu Fase 8, sesudah keputusan pengguna):

| Komponen | Lisensi | Catatan |
|---|---|---|
| `ocrs` + `rten` (mesin OCR) | MIT / Apache-2.0 | kode, ditautkan ke pekerja |
| Model OCR `ocrs` (deteksi + pengenalan) | **belum jelas** | dilatih pada HierText (CC BY-SA 4.0); repositori modelnya tanpa berkas LICENSE. Hanya diambil `vendor/ocrs/fetch.sh` untuk pengembangan |
| ONNX Runtime 1.24.4 | MIT (Microsoft) | dimuat saat berjalan |
| `DirectML.dll` | lisensi redistribusi Microsoft | `vendor/onnx/*/ThirdPartyNotices.txt` |
| Model `u2netp` (U²-Net kecil) | Apache-2.0 | Xuebin Qin dkk., rilis rembg |
