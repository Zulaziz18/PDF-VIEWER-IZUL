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

## Hasil Fase 4

| Suite | Jumlah | Status |
|---|---|---|
| `izul-model` (geometri, display list, objek anotasi, AP stream, undo) | 67 | lulus |
| `izul-ipc` (shm, ring, codec, transport, **indeks varian terpatok**) | 23 | lulus |
| `izul-store` (skema, sesi, indeks & FTS5, **draf**, **riwayat ekspor**) | 49 | lulus |
| `izul-pdf` (ubin, pencarian, golden, **simpan-buka 13 jenis**, **paritas berkas tersimpan**, **ratakan**, **ekstrak**) | 54 | lulus |
| `izul-render` (cache LRU, prioritas, pembatalan) | 29 | lulus |
| **`izul-write`** (kamus anotasi, AP + resource, bagian inkremental, simpan atomik) | 30 | lulus |
| `izul-worker` (epoch, galat, **kotak-keluar blob**) | 9 | lulus |
| `izul-app` (kolam, protokol, tab, regex, anotasi, **simpan/draf**, **penjaga berkas terbuka**) | 96 | lulus |
| `crash_isolation` + `render_pipeline` + `render_end_to_end` (pekerja nyata) | 26 | lulus |
| **`save_round_trip`** (pekerja nyata: simpan-buka-sunting-simpan, simpan sebagai, ekspor) | 3 | lulus |
| `izul-bench` (`multidoc`) | 3 | lulus |
| **Total Rust** | **389** | **lulus** |
| `src/viewport`, `src/state`, `src/annots`, **`src/app`** (alur tutup/simpan/draf) | 146 | lulus |
| **Total** | **535** | **lulus** |

**Koreksi tabel Fase 3 di bawah:** `izul-model` tertulis 85 dan `izul-pdf` 63,
padahal suite commit Fase 3 (`45c9903`), dijalankan ulang di worktree terpisah
pada 23 September 2026, berisi **67** dan **48**. Tidak ada test yang hilang
— angka lama salah hitung. Tabel lama dibiarkan sebagai catatan sejarah.

Angka Fase 4 lainnya (paritas, mesin lain, kecepatan simpan) ada di
`bench/results/phase4-parity.txt` dan `bench/results/phase4-linux.txt`:

```bash
cargo test -p izul-pdf --lib a_saved_annotation_renders_like_its_golden -- --nocapture
cargo run --release -p izul-bench --bin fase4-uji -- --bench   # butuh fixture 500 halaman
```

### Memeriksa berkas simpanan di Acrobat, Chrome, dan Edge

Kriteria lulus Fase 4 menyebut ketiga pembaca itu, dan tidak satu pun ada di
lingkungan pengembangan. Ini cara memeriksanya sendiri di Windows. Buka
**PowerShell** (tekan tombol Windows, ketik `powershell`, Enter — yang biasa,
bukan "Run as administrator"), lalu:

```powershell
cd C:\Users\muham\PDF-VIEWER-IZUL
cargo run -p izul-bench --bin fase4-uji
```

Pertama kali butuh beberapa menit (membangun). Kalau berhasil, dua baris
terakhirnya berbunyi kira-kira:

```text
14 anotasi ditulis ke C:\Users\muham\PDF-VIEWER-IZUL\test-fixtures\fase4-uji-simpan.pdf
versi rata ditulis ke C:\Users\muham\PDF-VIEWER-IZUL\test-fixtures\fase4-uji-rata.pdf
```

Buka folder `C:\Users\muham\PDF-VIEWER-IZUL\test-fixtures` di File Explorer,
klik kanan `fase4-uji-simpan.pdf` → **Open with** → pilih pembacanya. Ulangi
untuk Chrome, Edge, dan Acrobat Reader bila terpasang.

