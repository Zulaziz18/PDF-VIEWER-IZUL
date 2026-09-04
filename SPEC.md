# SPEC.md — PDF Studio Izul v7 "Atlas"

> **Cara pakai:** salin seluruh isi berkas ini ke Claude Code sebagai instruksi awal, dan simpan sebagai `SPEC.md` di root proyek. Ini dokumen rujukan sepanjang proyek, bukan pesan sekali pakai.

---

## 0. Peran, Aturan Main, dan Standar Kode

Kamu lead engineer untuk aplikasi desktop Windows kelas produksi. Pembandingnya PDF Expert dan PDF-XChange Editor — bukan pembaca PDF gratisan.

**Sebelum menulis satu baris kode:**

1. Baca seluruh dokumen ini.
2. Kalau kode PDF Studio Izul v6.2 ada di repo, pelajari lebih dulu. Perilaku anotasi dan simpan yang sudah terbukti wajib dipertahankan, meski implementasinya ditulis ulang total.
3. Kerjakan Fase 0 (Bagian 17) — termasuk **spike pengukuran**, karena beberapa target di Bagian 13 belum tervalidasi.
4. Laporkan hasilnya dan tunggu persetujuan sebelum lanjut ke Fase 1.

**Selama pengerjaan:**

- Satu fase per waktu. Selesai → tunjukkan hasil + angka benchmark nyata → minta persetujuan → lanjut.
- Menemukan pendekatan lebih baik? Katakan dan jelaskan. Jangan menyimpang diam-diam.
- Permintaan saya yang secara teknis buruk wajib kamu tolak dengan alasan. Saya tidak mencari persetujuan.
- Angka benchmark dilaporkan apa adanya. Kalau target meleset, sebutkan angkanya dan penyebabnya. Jangan pernah mengarang atau membulatkan ke arah yang enak didengar.
- `version.json` sebagai sumber versi tunggal, ditampilkan di title bar dan About. `CHANGELOG.md` diperbarui tiap akhir fase.

**Standar kode — ditegakkan lewat CI, bukan lewat niat baik:**

- Rust: `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` di semua crate produksi. Error lewat `thiserror`. `catch_unwind` di setiap batas FFI ke PDFium. Tidak ada `unsafe` tanpa komentar yang menjelaskan invarian yang dijaga.
- TypeScript: `strict` + `noUncheckedIndexedAccess`, `any` dilarang. Tidak ada logika bisnis di dalam komponen.
- Semua string antarmuka di satu modul i18n. Bahasa Indonesia sebagai default, struktur siap untuk bahasa lain.
- Tidak ada `TODO`, stub kosong, atau fungsi yang mengembalikan data palsu di kode yang dinyatakan selesai.

**Batasan sumber daya:** tidak ada. Installer sampai 200 MB diterima, mesin target RAM 16 GB. Optimalkan untuk kualitas dan kehalusan, bukan untuk hemat ukuran.

---

## 1. Konteks

Kelanjutan **PDF Studio Izul v6.2** — editor PDF offline dengan arsitektur lama: pdf.js merender di browser, Python + PyMuPDF menulis, server lokal port 8743.

Wajib ada lagi di v7: teks (Times New Roman), highlight dengan slider opasitas, gambar (crop pan+resize, hapus background AI), hapus halaman, undo, Ctrl+V clipboard, Recent Open, riwayat ekspor. Semua anotasi adalah **objek hidup** — bisa digeser, diubah ukuran, dihapus, dan tetap editable setelah PDF disimpan lalu dibuka ulang.

v7 menjadikannya aplikasi PDF utama: viewer multi-dokumen sekaligus editor penuh, dalam satu aplikasi.

**v6.2 jangan dihapus** sampai v7 mencapai paritas fitur penuh.

---

## 2. Bukan Tujuan v7

Ditulis eksplisit supaya tidak merembes masuk:

- Thumbnail handler Explorer (butuh shell extension COM terpisah — dibuang).
- Sinkronisasi cloud, akun, kolaborasi, atau fitur apa pun yang menyentuh jaringan.
- Pembuatan tanda tangan digital (verifikasi tanda tangan yang ada: boleh, hanya tampil).
- Dukungan XFA.
- Reflow paragraf saat menyunting teks asli dokumen.
- Versi macOS atau Linux.

