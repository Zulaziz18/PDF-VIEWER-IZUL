//! Replacing a document's own text, within one line (Phase 7, SPEC 17's
//! "dibatasi tegas").
//!
//! The rewrite is `izul_redact::replace_text` — the redaction interpreter,
//! taking the glyphs in the area out and writing the new text where the
//! first of them was, in the same embedded font, with everything after it
//! left exactly in place. This module is the part that does not trust it:
//! the result is opened with PDFium and checked, character by character,
//! before anything is saved.
//!
//! Why not PDFium's own editing (`FPDFText_SetText` and
//! `FPDFPage_GenerateContent`): measured on `tools/textedit-proof`'s page,
//! `SetText` given "Santosé 中" in a font whose subset has neither wrote
//! codes that poppler reads as "SantosŽ ÿ", while PDFium read its own result
//! back as "Santosé ˇ" — the check a PDFium-only path would make passes on
//! text that is wrong everywhere else. A code the font's `/ToUnicode` does
//! not name is a character the font cannot draw, and is refused.

use crate::engine::Document;
use crate::geom::PdfRectF;
use crate::redaction::{
    inside, overlaps, plan_areas, CharLayout, RedactFailure, MOVE, PDFIUM_TRUNCATION,
};

/// What a replacement did.
#[derive(Debug, Clone, PartialEq)]
pub struct Replaced {
    pub page: u32,
    /// The text that was there, as PDFium read it.
    pub before: String,
    /// Glyphs taken out.
    pub glyphs: u32,
}

/// The text drawn in `area`, in reading order, with the page's own spaces
/// but none PDFium inserts — what the user is told was there.
fn text_in(chars: &[CharLayout], area: &PdfRectF) -> String {
    chars
        .iter()
        .filter(|c| !c.generated && c.unicode != '\u{0}' && inside(area, c.centre()))
        .map(|c| c.unicode)
        .collect::<String>()
        .trim()
        .to_string()
}

