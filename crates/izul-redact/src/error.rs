//! What can stop a redaction.
//!
//! Every variant is a refusal, never a partial result: a redaction that could
//! not take out everything it was asked to must not write a file that looks
//! redacted (see the crate documentation).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RedactError {
    #[error("sintaks PDF tidak terbaca: {0}")]
    Syntax(String),
    #[error("struktur berkas tidak didukung: {0}")]
    Unsupported(String),
    #[error("dokumen terenkripsi tidak dapat diredaksi")]
    Encrypted,
    #[error("objek {0} tidak ditemukan")]
    Missing(u32),
    #[error("halaman {0} tidak ada")]
    NoPage(u32),
    #[error("stream tidak dapat didekode: {0}")]
    Filter(String),
    #[error("font {font} tidak dapat diukur: {why}")]
    Font { font: String, why: String },
}

pub type Result<T> = std::result::Result<T, RedactError>;
