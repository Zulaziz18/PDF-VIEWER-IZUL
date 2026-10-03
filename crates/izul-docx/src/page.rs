//! What the worker reads off one page for the conversion: every character
//! with its box, size, weight and colour, and every picture. Nothing here is
//! interpreted yet — that is `layout`'s job, and keeping the two apart is what
//! lets the layout rules be tested on hand-made pages.
//!
//! Coordinates are **display space**: points, origin at the bottom-left of the
//! page as shown, the page's own `/Rotate` already applied — the space the
//! viewer and the annotations use.

use serde::{Deserialize, Serialize};

/// A rectangle in points, y up.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

impl Rect {
    pub fn new(left: f32, bottom: f32, right: f32, top: f32) -> Self {
        Self {
            left,
            bottom,
            right,
            top,
        }
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.top - self.bottom
    }

    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            left: self.left.min(other.left),
            bottom: self.bottom.min(other.bottom),
            right: self.right.max(other.right),
            top: self.top.max(other.top),
        }
    }
}

/// One character, in the order PDFium's text page gives them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    pub ch: char,
    /// PDFium's loose box: the font's full ascent to descent, so every glyph
    /// on a line has the same top and bottom whatever its shape.
    pub rect: Rect,
    /// The baseline, from the character's origin.
    pub baseline: f32,
    /// Font size in points.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    /// 0..255 RGB.
    pub color: [u8; 3],
    /// Index into [`PageLayout::fonts`].
    pub font: u16,
    /// Inserted by PDFium rather than drawn: the spaces it infers between words
    /// and the line breaks between lines.
    pub generated: bool,
}

/// A picture placed on the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Picture {
    pub rect: Rect,
    /// `image/jpeg` (the original stream) or `image/png`.
    pub mime: String,
    pub bytes: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
}

/// Everything read from one page.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PageLayout {
    pub width: f32,
    pub height: f32,
    pub glyphs: Vec<Glyph>,
    /// Font family names, de-duplicated; glyphs point into this.
    pub fonts: Vec<String>,
    pub pictures: Vec<Picture>,
    /// Characters left out because they are not upright (a sideways
    /// "Downloaded from…" in the margin): a Word paragraph has no direction.
    pub skipped_turned: u32,
    /// Pictures left out because they could not be read back.
    pub skipped_pictures: u32,
}