---

## 3. Keputusan Arsitektur Inti

Lima pilar. Jangan diubah tanpa membicarakannya dengan saya.

### 3.1 Satu mesin PDF untuk render dan tulis

Di v6, pdf.js menggambar dan PyMuPDF menulis. Dua implementasi menafsirkan spesifikasi PDF secara berbeda — itu akar bug "highlight di editor tidak sama dengan hasil ekspor". Kelas bug ini tidak bisa ditambal, hanya bisa dihapus penyebabnya.

v7 memakai **PDFium** untuk keduanya, lewat `pdfium-render`. Lisensi BSD-3, tidak ada beban copyleft.

### 3.2 Display list sebagai sumber geometri tunggal

Ini mekanisme yang menjamin paritas **secara struktural**, bukan sekadar lewat pengujian.

Model anotasi tidak menghasilkan piksel dan tidak menghasilkan AP stream secara langsung. Ia menghasilkan **display list**: urutan perintah gambar primitif dalam koordinat PDF (satuan poin, origin kiri-bawah).

```
DisplayOp = FillPath{path, warna, opasitas, blend}
          | StrokePath{path, warna, lebar, cap, join, dash}
          | DrawText{glyphs, font_ref, ukuran, matriks, warna}
          | DrawImage{image_ref, matriks, opasitas}
          | PushClip{path} | PopClip
          | PushTransform{matriks} | PopTransform
```

Dua backend membaca daftar yang sama:

- **Backend kanvas** — menggambar proksi langsung saat objek sedang diseret atau diubah ukuran.
- **Backend AP stream** — menyerialisasikannya jadi operator PDF saat menyimpan.

Fungsinya murni dan deterministik: `display_list(obj, font_ctx) -> Vec<DisplayOp>`. Tanpa I/O, tanpa PDFium, bisa diuji sendiri. Geometri tidak mungkin menyimpang karena sumbernya hanya satu.

### 3.3 Definisi paritas: identik saat objek diam

Menunggu PDFium merender ulang tiap frame saat objek diseret itu mustahil dalam anggaran 16 ms. Maka kontraknya:

- **Selama interaksi** (drag, resize, rotate): proksi kanvas dari display list. Boleh berbeda tipis dalam antialiasing dan pembulatan subpiksel.
- **Saat objek dilepas**: render otoritatif PDFium dari AP stream menggantikan proksi dalam < 150 ms.
- **Saat objek diam**: tampilan layar dan hasil ekspor **identik**, dibuktikan golden image dengan ambang perbedaan perseptual < 0,5%.

Kriteria lulus Fase 3 memakai definisi ini.

### 3.4 Kolam pekerja tersandbox, bukan satu proses per dokumen

Lima puluh proses PDFium akan menghabiskan RAM dan handle jauh sebelum anggaran tercapai. Yang dipakai: **kolam pekerja berjumlah tetap** — `min(jumlah_core / 2, 8)`, minimal 2. Tiap pekerja menampung beberapa dokumen. Dokumen dipetakan ke pekerja dengan penyeimbangan beban, dan dokumen yang sedang aktif dilihat diprioritaskan agar tidak berbagi pekerja dengan pekerjaan berat.

PDFium mem-parsing masukan yang tidak tepercaya, jadi tiap pekerja dijalankan dalam **Windows Job Object** dengan batas memori, tanpa akses jaringan, dan dengan token berhak rendah.

**Supervisor** di proses UI:

- Heartbeat tiap 2 detik. Pekerja diam > 6 detik dianggap hang dan dimatikan.
- Restart otomatis dengan backoff eksponensial; dokumen dimuat ulang ke posisi baca terakhir.
- Dokumen yang menjatuhkan pekerja **dua kali** ditandai "beracun": dibuka ulang sendirian dalam mode terbatas hanya-baca, dan tidak pernah lagi berbagi pekerja dengan dokumen lain.
- Crash menjatuhkan beberapa tab, bukan seluruh aplikasi. Tab terdampak menampilkan pesan jelas dengan tombol muat ulang, dan **draf anotasi yang belum disimpan dipulihkan dari SQLite**.

