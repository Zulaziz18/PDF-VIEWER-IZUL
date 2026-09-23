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
  - **Tujuh bug ditemukan dan sudah diperbaiki** lewat pengujian langsung di
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
    3. **Akar sebenarnya dari halaman putih** (perbaikan URI di atas perlu,
       tapi tidak cukup): `pump()` di `src-tauri/src/supervisor/worker.rs`
       memakai `tokio::select!` dengan `read_frame` sebagai salah satu
       cabangnya. `read_frame` **tidak cancel-safe** — ia membaca 4 byte
       panjang lalu isinya. Kalau perintah baru datang di antara keduanya,
       `select!` membuang future itu berikut byte panjang yang sudah
       terlanjur dibaca; pembacaan berikutnya mulai dari tengah pesan,
       salah menafsirkan isi sebagai panjang, lalu menunggu selamanya.
       Pekerja tampak diam, supervisor membunuhnya di detik ke-40
       (`pekerja diam terlalu lama, dimatikan worker=N silent=40.3` di log),
       restart, lalu rusak lagi pada semburan ubin berikutnya.
       Diperbaiki: pembacaan dipindah ke task tersendiri yang tidak pernah
       dibatalkan, menyalurkan frame utuh lewat `mpsc`; kedua cabang
       `select!` sekarang cancel-safe. Tiga test regresi ditambahkan di
       `worker.rs` — yang pertama terbukti GAGAL pada kode lama dan lulus
       pada yang baru.
       **Pelajaran penting:** seluruh test integrasi bicara ke pekerja
       secara berurutan (kirim lalu tunggu) dan tidak pernah melewati
       `pump()`, jadi bug ini tidak mungkin tertangkap sampai aplikasi
       sungguhan dijalankan. Kalau ada gejala "pekerja diam" atau balasan
       hilang di masa depan, curigai cancel-safety di jalur IPC lebih dulu.
    4. **Binari pekerja usang** — ini yang membuat halaman tetap putih
       walau ketiga perbaikan di atas sudah benar. Log pengguna menunjukkan
       aplikasi mengirim `Ping` tapi pekerja mencatat
       `perintah shutdown diterima` lalu keluar. Sebabnya `postcard` tidak
       self-describing: enum dikirim sebagai **indeks varian**, dan Fase 1
       menambah dua varian `Request` di tengah daftar, sehingga `Ping`
       bergeser dari indeks 8 ke 9 dan dibaca pekerja lama sebagai
       `Shutdown` (indeks 9 di daftar lamanya). `npm run tauri dev` hanya
       membangun ulang aplikasi, **tidak pernah** `izul-worker.exe`, jadi
       binari pekerja di mesin pengguna tertinggal satu fase.
       Diperbaiki tiga lapis: (a) `npm run dev`/`build` sekarang menjalankan
       `cargo build -p izul-worker` lebih dulu; (b) pekerja mengirim salam
       `Response::Hello { protocol }` begitu terhubung dan supervisor
       menolak yang tidak cocok dengan pesan yang menyuruh
       `cargo build --workspace`; (c) `izul_ipc::PROTOCOL_VERSION` wajib
       dinaikkan tiap kali bentuk `Request`/`Response` berubah.
    5. **Balasan ubin diblokir CORS** — ini penyebab terakhir halaman putih,
       ditemukan setelah keempat perbaikan di atas benar tapi halaman masih
       kosong. Halaman berjalan di `http://localhost:5173` (mode dev),
       sedangkan ubin diambil dari `http://izul.localhost` — lintas asal.
       Balasan tanpa `Access-Control-Allow-Origin` dibuang browser sebelum
       kode frontend melihatnya: `fetch` menolak dengan `TypeError: Failed to
       fetch` dan DevTools melaporkan "Response headers (0)", yang tidak bisa
       dibedakan dari permintaan yang tidak pernah dijawab. Protokol IPC dan
       aset bawaan Tauri memasang header ini sendiri; protokol yang
       didaftarkan tangan harus melakukannya sendiri. Setengah keduanya wajib:
       tanpa `Access-Control-Expose-Headers`, header `X-Izul-Width` terbaca
       `null` dan ubin yang sampai utuh tetap ditolak karena "tanpa dimensi".
       **Dibuktikan** dengan menjalankan Chromium sungguhan (Playwright) dan
       dua endpoint — tanpa header CORS menghasilkan `TypeError: Failed to
       fetch`, dengan header menghasilkan 200 dan header terbaca. Kalau ada
       protokol kustom baru ditambahkan nanti, pasang `cors_headers()` di
       **semua** balasannya termasuk yang galat, kalau tidak status 409/410
       pun tidak akan pernah sampai ke frontend.
    6. **Heartbeat membunuh pekerja yang belum pernah disapa.** `sweep()`
       memeriksa "diam berapa lama" **sebelum** mencoba ping, padahal diam
       hanya berarti "belum diajak bicara" — dan penyebab utamanya adalah
       supervisor sendiri, karena menghidupkan 8 pekerja butuh ~2 detik
       masing-masing di build debug. Akibatnya pada sapuan pertama semua
       pekerja terlihat diam 6-14 detik dan langsung dibunuh, lalu masuk
       lingkaran restart yang tidak pernah keluar (terlihat di log sebagai
       `silent=14.4s, 12.4s, 10.3s...` menurun 2 detik per pekerja — persis
       jarak spawn mereka). Diperbaiki: ping dulu, dan hanya vonis mati bila
       ping gagal **dan** sudah diam melewati ambang. Ping ke semua pekerja
       kini dilakukan serentak, supaya satu pekerja macet tidak menahan
       kunci kolam selama 8 x 6 detik — selama kunci itu dipegang, viewport
       tidak bisa merender apa pun.
    7. **Setiap ubin di atas zoom 100 % tergambar putih.** Ini cacat terakhir
       yang tersisa setelah keenam di atas benar, dan yang paling halus.
       `FPDF_RenderPageBitmapWithMatrix` **tidak** menerima matriks ruang
       pengguna. PDFium menyusun matriks tampilan halaman sendiri lebih dulu
       (yang sudah mengurangi `/MediaBox`, sudah menerapkan `/Rotate`, dan sudah
       membalik sumbu y), lalu menerapkan matriks kita **di atas** hasil itu.
       Matriks ubin lama mengerjakan ketiganya sekali lagi. Pada zoom 100 %
       faktor skalanya kebetulan 1,0 dan sebagian isi masih mendarat di bitmap
       — makanya ada tinta; begitu skalanya 1,5 atau 2,0 seluruh isi terdorong
       ke luar area klip dan ubin keluar putih bersih.
       Diperbaiki dengan menulis ulang `tile_matrix` di ruang yang benar
       (*ruang halaman*: titik, origin kiri-atas, y ke bawah, `/MediaBox` dan
       `/Rotate` sudah ditangani PDFium), sehingga yang tersisa hanya rotasi
       tambahan pengguna, offset rect sumber, dan skala.
       **Pelajaran penting:** header PDFium hanya menulis "the transform
       matrix, which must be invertible" — ruangnya tidak dijelaskan sama
       sekali, dan dua kali sesi ini rugi waktu karena menebaknya. Ruang itu
       akhirnya dipatok dengan **eksperimen**: render bermatriks identitas
       ternyata identik byte-per-byte dengan `FPDF_RenderPageBitmap`, yang hanya
       mungkin kalau PDFium mengalikan matriks tampilannya lebih dulu. Test
       `renders_identically_to_the_plain_api` di `crates/izul-pdf/src/render.rs`
       menjaga kesimpulan itu, ditemani halaman PDF sintetis (`corner_page`)
       dengan `/MediaBox` bergeser dan `/Rotate` 90/180/270 yang membuktikan
       tidak ada yang diterapkan dua kali. Ketiga belas test itu **gagal pada
       matriks lama** — sudah diperiksa dengan mengembalikannya sementara.
       Kalau nanti ada gejala "halaman putih hanya saat di-zoom", curigai ruang
       koordinat matriks lebih dulu, dan **ukur**, jangan mengingat.

    8. **`localhost` yang tersisa dari wry ditolak sebagai jenis sumber daya
       yang tidak dikenal — semua 1400 ubin ditolak dalam satu sesi, semua
       terkirim=0.** Ini yang akhirnya ditemukan lewat log lalu lintas ubin
       (lihat bug #9 di bawah) begitu instrumennya terpasang. `wry`
       menerjemahkan `http://izul.localhost/tile/...` bukan menjadi
       `izul://tile/...`, melainkan `izul://localhost/tile/...` — kode
       sumbernya sendiri (`custom_protocol_workaround.rs`) menyebut bentuk
       kanoniknya `{protocol}://localhost/abc`, jadi `localhost` memang selalu
       ikut. Pengurai kita mengasumsikan tidak ada authority sama sekali dan
       membaca `localhost` sebagai segmen pertama path — yang seharusnya
       `tile` — lalu menolaknya sebagai "jenis sumber daya tidak dikenal".
       Diperbaiki: authority `localhost/` dilucuti tepat di awal, sebelum
       path dibaca, dan hanya di posisi itu — supaya segmen path yang
       kebetulan bertulisan `localhost` lebih dalam tetap ditolak sebagaimana
       mestinya. Dijaga dengan test yang memakai URI persis dari log pengguna
       (`izul://localhost/tile/1/0/0/256/0/0/preview?g=2&p=0`), test yang
       memastikan keempat ejaan URI (dua bentuk `izul://`, `http://`,
       `https://`) mengurai ke kunci ubin yang sama, dan satu baris tambahan
       di test ujung-ke-ujung yang memakai bentuk Windows ini secara eksplisit
       — sebelumnya test itu hanya memakai bentuk `http://`, yang tidak
       pernah benar-benar dikirim di Windows, sehingga tetap hijau sementara
       Windows sungguhan menolak semuanya.
    9. **Instrumen yang membuat bug #8 ketemu.** Sebelum ini, `serve_tile`
       mencatat URI yang ditolak lewat `tracing::debug!` — dibuang saringan
       bawaan `info` — dan tidak mencatat sama sekali penolakan Superseded,
       SlotGone, atau NotFound, atau ubin yang berhasil. Log jadi terlihat
       persis sama baik ketika viewport tidak pernah meminta ubin maupun
       ketika backend menolak semuanya — dua kemungkinan yang paling perlu
       dibedakan, dan satu-satunya diagnosis yang bisa diminta dari pengguna
       tanpa membuka DevTools. Tiga putaran pelaporan "halaman putih"
       sebelumnya tidak konklusif karena ini, bukan karena dugaan yang
       kurang tajam. Ditambahkan: tiap permintaan ubin dihitung menurut
       hasilnya (terkirim/uri ditolak/generasi lama/slot hilang/tidak
       ada/pekerja gagal), tiga yang pertama dari tiap jenis dicatat utuh,
       sesudahnya ringkasan tiap 200 permintaan.
       **Pelajaran penting:** kalau laporan bug lewat beberapa putaran tidak
       konklusif, curigai alat ukurnya sebelum mempertajam dugaan lebih jauh.

## Keadaan CI

Sejak PR #1, **CI hijau penuh untuk pertama kalinya** di repositori ini: Rust
di ubuntu dan windows, Frontend, dan Version consistency. Sebelumnya selalu
merah — enam run terakhir di branch basis gagal karena dua cacat yang tidak
pernah diperbaiki, dan karenanya tahap Test tidak pernah dijalankan sama
sekali di Linux, dan tidak pernah di Windows.

Yang perlu diingat saat CI merah lagi nanti:

- **Tiap perbaikan membuka tahap berikutnya, dan tahap itu punya cacatnya
  sendiri.** Lima cacat beruntun ditemukan begitu, satu per push. Kalau CI
  baru saja lolos ke tahap yang belum pernah dijalankan, harapkan ia gagal —
  itu bukan tanda perbaikan sebelumnya salah.
- **Lingkungan pengembangan di sini punya kedua pohon PDFium** (linux-x64 dan
  win-x64), sedangkan runner hanya punya miliknya sendiri sampai CI diperbaiki
  supaya mengambil keduanya. Suite yang lulus di sini karena itu tidak
  membuktikan CI hijau. Kalau ada kegagalan yang hanya muncul di CI, curigai
  asimetri lingkungan lebih dulu.
- **Cara mensimulasikan Windows dari sini:** ganti sementara semua
  `#[cfg(unix)]`/`#![cfg(unix)]` di berkas test jadi `cfg(any())`, lalu
  jalankan `cargo clippy --workspace --all-targets -- -D warnings`. Itu
  memunculkan galat unused-import yang sama persis dengan yang dilaporkan CI
  Windows, tanpa perlu mesin Windows.
- **Harness dan benchmark ikut dibangun di kedua runner.** `cargo build
  --workspace` dan `cargo clippy --workspace --all-targets` mencakup `bench/`
  dan `tests-integration/`, jadi kode yang hanya benar di Unix di sana adalah
  CI Windows merah — bukan sekadar test yang dilewati. Ini yang menjatuhkan
  `Rust (windows-latest)` pada push pertama Fase 2: `bench/src/multidoc.rs`
  menyebut `tokio::net::UnixStream` langsung. Berkas test punya `#![cfg(unix)]`
  yang menjaganya; binari benchmark tidak, dan tidak bisa punya — sebuah
  `[[bin]]` tetap dibangun. Pilih tipe per platform (seperti `izul-ipc`
  melakukannya di dalam `transport.rs`) sejak baris pertama ditulis.
- **Jalankan `cargo test --workspace --no-fail-fast`** sebelum push. Tanpa itu
  cargo berhenti di binari test pertama yang gagal, dan kegagalan berikutnya
  baru terlihat satu putaran CI kemudian.

**Cakupan yang belum ada:** seluruh test integrasi memakai soket Unix dan
digerbangi `#![cfg(unix)]`, jadi di Windows berkas-berkas itu kosong. Yang
benar-benar berjalan di Windows hanya test unit dan test piksel `izul-pdf`.
Padahal Windows-lah platform yang dikirim, dan ketujuh bug Fase 1 ditemukan di
sana oleh pengguna, bukan oleh test. Menutupnya berarti memberi harness jalur
named pipe di samping soket unix — pekerjaan tersendiri, sudah dicatat di
`tests/render_pipeline.rs` dan `src-tauri/tests/render_end_to_end.rs`.

## Cara kerja yang terbukti berguna di proyek ini

Tiga dari delapan cacat sesi ini lahir dari menebak perilaku pustaka pihak
ketiga dari ingatan. Yang menyelesaikannya selalu salah satu dari:

- **Membaca kode sumber yang benar-benar terpasang** (`wry` di `~/.cargo`,
  header PDFium di `vendor/pdfium/*/include`), bukan dokumentasi dari ingatan.
- **Menjalankan eksperimen kecil yang jawabannya cuma satu bit** — Chromium
  sungguhan lewat Playwright untuk CORS, matriks identitas untuk ruang
  koordinat PDFium.
- **Membuktikan test regresinya gagal pada kode lama** sebelum percaya ia
  menjaga sesuatu.

Kalau sebuah dugaan tidak bisa diuji dalam sepuluh menit, itu tanda dugaannya
belum cukup tajam — bukan tanda harus dicoba di komputer pengguna.

## Keadaan Fase 2 (Multi-Dokumen & Pencarian)

Dikerjakan sekaligus — bagian multi-dokumen dan bagian pencarian — atas
keputusan pengguna, di branch `claude/pdf-studio-izul-v7-fase-2`. **Seluruh
cakupan fase ini selesai**; yang tersisa hanya persetujuan pengguna sebelum
Fase 3.

**Lapisan Rust (dari putaran pertama):**

1. `izul-store/sessions.rs` — mengisi tabel `sessions`/`session_tabs`. Satu
   baris sesi per sekali jalan; susunan tab ditulis ulang di tempat.
   `latest()` **sengaja melewati sesi kosong**: startup membuat sesi baru
   sebelum memulihkan yang lama, dan tanpa saringan itu baris kosong yang baru
   jadi "paling baru" lalu menghapus susunan yang mau dipulihkan.
2. `izul-store/search.rs` + migrasi `app_002_index_state.sql` — teks halaman ke
   `doc_text`. `doc_index_state` menjawab apakah indeks masih cocok dengan
   berkas di disk dan sampai halaman berapa pengindeksan sempat berjalan.
3. `izul-pdf/find.rs` — membungkus `FPDFText_FindStart`. Kotak sorot
   dikembalikan sebagai **daftar**, bukan satu kotak.
4. `Request::Search` di pekerja — `PROTOCOL_VERSION` naik 2 → 3.

**Yang ditambahkan di putaran kedua:**

5. `src-tauri/src/workspace.rs` — registri dokumen terbuka: urutan tab, fokus,
   dan baris `session_tabs` yang ditulis darinya. Urutan tab **adalah** urutan
   vektornya; field `order` terpisah akan jadi sumber kebenaran kedua yang
   melenceng saat close dan reorder berbalapan.
6. `src-tauri/src/indexing.rs` — pengindeksan latar per halaman, dapat
   dilanjutkan dan dibatalkan, jeda 15 ms antar halaman, komit tiap 16 halaman.
7. `src-tauri/src/textsearch.rs` — jalur regex dan geometri sorotan (rentang
   karakter → kotak per baris). Alasan regex harus jalur terpisah ditulis
   panjang di kepala berkas itu.
8. `src-tauri/src/thumbs.rs` — sampul berkas terakhir, PNG di folder data,
   direkam saat dokumen dibuka. Base64-nya ditulis tangan dan diuji terhadap
   vektor RFC, bukan terhadap dirinya sendiri.
9. Perintah Tauri baru: `list_tabs`, `activate_document`, `reorder_tabs`,
   `set_tab_pinned`, `restore_session`, `startup_files`, `pin_recent`,
   `index_document`, `index_progress`, `search_page`, `search_document`,
   `search_library`, `search_regex_page`.
10. Frontend dipecah: `src/state/documentSession.ts` (satu store per dokumen,
    dibuat dari balasan `open_document`) + `src/state/workspaceStore.ts`
    (daftar tab, fokus, kebijakan memori). `documentStore.ts` sekarang tinggal
    lapisan tipis yang berlangganan ke sesi yang aktif — itulah yang membuat
    komponen Fase 1 tidak perlu diubah satu per satu.
11. `TabBar.tsx`, `SearchPanel.tsx`, `dropTarget.tsx`, `EmptyState` bersampul,
    `src/viewport/highlights.ts` + integrasinya di `renderer.ts`.
12. `bench/src/multidoc.rs` — kriteria lulus fase ini, diukur.

**Keputusan teknis yang diambil sendiri, beserta alasannya:**

- **Pencarian per-halaman, bukan per-dokumen.** Pekerja yang pergi mencari di
  500 halaman berhenti menjawab heartbeat dan dibunuh di detik keenam
  (SPEC 3.4). Indeks FTS5 menjawab "halaman mana", pekerja menjawab "di sebelah
  mana". Ini juga yang membuat pencarian bisa dibatalkan per ketukan.
- **`Response::SearchReady` ditambahkan di ujung enum**, karena `postcard`
  mengenali varian lewat indeksnya (bug #4 di atas).
- **Terjemahan ketikan pengguna ke sintaks FTS5 ditangani serius.** Tiap token
  dibungkus kutip, kutip di dalamnya digandakan; ada test yang melempar empat
  belas bentuk ketikan bermasalah.
- **Tiga tab tetap "hangat", bukan satu.** Pembaca yang membandingkan dua
  dokumen membolak-balik keduanya tiap beberapa detik; menyusutkan tiap pindah
  berarti merender ulang keduanya setiap kali. Aturannya murni (`tabsToTrim`)
  supaya bisa diuji tanpa menonton grafik memori.
- **Sesi ditulis tiap kali berubah, bukan saat keluar.** Kasus yang membuat
  session restore ada justru kasus aplikasi tidak ditutup baik-baik.
- **Sampul direkam saat dokumen dibuka, bukan saat daftar digambar.** Membuat
  sampul berarti membuka PDF; daftar dua puluh berkas akan membuka dua puluh
  dokumen untuk panel yang belum diklik siapa pun.
- **Seret & lepas lewat kanal Tauri, bukan DOM.** Drag-and-drop webview
  menyerahkan objek `File` tanpa path, sedangkan pekerja membuka berkas lewat
  path.

**Temuan pengukuran yang perlu diingat:** `Trim` **melepas** seluruh pegangan
halaman (dijaga test `releasing_the_pages_empties_the_page_cache`), tetapi
resident set pekerja **tidak turun** — PDFium menyimpan arena alokatornya. Yang
dibeli `Trim` di sisi pekerja adalah memori yang dapat dipakai ulang, bukan RAM
yang kembali ke sistem. Sisi proses UI berbeda: di sana bitmap benar-benar
dibuang dari cache ubin. Kalau nanti ada yang melaporkan "Trim tidak
menghemat apa-apa", inilah jawabannya — dan angkanya ada di
`bench/results/phase2-linux.txt`.

**Single-instance sudah ada** (diminta pengguna setelah laporan fase):
`tauri_plugin_single_instance` didaftarkan **paling awal** di builder — ia yang
memutuskan apakah proses ini adalah aplikasinya sama sekali — dan meneruskan
argumen ke instance yang sudah jalan lewat event `izul://open-files`;
`src/app/openFiles.ts` yang mendengarkannya.

**Jumlah dokumen di benchmark:** SPEC Bagian 17 menulis kriterianya *50
dokumen*, tetapi pengguna meminta angka yang dilaporkan adalah **30**. Harness
sekarang menerima `--documents N` dengan bawaan 30, dan
`bench/results/phase2-linux.txt` memuat **keduanya** — menghapus angka 50 berarti
berkas itu diam-diam berhenti menjawab kriteria yang tertulis di SPEC. Kalau
kriterianya memang mau diturunkan, itu perubahan SPEC dan perlu dibahas.

**Yang sengaja tidak dikerjakan di fase ini:** dua tab untuk satu berkas
(menunggu split view di Fase 5); test integrasi Windows (masih `#![cfg(unix)]`,
sama seperti Fase 1).

**Cacat harness yang ditemukan dan diperbaiki (bukan bug aplikasi):** suite
test `izul-pdf` mati dengan SIGSEGV begitu test yang memakai PDFium bertambah.
Dua sebab: tiap modul test memegang `OnceLock<Engine>` sendiri — `Engine::load_from`
menolak panggilan kedua, jadi modul yang kalah start **melewati seluruh
tesnya tanpa suara** — dan tidak ada yang menjaga aturan satu-thread yang
dipatuhi produksi. Diperbaiki dengan `izul-pdf/src/test_support.rs`: satu engine
untuk seluruh binari test, dan `pdfium_lock()` yang wajib dipegang selama
sebuah `Document` hidup. **Kalau nanti menambah test yang membuka `Document`
di crate itu, pakai `engine_and_lock!()` — jangan bikin engine sendiri.**

**Catatan lingkungan:** membangun `izul-app` di kontainer Linux butuh
`libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev patchelf` (sama seperti CI).
Tanpa itu `cargo check -p izul-app` gagal di `gdk-sys`, bukan di kode kita.

## Keadaan Fase 3 (Mesin Anotasi & Paritas)

Diminta pengguna langsung setelah Fase 2, dikerjakan **di branch yang sama**
(`claude/pdf-studio-izul-v7-fase-2`) karena aturan branch sesi ini melarang
push ke branch lain tanpa izin eksplisit. Akibatnya PR #2 memuat dua fase;
kalau pengguna lebih suka terpisah, itu perlu branch baru dan izinnya.

**Lapisan model (`izul-model`, murni, tanpa PDFium dan tanpa I/O):**

1. `annot.rs` — tiga belas jenis SPEC 11.2 sebagai satu enum tertutup dengan
   payload per jenis. Koordinat selalu ruang PDF.
2. `build.rs` — `display_list(obj, font_ctx)`, murni dan deterministik. **Semua**
   yang menggoda untuk diserahkan ke backend diputuskan di sini: penghalusan
   tinta jadi Bézier eksplisit, kepala panah jadi jalur, elips jadi empat kurva,
   teks jadi glif berposisi, rotasi jadi satu transform.
3. `font.rs` — `FontCtx` sebagai trait; model tidak boleh menyentuh berkas font.
4. `ap.rs` — backend AP stream. Byte deterministik (satu formatter angka),
   state seimbang, dan penulisnya menutup apa pun yang tidak seimbang alih-alih
   memercayainya.
5. `ops.rs` — `Op` yang tahu kebalikannya, satu gestur satu transaksi, rollback
   penuh saat transaksi gagal di tengah, batas 200 langkah.

**Lapisan aplikasi:**

6. `izul-pdf/src/fonts.rs` — metrik standard-14 **diukur dari PDFium**.
   Asumsinya (kode karakter = indeks glif) diuji dengan render sungguhan di
   `the_measured_widths_match_what_pdfium_draws`, bukan dipercaya dari ingatan.
7. `Request::FontMetrics` di pekerja — `PROTOCOL_VERSION` naik 3 → 4. Proses UI
   tidak boleh menaut PDFium, jadi ia bertanya dan menyimpan jawabannya.
8. `src-tauri/src/annots.rs` — `AnnotDoc` + `CommandStack` per dokumen, cache
   metrik font, dan registry gambar. Di proses UI, bukan di pekerja: pekerja
   bisa dibunuh supervisor kapan saja dan anotasi yang belum disimpan tidak
   boleh ikut mati.
9. Rute protokol `izul://image/{doc}/{ref}` untuk piksel gambar.

**Frontend:**

10. `src/annots/canvas.ts` — backend kanvas, konsumen kedua daftar yang sama.
11. `src/annots/interaction.ts` — uji tembak, pegangan, resize, rotasi, murni.
12. `src/annots/factory.ts` — gestur jadi objek, murni.
13. Gestur dan chrome seleksi di `renderer.ts`; toolbar, panel properti, dan
    daftar anotasi di `src/app/`.

**Kriteria lulus, diukur:**

- Golden image: 15 baseline, semua lulus < 0,5 persen (SPEC 3.3). Di
  `crates/izul-pdf/golden/`, dijalankan CI.
- Paritas kanvas: `tools/canvas-parity/run.mjs`, butuh Chromium sungguhan, tidak
  di CI. **Dua belas jenis non-teks di bawah 0,53 persen.** Tiga kasus berteks
  dikecualikan dari angka itu sejak baseline pindah ke font uji Type 3 — kanvas
  menggambar huruf sungguhan, baselinenya blok, jadi membandingkan pikselnya
  tidak berarti; yang masih berarti di sana cakupan tintanya (56,5 vs 56,6
  persen), artinya posisinya sama. Angka lengkapnya di
  `bench/results/phase3-parity.txt`.

**Cacat nyata yang ditemukan harness paritas:** anotasi gambar berbeda 30,7
persen karena kanvas menghaluskan gambar yang diperbesar sementara PDFium
menampilkan pikselnya — pergantian yang terlihat saat proksi digantikan render
otoritatif. Diperbaiki; sesudahnya 0,00 persen.

**Pelajaran CI yang mahal dan baru:** golden image untuk teks **tidak boleh**
memakai font sistem. Versi pertama memakai metrik standard-14 dari PDFium dan
glif Helvetica/Times; lulus di Linux, gagal di Windows pada ketiga baseline
berteks (1,6–3,0 persen, ambang 0,5) sementara dua belas lainnya lulus tanpa
disentuh. PDFium tidak membawa satu set outline lintas platform. Diperbaiki
dengan memindahkan kedua sisinya ke dalam `golden.rs`: metrik `FixedFont` dan
font **Type 3** yang charproc-nya ditulis di sana. Kalau nanti menambah baseline
yang memuat teks, jangan kembali ke font sungguhan sebelum Fase 4 menanam font
ke dalam PDF-nya.

**Jebakan harness yang sempat memakan waktu:** Chromium membatasi
`--window-size` (jendela 480x240 melaporkan `innerHeight` 153), jadi
`--screenshot` mengembalikan gambar yang terpotong dan itu **terlihat persis
seperti bug rendering**. Harness sekarang membandingkan piksel **di dalam
halaman** lewat `--dump-dom`, dan butuh `--allow-file-access-from-files` karena
tanpa itu gambar `file://` mencemari kanvas dan `getImageData` melempar.

**Cacat UI yang dilaporkan pengguna saat menguji Fase 2/3, sudah diperbaiki:**
jendela `decorations: false` tidak punya tombol perkecil/perbesar/tutup sama
sekali — bilah judul kita tidak pernah menggambarnya, jadi satu-satunya jalan
keluar adalah Alt+F4. Dan tombol "Buka Berkas" hanya ada di layar kosong, jadi
begitu satu dokumen terbuka tidak ada cara terlihat untuk membuka yang kedua
(hanya `Ctrl+O`, yang tidak seorang pun tahu). Keduanya kelas kesalahan yang
sama: **fitur yang hanya bisa dicapai lewat pintasan atau tidak bisa dicapai
sama sekali**. Perintah jendela juga butuh izin eksplisit di
`capabilities/default.json` (`core:window:allow-minimize`,
`allow-toggle-maximize`, `allow-close`, `allow-start-dragging`) — tanpa itu
tombolnya diam saja, persis bug #1 Fase 1.

**Tab berganda: tiga berkas jadi enam tab.** Dilaporkan pengguna sesudahnya.
Penjaga "sudah terbuka" di `openFile` membaca daftar tab, dan sebuah tab baru
ada di daftar itu **setelah** `open_document` menjawab — jadi dua pemanggil
yang tumpang tindih sama-sama melihat daftar kosong dan sama-sama membuka.
Kedua pemanggilnya ternyata satu kode: efek startup di `App.tsx` yang
dijalankan dua kali oleh mount ganda `StrictMode` React. Diperbaiki dua lapis
— peta "sedang dibuka" per path di `openFile` (menutup lubangnya untuk semua
pemanggil, bukan hanya StrictMode) dan penjaga sekali-per-proses di efek
startup. **Pelajaran:** penjaga "sudah ada" yang membaca state yang baru terisi
setelah sebuah `await` bukan penjaga; yang menjaga adalah pendaftaran niat
**sebelum** await-nya. Mount ganda StrictMode adalah alat, bukan gangguan —
ia yang menjaring ini.

**Yang belum dikerjakan di Fase 3:** menyimpan ke PDF (itu Fase 4 — anotasi
masih hidup di memori sampai tab ditutup); penyuntingan teks langsung di atas
halaman (isinya diketik lewat panel properti); dan **UI-nya belum pernah
dijalankan di jendela sungguhan** karena kontainer ini tidak punya layar.

## Keadaan Langkah 0 (kerangka UI gaya WPS, 23 September 2026)

Pengguna memutuskan UI meniru WPS Office semirip mungkin; SPEC Bagian 12 sudah
ditulis ulang (bertanggal, dengan alasan). SPEC Bagian 2 tidak berubah.

- **Kerangka:** `TitleBar.tsx` (tab Beranda tetap + tab dokumen + "Baru" +
  tombol jendela), `Ribbon.tsx` (`MenuBar` + `Ribbon`: tab Beranda/Edit/
  Komentar), `Sidebar.tsx` (`LeftRail` + panel; pencarian kini tab sidebar),
  `BottomBar.tsx`, `Home.tsx` (Terbaru/Berbintang/PC Ini/Desktop/Dokumen/
  Unduhan/Sering Dipakai + Info Berkas), `About.tsx`. Toolbar, AnnotToolbar,
  TabBar, StatusBar, EmptyState lama dihapus.
- **Aturan tab pita:** sebuah tab/tombol hanya ditambahkan ketika fiturnya
  benar-benar bekerja (`RibbonTab` di `src/state/uiStore.ts`). Fase berikutnya
  menambah Halaman, Lindungi, Konversi, Isi & Tanda Tangan di situ.
- **Logika tombol** ada di `src/app/actions.ts`, bukan di komponen. Stabilo
  tanpa seleksi teks *mensiagakan* alat (`useUi.markup`) dan
  `armedMarkup.ts` menerapkannya ke seleksi berikutnya.
- **Ikon:** Fluent UI System Icons (MIT), `npm run icons` menyalin path yang
  dipakai ke `src/design/icons.generated.ts`. Ditampilkan dua nada (filled
  tipis + regular) lewat `Icon.tsx`. Menambah ikon = tambah baris di
  `tools/icons/build.mjs` lalu jalankan ulang (butuh jaringan sekali).
- **Tema gelap** baru benar-benar tersambung sekarang (`src/design/theme.ts`);
  sebelumnya token `data-theme="dark"` ada tapi tidak pernah dipasang.
- **Rust baru:** `src-tauri/src/folders.rs` + perintah `known_folders`,
  `browse_folder`; `recent_files` kini membawa `size` dan `modified`.
  **Semua cap waktu di store dalam detik** (`as_secs()`), bukan milidetik.

**Harness screenshot — `npm run ui:shots`** (`tools/ui-harness/`): Vite dev
server + `mockIPC`, Chromium lewat `playwright-core` (dipatok 1.56.1, cocok
dengan `/opt/pw-browsers/chromium-1194`), ubin dijawab lewat `page.route` oleh
`izul-bench --bin ui-harness` yang merender dengan PDFium sungguhan, dan
anotasi contoh dibangun oleh `izul_model::build::display_list` yang sama.
Sampel PDF dibuat `tools/ui-harness/make_samples.py` (isi karangan sendiri).
Opsi: `--scene=home,document,edit,comment`, `--size=1366x768`,
`--theme=light|dark`, `--scale=1.5`, `--out=...`, `--serve`. **Tiap UI baru
wajib ditambah scene-nya di `SCENES` dalam `shoot.mjs`** dan dilihat sebelum
dinyatakan selesai. Mock hanya di `tools/ui-harness/`; build produksi tidak
pernah menyentuhnya.

**Cacat yang ditemukan harness:** (1) membuka dokumen menggulir halaman
pertama ~40 px ke bawah — zoom tanpa kursor menahan titik tengah, padahal
pembaca di paling atas mengharapkan tetap di atas. Diperbaiki dengan
`zoomHoldingView` (test terbukti gagal pada aturan lama). (2) Halaman putih
tanpa tepi hilang di kanvas terang — kini ada garis tepi + bayangan tipis.
(3) Kontras tombol aksen di mode gelap 2,6:1 — token `--izul-on-accent`.

## Keadaan Fase 4 (Tulis & Simpan, 23 September 2026)

Selesai di branch `claude/pdf-studio-izul-v7-fase-4` (PR #3, base
`claude/pdf-studio-izul-v7-fase-2`). Laporan SPEC 18 ada di badan PR #3.

**Arsitektur simpan** (rincian di kepala `src-tauri/src/saving.rs`):
pekerja membuka **salinan kerja**, menaruh penanda `/NM (izul-<id>)`, PDFium
menulis berkas utuh (`FPDF_SaveAsCopy`, `FPDF_NO_INCREMENTAL`), proses UI
menambah **satu bagian pembaruan inkremental** (`crates/izul-write`) berisi
anotasi standar + AP dari display list + `/IzulObj` (JSON ASCII, versi 1).
Ditulis ke temp di samping target, `fsync`, **diverifikasi pekerja**, baru
di-rename. Dokumen tampilan **melepas** anotasi Izul saat halaman pertama
dimuat (`strip_izul`), lalu editor menggambarnya dari objek — kalau tidak,
tergambar dua kali.

**Temuan terukur tentang PDFium (jangan ditebak ulang):**

- `FPDFAnnot_SetAP` hanya menulis `/GS` dari opasitas dengan BM Normal — tidak
  cukup untuk AP kita, makanya AP ditulis sendiri di bagian inkremental.
- Simpan penuh PDFium **selalu** xref klasik dengan trailer langsung, dan
  **mempertahankan nomor objek lama**. Anotasi baru ditulis **inline** di
  `/Annots` halaman (bukan objek tak langsung) — `Placement::Inline` ada karena
  ini; versi pertama gagal membuka halaman 0 karena menganggap semuanya objek.
- `FPDFImageObj_GetImageDataRaw` = byte stream mentah (JPEG asli utuh);
  `GetRenderedBitmap` setelah `SetMatrix(w,0,0,h)` = BGRA alfa lurus.
- Windows **menolak mengganti berkas yang sedang di-mmap** → `Pool::release`
  (Close) sebelum rename, `Pool::reload` sesudahnya.
- Jangan tulis `/CA` pada kamus anotasi: opasitas sudah dibakar ke AP, dan
  Acrobat akan mengalikannya dua kali.

**Cacat yang ditemukan dan pelajarannya:**

1. **Simpan kedua menghapus FreeText.** `write_set` diam-diam membuang objek
   tanpa metrik font di cache — dan objek hasil impor tidak pernah punya.
   Sekarang galat, dan metrik disiapkan untuk semua objek dulu. **Pelajaran:**
   "lewati yang tidak bisa ditulis" di jalur simpan = kehilangan data diam-diam.
   Gagal keras.
2. **"5 0 R7 0 R"** — referensi yang ditempel tanpa spasi membuat `/Annots`
   tak terbaca; 0 dari 13 anotasi kembali. Selalu spasi di sekitar referensi.
3. **Titik "belum disimpan" memakai `canUndo`.** Salah dua arah: dokumen yang
   baru disimpan masih bisa di-undo (ditanya percuma), dan undo melewati titik
   simpan membuat dokumen kotor lagi. Kini `dirty` dari backend (`revision !=
   saved`). Test `closes a saved document without asking` terbukti gagal pada
   aturan lama.
4. **Autosave vs "Jangan Simpan".** Autosave yang jalan di antara membuang draf
   dan menutup tab menulisnya lagi. `draft_discard` kini juga `mark_drafted`.
5. **Ekspor ke berkas yang terbuka di tab lain** — rename gagal di Windows,
   diam-diam sukses di tempat lain. Kini ditolak (`refuse_open_target`).

**Alat ukur yang berguna, dan satu yang menipu:**

- `python3` + `pikepdf`/`pypdf` (strict) untuk struktur; `pdftoppm` (poppler)
  dan `mutool` (MuPDF) untuk **mesin render yang bukan PDFium** — Chrome dan
  Edge memakai PDFium, jadi keduanya bukan bukti independen. Pasang lewat
  `apt-get install poppler-utils mupdf-tools`.
- Mata membaca screenshot yang diperkecil: saya sempat "melihat" latar dialog
  tidak meredup; nilai piksel (255 → 178) membuktikan sebaliknya. **Ukur
  piksel sebelum memperbaiki cacat visual.**
- Tabel test Fase 3 di TESTING.md ternyata salah hitung (85/63 vs 67/48
  sebenarnya, diverifikasi dengan menjalankan commit lama di worktree). Hitung
  dari keluaran `cargo test`, jangan dari ingatan.

**Frontend Fase 4:** logika di `src/app/fileActions.ts` (simpan, tutup dengan
konfirmasi, draf, pantau berkas, ekspor) — komponen hanya memanggil.
`PromptDialog` (tiga tombol; dialog platform hanya dua), `NoticeToast`,
`ExportDialog` (rentang halaman lewat `src/state/pageRange.ts`, teruji),
`FileBanner`. Pintasan: Ctrl+S, Ctrl+Shift+S, Ctrl+W bertanya dulu. Penjaga
tutup jendela lewat `onCloseRequested` → butuh `core:window:allow-destroy`.
Scene harness baru: `convert, export, close, draft, changed, exports`.

**Belum dikerjakan / diketahui:** belum diuji di Acrobat/Edge sungguhan
(langkahnya di TESTING.md "Hasil Fase 4"); font standard-14 tidak ditanam
(pembaca lain memakai padanan); dokumen terenkripsi tidak bisa disimpan;
`npm audit` 2 moderate di `vitest` (dev saja, sudah ada sebelumnya).

## Alur kerja proyek ini

- Branch per fase: `claude/pdf-studio-izul-v7-fase-4` (Langkah 0 + Fase 4,
  PR #3) bercabang dari `claude/pdf-studio-izul-v7-fase-2`; Fase 5 dikerjakan
  di `claude/pdf-studio-izul-v7-fase-5` yang bercabang dari fase-4, dan
  seterusnya — satu draft PR per fase, base = branch fase sebelumnya. Pengguna mengizinkan
  branch `claude/pdf-studio-izul-v7-fase-N` per fase, masing-masing bercabang
  dari fase sebelumnya dengan draft PR ber-base fase sebelumnya, dan meminta
  Fase 4–8 dikerjakan berturut-turut tanpa menunggu persetujuan (tetap wajib
  laporan SPEC 18, CI hijau, dan CLAUDE.md diperbarui tiap akhir fase).
- Sebelumnya: `claude/pdf-studio-izul-v7-fase-2`.
- Trunk proyek ini **bukan** `main` — tidak ada branch `main`. Trunk-nya
  `claude/pdf-studio-izul-v7-atlas-r29mdh`, dan Fase 1 sudah di-merge ke sana
  lewat PR #1.
- Dokumen rujukan: `SPEC.md` (jangan diubah tanpa dibahas). Progres per fase
  dicatat di `CHANGELOG.md`. Panduan pengguna di `PANDUAN.md`.
- Setiap akhir fase: laporkan hasil + angka benchmark nyata (SPEC 18). Untuk
  Fase 4–8 pengguna **membebaskan** jeda persetujuan; tetap wajib laporan
  SPEC 18, CI hijau, CHANGELOG, version.json, TESTING.md, dan bagian
  "Keadaan Fase N" di berkas ini sebelum lanjut.
- Total 9 fase (0–8). Fase 0 dan 1 selesai dan disetujui pengguna; Fase 2 dan
  Fase 3 selesai (satu branch, PR #2); Langkah 0 dan Fase 4 selesai (PR #3).
  Fase 5–8 dikerjakan berturut-turut tanpa menunggu persetujuan, atas
  keputusan pengguna.
- Panduan menjalankan & menguji aplikasi di Windows (untuk pemula) ada di
  `TESTING.md`, bagian "Menjalankan sendiri di Windows (langkah demi
  langkah)" — termasuk cara memasang alat, mengambil PDFium, menjalankan
  `npm run tauri dev`, dan melihat frame rate lewat DevTools.