Yang benar: halaman berisi 14 kotak berlabel, dan **tiap kotak berisi gambar
yang disebut labelnya** — stabilo kuning di atas teks, garis bawah biru, coret
merah, kotak teks ungu dua baris, gambar bulat berwarna, gelombang hijau, garis
putus-putus, panah merah, kotak biru, elips, segitiga kuning, ikon catatan
oranye, stempel "DISETUJUI", dan kotak merah miring setengah tembus pandang.
Di Acrobat, panel **Comments** mencantumkan 14 komentar. Kotak yang kosong,
bergeser keluar dari kotaknya, atau berwarna lain adalah temuan — ambil
screenshot dan sebutkan pembacanya.

`fase4-uji-rata.pdf` harus terlihat **sama persis**, tetapi di Acrobat panel
Comments-nya kosong: semuanya sudah menyatu ke halaman.

## Hasil Fase 3

| Suite | Jumlah | Status |
|---|---|---|
| `izul-model` (geometri, display list, **objek anotasi**, **AP stream**, **undo**) | 85 | lulus |
| `izul-ipc` (shm, ring, codec, transport) | 21 | lulus |
| `izul-store` (skema, identitas, preferensi, sesi, indeks & FTS5) | 46 | lulus |
| `izul-pdf` (ubin & rotasi, pencarian, Trim, **metrik font**, **golden image**) | 63 | lulus |
| `izul-render` (cache LRU, prioritas, penggabungan, pembatalan) | 29 | lulus |
| `izul-worker` (epoch pembatalan, klasifikasi galat) | 7 | lulus |
| `izul-app` (kolam, protokol, registri tab, regex, **anotasi**, **gambar**) | 91 | lulus |
| `crash_isolation` + `render_pipeline` + `render_end_to_end` (pekerja nyata) | 26 | lulus |
| **Total Rust** | **338** | **lulus** |
| `src/viewport`, `src/state`, **`src/annots`** | 106 | lulus |
| **Total** | **444** | **lulus** |

Golden image Fase 3 ada di `crates/izul-pdf/golden/` — lima belas berkas, satu
per jenis anotasi plus satu yang berputar dan tembus pandang. Memperbarui
baseline:

```bash
IZUL_UPDATE_GOLDEN=1 cargo test -p izul-pdf --lib golden
```

Lakukan itu hanya dengan perubahannya di depan mata, lalu **lihat gambarnya**.
Baseline yang diperbarui tanpa dilihat adalah test yang dimatikan diam-diam.

Paritas kanvas (butuh Chromium sungguhan, tidak dijalankan CI):

```bash
cargo test -p izul-pdf --lib golden      # menulis ulang daftar tampilan
node tools/canvas-parity/run.mjs
```

Hasil dan penjelasan dua ambangnya ada di `bench/results/phase3-parity.txt`.

## Hasil Fase 2

| Suite | Jumlah | Status |
|---|---|---|
| `izul-model` (geometri, display list) | 19 | lulus |
| `izul-ipc` (shm, ring, codec, transport) | 21 | lulus |
| `izul-store` (skema, identitas, preferensi, **sesi**, **indeks & FTS5**) | 46 | lulus |
| `izul-pdf` (matriks ubin & rotasi, ruang tampilan, **pencarian**, **Trim**) | 29 | lulus |
| `izul-render` (cache LRU, prioritas, penggabungan, pembatalan) | 29 | lulus |
| `izul-worker` (epoch pembatalan, klasifikasi galat) | 7 | lulus |
| `izul-app` (kolam, racun, sandbox, protokol, **registri tab**, **regex**, **sampul**, **indeks**) | 77 | lulus |
| `crash_isolation` (proses pekerja nyata) | 7 | lulus |
| `render_pipeline` (proses pekerja nyata) | 11 | lulus |
| `render_end_to_end` (aplikasi + pekerja nyata, **multi-dokumen & pencarian**) | 8 | lulus |
| **Total Rust** | **254** | **lulus** |
| `src/viewport` + `src/state` (geometri, tata letak, prediksi, teks, cache, URI, **sorotan**, **kebijakan memori tab**) | 71 | lulus |
| **Total** | **325** | **lulus** |