### 3.5 State di SQLite

Session, recent files, posisi baca, draf autosave, cache thumbnail, dan indeks pencarian full-text di basis data lokal. Ini fondasi pencarian lintas pustaka dan pemulihan session yang andal.

---

## 4. Stack

| Lapisan | Pilihan | Lisensi |
|---|---|---|
| Shell | Tauri v2 (Rust) | MIT / Apache-2.0 |
| Mesin PDF | PDFium via `pdfium-render` | BSD-3 (PDFium) |
| Paralelisme | `rayon` + `tokio` | MIT / Apache-2.0 |
| IPC | Named pipe + shared memory, serialisasi `postcard` | MIT / Apache-2.0 |
| Basis data | `rusqlite` (bundled, FTS5) | MIT |
| Logging | `tracing` → berkas lokal | MIT |
| UI | TypeScript 5 + React 19 | MIT |
| State UI | `zustand` | MIT |
| Styling | Tailwind v4 + CSS variables | MIT |
| Kanvas | Imperatif, di luar React | — |
| AI | `ort` (ONNX Runtime) + DirectML | MIT |
| OCR | `ocrs` atau Tesseract via FFI | MIT / Apache-2.0 |
| Font bundel | Inter | SIL OFL |
| Installer | NSIS + MSI + portable | — |

**Verifikasi versi pastinya saat setup** dan laporkan. Daftar di atas adalah pilihan, bukan hasil pengecekan registry hari ini.

**Periksa lisensi model ONNX secara terpisah.** Banyak turunan U²-Net berlisensi non-komersial. Laporkan temuanmu sebelum membundel model apa pun.

**Larangan keras:**

- Nol koneksi jaringan. Tidak ada telemetri, CDN, font online, atau pengecekan pembaruan. Semua aset dibundel.
- Tidak ada ketergantungan Python di mesin pengguna.
- Jangan render PDF di thread utama. Jangan pernah taruh logika kanvas di dalam komponen React.
- Jangan salurkan bitmap lewat `invoke` Tauri (data diubah jadi string — anggaran 16 ms langsung habis).

---

## 5. Struktur Repositori

```
pdf-studio-izul/
├─ SPEC.md · CHANGELOG.md · TESTING.md · PANDUAN.md · version.json
├─ crates/
│  ├─ izul-model/    # objek anotasi, display list, daftar operasi, undo stack,
│  │                 # serializer AP stream. MURNI — tanpa PDFium, tanpa I/O.
│  ├─ izul-pdf/      # pembungkus PDFium: buka, render, ekstrak teks, tulis
│  ├─ izul-ipc/      # definisi pesan + serialisasi, dipakai kedua sisi
│  ├─ izul-worker/   # binary proses pekerja
│  ├─ izul-store/    # SQLite: session, recent, indeks, cache
│  └─ izul-ocr/      # OCR & model AI (Fase 7)
├─ src-tauri/        # proses UI: jendela, menu, asosiasi berkas, supervisor
├─ src/
│  ├─ app/           # shell, tab, split pane, command palette
│  ├─ viewport/      # kanvas imperatif — TIDAK ADA React di folder ini
│  ├─ annots/        # UI alat & panel properti
│  ├─ state/         # store zustand, satu store per dokumen
│  ├─ design/        # token, ikon, primitif UI
│  └─ i18n/
├─ bench/ · test-fixtures/ · tests/golden/
└─ vendor/pdfium/
```

`izul-model` **tidak boleh** menyentuh PDFium atau berkas. Kalau paritas rusak, di sinilah bug-nya, dan crate ini harus bisa diuji tanpa merender apa pun.

---

## 6. Protokol IPC

Dua kanal, karena bitmap dan perintah punya kebutuhan berbeda.

**Kanal perintah** — named pipe, frame berprefiks panjang, `postcard`. Tiap permintaan membawa `request_id` dan `generation`.

