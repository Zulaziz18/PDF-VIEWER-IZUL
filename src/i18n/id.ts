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

  "status.page": "Halaman",
  "status.of": "dari",
  "status.zoom": "Perbesaran",
  "status.workers": "Pekerja",
  "status.rendering": "Merender…",
  "status.ready": "Siap",

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
