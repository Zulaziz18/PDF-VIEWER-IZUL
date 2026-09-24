//! The 14 standard fonts, which a PDF may use without `/Widths` (§9.6.2.2).
//!
//! Widths come from the generated AFM tables in `std14_data.rs` (see
//! `tools/std14/build.py`). The name matching follows what renderers accept:
//! the canonical names, a subset prefix (`ABCDEF+`), and the common Windows
//! aliases — Arial is metrically Helvetica, Times New Roman is Times, Courier
//! New is Courier — with the style read from the rest of the name.

#[rustfmt::skip]
#[allow(clippy::all)]
pub mod data {
    include!("std14_data.rs");
}

/// One of the 14.
#[derive(Debug, Clone, Copy)]
pub struct Std14 {
    pub base: &'static str,
    pub ascent: i16,
    pub descent: i16,
    widths: &'static [(&'static str, u16)],
    /// The encoding the font uses when the PDF names none.
    pub builtin: &'static [&'static str; 256],
}

impl Std14 {
    pub fn width(&self, glyph: &str) -> Option<u16> {
        self.widths
            .binary_search_by(|(g, _)| (*g).cmp(glyph))
            .ok()
            .and_then(|i| self.widths.get(i))
            .map(|(_, w)| *w)
    }
}

/// The standard font a `/BaseFont` name stands for, if any.
pub fn lookup(base_font: &str) -> Option<Std14> {
    // "ABCDEF+Name": a subset tag is six capitals and a plus.
    let name = match base_font.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => base_font,
    };
    let lower = name.to_ascii_lowercase();
    let family = if lower.starts_with("courier") {
        "Courier"
    } else if lower.starts_with("helvetica") || lower.starts_with("arial") {
        "Helvetica"
    } else if lower.starts_with("times") {
        "Times"
    } else if lower.starts_with("symbol") {
        "Symbol"
    } else if lower.starts_with("zapfdingbats") || lower.starts_with("dingbats") {
        "ZapfDingbats"
    } else {
        return None;
    };
    let bold = lower.contains("bold") || lower.contains("black") || lower.contains("heavy");
    let italic = lower.contains("italic") || lower.contains("oblique");
    let base = match (family, bold, italic) {
        ("Courier", false, false) => "Courier",
        ("Courier", true, false) => "Courier-Bold",
        ("Courier", false, true) => "Courier-Oblique",
        ("Courier", true, true) => "Courier-BoldOblique",
        ("Helvetica", false, false) => "Helvetica",
        ("Helvetica", true, false) => "Helvetica-Bold",
        ("Helvetica", false, true) => "Helvetica-Oblique",
        ("Helvetica", true, true) => "Helvetica-BoldOblique",
        ("Times", false, false) => "Times-Roman",
        ("Times", true, false) => "Times-Bold",
        ("Times", false, true) => "Times-Italic",
        ("Times", true, true) => "Times-BoldItalic",
        (other, _, _) => other,
    };
    let (_, ascent, descent, widths) = *data::FONTS.iter().find(|f| f.0 == base)?;
    let builtin = match base {
        "Symbol" => &data::SYMBOL,
        "ZapfDingbats" => &data::ZAPF_DINGBATS,
        _ => &data::STANDARD,
    };
    Some(Std14 {
        base,
        ascent,
        descent,
        widths,
        builtin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_to_the_fourteen() {
        let base = |n: &str| lookup(n).map(|s| s.base);
        assert_eq!(base("Helvetica"), Some("Helvetica"));
        assert_eq!(base("Times-BoldItalic"), Some("Times-BoldItalic"));
        assert_eq!(base("ABCDEF+Arial-BoldMT"), Some("Helvetica-Bold"));
        assert_eq!(base("TimesNewRomanPS-ItalicMT"), Some("Times-Italic"));
        assert_eq!(base("CourierNew,BoldItalic"), Some("Courier-BoldOblique"));
        assert_eq!(base("Symbol"), Some("Symbol"));
        assert_eq!(base("Calibri"), None);
    }

    /// Spot checks against the published AFM files, to catch a generator
    /// that sorted or paired the tables wrongly.
    #[test]
    fn widths_match_the_afm_files() {
        let h = lookup("Helvetica").unwrap();
        assert_eq!(h.width("space"), Some(278));
        assert_eq!(h.width("W"), Some(944));
        let t = lookup("Times-Roman").unwrap();
        assert_eq!(t.width("a"), Some(444));
        assert_eq!(t.width("M"), Some(889));
        let c = lookup("Courier").unwrap();
        assert!(["a", "W", "space", "period"]
            .iter()
            .all(|g| c.width(g) == Some(600)));
        assert_eq!(data::WIN_ANSI.get(0x80).copied(), Some("Euro"));
        assert_eq!(data::STANDARD.get(0x27).copied(), Some("quoteright"));
    }
}