Tiga test ujung-ke-ujung baru menjalankan proses pekerja sungguhan: enam
dokumen terbuka sekaligus dengan penutupan salah satunya, pencarian yang
mengembalikan kotak sorot beserta penolakan generasi lama, dan pengindeksan
sampai kata yang terlihat di halaman benar-benar ditemukan lewat FTS5. Seperti
Fase 1, ketiganya digerbangi `#![cfg(unix)]` — cakupan Windows masih test unit
dan checklist manual.

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

### Fase 2

```bash
cargo build --workspace --release      # pekerjanya yang diukur, jadi rilis
cargo run --release -p izul-bench --bin multidoc           # bawaan 30 dokumen
cargo run --release -p izul-bench --bin multidoc -- --documents=50
```

`multidoc` menjawab kriteria lulus Fase 2 langsung: 50 dokumen terbuka dan
dirender pada kolam pekerja sungguhan, biaya per dokumen, efek `Trim`, lalu 200
siklus buka-tutup dengan RSS dicuplik tiap 25 siklus — karena bentuk kurvanya,
bukan selisih ujung ke ujung, yang membedakan kebocoran dari alokator yang
sedang memanas.

Hasilnya di `bench/results/phase2-linux.txt`, berikut catatan tentang satu hasil
yang tidak seperti harapan: `Trim` melepas seluruh pegangan halaman, tetapi RSS
pekerja tidak turun karena PDFium menyimpan arena alokatornya.

## Regresi dari v6.2

Kode v6.2 ada di branch `v6.2-reference` dan sudah dibaca pada Fase 1:
`app.js` (1353 baris), `core.js` (165 baris, fungsi murni), `annots.js`
(290 baris), plus `index.html`, `style.css`, dan `serve.py`.

**Tidak ada berkas test di sana.** 21 test case yang disebut SPEC Bagian 16
tidak ada sebagai kode di repositori v6.2 — tampaknya itu daftar pemeriksaan
manual, bukan suite otomatis. Karena itu "membawa 21 test case ke v7" berarti
menuliskannya ulang sebagai test otomatis terhadap perilaku v6.2 yang bisa
dibaca dari kodenya, bukan menyalin berkas. Pekerjaan itu jatuh di Fase 3 dan
Fase 4, tempat perilaku anotasi dan simpan dibangun; di sanalah daftar kasusnya
akan disusun dari `annots.js` dan jalur simpan `app.js`.

Yang sudah diperiksa pada Fase 1 (bagian viewer dari v6.2):

| Perilaku v6.2 | Di v7 Fase 1 |
|---|---|
| Zoom 40–300 %, langkah 0,15 | 10–1600 % (SPEC 11.1), tangga langkah tetap |
| `Ctrl` + roda memperbesar | Ada, dan kini mempertahankan titik di bawah kursor |
| Indikator halaman = halaman terdekat ke atas viewport | Halaman yang menutupi area terbesar (lebih benar untuk mode dua halaman) |
| Render/lepas per halaman lewat IntersectionObserver, margin 800 px | Tata letak tervirtualisasi, margin 200 px, ditambah prefetch prediktif |
| Lapisan teks DOM di atas kanvas | Sama pendekatannya, kotak dari PDFium |
| Daftar isi dari outline bawaan PDF | Ada |
| **Daftar isi cadangan: deteksi judul bab dari teks** (BAB/BAGIAN/DAFTAR PUSTAKA dst.) | **Belum ada** — lihat catatan di bawah |

`findChapters` di `core.js` v6.2 membaca teks seluruh dokumen dan menyusun
daftar isi sendiri ketika PDF tidak membawa outline — dengan pola yang jelas
disetel untuk dokumen berbahasa Indonesia. Untuk skripsi hasil pindai, itu
kemungkinan besar satu-satunya cara panel daftar isi pernah berguna di v6.2.

v7 belum punya padanannya, dan itu **disengaja untuk sekarang**: ia butuh sapuan
teks seluruh dokumen, yang justru dibangun di Fase 2 bersama indeks FTS5.
Menambahkannya di Fase 2 nyaris tanpa biaya tambahan; menambahkannya di Fase 1
berarti membangun sapuan teks dua kali.

## Menjalankan sendiri di Windows (langkah demi langkah)

