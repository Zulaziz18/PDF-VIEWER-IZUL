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
npm run typecheck && npm run lint && npm run build
```

Agar test integrasi tidak diam-diam dilewati ketika prasyaratnya hilang:

```bash
IZUL_REQUIRE_FIXTURES=1 cargo test --workspace
```

Tanpa variabel itu, test yang kekurangan fixture atau pustaka akan mencetak
`LEWATI: ...` dan lulus — nyaman untuk klon baru, berbahaya untuk CI. CI
menyetelnya.

Test integrasi memerlukan PDFium di samping binari test:

```bash
cp vendor/pdfium/linux-x64/lib/libpdfium.so target/debug/    # atau pdfium.dll
```

## Hasil Fase 0

| Suite | Jumlah | Status |
|---|---|---|
| `izul-model` (geometri, display list) | 19 | lulus |
| `izul-ipc` (shm, ring, codec, transport) | 21 | lulus |
| `izul-store` (skema, migrasi, identitas berkas) | 19 | lulus |
| `izul-pdf` (matriks ubin) | 5 | lulus |
| `izul-worker` (epoch pembatalan, klasifikasi galat) | 7 | lulus |
| `izul-app` (kebijakan kolam, racun, sandbox, protokol, versi) | 33 | lulus |
| `crash_isolation` (proses pekerja nyata) | 7 | lulus |
| **Total** | **111** | **lulus** |

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

### Menyusul (fase terkait)

- [ ] Scroll terkunci di refresh rate, tanpa halaman putih (Fase 1).
- [ ] Zoom mempertahankan titik fokus kursor (Fase 1).
- [ ] Dark mode dengan invert cerdas: teks terang, foto tidak terbalik (Fase 8).
- [ ] Paritas anotasi saat objek diam, ambang perseptual < 0,5 % (Fase 3).
