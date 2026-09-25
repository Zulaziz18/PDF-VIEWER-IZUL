//! The smallest pieces of PDF syntax: numbers, names, strings, dates.

use std::fmt::Write as _;

pub use izul_model::ap::num;

/// A PDF text string (ISO 32000-1 §7.9.2.2).
///
/// Plain ASCII is written as a literal string, escaped, which keeps the large
/// ASCII-only metadata compact. Anything else is written as UTF-16BE with a
/// byte-order mark in hexadecimal: every reader decodes that form, it has no
/// escaping rules to get wrong, and it carries characters PDFDocEncoding
/// cannot (CJK, Arabic, emoji).
pub fn text_string(text: &str) -> String {
    if text
        .bytes()
        .all(|b| (0x20..0x7f).contains(&b) || b == b'\n' || b == b'\r' || b == b'\t')
    {
        let mut out = String::with_capacity(text.len() + 2);
        out.push('(');
        for ch in text.chars() {
            match ch {
                '(' => out.push_str("\\("),
                ')' => out.push_str("\\)"),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c => out.push(c),
            }
        }
        out.push(')');
        out
    } else {
        let mut out = String::with_capacity(text.len() * 4 + 6);
        out.push_str("<FEFF");
        for unit in text.encode_utf16() {
            let _ = write!(out, "{unit:04X}");
        }
        out.push('>');
        out
    }
}

/// A PDF name, with the `#xx` escape for anything outside the regular
/// characters (ISO 32000-1 §7.3.5).
pub fn name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 1);
    out.push('/');
    for b in raw.bytes() {
        let regular = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+');
        if regular {
            out.push(b as char);
        } else {
            let _ = write!(out, "#{b:02X}");
        }
    }
    out
}

/// A PDF date string for a Unix time in milliseconds, in UTC:
/// `D:YYYYMMDDHHmmSSZ`. Civil-from-days after Howard Hinnant, so no date
/// library is needed for one format.
pub fn date(unix_ms: i64) -> String {
    let secs = unix_ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("(D:{y:04}{mo:02}{d:02}{h:02}{m:02}{s:02}Z)")
}

/// `[a b c d]` for a rectangle, `[l b r t]` order.
pub fn rect(r: izul_model::geom::PdfRectF) -> String {
    format!(
        "[{} {} {} {}]",
        num(r.left),
        num(r.bottom),
        num(r.right),
        num(r.top)
    )
}

/// `[a b c d e f]`.
pub fn matrix(m: &izul_model::geom::Matrix) -> String {
    format!(
        "[{} {} {} {} {} {}]",
        num(m.a),
        num(m.b),
        num(m.c),
        num(m.d),
        num(m.e),
        num(m.f)
    )
}

/// `[r g b]` in 0..1.
pub fn rgb(c: izul_model::display::Rgba) -> String {
    format!("[{} {} {}]", num(c.r), num(c.g), num(c.b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_literal_and_escaped() {
        assert_eq!(text_string("a(b)c\\d"), "(a\\(b\\)c\\\\d)");
        assert_eq!(text_string("dua\nbaris"), "(dua\\nbaris)");
    }

    #[test]
    fn anything_else_is_utf16be_with_a_bom() {
        assert_eq!(text_string("é"), "<FEFF00E9>");
        // A character outside the basic plane becomes a surrogate pair.
        assert_eq!(text_string("😀"), "<FEFFD83DDE00>");
        assert_eq!(text_string("مرحبا").len(), 5 * 4 + 6);
    }

    #[test]
    fn names_escape_what_is_not_regular() {
        assert_eq!(name("Izul"), "/Izul");
        assert_eq!(name("a b/c"), "/a#20b#2Fc");
    }

    #[test]
    fn dates_are_utc_and_zero_padded() {
        assert_eq!(date(0), "(D:19700101000000Z)");
        // 2026-09-23 03:04:05 UTC.
        assert_eq!(date(1_790_132_645_000), "(D:20260923030405Z)");
        assert_eq!(date(951_782_400_000), "(D:20000229000000Z)");
    }
}
