use std::path::PathBuf;

/// Everything that can go wrong inside the PDFium layer.
///
/// `message_id` is what the UI shows: an i18n key, never a raw PDFium code.
/// The `Debug`/`Display` text is for the log file only.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("library PDFium tidak dapat dimuat dari {path:?}: {source}")]
    LibraryLoad {
        path: PathBuf,
        #[source]
        source: pdfium_render::prelude::PdfiumError,
    },

    #[error("PDFium sudah dimuat di proses ini")]
    LibraryAlreadyLoaded,

    #[error("berkas tidak dapat dibuka: {path:?}")]
    FileNotReadable {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("dokumen memerlukan kata sandi")]
    PasswordRequired,

    #[error("kata sandi salah")]
    PasswordWrong,

    #[error("dokumen rusak dan tidak dapat diperbaiki: {detail}")]
    Corrupt { detail: String },

    #[error("halaman {page} tidak ada (dokumen punya {page_count} halaman)")]
    PageOutOfRange { page: u32, page_count: u32 },

    #[error("ukuran ubin tidak masuk akal: {w}x{h}")]
    BadTileSize { w: u32, h: u32 },

    #[error("kotak sumber tidak valid: {rect:?}")]
    BadSourceRect { rect: crate::geom::PdfRectF },

    #[error("buffer tujuan terlalu kecil: butuh {needed} byte, tersedia {available}")]
    BufferTooSmall { needed: usize, available: usize },

    /// PDFium unwound across the FFI boundary. The document that caused it is a
    /// candidate for the supervisor's "poison" list.
    #[error("PDFium panik saat {operation}")]
    EnginePanic { operation: &'static str },

    #[error("operasi dibatalkan")]
    Cancelled,

    #[error("PDFium menolak operasi {operation}: {source}")]
    Pdfium {
        operation: &'static str,
        #[source]
        source: pdfium_render::prelude::PdfiumError,
    },

    #[error("lampiran {index}: {detail}")]
    Attachment { index: u32, detail: String },
}

impl PdfError {
    /// Stable key for the i18n table. Never expose PDFium internals to the user.
    pub fn message_id(&self) -> &'static str {
        match self {
            Self::LibraryLoad { .. } => "err.pdf.library_load",
            Self::LibraryAlreadyLoaded => "err.pdf.library_already_loaded",
            Self::FileNotReadable { .. } => "err.pdf.file_not_readable",
            Self::PasswordRequired => "err.pdf.password_required",
            Self::PasswordWrong => "err.pdf.password_wrong",
            Self::Corrupt { .. } => "err.pdf.corrupt",
            Self::PageOutOfRange { .. } => "err.pdf.page_out_of_range",
            Self::BadTileSize { .. } => "err.pdf.bad_tile_size",
            Self::BadSourceRect { .. } => "err.pdf.bad_source_rect",
            Self::BufferTooSmall { .. } => "err.pdf.buffer_too_small",
            Self::EnginePanic { .. } => "err.pdf.engine_panic",
            Self::Cancelled => "err.pdf.cancelled",
            Self::Pdfium { .. } => "err.pdf.engine",
            Self::Attachment { .. } => "err.pdf.attachment",
        }
    }

    /// True when this error means the worker itself is suspect, not just the request.
    pub fn is_engine_fault(&self) -> bool {
        matches!(self, Self::EnginePanic { .. })
    }
}

pub type Result<T> = std::result::Result<T, PdfError>;
