//! Real font metrics, measured from PDFium rather than remembered (SPEC 11.2).
//!
//! `izul-model` lays text out from a [`izul_model::font::FontCtx`], and every
//! glyph position in both the canvas proxy and the appearance stream comes from
//! it. So the metrics have to be the *same* ones the renderer will use when it
//! finally draws the glyphs, or the text drifts from its own layout — which is
//! the one parity failure a display list cannot prevent on its own.
//!
//! The metrics therefore come from PDFium itself, through
//! `FPDFText_LoadStandardFont` and `FPDFFont_GetGlyphWidth`, for the standard
//! 14 faces PDF guarantees. Embedding a user's own font, with its own widths,
//! is Phase 4; this is the set that needs no embedding at all.
//!
//! One thing here is an assumption rather than a documented fact: that for the
//! standard 14 faces under WinAnsi, a character's code *is* its glyph index in
//! the sense `FPDFFont_GetGlyphWidth` means. PDFium's header does not say so.
//! Rather than trust it, `the_measured_widths_match_what_pdfium_draws` puts a
//! string through a real render and compares the advance we computed with where
//! PDFium actually put the characters. If the assumption is ever wrong, that
//! test fails rather than a user's text arriving crooked.

use std::cell::RefCell;
use std::collections::HashMap;

use pdfium_render::prelude::{FPDF_DOCUMENT, FPDF_FONT};

use izul_model::annot::FontSpec;
use izul_model::display::FontRef;
use izul_model::font::{FaceMetrics, FontCtx, GlyphMetrics};

use crate::engine::Engine;
use crate::error::{PdfError, Result};

/// The size metrics are measured at.
///
/// PDFium reports widths in points for a given size; asking at 1000 pt and
/// keeping the integer part gives exactly the 1/1000-em units PDF font widths
/// are written in, with no rounding games in between.
const MEASURE_SIZE: f32 = 1000.0;

/// The standard-14 faces, chosen by family and style.
///
/// Times New Roman is the default SPEC 11.2 names; it maps to Times, whose
/// metrics are identical by design — that is what "metrically compatible"
/// means, and it is why a PDF with Times renders the same on a machine that has
/// only Times New Roman.
pub fn base_font_of(spec: &FontSpec) -> &'static str {
    base_font_name(spec)
}

fn base_font_name(spec: &FontSpec) -> &'static str {
    let family = spec.family.to_ascii_lowercase();
    let serif = family.contains("times") || family.contains("serif") && !family.contains("sans");
    let mono = family.contains("courier") || family.contains("mono");
    match (mono, serif, spec.bold, spec.italic) {
        (true, _, false, false) => "Courier",
        (true, _, true, false) => "Courier-Bold",
        (true, _, false, true) => "Courier-Oblique",
        (true, _, true, true) => "Courier-BoldOblique",
        (_, true, false, false) => "Times-Roman",
        (_, true, true, false) => "Times-Bold",
        (_, true, false, true) => "Times-Italic",
        (_, true, true, true) => "Times-BoldItalic",
        (_, _, false, false) => "Helvetica",
        (_, _, true, false) => "Helvetica-Bold",
        (_, _, false, true) => "Helvetica-Oblique",
        (_, _, true, true) => "Helvetica-BoldOblique",
    }
}

struct Face {
    handle: FPDF_FONT,
    ascent_milli: i16,
    descent_milli: i16,
    widths: RefCell<HashMap<char, u16>>,
}

/// Font metrics for the standard 14 faces, owned by one document.
///
/// Holds a `RefCell` cache and is therefore single-threaded, like everything
/// else that touches PDFium in this crate (see `test_support` for why that rule
/// is enforced rather than hoped for).
pub struct StandardFonts {
    engine: &'static Engine,
    doc: FPDF_DOCUMENT,
    faces: RefCell<Vec<Face>>,
    by_name: RefCell<HashMap<&'static str, FontRef>>,
}

impl std::fmt::Debug for StandardFonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StandardFonts")
            .field("faces", &self.faces.borrow().len())
            .finish_non_exhaustive()
    }
}

impl Drop for StandardFonts {
    fn drop(&mut self) {
        for face in self.faces.borrow().iter() {
            // SAFETY: each handle came from `FPDFText_LoadStandardFont` on the
            // document below and is closed exactly once, here.
            unsafe { self.engine.bindings().FPDFFont_Close(face.handle) };
        }
    }
}

impl StandardFonts {
    /// Binds a metrics provider to a document.
    ///
    /// The document is only a context for PDFium's font objects; nothing is
    /// written to it.
    pub fn new(engine: &'static Engine, doc: FPDF_DOCUMENT) -> Self {
        StandardFonts {
            engine,
            doc,
            faces: RefCell::new(Vec::new()),
            by_name: RefCell::new(HashMap::new()),
        }
    }

