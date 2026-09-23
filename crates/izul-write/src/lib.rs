//! Writing saved files (SPEC 8, Phase 4).
//!
//! ## Why this crate exists at all
//!
//! SPEC 3.1 has PDFium write the document, and it does: the worker loads a
//! fresh copy, puts one placeholder annotation per object on each page it
//! owns, and saves the whole file with `FPDF_SaveAsCopy`. What PDFium's public
//! API cannot do is give an annotation the appearance stream SPEC 3.2 requires.
//! `FPDFAnnot_SetAP` takes a content string only, and — measured, not assumed,
//! on the pinned build — wraps it in a form XObject whose `/Resources` hold a
//! single `/GS` built from the annotation's opacity with `/BM /Normal`. No
//! fonts, no images, no Multiply: a highlight, a text box or a picture drawn
//! from our display list cannot be expressed through it.
//!
//! So this crate appends one **incremental update** to the bytes PDFium wrote.
//! It redefines each placeholder annotation with a complete standard
//! dictionary and gives it an appearance stream serialised from the display
//! list, with the fonts, graphics states and images that stream names. The
//! rest of the document is PDFium's, untouched. The appended section is
//! ordinary PDF — objects, a cross-reference table, a trailer pointing back at
//! PDFium's — so every reader applies it the way it applies any incremental
//! save.
//!
//! What it parses of PDFium's output is deliberately little and all of it is
//! PDFium's own, freshly written, never the user's original bytes: the final
//! trailer, the cross-reference table, and the dictionaries of the placeholder
//! objects, found by their `/NM (izul-<id>)`.
//!
//! [`atomic`] is the other half of SPEC 8's "simpan atomik": the finished bytes
//! go to a temporary file beside the target, are flushed to disk, are verified
//! by reopening, and only then replace the original with one rename.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod annot_dict;
pub mod atomic;
pub mod bounds;
pub mod images;
pub mod incremental;
pub mod metadata;
pub mod save;
pub mod syntax;

pub use save::{patch, AnnotWrite, SaveError};
