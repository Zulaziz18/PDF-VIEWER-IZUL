# TESTING.md

Cara menjalankan suite, dan hal-hal visual yang hanya bisa dinilai mata.

## Prasyarat

```bash
./vendor/pdfium/fetch.sh linux-x64 win-x64   # PDFium terpatok chromium/7881
python3 -m pip install reportlab pikepdf pypdf pillow
python3 bench/make_fixtures.py test-fixtures # ~10 menit, ~470 MB
npm ci
```

Berkas uji tidak masuk repositori: ukurannya ratusan megabita dan seluruhnya
dapat dibuat ulang secara deterministik dari seed tetap di `bench/make_fixtures.py`.

## Suite otomatis

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run typecheck && npm run lint && npm test && npm run build
```

Agar test integrasi tidak diam-diam dilewati ketika prasyaratnya hilang:

```bash
IZUL_REQUIRE_FIXTURES=1 cargo test --workspace
```

Tanpa variabel itu, test yang kekurangan fixture atau pustaka akan mencetak
`LEWATI: ...` dan lulus — nyaman untuk klon baru, berbahaya untuk CI. CI
menyetelnya.

Test integrasi (`crash_isolation`) membaca PDFium langsung dari
`vendor/pdfium/`, dan `src-tauri/build.rs` menyalin PDFium ke samping binari
aplikasi secara otomatis pada setiap `cargo build`. Tidak ada langkah salin
manual yang diperlukan di kedua kasus — cukup `vendor/pdfium/fetch.sh` di atas.

## Hasil Fase 1

| Suite | Jumlah | Status |
|---|---|---|
| `izul-model` (geometri, display list) | 19 | lulus |
| `izul-ipc` (shm, ring, codec, transport) | 21 | lulus |
| `izul-store` (skema, migrasi, identitas, preferensi) | 24 | lulus |
| `izul-pdf` (matriks ubin & rotasi, ruang tampilan) | 14 | lulus |
| `izul-render` (cache LRU, prioritas, penggabungan, pembatalan) | 29 | lulus |
| `izul-worker` (epoch pembatalan, klasifikasi galat) | 7 | lulus |
| `izul-app` (kolam, racun, sandbox, protokol ubin, versi) | 37 | lulus |
| `crash_isolation` (proses pekerja nyata) | 7 | lulus |
| `render_pipeline` (proses pekerja nyata, Fase 1) | 8 | lulus |
| **Total Rust** | **166** | **lulus** |
| `src/viewport` (geometri, tata letak, prediksi, teks, cache bitmap, URI) | 63 | lulus |
| **Total** | **229** | **lulus** |

Suite frontend dijalankan dengan `npm test` (vitest). Yang diuji adalah modul
murni: konversi koordinat, tata letak dokumen dan kueri visibilitas, prediksi
scroll, pengelompokan karakter menjadi baris, cache bitmap, dan bentuk URI ubin.
Komponen React tidak diuji di sini — yang bisa salah pada mereka adalah hal
visual, dan itu ada di checklist manual.

Grid ubin didefinisikan di dua tempat — `izul-render` dan
`src/viewport/geometry.ts` — dan keduanya punya test yang menegaskan angka yang
sama. Perbedaan satu piksel di antara keduanya akan tampil sebagai garis rambut
di tiap batas ubin.

## Hasil Fase 0 (rujukan)

| Suite | Jumlah | Status |
|---|---|---|
| Seluruh suite Fase 0 | 111 | lulus |

## Benchmark

```bash
cargo build --release -p izul-bench
./target/release/spike --json bench/results/hasil.json
./target/release/spike --only first-page,render        # sebagian saja
```

Bagian `open` dan `first-page` membuang page cache sistem sebelum tiap
pengukuran, jadi angka "cold" hanya benar bila dijalankan sebagai root. Tanpa
hak itu angka cold akan menyerupai angka warm — periksa selisihnya sebelum
mengutip.

Hasil Fase 0 ada di `bench/results/phase0-linux-full.txt` (tabel) dan
`.json` (mentah).

### Fase 1

```bash
cargo build --release          # izul-worker dan viewport
./target/release/viewport --json bench/results/phase1-linux.json
./target/release/viewport --only scroll,zoom       # sebagian saja
```

`viewport` mendorong pipeline yang sebenarnya — pekerja tersandbox, ring memori
bersama, cache ubin, antrean prioritas, pembatalan — lewat satu proses pekerja
sungguhan. Ia butuh `test-fixtures/text-500p.pdf` dan `mixed-500p.pdf`.

Hasilnya di `bench/results/phase1-linux.txt`, berikut catatan tentang apa yang
**tidak** diukurnya: tanpa webview tidak ada kompositor, jadi klaim SPEC 13
"mengunci di refresh rate" belum terbukti dan ditulis begitu.

## Regresi dari v6.2

v6.2 punya 21 test case yang seluruhnya lulus. **Belum satupun dibawa ke v7.**
Alasannya bukan kelalaian: repositori ini kosong saat Fase 0 dimulai, sehingga
tidak ada kode maupun daftar test v6.2 yang bisa dibaca. Begitu v6.2 tersedia,
21 kasus itu masuk sebagai suite regresi sebelum Fase 3 dinyatakan selesai —
paritas anotasi tidak bisa dibuktikan tanpa keduanya.

## Checklist manual

Hal-hal yang tidak bisa dinilai selain dengan melihat. Dijalankan tiap akhir
fase, pada Windows dengan skala tampilan 100 %, 125 %, 150 %, dan 175 %.

### Fase 0

- [ ] `pdf-studio-izul.exe` berjalan di Windows bersih tanpa runtime tambahan.
- [ ] Title bar menampilkan versi dari `version.json`, bukan angka tertanam.
- [ ] Status bar menampilkan jumlah pekerja hidup, dan angkanya turun lalu pulih
      ketika satu proses pekerja dimatikan dari Task Manager.
- [ ] Mematikan pekerja tidak menutup jendela dan tidak membekukan UI.
- [ ] Tidak ada jendela konsol yang berkedip saat aplikasi atau pekerja dimulai.
- [ ] Menutup aplikasi tidak meninggalkan proses `izul-worker.exe` yatim
      (periksa Task Manager; ini yang dijamin `KILL_ON_JOB_CLOSE`).
- [ ] Folder log berisi berkas JSON yang terisi, dan tidak ada koneksi jaringan
      keluar sama sekali (periksa dengan Resource Monitor).

### Fase 1

Hal-hal yang hanya bisa dinilai dengan melihat, dan — tiga yang pertama —
satu-satunya cara membuktikan klaim yang benchmark headless tidak bisa sentuh.

- [ ] Scroll cepat pada dokumen 500 halaman terkunci di refresh rate monitor,
      tanpa frame drop (buka Task Manager → GPU, atau `dxdiag` refresh rate).
- [ ] Selama scroll cepat, tidak pernah ada halaman putih: yang tampak adalah
      pratinjau buram yang berganti tajam, bukan kekosongan.
- [ ] Setelah scroll berhenti, versi tajam datang dalam waktu yang terasa
      seketika (< 150 ms).
- [ ] Ctrl+scroll: titik di bawah kursor tidak bergeser selama zoom.
- [ ] Zoom cepat bolak-balik tidak membuat halaman berkedip putih.
- [ ] Scrollbar tidak pernah melompat saat halaman selesai dirender.
- [ ] Mode dua halaman dan dua halaman dengan sampul: halaman ganjil di kanan.
- [ ] Rotasi dokumen dan rotasi satu halaman: teks yang diseleksi tetap
      mendarat di glif, bukan di posisi sebelum diputar.
- [ ] Seleksi teks lintas baris menyalin teks dengan tata letak terjaga.
- [ ] Sidebar thumbnail pada dokumen 500 halaman: menggulir mulus, dan halaman
      yang sedang dibaca selalu terlihat di panel.
- [ ] Daftar isi melompat ke posisi yang benar, termasuk ke tengah halaman.
- [ ] Menutup dan membuka ulang berkas mengembalikan halaman, scroll, zoom,
      rotasi, dan mode tampilan yang sama.
- [ ] Memindahkan jendela ke monitor dengan DPI berbeda: halaman tetap tajam.
- [ ] Rapi pada skala Windows 100 %, 125 %, 150 %, dan 175 %.
- [ ] Seluruh viewport bisa dioperasikan tanpa mouse: Tab, panah, Page Up/Down,
      Home/End, Ctrl+0/+/−.

### Menyusul (fase terkait)

- [ ] Dark mode dengan invert cerdas: teks terang, foto tidak terbalik (Fase 8).
- [ ] Paritas anotasi saat objek diam, ambang perseptual < 0,5 % (Fase 3).