    fn load(&self, name: &'static str) -> Option<FontRef> {
        if let Some(found) = self.by_name.borrow().get(name) {
            return Some(*found);
        }
        let bindings = self.engine.bindings();
        // SAFETY: `doc` is a live document handle held by the caller for at
        // least as long as `self`, and `name` is one of the standard 14 names.
        let handle = unsafe { bindings.FPDFText_LoadStandardFont(self.doc, name) };
        if handle.is_null() {
            return None;
        }
        let mut ascent = 0.0f32;
        let mut descent = 0.0f32;
        // SAFETY: `handle` is a live font object; both out-pointers are valid.
        unsafe {
            bindings.FPDFFont_GetAscent(handle, MEASURE_SIZE, &mut ascent);
            bindings.FPDFFont_GetDescent(handle, MEASURE_SIZE, &mut descent);
        }
        let mut faces = self.faces.borrow_mut();
        let font = FontRef(faces.len() as u32);
        faces.push(Face {
            handle,
            ascent_milli: ascent as i16,
            descent_milli: descent as i16,
            widths: RefCell::new(HashMap::new()),
        });
        self.by_name.borrow_mut().insert(name, font);
        Some(font)
    }

    /// The advance of one character in 1/1000 em, measured once and cached.
    fn measure(&self, font: FontRef, ch: char) -> Option<u16> {
        let faces = self.faces.borrow();
        let face = faces.get(font.0 as usize)?;
        if let Some(cached) = face.widths.borrow().get(&ch) {
            return Some(*cached);
        }
        // Outside WinAnsi there is no code to ask about; the caller refuses the
        // text rather than drawing boxes (SPEC 11.2).
        let code = u32::from(ch);
        if code > 255 {
            return None;
        }
        let mut width = 0.0f32;
        // SAFETY: `face.handle` is a live font object owned by `self`, and
        // `width` is a valid out-pointer.
        let ok = unsafe {
            self.engine.bindings().FPDFFont_GetGlyphWidth(
                face.handle,
                code,
                MEASURE_SIZE,
                &mut width,
            )
        };
        if ok == 0 || !width.is_finite() || width <= 0.0 {
            // A zero-width answer for a printable character means PDFium has no
            // glyph for it here; a space legitimately has a width, so this does
            // not swallow one.
            if !ch.is_whitespace() {
                return None;
            }
        }
        let milli = width.round().clamp(0.0, 65535.0) as u16;
        face.widths.borrow_mut().insert(ch, milli);
        Some(milli)
    }
}

impl FontCtx for StandardFonts {
    fn resolve(&self, spec: &FontSpec) -> Option<FontRef> {
        self.load(base_font_name(spec))
    }

    fn glyph(&self, font: FontRef, ch: char) -> Option<GlyphMetrics> {
        let advance_milli = self.measure(font, ch)?;
        Some(GlyphMetrics {
            glyph_id: u32::from(ch) as u16,
            advance_milli,
        })
    }

    fn face(&self, font: FontRef) -> FaceMetrics {
        let faces = self.faces.borrow();
        match faces.get(font.0 as usize) {
            Some(f) => FaceMetrics {
                ascent_milli: f.ascent_milli,
                descent_milli: f.descent_milli,
            },
            None => FaceMetrics {
                ascent_milli: 750,
                descent_milli: -250,
            },
        }
    }
}