Bagian ini ditulis untuk yang belum pernah membangun aplikasi dari kode.
Dikerjakan sekali; sesudahnya cukup langkah 6.

### 1. Pasang alat (sekali saja)

Buka **PowerShell sebagai Administrator**, lalu jalankan satu per satu:

```powershell
winget install --id Git.Git -e
winget install --id OpenJS.NodeJS.LTS -e
winget install --id Rustlang.Rustup -e
winget install --id Microsoft.EdgeWebView2Runtime -e
winget install --id Microsoft.VisualStudio.2022.BuildTools -e ^
  --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Yang terakhir adalah kompilator C++ milik Microsoft. Rust memerlukannya untuk
menautkan program di Windows, dan ukurannya beberapa gigabita — biarkan selesai.

**Tutup PowerShell, lalu buka lagi** supaya perintah `git`, `node`, dan `cargo`
dikenali. Periksa:

```powershell
git --version
node --version
cargo --version
```

Ketiganya harus menjawab dengan nomor versi. Kalau ada yang bilang "not
recognized", restart komputer dan periksa lagi.

### 2. Ambil kodenya

```powershell
cd $HOME\Documents
git clone https://github.com/Zulaziz18/PDF-VIEWER-IZUL.git
cd PDF-VIEWER-IZUL
git checkout claude/pdf-studio-izul-v7-fase-1-tki5pi
```

### 3. Ambil PDFium

PDFium tidak ikut di repositori. Klik kanan di dalam folder proyek →
**Open Git Bash here** (dipasang bersama Git), lalu:

```bash
./vendor/pdfium/fetch.sh win-x64
```

Kalau berhasil, baris terakhirnya menyebut `MAJOR=151 ... BUILD=7881`.

### 4. Pasang paket frontend

Kembali ke PowerShell, di folder proyek:

```powershell
npm ci
```

### 5. Jalankan

```powershell
npm run tauri dev
```

Pertama kali perlu **5–15 menit**: Rust mengompilasi ratusan pustaka. Layar akan
penuh baris `Compiling ...` — itu normal, bukan galat. Jendela aplikasi terbuka
sendiri setelah selesai. Berikutnya jauh lebih cepat.

Kalau berhenti dengan pesan merah, salin lima baris terakhirnya — itu yang
dibutuhkan untuk menolong.

### 6. Siapkan berkas uji

Checklist di bawah butuh PDF **besar** (ratusan halaman) supaya scroll benar-benar
diuji. Pakai apa saja yang Anda punya: skripsi, buku pindaian, manual tebal.
Bila tidak ada, buat sendiri (butuh Python):

```powershell
python -m pip install reportlab pikepdf pypdf pillow
python bench/make_fixtures.py test-fixtures text
```

Hasilnya `test-fixtures/text-500p.pdf`, 500 halaman.

### 7. Cara melihat frame rate (untuk tiga item pertama checklist)

Ini satu-satunya bagian yang butuh trik, dan hanya bekerja pada
`npm run tauri dev` (bukan hasil `build`):

1. Klik kanan di area dokumen → **Inspect** (DevTools terbuka).
2. Tekan `Ctrl` + `Shift` + `P`.
3. Ketik `frame`, pilih **Show frame rendering stats**, tekan Enter.
4. Kotak kecil muncul di pojok kanan atas dengan angka FPS.
5. Gulir dokumen cepat-cepat sambil melihat angka itu.

Yang dicari: angka bertahan mendekati refresh rate monitor (60, 120, atau 144),
dan grafiknya tidak menunjukkan batang merah panjang. Kalau angkanya jatuh ke
20–30 saat menggulir, itu temuan — catat berkas apa dan di zoom berapa.

Tutup DevTools sesudahnya; ia sendiri memakan sebagian tenaga mesin.

### 8. Jalankan checklist

Kerjakan daftar **Fase 1** di bawah satu per satu, dan catat yang gagal beserta
apa yang Anda lihat. Yang gagal jauh lebih berguna daripada yang lulus.

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

### Fase 2

Hal-hal yang hanya bisa dinilai dengan memakainya, pada Windows sungguhan.

- [ ] Membuka lima dokumen: strip tab muncul saat dokumen kedua dibuka, tiap tab
      membawa nama berkasnya.
- [ ] Berpindah tab mengembalikan zoom, rotasi, dan posisi baca masing-masing —
      bukan keadaan tab yang barusan ditinggalkan.
- [ ] Menutup tab yang sedang aktif memindahkan fokus ke tab sebelahnya, bukan
      mengosongkan jendela.
- [ ] Menyeret tab mengubah urutannya, dan urutan itu bertahan setelah aplikasi
      dijalankan ulang.
- [ ] `Ctrl+W` menutup tab, `Ctrl+Tab` berpindah tab.
- [ ] Menutup aplikasi dengan lima tab terbuka lalu menjalankannya lagi:
      kelimanya kembali, dan tab yang aktif adalah yang aktif sebelumnya.
- [ ] Memindahkan salah satu berkasnya ke folder lain sebelum menjalankan ulang:
      berkas itu dilewati tanpa dialog galat, sisanya tetap kembali.
- [ ] Tombol 📂 di toolbar membuka pemilih berkas, dan memilih dua berkas
      sekaligus menghasilkan dua tab.
- [ ] Tombol perkecil, perbesar, dan tutup di ujung kanan bilah judul bekerja;
      bilah judulnya bisa diseret untuk memindahkan jendela.
- [ ] Menutup aplikasi saat ada anotasi memunculkan pertanyaan lebih dulu.
- [ ] Menyeret berkas PDF ke jendela membukanya sebagai tab baru.
- [ ] Klik ganda berkas PDF di Explorer membukanya (perlu asosiasi berkas dari
      installer).
- [ ] Klik ganda berkas PDF **kedua** saat aplikasi sudah berjalan: muncul
      sebagai tab baru di jendela yang sama, jendelanya maju ke depan, dan
      **tidak** ada jendela kedua di taskbar.
- [ ] Layar awal menampilkan sampul halaman pertama untuk berkas yang pernah
      dibuka; berkas yang belum pernah dibuka mendapat kartu polos, bukan kotak
      rusak.
- [ ] `Ctrl+F` membuka panel pencarian; mengetik menampilkan hasil tanpa jeda
      yang terasa, dan menghapus ketikan menghapus sorotannya.
- [ ] Kecocokan di halaman yang tampak tersorot **tepat di atas katanya**, juga
      pada zoom 200 % dan pada halaman yang diputar.
- [ ] Cakupan "Semua dokumen" menemukan kata dari berkas yang **tidak** sedang
      terbuka, dan mengkliknya membukanya di tab.
- [ ] Cakupan "Regex": pola seperti `\d{3}-\d{4}` menyorot di halaman yang
      dibuka; pola yang belum lengkap (`(abc`) memunculkan pesan, bukan diam.
- [ ] Pada dokumen 500 halaman: panel menunjukkan kemajuan pengindeksan, dan
      menggulir tetap mulus selama pengindeksan berjalan.
- [ ] Membuka 20 dokumen sekaligus: Task Manager menunjukkan memori yang tidak
      terus menanjak setelah tab-tab lama berhenti dilihat.

### Fase 3

Yang hanya bisa dinilai dengan memakainya, dan yang **belum pernah dijalankan
di jendela sungguhan** — kontainer pengembangan tidak punya layar.

- [ ] Tiap alat menggambar objeknya: pena, garis, panah, kotak, elips, poligon,
      kotak teks, catatan tempel, stempel.
- [ ] Menyeret objek: bergerak mengikuti kursor tanpa tersendat, dan berhenti
      persis di tempat kursor dilepas.
- [ ] Delapan pegangan ubah ukuran bekerja, sudut seberangnya tetap diam.
- [ ] Pegangan rotasi memutar terhadap pusat objek; menahan Shift mengunci ke
      kelipatan 15 derajat.
- [ ] Pita karet di ruang kosong memilih objek yang sepenuhnya di dalamnya.
- [ ] Shift+klik menambah dan mengurangi dari seleksi.
- [ ] Panel properti mengubah warna, opasitas, tebal garis, dan ukuran font, dan
      perubahannya langsung terlihat.
- [ ] Ctrl+Z membatalkan satu gestur utuh — bukan setengah geseran — dan Ctrl+Y
      mengulanginya.
- [ ] Menandai teks lalu menekan tombol stabilo menyorot **baris yang dipilih
      saja**, termasuk saat seleksinya melewati pergantian baris.
- [ ] Objek yang dikunci tidak bisa diseret dan tidak menghalangi klik ke objek
      di bawahnya.
- [ ] Panel daftar anotasi mencantumkan semuanya per halaman, dan mengkliknya
      melompat ke halaman itu.
- [ ] Menyisipkan gambar: gambarnya muncul, bisa digeser dan diubah ukuran, dan
      **tidak berubah ketajamannya** saat dilepas.
- [ ] Zoom 400 persen: anotasi tetap tajam dan tetap di tempat yang sama
      relatif terhadap teks halaman.
- [ ] Memutar halaman: anotasi ikut berputar bersama isinya.

### Fase 4

Menyimpan menimpa berkas sungguhan. **Pakai salinan**, bukan dokumen yang
penting, untuk daftar ini.

- [ ] Menambah anotasi memunculkan titik oranye di tab dan "Belum disimpan" di
      bilah bawah; tombol Simpan (ikon disket di kiri atas) menjadi aktif.
- [ ] Ctrl+S menyimpan: muncul "Tersimpan: nama.pdf (ukuran)" di pojok kanan
      bawah, titik oranye hilang, ukuran di bilah bawah berubah.
- [ ] Tutup berkas itu, buka lagi dari Beranda: semua anotasi kembali, dan bisa
      digeser, diubah, dihapus seperti sebelum disimpan.
- [ ] Berkas yang sama dibuka di Chrome atau Edge: anotasinya tampil di tempat
      yang sama.
- [ ] Ctrl+Shift+S (Simpan Sebagai) ke nama baru: tab berganti nama, berkas lama
      tidak berubah.
- [ ] Menutup tab yang belum disimpan menanyakan Simpan / Jangan Simpan / Batal.
      Batal membiarkan tab terbuka; Jangan Simpan menutupnya tanpa menulis
      apa pun.
- [ ] Tombol × jendela **dan** Alt+F4 dengan pekerjaan belum disimpan: pertanyaan
      yang sama, per dokumen.
- [ ] Pemulihan: beri anotasi, tunggu 30 detik, lalu matikan aplikasi lewat Task
      Manager (klik kanan PDF Studio Izul → End task). Buka lagi berkasnya:
      muncul "Pulihkan pekerjaan yang belum disimpan?", dan Pulihkan
      mengembalikan anotasinya.
- [ ] Buka sebuah PDF di aplikasi, lalu ubah berkas yang sama dengan program
      lain (mis. simpan ulang dari Edge). Dalam beberapa detik muncul pita
      kuning "Berkas ini diubah oleh program lain" dengan Muat Ulang / Abaikan.
- [ ] Konversi → PDF ke Gambar, rentang `1-3`, JPG, 150 DPI, pilih folder: tiga
      berkas `nama-1.jpg` … `nama-3.jpg` muncul di folder itu.
- [ ] Konversi → Ekspor Halaman `2, 5`: PDF baru berisi dua halaman itu, dan
      anotasinya masih bisa disunting bila dibuka di aplikasi ini.
- [ ] Konversi → Ekspor Rata: di Acrobat/Edge anotasinya tidak bisa dipilih lagi,
      tetapi tampilannya sama.
- [ ] Beranda → Riwayat Ekspor mencantumkan ketiga ekspor di atas.
- [ ] Ekspor ke nama berkas yang sedang terbuka di tab lain: ditolak dengan
      pesan, tidak ada yang tertimpa.

### Menyusul (fase terkait)

- [ ] Dark mode dengan invert cerdas: teks terang, foto tidak terbalik (Fase 8).
- [x] Paritas anotasi saat objek diam, ambang perseptual < 0,5 % (Fase 3;
      berkas tersimpan 0,000 % di Fase 4).
