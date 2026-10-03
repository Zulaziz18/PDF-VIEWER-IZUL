//! PDF to Word, with editable text (7.1.0).
//!
//! The conversion is in three steps, each its own module:
//!
//! 1. [`page`]: what the worker reads off a page with PDFium — characters with
//!    their boxes and style, and pictures. Plain data.
//! 2. [`layout`]: characters become lines, lines become paragraphs, and each
//!    paragraph gets a style, an alignment and its spacing. All heuristics, all
//!    here, all tested on hand-made pages.
//! 3. [`docx`]: paragraphs and pictures become a WordprocessingML package.
//!
//! What it does not try to be: a copy of the page. A PDF says where every
//! character goes; a Word document says what the paragraphs are and lets Word
//! place them. The text, its order, its sizes, weights and colours, headings,
//! paragraphs, alignment and pictures come across; tables arrive as lines with
//! tabs between the cells, and vector drawings do not arrive at all.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod docx;
pub mod layout;
pub mod page;
mod zip;

pub use docx::{build, Built};
pub use layout::{analyse, Block, Document};
pub use page::{Glyph, PageLayout, Picture, Rect};
