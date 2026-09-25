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
//! Phase 0 delivered the coordinate system and the display-list vocabulary.
//! Phase 3 adds what stands on them: the annotation objects themselves
//! (`annot`), the pure `display_list(obj, font_ctx)` that turns one into
//! primitives (`build`), the appearance-stream backend that serialises those
//! primitives into PDF operators (`ap`), and the command stack that makes every
//! edit undoable (`ops`). The canvas backend consumes the same list from the
//! frontend, which is why nothing here knows what a canvas is.

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

pub mod annot;
pub mod ap;
pub mod build;
pub mod display;
pub mod font;
pub mod geom;
pub mod ops;
pub mod pages;

pub use annot::{
    AnnotId, AnnotKind, AnnotObject, AnnotPayload, FontSpec, NoteIcon, ShapeStyle, TextAlign,
};
pub use ap::{appearance, Appearance, GState, Resources};
pub use build::display_list;
pub use display::{
    BlendMode, DisplayList, DisplayOp, FillRule, FontRef, ImageRef, LineCap, LineJoin, Path,
    PathSeg, PositionedGlyph, Rgba, StrokeStyle,
};
pub use font::{FaceMetrics, FixedFont, FontCtx, GlyphMetrics, TextError};
pub use geom::{Matrix, PdfPointF, PdfRectF, RotationQuarter};
pub use ops::{AnnotDoc, CommandStack, EditError, FormValue, Op, Transaction, DEFAULT_UNDO_LIMIT};
pub use pages::{PageCommand, PageEntry, PageError, SourceId, OWN_FILE};