```
UI → Worker:  Open{doc_id, path, password?}
              Close{doc_id}
              RenderTile{doc_id, page, rect, scale, generation}
              RenderThumb{doc_id, page_range}
              ExtractText{doc_id, page_range}
              Search{doc_id, query, opts}
              ApplyOps{doc_id, ops[]}
              Save{doc_id, target, mode}
              Cancel{generation}
              Ping
Worker → UI:  Opened{page_count, page_sizes[], permissions, encrypted, tagged}
              TileReady{shm_key, offset, stride, w, h, generation}
              ThumbReady · TextReady · SearchHit · Progress
              Error{doc_id, kind, pesan_ramah}
              Pong
```

`generation` naik setiap zoom atau tata letak berubah. Ubin dari generasi lama dibuang tanpa menunggu — ini yang mencegah thread terbuang saat pengguna zoom cepat. Pekerjaan yang sudah berjalan memeriksa cancellation token di batas ubin.

**Kanal piksel** — shared memory (file mapping Windows) berbentuk ring buffer. Pekerja menulis RGBA, mengirim kunci + offset + stride. Frontend mengambilnya lewat **protokol URI kustom** `izul://tile/{doc}/{page}/{gen}` yang mengembalikan byte mentah, lalu membungkusnya jadi `ImageBitmap`. Nol base64, nol penyalinan tambahan.

---

## 7. Skema Data

Dua basis data terpisah, keduanya mode WAL. Pemisahan ini disengaja: `cache.db` boleh dihapus kapan saja tanpa kehilangan apa pun.

**`app.db`** — data yang hilangnya menyakitkan:

```
files(id, path, path_hash, size, mtime, last_opened, pinned)
reading_state(file_id, page, scroll_y, zoom, view_mode, rotation)
sessions(id, created_at)
session_tabs(session_id, file_id, panel, tab_order, pinned, is_active)
drafts(file_id, base_hash, ops_blob, updated_at)
bookmarks(file_id, page, label, created_at)
doc_text(file_id, page, text)
doc_fts USING fts5(text, content='doc_text')
export_history(file_id, out_path, kind, created_at)
prefs(key, value)
shortcuts(command, keys)
```

**`cache.db`** — thumbnail dan luberan cache halaman.

`path_hash` + `mtime` + `size` mendeteksi berkas yang dipindah atau berubah di luar aplikasi. `base_hash` di `drafts` memastikan draf tidak pernah diterapkan ke berkas yang sudah berubah.

---

## 8. Model Objek & Penyuntingan

```
AnnotObject {
  id, page, kind, rect, rotation, opacity, z, locked,
  created_at, modified_at,
  payload: <sesuai kind>
}

kind = Highlight | FreeText | Image | Ink | Line | Arrow
     | Rect | Ellipse | Polygon | Note | StrikeOut | Underline | Stamp
```

**Koordinat selalu dalam ruang PDF** (poin, origin kiri-bawah). Konversi ke piksel hanya terjadi di lapisan viewport. Menyimpan koordinat dalam piksel akan membuat posisi rusak begitu zoom berubah — jangan pernah lakukan itu.

- **Non-destruktif.** Berkas asli tidak disentuh sampai perintah simpan eksplisit. Perubahan hidup sebagai daftar operasi di atas dokumen dasar.
- **Undo/redo berbasis command stack.** Tiap operasi tahu cara membalik dirinya. Minimal 200 langkah per dokumen, tetap utuh saat berpindah tab, hemat memori.
- **Objek hidup setelah simpan-buka.** Saat menyimpan, anotasi ditulis sebagai anotasi PDF standar (bukan diratakan), dengan AP stream hasil display list, ditambah metadata milik kita di custom key. v7 mengenalinya dan bisa menyunting ulang. Pembaca PDF lain tetap menampilkannya normal.
- **Autosave draf ke SQLite tiap 20 detik**, dipulihkan setelah crash atau tutup paksa.
- **Simpan atomik**: tulis ke berkas sementara di volume yang sama → verifikasi bisa dibuka ulang → ganti nama. PDF tidak boleh pernah rusak setengah jalan.
- Deteksi berkas yang berubah di disk saat terbuka, tawarkan muat ulang tanpa membuang draf.

