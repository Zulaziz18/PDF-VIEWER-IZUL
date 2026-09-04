/**
 * Bahasa Indonesia — the default and, for now, only locale.
 *
 * Every user-visible string in the application lives here (SPEC 0). A string
 * literal in a component is a bug, because it is the one that gets missed when
 * a second language is added.
 */
export const id = {
  "app.name": "PDF Studio Izul",
  "app.tagline": "Editor PDF offline",

  "empty.title": "Belum ada dokumen terbuka",
  "empty.subtitle": "Buka berkas PDF, atau lepaskan berkas ke jendela ini.",
  "empty.open": "Buka Berkas",
  "empty.recent": "Berkas Terakhir",
  "empty.noRecent": "Belum ada riwayat.",
  "empty.missing": "Berkas tidak ditemukan di lokasi terakhir",

  "toolbar.sidebar": "Panel samping",

  "nav.page": "Halaman",
  "nav.previous": "Halaman sebelumnya",
  "nav.next": "Halaman berikutnya",

  "zoom.in": "Perbesar",
  "zoom.out": "Perkecil",
  "zoom.fitWidth": "Lebar",
  "zoom.fitPage": "Muat",
  "zoom.actual": "100%",

  "rotate.left": "Putar kiri",
  "rotate.right": "Putar kanan",
  "rotate.page": "Putar halaman ini",

  "view.single": "Satu",
  "view.dual": "Dua",
  "view.dualCover": "Dua + sampul",
  "view.horizontal": "Mendatar",

  "sidebar.label": "Panel samping",
  "sidebar.thumbnails": "Halaman",
  "sidebar.outline": "Daftar Isi",
  "sidebar.noOutline": "Dokumen ini tidak punya daftar isi.",
  "sidebar.untitled": "(tanpa judul)",

  "viewport.label": "Tampilan dokumen",
  "viewport.textLayer": "Lapisan teks",

  "status.page": "Halaman",
  "status.of": "dari",
  "status.zoom": "Perbesaran",
  "status.workers": "Pekerja",
  "status.rendering": "Merender…",
  "status.ready": "Siap",
  "status.cache": "Cache",
  "status.cacheHit": "Kena cache",
  "status.encrypted": "Terenkripsi",

  "about.title": "Tentang",
  "about.version": "Versi",
  "about.pdfium": "Mesin PDF",
  "about.phase": "Fase",
  "about.openLogs": "Buka folder log",
  "about.sandboxReal": "Pekerja berjalan dalam Job Object Windows",
  "about.sandboxDev": "Pengurungan pengembangan (bukan batas keamanan)",

  "err.open": "Dokumen tidak dapat dibuka",
  "err.pdf.password_required": "Dokumen ini memerlukan kata sandi.",
  "err.pdf.password_wrong": "Kata sandi salah.",
  "err.pdf.file_not_readable": "Berkas tidak dapat dibaca.",
  "err.pdf.corrupt": "Dokumen rusak.",
  "err.pdf.engine_panic": "Mesin PDF berhenti tak terduga pada dokumen ini.",
  "err.pdf.library_load": "Pustaka PDFium tidak dapat dimuat.",
  "err.not_in_this_phase": "Fitur ini belum tersedia pada fase ini.",
  "err.workerGone": "Pekerja berhenti. Tab dimuat ulang.",
} as const;

export type StringKey = keyof typeof id;
