#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("tidak dapat membuat folder data {path}: {source}")]
    DataDir {
        path: String,
        source: std::io::Error,
    },

    #[error("skema basis data lebih baru ({found}) dari yang dikenali aplikasi ({expected})")]
    SchemaFromTheFuture { found: i64, expected: i64 },

    #[error("berkas tidak ada di basis data: {path}")]
    UnknownFile { path: String },
}

pub type Result<T> = std::result::Result<T, StoreError>;