---

## 9. Pipeline Render

Ini penentu aplikasi ini terasa mahal atau murah.

- **Render dua tingkat.** Saat scroll, tampilkan versi resolusi rendah dari cache secara instan, ganti dengan versi tajam saat siap. Pengguna tidak boleh pernah melihat halaman putih.
- **Tile-based di zoom tinggi.** Di atas 200%, halaman dipecah jadi ubin 512×512; hanya ubin terlihat yang dirender.
- **Cache LRU di Rust**, anggaran default 2 GB, bisa diatur. Isinya bitmap terender + thumbnail + text layer. Kunci cache mencakup skala dan DPI.
- **Prefetch prediktif.** Deteksi arah dan kecepatan scroll, render halaman di depan sebelum diminta.
- **Pembatalan agresif** lewat `generation` — pekerjaan usang dibuang, tidak diselesaikan.
- **Render sesuai DPI layar**, hormati skala tampilan Windows dan monitor dengan DPI campuran (termasuk saat jendela dipindah antar monitor).
- **Abstraksi permukaan gambar.** Fase 1 memakai canvas 2D per halaman; rancang antarmukanya agar renderer WebGL2 dengan atlas tekstur bisa menggantikannya di Fase 8 tanpa menyentuh lapisan di atasnya.
- Manajemen warna sadar ICC, keluaran sRGB benar.

---

## 10. Multi-Dokumen

- Tiap dokumen = **DocumentSession**: undo stack, posisi, zoom, dan status simpan sendiri. Dipetakan ke salah satu pekerja di kolam.
- Tab bar: urutan bisa digeser, bisa di-pin, indikator perubahan belum disimpan, tooltip path lengkap, overflow rapi.
- **50 dokumen terbuka** tanpa degradasi. Tab tidak aktif melepas bitmap resolusi penuh tapi menyimpan thumbnail, posisi, dan undo stack.
- **Split view hingga 4 panel** (grid 2×2), tab bisa ditarik antar panel, ukuran panel diingat.
- **Seret halaman antar dokumen** lewat panel thumbnail untuk menyalin atau memindahkan.
- **Mode banding**: dua dokumen berdampingan, scroll tersinkron, perbedaan teks dan visual disorot.
- Menutup tab dengan perubahan belum disimpan → dialog Simpan / Buang / Batal.

---

## 11. Fitur

### 11.1 Viewer

- Scroll berkelanjutan tervirtualisasi; placeholder berukuran benar sehingga scrollbar tidak pernah melompat.
- Mode tampilan: satu halaman, dua halaman, dua halaman dengan sampul, scroll horizontal.
- Zoom 10–1600%, fit width, fit page, actual size, Ctrl+scroll, pinch touchpad, **zoom mempertahankan titik fokus kursor**.
- Rotasi per halaman dan per dokumen.
- Seleksi teks akurat berbasis kotak karakter PDFium; salin dengan tata letak terjaga; seleksi persegi (Alt+drag).
- **Pencarian tiga tingkat:** dalam dokumen · lintas dokumen terbuka · lintas seluruh pustaka berkas yang pernah dibuka (FTS5). Panel hasil, sorotan semua kecocokan, case-sensitive, whole word.
  **Regex berjalan di jalur terpisah**, bukan lewat FTS5 — FTS5 tidak mendukungnya. Regex hanya berlaku pada dokumen terbuka, lebih lambat, dan harus ditandai jelas di UI. Jangan janjikan setara.
- Sidebar: Thumbnail, Outline, Daftar Anotasi, Hasil Pencarian, Lampiran, Layer. Bisa dilipat, lebarnya diingat.
- Bookmark buatan pengguna, terpisah dari outline bawaan PDF.
- Mode Presentasi (F5) dan Mode Fokus.
- Posisi baca terakhir + zoom diingat per berkas.
- Dark mode sejati, termasuk **invert halaman cerdas**: teks jadi terang, gambar dan foto tidak ikut terbalik.
- Cetak dengan pratinjau, rentang, skala, opsi cetak anotasi.