fn visible(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Replaces the text in `display` (display space, as the viewer selects it)
/// on `page` with `text`, returning the result as a new document.
pub fn replace_text_document(
    doc: &Document,
    page: u32,
    display: PdfRectF,
    text: &str,
) -> std::result::Result<(Document, Replaced), RedactFailure> {
    use RedactFailure::{Pdf, Refused};
    if page >= doc.page_count() {
        return Err(Refused(format!("halaman {} tidak ada", page + 1)));
    }
    let geometry = doc.page_geometry(page).map_err(Pdf)?;
    let chars = doc.char_layout(page).map_err(Pdf)?;
    let area = plan_areas(&geometry, &[display], &chars)
        .into_iter()
        .next()
        .filter(|a| a.width() > 0.0 && a.height() > 0.0)
        .ok_or_else(|| Refused("bagian yang dipilih kosong".into()))?;
    let taken: Vec<&CharLayout> = chars
        .iter()
        .filter(|c| c.counts() && inside(&area, c.centre()))
        .collect();
    if taken.is_empty() {
        return Err(Refused(
            "tidak ada teks halaman di bagian yang dipilih".into(),
        ));
    }
    let before = text_in(&chars, &area);
    let bytes = doc.save_to_vec().map_err(Pdf)?;
    let (out, glyphs) = izul_redact::replace_text(
        &bytes,
        page,
        izul_redact::Rect::new(
            f64::from(area.left),
            f64::from(area.bottom),
            f64::from(area.right),
            f64::from(area.top),
        ),
        text,
    )
    .map_err(|e| Refused(e.to_string()))?;
    let copy = doc
        .engine()
        .open_bytes(out, None::<&str>, None)
        .map_err(|e| Refused(format!("hasil penyuntingan tidak terbaca: {e}")))?;
    if copy.page_count() != doc.page_count() {
        return Err(Refused("jumlah halaman berubah".into()));
    }
    let after = copy.char_layout(page).map_err(Pdf)?;
    check(&chars, &area, &after, text).map_err(Refused)?;
    Ok((
        copy,
        Replaced {
            page,
            before,
            glyphs,
        },
    ))
}

/// Every character outside `area` is where it was; the characters that are
/// new read `text`; and none of them runs into a character that stayed.
pub(crate) fn check(
    before: &[CharLayout],
    area: &PdfRectF,
    after: &[CharLayout],
    text: &str,
) -> std::result::Result<(), String> {
    let kept: Vec<&CharLayout> = before
        .iter()
        .filter(|c| c.counts() && !inside(area, c.centre()))
        .collect();
    let taken = before
        .iter()
        .filter(|c| c.counts() && inside(area, c.centre()))
        .count();
    let written = visible(text).chars().count();
    let mut unmatched: Vec<&CharLayout> = after.iter().filter(|c| c.counts()).collect();
    for c in &kept {
        // Glyphs on the line gaps were left or written on can move by
        // PDFium's width truncation, one unit per glyph (see `redaction.rs`).
        let same_line = (area.bottom as f64..=area.top as f64).contains(&c.origin.1);
        let tolerance = MOVE
            + if same_line {
                (taken + written) as f64 * PDFIUM_TRUNCATION * c.size / 1000.0
            } else {
                0.0
            };
        let at = unmatched.iter().position(|a| {
            let (dx, dy) = (a.origin.0 - c.origin.0, a.origin.1 - c.origin.1);
            a.unicode == c.unicode && (dx * dx + dy * dy).sqrt() < tolerance
        });
        match at {
            Some(i) => {
                unmatched.remove(i);
            }
            None => {
                return Err(format!(
                    "karakter '{}' di luar bagian yang diganti bergeser (x {:.1}, y {:.1})",
                    c.unicode, c.origin.0, c.origin.1
                ))
            }
        }
    }
    let new: String = unmatched.iter().map(|c| c.unicode).collect();
    if new != visible(text) {
        return Err(format!(
            "teks yang terbaca sesudahnya \"{new}\", bukan \"{}\"",
            visible(text)
        ));
    }
    // Loose boxes overlap their neighbours by design (a font's box is
    // taller and wider than its ink); the ink is what must not.
    let ink = |c: &CharLayout| {
        let r = c.tight;
        PdfRectF::new(r.left + 0.2, r.bottom + 0.2, r.right - 0.2, r.top - 0.2)
    };
    for n in &unmatched {
        if n.tight.width() <= 0.0 {
            continue;
        }
        if let Some(k) = kept
            .iter()
            .find(|k| k.tight.width() > 0.0 && overlaps(&ink(n), &ink(k)))
        {
            return Err(format!(
                "teks pengganti terlalu panjang: menabrak \"{}\" di sebelahnya",
                k.unicode
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RotationQuarter;
    use crate::{engine_and_lock, viewer_fixture};

    /// The display-space box of the first `needle` on `page`, as the
    /// viewer's selection would give it.
    fn rect_of(doc: &Document, page: u32, needle: &str) -> PdfRectF {
        let t = doc
            .page_text_boxed(page, RotationQuarter::None)
            .expect("teks");
        let start = t.text.find(needle).expect("ada di halaman");
        let first = t.text[..start].chars().count();
        let n = needle.chars().count();
        t.chars[first..first + n]
            .iter()
            .map(|c| c.rect)
            .reduce(|a, b| {
                PdfRectF::new(
                    a.left.min(b.left),
                    a.bottom.min(b.bottom),
                    a.right.max(b.right),
                    a.top.max(b.top),
                )
            })
            .expect("kotak")
    }

    fn page_text(doc: &Document, page: u32) -> String {
        doc.page_text_boxed(page, RotationQuarter::None)
            .expect("teks")
            .text
    }

    #[test]
    fn a_word_is_replaced_and_the_rest_of_the_line_stays() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine.open(viewer_fixture!(), None).expect("buka");
        let area = rect_of(&doc, 0, "halaman");
        let (copy, done) = replace_text_document(&doc, 0, area, "lembar").expect("ganti");
        assert_eq!(done.before, "halaman");
        assert_eq!(done.glyphs, 7);
        let text = page_text(&copy, 0);
        assert!(text.starts_with("Bagian 1 — lembar 1"), "{text:?}");
        // The rest of the page reads as it did.
        let (old, new) = (page_text(&doc, 0), text);
        assert_eq!(old.replacen("halaman", "lembar", 1), new);
    }

    #[test]
    fn a_character_the_font_does_not_have_is_refused_by_name() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine.open(viewer_fixture!(), None).expect("buka");
        let area = rect_of(&doc, 0, "halaman");
        match replace_text_document(&doc, 0, area, "halamän") {
            Err(RedactFailure::Refused(why)) => assert!(why.contains("\"ä\""), "{why}"),
            other => panic!("harus ditolak: {:?}", other.map(|(_, r)| r)),
        }
    }

    #[test]
    fn a_selection_over_two_lines_is_refused() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine.open(viewer_fixture!(), None).expect("buka");
        let t = doc.page_text_boxed(0, RotationQuarter::None).expect("teks");
        let lines: Vec<&str> = t.text.lines().collect();
        let (a, b) = (rect_of(&doc, 0, lines[1]), rect_of(&doc, 0, lines[2]));
        let both = PdfRectF::new(
            a.left.min(b.left),
            a.bottom.min(b.bottom),
            a.right.max(b.right),
            a.top.max(b.top),
        );
        match replace_text_document(&doc, 0, both, "satu") {
            Err(RedactFailure::Refused(why)) => assert!(why.contains("satu baris"), "{why}"),
            other => panic!("harus ditolak: {:?}", other.map(|(_, r)| r)),
        }
    }

    #[test]
    fn a_replacement_that_runs_into_the_next_word_is_refused() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine.open(viewer_fixture!(), None).expect("buka");
        let area = rect_of(&doc, 0, "Bagian");
        match replace_text_document(&doc, 0, area, "Bagian yang sangat panjang") {
            Err(RedactFailure::Refused(why)) => assert!(why.contains("menabrak"), "{why}"),
            other => panic!("harus ditolak: {:?}", other.map(|(_, r)| r)),
        }
    }
}
