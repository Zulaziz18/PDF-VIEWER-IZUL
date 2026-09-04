//! PDFium wrapper: open, render, extract text, and (from Phase 4) write.
//!
//! Everything that touches PDFium lives behind this crate. Two rules hold
//! throughout:
//!
//! 1. Every call into the library crosses [`ffi_guard::guard`], so a panic
//!    raised inside our FFI frames becomes a [`PdfError::EnginePanic`] instead
//!    of unwinding into C++ (SPEC 15).
//! 2. Geometry crossing this crate's public API is in PDF user space: points,
//!    origin bottom-left. Pixels appear only where a caller asks for a specific
//!    destination bitmap size (SPEC 8).

#![forbid(unsafe_op_in_unsafe_fn)]
// SPEC 0 bans `unwrap`, `expect` and `panic` in production code, and the
// workspace lints deny them. Test code is the exception on purpose: inside a
// test, `expect` *is* the failure report, and rewriting every assertion into
// error propagation would make the tests harder to read without making anything
// safer. The relaxation is scoped to `cfg(test)`, so it can never reach a
// shipped build.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod engine;
pub mod error;
mod ffi_guard;
pub mod geom;
pub mod outline;
pub mod render;
pub mod sys;
pub mod text;

pub use engine::{Document, Engine, PageGeometry};
pub use error::{PdfError, Result};
pub use geom::{Matrix, PageSize, PdfPointF, PdfRectF, RotationQuarter};
pub use outline::OutlineNode;
pub use render::{PixelFormat, Quality, TileGeometry, TileRequest, BYTES_PER_PIXEL, MAX_TILE_DIM};
pub use text::{CharBox, PageText};

/// PDFium build this crate is pinned to. Kept in step with
/// `vendor/pdfium/fetch.sh` and the `pdfium_*` feature in the workspace manifest.
pub const PDFIUM_PINNED_VERSION: &str = "151.0.7881.0";
