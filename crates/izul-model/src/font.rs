//! Resolving a font request into something both backends can draw (SPEC 3.2).
//!
//! The display list carries *positioned glyphs*, not strings. That is the whole
//! reason text lands in the same place on screen and in the exported file: if
//! each backend shaped the text itself, the canvas would use the browser's
//! metrics and the appearance stream would use the embedded font's, and the two
//! would disagree by a fraction of a point per character — which reads as a
//! line of text that shifts the moment the object is deselected.
//!
//! So shaping happens once, here, against metrics supplied by whoever owns the
//! actual font. This crate never opens a font file (SPEC 5); it asks.

use crate::annot::FontSpec;
use crate::display::FontRef;

/// One glyph's metrics, in the PDF convention of 1/1000 em.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphMetrics {
    pub glyph_id: u16,
    /// Advance width in 1/1000 of the em square, as PDF font widths are written.
    pub advance_milli: u16,
}

/// Vertical metrics of a face, also in 1/1000 em.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaceMetrics {
    pub ascent_milli: i16,
    pub descent_milli: i16,
}

/// What the model needs to know about fonts, and nothing more.
///
/// Implemented by the application over real faces, and by a fixed-metric double
/// in tests. A trait rather than a concrete type because the model must stay
/// testable without a font file anywhere near it.
pub trait FontCtx {
    /// Resolves a request to a handle, or `None` when the family is not
    /// available.
    ///
    /// `None` is a real answer, not a failure to be papered over: SPEC 11.2 is
    /// explicit that a missing font must be refused with a clear message rather
    /// than drawn as empty boxes.
    fn resolve(&self, spec: &FontSpec) -> Option<FontRef>;

    /// Metrics for one character, or `None` when the face has no glyph for it —
    /// the CJK and RTL case SPEC 11.2 asks to refuse rather than fake.
    fn glyph(&self, font: FontRef, ch: char) -> Option<GlyphMetrics>;

    fn face(&self, font: FontRef) -> FaceMetrics;
}

/// Why a text object could not be laid out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextError {
    /// The family is not installed or not embeddable.
    FontUnavailable(String),
    /// The face is there but has no glyph for these characters. Carries the
    /// first few, so the message can say *which* characters are missing rather
    /// than only that some are.
    MissingGlyphs(String),
}

impl std::fmt::Display for TextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextError::FontUnavailable(family) => {
                write!(f, "font tidak tersedia: {family}")
            }
            TextError::MissingGlyphs(chars) => {
                write!(f, "font ini tidak memuat karakter yang dibutuhkan: {chars}")
            }
        }
    }
}

impl std::error::Error for TextError {}

/// A face with one width for every glyph, for tests and for the metrics of last
/// resort.
///
/// Exported rather than kept in `cfg(test)` because the golden-image harness in
/// another crate needs the same deterministic metrics: a baseline image that
/// depended on whichever fonts a machine happened to have installed would fail
/// on the next machine for reasons that have nothing to do with the code.
#[derive(Debug, Clone)]
pub struct FixedFont {
    pub font: FontRef,
    pub advance_milli: u16,
    pub ascent_milli: i16,
    pub descent_milli: i16,
    /// Characters this face claims to have. Empty means "all of them".
    pub covers: Option<Vec<char>>,
}

impl Default for FixedFont {
    fn default() -> Self {
        FixedFont {
            font: FontRef(1),
            advance_milli: 500,
            ascent_milli: 750,
            descent_milli: -250,
            covers: None,
        }
    }
}

impl FontCtx for FixedFont {
    fn resolve(&self, _spec: &FontSpec) -> Option<FontRef> {
        Some(self.font)
    }

    fn glyph(&self, _font: FontRef, ch: char) -> Option<GlyphMetrics> {
        if let Some(covers) = &self.covers {
            if !covers.contains(&ch) {
                return None;
            }
        }
        Some(GlyphMetrics {
            // Deterministic and reversible, so a test can tell which character
            // produced a glyph without a table.
            glyph_id: (ch as u32 % 65535) as u16,
            advance_milli: self.advance_milli,
        })
    }

    fn face(&self, _font: FontRef) -> FaceMetrics {
        FaceMetrics {
            ascent_milli: self.ascent_milli,
            descent_milli: self.descent_milli,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_face_that_lacks_a_glyph_says_so() {
        let font = FixedFont {
            covers: Some(vec!['a', 'b']),
            ..Default::default()
        };
        assert!(font.glyph(FontRef(1), 'a').is_some());
        assert!(
            font.glyph(FontRef(1), '字').is_none(),
            "SPEC 11.2: jangan menggambar kotak kosong untuk glif yang tidak ada"
        );
    }

    #[test]
    fn the_error_names_what_is_missing() {
        let e = TextError::FontUnavailable("Fraktur".into()).to_string();
        assert!(e.contains("Fraktur"), "{e}");
        let e = TextError::MissingGlyphs("字漢".into()).to_string();
        assert!(e.contains('字'), "{e}");
    }
}