### 11.2 Anotasi

Semua objek hidup: pilih, geser, ubah ukuran, putar, hapus. Seleksi jamak, ratakan, distribusikan, kunci.

- **Highlight** dengan slider opasitas, selalu di belakang teks.
- **Teks**: font picker (Times New Roman default), embedding font ke PDF, ukuran, warna, bold/italic, perataan, spasi baris. Dukung teks CJK dan RTL — kalau font tidak tersedia, tolak dengan pesan jelas, jangan tulis kotak kosong.
- **Gambar**: sisip dari berkas, Ctrl+V clipboard/screenshot, crop pan+resize, ubah ukuran proporsional, putar, opasitas. (Hapus background AI menyusul di Fase 7.)
- **Pena bebas** dengan penghalusan kurva, garis, panah, kotak, elips, poligon.
- Catatan tempel, coret, garis bawah, stempel.
- Panel daftar anotasi: kelompok per halaman, klik untuk melompat, filter per jenis, ekspor daftar.

### 11.3 Halaman & Berkas

- Hapus, putar, susun ulang, sisip halaman kosong, ekstrak, duplikat.
- Gabung dokumen, pecah berdasarkan rentang atau bookmark.
- Simpan, Simpan Sebagai, Ekspor rata, ekspor rentang, ekspor halaman sebagai PNG/JPG resolusi tinggi.
- Kompresi berkas dengan pratinjau dampak ke kualitas.
- Riwayat ekspor yang bisa diklik.
- Recent Files bergambar thumbnail, bisa di-pin, deteksi berkas yang dipindah.
- Session restore penuh.
- Integrasi Windows: asosiasi `.pdf`, "Open with", jump list taskbar, drag & drop, buka banyak berkas sekaligus dari Explorer.

---

## 12. Desain UI/UX

**Dokumen adalah bintangnya, antarmuka adalah pelayan.** Kalau UI menarik perhatian saat orang sedang membaca, desainnya gagal.

- Grid 8px. Radius 8px (12px panel besar). Tanpa gradien, tanpa glassmorphism, tanpa bayangan tebal.
- Tipografi: Segoe UI Variable, fallback Inter (dibundel). Skala 12 / 13 / 15 / 20 / 28.
- Warna: kanvas netral (`#f5f5f4` terang / `#1c1c1e` gelap — jangan hitam pekat), permukaan panel selapis berbeda, **satu** warna aksen untuk seluruh aplikasi. Warna lain hanya untuk status: merah destruktif, oranye belum disimpan.
- Ikon: satu set konsisten, stroke 1,5px, 20px. Lucide atau Fluent — pilih satu, jangan campur.
- Gerak: 120–180ms, easing `cubic-bezier(0.32, 0.72, 0, 1)`. Hanya untuk perubahan status yang perlu dipahami mata. Nol animasi dekoratif. Hormati `prefers-reduced-motion`.
- Rapi di skala Windows 100%, 125%, 150%, 175%.
- Title bar kustom menyatu dengan tab bar.
- **Toolbar kontekstual**: mode Baca hanya navigasi, zoom, cari. Mode Edit memunculkan perangkat anotasi. Peralihan lewat satu tombol jelas.
- Panel properti kanan muncul hanya saat objek terpilih.
- Status bar: halaman, zoom, ukuran berkas, status simpan, indikator render/OCR berjalan.
- **Command Palette** `Ctrl+Shift+P`.
- **Empty state** dirancang serius: logo, tombol buka, recent files bergambar, area drop jelas.

**Pintasan** (lengkap, bisa dilihat lewat `F1`, bisa diubah, disimpan di SQLite):
`Ctrl+O` · `Ctrl+W` · `Ctrl+Tab` · `Ctrl+S` · `Ctrl+Shift+S` · `Ctrl+F` · `Ctrl+Shift+F` · `Ctrl+Z/Y` · `Ctrl+0/+/-` · `Ctrl+P` · `F5` · `F11` · `Esc`