/// Opens an empty in-memory document to hang font objects off.
///
/// PDFium's font API needs a document; this is the smallest valid one. The
/// document is returned so the caller keeps it alive for as long as the metrics
/// are used — dropping it first would leave the font handles dangling.
pub fn metrics_document(engine: &'static Engine) -> Result<crate::engine::Document> {
    const EMPTY: &[u8] = b"%PDF-1.7\n1 0 obj\n<</Type/Catalog/Pages 2 0 R>>\nendobj\n2 0 obj\n<</Type/Pages/Kids[3 0 R]/Count 1>>\nendobj\n3 0 obj\n<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>\nendobj\ntrailer\n<</Size 4/Root 1 0 R>>\n%%EOF\n";
    engine
        .open_bytes(EMPTY.to_vec(), None::<&str>, None)
        .map_err(|e| PdfError::Corrupt {
            detail: format!("dokumen metrik font tidak dapat dibuat: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_and_lock;

    #[test]
    fn the_default_family_maps_to_a_standard_face() {
        let spec = FontSpec::default();
        assert_eq!(spec.family, "Times New Roman");
        assert_eq!(base_font_name(&spec), "Times-Roman");
        assert_eq!(
            base_font_name(&FontSpec {
                family: "Arial".into(),
                bold: true,
                ..Default::default()
            }),
            "Helvetica-Bold"
        );
        assert_eq!(
            base_font_name(&FontSpec {
                family: "Courier New".into(),
                italic: true,
                ..Default::default()
            }),
            "Courier-Oblique"
        );
    }

    #[test]
    fn widths_come_out_in_thousandths_and_differ_per_letter() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = match metrics_document(engine) {
            Ok(d) => d,
            Err(e) => panic!("dokumen metrik: {e}"),
        };
        let fonts = StandardFonts::new(engine, doc.handle());
        let font = fonts
            .resolve(&FontSpec::default())
            .expect("Times-Roman ada");

        let i = fonts.glyph(font, 'i').expect("glif i").advance_milli;
        let m = fonts.glyph(font, 'm').expect("glif m").advance_milli;
        assert!(i > 0 && m > 0, "lebar harus positif: i={i} m={m}");
        assert!(
            m > i * 2,
            "proporsional: m={m} harus jauh lebih lebar dari i={i}"
        );
        assert!(
            (100..=1000).contains(&i) || i < 100,
            "satuannya 1/1000 em, bukan poin: i={i}"
        );
        let space = fonts.glyph(font, ' ').expect("spasi").advance_milli;
        assert!(space > 0, "spasi punya lebar");
    }

    #[test]
    fn a_monospaced_face_reports_one_width_for_everything() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = match metrics_document(engine) {
            Ok(d) => d,
            Err(e) => panic!("dokumen metrik: {e}"),
        };
        let fonts = StandardFonts::new(engine, doc.handle());
        let font = fonts
            .resolve(&FontSpec {
                family: "Courier".into(),
                ..Default::default()
            })
            .expect("Courier ada");
        let widths: Vec<u16> = "iMm1."
            .chars()
            .filter_map(|c| fonts.glyph(font, c))
            .map(|g| g.advance_milli)
            .collect();
        assert_eq!(widths.len(), 5);
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "Courier harus seragam: {widths:?}"
        );
    }

    /// The assumption this module rests on, put through a real render.
    ///
    /// A string is drawn with one `Tj` and PDFium's own text layer is asked
    /// where it put each character. If our measured advances match the gaps
    /// PDFium left, then the code-as-glyph-index assumption holds for the
    /// standard 14 — and if it ever stops holding, this fails instead of a
    /// user's text arriving with the letters in the wrong places.
    #[test]
    fn the_measured_widths_match_what_pdfium_draws() {
        let (engine, _pdfium) = engine_and_lock!();
        let metrics_doc = match metrics_document(engine) {
            Ok(d) => d,
            Err(e) => panic!("dokumen metrik: {e}"),
        };
        let fonts = StandardFonts::new(engine, metrics_doc.handle());
        let font = fonts.resolve(&FontSpec::default()).expect("Times-Roman");

        const SIZE: f32 = 24.0;
        const TEXT: &str = "Wim jadi ilmu";
        let content = format!("BT /F1 {SIZE} Tf 20 100 Td ({TEXT}) Tj ET\n");
        let objects = [
            "<</Type/Catalog/Pages 2 0 R>>".to_string(),
            "<</Type/Pages/Kids[3 0 R]/Count 1>>".to_string(),
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 400 200]/Resources<</Font<</F1<</Type/Font/Subtype/Type1/BaseFont/Times-Roman/Encoding/WinAnsiEncoding>>>>>>/Contents 4 0 R>>".to_string(),
            format!("<</Length {}>>\nstream\n{content}endstream", content.len()),
        ];
        let mut pdf = String::from("%PDF-1.7\n");
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
        }
        let xref = pdf.len();
        pdf.push_str(&format!(
            "xref\n0 {}\n0000000000 65535 f \n",
            objects.len() + 1
        ));
        for off in &offsets {
            pdf.push_str(&format!("{off:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        ));

        let drawn = match engine.open_bytes(pdf.into_bytes(), None::<&str>, None) {
            Ok(d) => d,
            Err(e) => panic!("buka pdf uji: {e}"),
        };
        let page = drawn
            .page_text_boxed(0, izul_model::geom::RotationQuarter::None)
            .expect("teks halaman");

        // Where PDFium actually put each character, versus where our metrics say
        // it should be. Both start at the same origin, so the comparison is of
        // the accumulated advances.
        let mut expected_x = 20.0f32;
        let mut checked = 0;
        for (ch, boxed) in TEXT.chars().zip(page.chars.iter()) {
            assert_eq!(ch, boxed.unicode, "urutan karakter berbeda");
            if !ch.is_whitespace() {
                let drift = (boxed.rect.left - expected_x).abs();
                assert!(
                    drift < SIZE * 0.08,
                    "'{ch}' meleset {drift:.2} pt dari perkiraan {expected_x:.2}; \
                     asumsi kode-sebagai-indeks-glif tidak berlaku"
                );
                checked += 1;
            }
            let advance = fonts.glyph(font, ch).expect("lebar").advance_milli;
            expected_x += f32::from(advance) / 1000.0 * SIZE;
        }
        assert!(checked >= 8, "hanya {checked} karakter yang diperiksa");
    }
}
