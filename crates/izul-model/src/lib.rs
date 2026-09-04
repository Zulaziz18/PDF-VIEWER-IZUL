//! Pure model: geometry, the display list, and (from Phase 3) annotation
//! objects, the operation list, the undo stack, and the appearance-stream
//! serialiser.
//!
//! This crate must never reach PDFium or the filesystem (SPEC 5). The rule is
//! not stylistic: the display list is what guarantees that the editor and the
//! exported file agree, and that guarantee is only worth something if it can be
//! tested without rendering anything. If parity ever breaks, the bug is in here,
//! and it has to be reproducible in a unit test.
//!
//! What Phase 0 delivers: the coordinate system and the display-list vocabulary,
//! both fully specified and tested. The `display_list(obj, font_ctx)` function
//! and the two backends that consume it belong to Phase 3 and are not stubbed
//! here — an empty function that returns nothing would be worse than an absent
//! one.

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

pub mod display;
pub mod geom;

pub use display::{
    BlendMode, DisplayList, DisplayOp, FillRule, FontRef, ImageRef, LineCap, LineJoin, Path,
    PathSeg, PositionedGlyph, Rgba, StrokeStyle,
};
pub use geom::{Matrix, PdfPointF, PdfRectF, RotationQuarter};