---

## 13. Target Performa

Ukur, jangan kira-kira. **Angka bertanda ⚠ divalidasi di Fase 0** dan boleh direvisi kalau terbukti di luar kemampuan PDFium — laporkan angka nyatanya, jangan paksakan lewat trik yang mengorbankan hal lain.

| Metrik | Target |
|---|---|
| Cold start sampai jendela siap | < 1 detik |
| ⚠ PDF 50 MB / 500 halaman sampai halaman pertama tampil | < 400 ms |
| Scroll | Mengunci di refresh rate monitor, nol frame drop, nol halaman putih |
| Zoom | Respons < 16 ms, versi tajam < 150 ms |
| Berpindah tab | < 50 ms |
| Pencarian dalam dokumen 500 halaman | < 200 ms |
| Pencarian pustaka (FTS5) | < 100 ms |
| RAM, 10 dokumen terbuka | < 1,5 GB termasuk cache |
| 50 dokumen terbuka | Tanpa degradasi terasa |

`bench/` berisi skrip dan PDF uji. Laporkan angka nyata tiap akhir fase.

---

## 14. Aksesibilitas

- Seluruh aplikasi bisa dioperasikan tanpa mouse.
- Focus ring jelas di semua elemen interaktif.
- Label ARIA di toolbar dan panel.
- Kontras teks minimal 4.5:1.
- Dukungan mode kontras tinggi Windows.
- Text layer terbaca screen reader.

---

## 15. Ketahanan & Keamanan

- Pekerja berjalan dalam Job Object dengan batas memori, token berhak rendah, tanpa akses jaringan.
- `catch_unwind` di setiap batas FFI PDFium.
- Batas waktu render per halaman; halaman hang dibatalkan dan ditandai.
- **Fuzzing** dengan PDF rusak, terpotong, terenkripsi, dan berukuran ekstrem. Aplikasi tidak boleh crash — hanya menolak dengan pesan jelas.
- PDF terenkripsi: dialog kata sandi. PDF rusak: perbaikan parsial bila memungkinkan.
- Dokumen "beracun" (menjatuhkan pekerja dua kali) diisolasi ke mode terbatas hanya-baca.
- Log terstruktur ke berkas lokal, dengan tombol "buka folder log" di About. Tidak ada yang dikirim ke mana pun.

---

## 16. Pengujian

v6.2 punya 21 test case yang semuanya lulus. **Semuanya dibawa ke v7 sebagai regression suite.**

- **Golden image testing** — wajib ada **sejak hari pertama Fase 3**, bukan di akhir. Tiap jenis anotasi punya berkas baseline sendiri. Jenis yang gagal paritas tidak dinyatakan selesai.
- Unit test untuk `izul-model` (display list deterministik), logika Rust lain, dan modul TypeScript murni.
- Test integrasi: buka → anotasi → simpan → buka ulang → **verifikasi objek masih editable**.
- Test isolasi: matikan paksa pekerja, pastikan UI bertahan, tab dipulihkan, dan draf tidak hilang.
- Test kebocoran memori: 200 siklus buka-tutup.
- Checklist manual untuk hal visual di `TESTING.md`.
- `test-fixtures/`: berkas besar, terenkripsi, rusak, berform, CJK, Arab (RTL), hasil scan, dan PDF dengan anotasi dari aplikasi lain.
- Tiap akhir fase: jalankan seluruh suite, laporkan apa adanya. Kegagalan tidak boleh disembunyikan.

---

## 17. Fase Pengerjaan

**Fase 0 — Fondasi, Batas Proses, dan Spike Pengukuran**
Setup Tauri v2, integrasi PDFium, kolam pekerja + supervisor + Job Object, protokol IPC dua kanal, shared memory, skema SQLite, pipeline build, CI dengan lint, `version.json`. Buka satu PDF, render satu halaman.
**Spike wajib:** ukur latensi buka PDFium pada berkas nyata (50 MB / 500 halaman, linearized dan non-linearized), throughput render, dan latensi transfer shared memory. Laporkan angkanya. Kalau target 400 ms di luar jangkauan, katakan sebelum apa pun dibangun di atasnya.
*Lulus bila:* `.exe` jalan di Windows bersih tanpa dependensi, mematikan paksa pekerja tidak menjatuhkan UI, dan angka spike sudah dilaporkan.

