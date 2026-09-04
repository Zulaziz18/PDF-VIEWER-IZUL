# PDF Studio Izul v7 "Atlas"

Editor dan pembaca PDF offline untuk Windows. Satu mesin PDF untuk render dan
tulis, anotasi sebagai objek hidup, dan nol koneksi jaringan.

Dokumen rujukan proyek adalah [`SPEC.md`](SPEC.md). Berkas ini hanya menjelaskan
cara membangun.

**Status: Fase 0 selesai.** Fondasi, batas proses, dan spike pengukuran. Belum
ada viewer yang bisa dipakai membaca — lihat [`CHANGELOG.md`](CHANGELOG.md)
untuk apa yang ada dan apa yang belum.

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
| `crates/izul-pdf` | Pembungkus PDFium: buka, render, ekstrak teks. |
| `crates/izul-ipc` | Pesan, framing, memori bersama. Dipakai kedua sisi. |
| `crates/izul-worker` | Binari proses pekerja tersandbox. |
| `crates/izul-store` | SQLite: sesi, recent, indeks, cache. |
| `src-tauri` | Proses UI: jendela, supervisor, protokol `izul://`. |
| `src/viewport` | Kanvas imperatif. **Tanpa React** (ditegakkan ESLint). |
| `bench` | Harness pengukuran dan pembangkit berkas uji. |

`izul-model` tidak boleh menyentuh PDFium atau berkas. Aturan itu bukan gaya:
display list adalah yang menjamin tampilan layar dan hasil ekspor sama, dan
jaminan itu hanya berarti bila bisa diuji tanpa merender apa pun.

## Menjalankan pengukuran

```bash
python3 bench/make_fixtures.py test-fixtures   # ~10 menit
cargo build --release -p izul-bench
sudo ./target/release/spike --json bench/results/hasil.json
```

`sudo` diperlukan agar page cache bisa dibuang sebelum pengukuran "cold";
tanpa itu angka cold akan menyerupai angka warm.

## Lisensi pihak ketiga

PDFium: BSD-3-Clause (`vendor/pdfium/*/LICENSE`). Tauri, React, dan seluruh
crate: MIT atau Apache-2.0. Tidak ada beban copyleft.