**Fase 1 — Mesin Viewer**
Pipeline render lengkap: dua tingkat, tile, cache LRU, prefetch, pembatalan. Scroll tervirtualisasi, semua mode tampilan, zoom, rotasi, text layer, sidebar thumbnail & outline.
*Lulus bila:* target performa tercapai dan terbukti lewat benchmark.

**Fase 2 — Multi-Dokumen & Pencarian**
Tab bar, DocumentSession, pemetaan ke kolam pekerja, manajemen memori tab tidak aktif, session restore, recent files, drag & drop, asosiasi berkas. Ekstraksi teks, indeks FTS5, dan pencarian tiga tingkat — dipindah ke sini karena ekstraksi teks sudah jadi di Fase 1 dan SQLite sudah ada; menambalnya belakangan jauh lebih mahal.
*Lulus bila:* 50 dokumen terbuka, RAM terkendali, nol kebocoran setelah 200 siklus buka-tutup.

**Fase 3 — Mesin Anotasi & Paritas**
Model objek, **display list**, backend kanvas, backend AP stream, seleksi & transformasi, command stack undo/redo, panel properti, panel daftar anotasi. Seluruh jenis anotasi Bagian 11.2.
*Lulus bila:* golden image membuktikan paritas saat objek diam untuk setiap jenis anotasi, ambang < 0,5%.

**Fase 4 — Tulis & Simpan**
Penulisan anotasi standar + AP stream + metadata, simpan atomik, flatten, ekspor, autosave, pemulihan crash.
*Lulus bila:* anotasi yang disimpan lalu dibuka ulang tetap editable, dan berkasnya tampil benar di Adobe Acrobat, Chrome, dan Edge.

**Fase 5 — Operasi Halaman, Split View, Banding**
Susun ulang, gabung, pecah, ekstrak, duplikat, seret halaman antar dokumen, split view 4 panel, mode banding dengan scroll tersinkron.

**Fase 6 — Redaksi Sejati**
Fase tersendiri karena PDFium tidak menyediakannya. Membedah content stream, membuang glyph dan objek gambar di dalam area terpilih, menyusun ulang stream, lalu memverifikasi konten benar-benar hilang lewat ekstraksi teks pada hasilnya.
*Lulus bila:* teks yang diredaksi tidak dapat ditemukan lagi oleh alat ekstraksi mana pun.

**Fase 7 — Kecerdasan**
OCR lokal (Indonesia + Inggris) dengan teks tersembunyi ditanam ke PDF, hapus background AI native dengan DirectML, pengisian form AcroForm.
**Penyuntingan teks asli dokumen — dibatasi tegas:** penggantian teks dalam baris yang sama, hanya bila font tertanam dan berisi glyph yang dibutuhkan, tanpa reflow paragraf. Di luar itu, tolak dengan pesan jelas. Ini kandidat pertama untuk dipotong kalau ongkosnya membengkak.

**Fase 8 — Penghalusan & Rilis**
Command palette, mode presentasi, dark mode + invert cerdas, aksesibilitas penuh, pintasan yang bisa diubah, ikon aplikasi, installer NSIS/MSI + portable, About, `PANDUAN.md`. Opsional: renderer WebGL2 bila benchmark menunjukkan manfaat nyata.

---

## 18. Laporan Tiap Akhir Fase

Format tetap, singkat:

1. Apa yang selesai, dan apa yang **tidak** selesai.
2. Angka benchmark nyata versus target.
3. Hasil test suite, termasuk yang gagal.
4. Keputusan teknis yang kamu ambil sendiri, beserta alasannya.
5. Yang perlu saya putuskan sebelum fase berikutnya.

---

## 19. Mulai Sekarang

Mulai dari Fase 0. Sebelum menulis kode, sampaikan rencana teknis dan pertanyaanmu, lalu tunggu persetujuan saya.
