//! Just enough of a font to lay out its glyphs: how a string splits into
//! character codes, how far each code advances, and how tall a glyph is.
//!
//! **Why this must be exact.** Taking a glyph out of a line leaves a gap that
//! is filled with a `TJ` adjustment equal to its advance. If the advance used
//! here differs from the one the renderer uses, every glyph after the gap on
//! that line moves. The worker checks, with PDFium, that every glyph outside
//! the marked areas is exactly where it was; this module's job is to make that
//! check pass, and a font it cannot measure is refused rather than estimated.
//!
//! Sources, in the order a renderer uses them (ISO 32000-1 §9.2.4, §9.6–9.7):
//! `/Widths` for simple fonts, `/W` and `/DW` for CID fonts, the Core 14 AFM
//! widths for the standard fonts that may omit `/Widths` (`std14`), and the
//! font matrix for Type 3.

use std::collections::BTreeMap;

use crate::error::{RedactError, Result};
use crate::file::Pdf;
use crate::filters::decode_plain;
use crate::geom::Matrix;
use crate::object::{Dict, Lexer, Obj, Token};
use crate::std14;

/// How a string's bytes group into codes.
#[derive(Debug, Clone, PartialEq)]
enum CodeSpace {
    OneByte,
    /// Codespace ranges: per length, byte-wise low and high bounds.
    Ranges(Vec<(Vec<u8>, Vec<u8>)>),
}

/// Code to CID, for widths.
#[derive(Debug, Clone, PartialEq)]
enum CidMap {
    Identity,
    /// `(first code, last code, first CID)`, plus single codes.
    Ranges(Vec<(u32, u32, u32)>),
}

#[derive(Debug, Clone, PartialEq)]
enum Widths {
    Simple {
        first: u32,
        widths: Vec<f64>,
        missing: f64,
    },
    Cid {
        ranges: Vec<(u32, u32, f64)>,
        default: f64,
        vertical: Vec<(u32, u32, [f64; 3])>,
        default_vertical: [f64; 2],
    },
}

/// A loaded font.
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    pub name: String,
    codes: CodeSpace,
    cids: CidMap,
    widths: Widths,
    /// Glyph space to text space: 1/1000 scale for everything but Type 3.
    pub matrix: Matrix,
    /// Top and bottom of a glyph, in glyph space.
    pub ascent: f64,
    pub descent: f64,
    pub vertical: bool,
}

/// One code in a shown string.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// Byte range within the string.
    pub start: usize,
    pub len: usize,
    pub code: u32,
}

impl Font {
    /// Splits a string into codes (§9.7.6.2).
    pub fn split(&self, s: &[u8]) -> Vec<Glyph> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < s.len() {
            let len = match &self.codes {
                CodeSpace::OneByte => 1,
                CodeSpace::Ranges(ranges) => match_code(ranges, s.get(i..).unwrap_or_default()),
            };
            let len = len.min(s.len() - i).max(1);
            let code = s
                .get(i..i + len)
                .unwrap_or_default()
                .iter()
                .fold(0u32, |acc, &b| acc << 8 | u32::from(b));
            out.push(Glyph {
                start: i,
                len,
                code,
            });
            i += len;
        }
        out
    }

    fn cid(&self, code: u32) -> u32 {
        match &self.cids {
            CidMap::Identity => code,
            CidMap::Ranges(r) => r
                .iter()
                .find(|(lo, hi, _)| (*lo..=*hi).contains(&code))
                .map_or(0, |(lo, _, cid)| cid + (code - lo)),
        }
    }

    /// Horizontal advance, glyph space.
    pub fn width(&self, code: u32) -> f64 {
        match &self.widths {
            Widths::Simple {
                first,
                widths,
                missing,
            } => code
                .checked_sub(*first)
                .and_then(|i| widths.get(i as usize))
                .copied()
                .unwrap_or(*missing),
            Widths::Cid {
                ranges, default, ..
            } => {
                let cid = self.cid(code);
                ranges
                    .iter()
                    .find(|(lo, hi, _)| (*lo..=*hi).contains(&cid))
                    .map_or(*default, |r| r.2)
            }
        }
    }

    /// Vertical metrics `(w1y, vx, vy)`, glyph space (§9.7.4.3).
    pub fn vertical_metrics(&self, code: u32) -> (f64, f64, f64) {
        let w0 = self.width(code);
        match &self.widths {
            Widths::Cid {
                vertical,
                default_vertical,
                ..
            } => {
                let cid = self.cid(code);
                match vertical
                    .iter()
                    .find(|(lo, hi, _)| (*lo..=*hi).contains(&cid))
                {
                    Some((_, _, [w1, vx, vy])) => (*w1, *vx, *vy),
                    None => (default_vertical[1], w0 / 2.0, default_vertical[0]),
                }
            }
            Widths::Simple { .. } => (-1000.0, w0 / 2.0, 880.0),
        }
    }

    /// Whether this code is the single-byte space word spacing applies to
    /// (§9.3.3: only a one-byte code 32).
    pub fn is_word_space(&self, g: &Glyph) -> bool {
        g.len == 1 && g.code == 32
    }
}

fn match_code(ranges: &[(Vec<u8>, Vec<u8>)], s: &[u8]) -> usize {
    for n in 1..=4usize {
        let Some(bytes) = s.get(..n) else { break };
        let hit = ranges.iter().any(|(lo, hi)| {
            lo.len() == n
                && bytes
                    .iter()
                    .zip(lo.iter().zip(hi))
                    .all(|(b, (l, h))| (*l..=*h).contains(b))
        });
        if hit {
            return n;
        }
    }
    // No range matches: consume as many bytes as the shortest range, which is
    // what renderers do with an invalid code.
    ranges.iter().map(|(lo, _)| lo.len()).min().unwrap_or(1)
}

fn num(o: Option<&Obj>) -> Option<f64> {
    o.and_then(Obj::as_f64)
}

fn font_err(name: &str, why: impl Into<String>) -> RedactError {
    RedactError::Font {
        font: name.to_string(),
        why: why.into(),
    }
}

/// Loads the font dictionary `dict`.
pub fn load(pdf: &Pdf<'_>, dict: &Dict) -> Result<Font> {
    let subtype = dict.name(b"Subtype").unwrap_or(b"Type1").to_vec();
    let name = dict
        .name(b"BaseFont")
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .unwrap_or_else(|| String::from_utf8_lossy(&subtype).into_owned());
    match subtype.as_slice() {
        b"Type0" => load_type0(pdf, dict, name),
        b"Type3" => load_type3(pdf, dict, name),
        _ => load_simple(pdf, dict, name),
    }
}

fn descriptor_metrics(pdf: &Pdf<'_>, desc: Option<&Dict>) -> Result<Option<(f64, f64)>> {
    let Some(desc) = desc else { return Ok(None) };
    let asc = num(desc.get(b"Ascent")).unwrap_or(0.0);
    let dsc = num(desc.get(b"Descent")).unwrap_or(0.0);
    if asc > 0.0 || dsc < 0.0 {
        return Ok(Some((asc.max(dsc + 1.0), dsc.min(0.0))));
    }
    if let Some(bbox) = desc.get(b"FontBBox") {
        if let Obj::Array(b) = pdf.resolve(bbox)? {
            let y0 = num(b.get(1)).unwrap_or(0.0);
            let y1 = num(b.get(3)).unwrap_or(0.0);
            if y1 > y0 {
                return Ok(Some((y1, y0)));
            }
        }
    }
    Ok(None)
}

/// The glyph names of an `/Encoding`, over a base vector.
fn encoding(pdf: &Pdf<'_>, dict: &Dict, builtin: &[&'static str; 256]) -> Result<Vec<String>> {
    let base_of = |n: &[u8]| -> &[&'static str; 256] {
        match n {
            b"WinAnsiEncoding" => &std14::data::WIN_ANSI,
            b"MacRomanEncoding" => &std14::data::MAC_ROMAN,
            b"StandardEncoding" => &std14::data::STANDARD,
            _ => builtin,
        }
    };
    let enc = match dict.get(b"Encoding") {
        Some(o) => pdf.resolve(o)?,
        None => Obj::Null,
    };
    let (base, diffs) = match &enc {
        Obj::Name(n) => (base_of(n), None),
        Obj::Dict(d) => (
            d.name(b"BaseEncoding").map_or(builtin, base_of),
            d.get(b"Differences").cloned(),
        ),
        _ => (builtin, None),
    };
    let mut names: Vec<String> = base.iter().map(|s| (*s).to_string()).collect();
    if let Some(diffs) = diffs {
        if let Obj::Array(items) = pdf.resolve(&diffs)? {
            let mut code: usize = 0;
            for item in items {
                match item {
                    Obj::Int(c) => code = usize::try_from(c).unwrap_or(0),
                    Obj::Name(n) => {
                        if let Some(slot) = names.get_mut(code) {
                            *slot = String::from_utf8_lossy(&n).into_owned();
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(names)
}

fn load_simple(pdf: &Pdf<'_>, dict: &Dict, name: String) -> Result<Font> {
    let desc = match dict.get(b"FontDescriptor") {
        Some(d) => pdf.resolve_dict(d)?,
        None => None,
    };
    let embedded = desc.as_ref().is_some_and(|d| {
        d.contains(b"FontFile") || d.contains(b"FontFile2") || d.contains(b"FontFile3")
    });
    let widths_obj = match dict.get(b"Widths") {
        Some(w) => Some(pdf.resolve(w)?),
        None => None,
    };
    let metrics = descriptor_metrics(pdf, desc.as_ref())?;
    let std = std14::lookup(&name);

    let widths = match (widths_obj, std) {
        (Some(Obj::Array(items)), _) => {
            let first = dict
                .get(b"FirstChar")
                .and_then(Obj::as_i64)
                .and_then(|f| u32::try_from(f).ok())
                .unwrap_or(0);
            let widths = items
                .iter()
                .map(|w| Ok(pdf.resolve(w)?.as_f64().unwrap_or(0.0)))
                .collect::<Result<Vec<_>>>()?;
            let missing = desc
                .as_ref()
                .and_then(|d| num(d.get(b"MissingWidth")))
                .unwrap_or(0.0);
            Widths::Simple {
                first,
                widths,
                missing,
            }
        }
        (_, Some(std)) if !embedded => {
            let names = encoding(pdf, dict, std.builtin)?;
            let widths = names
                .iter()
                .map(|g| f64::from(std.width(g).unwrap_or(0)))
                .collect();
            Widths::Simple {
                first: 0,
                widths,
                missing: 0.0,
            }
        }
        _ => {
            return Err(font_err(
                &name,
                "tanpa /Widths dan bukan salah satu dari 14 font standar",
            ))
        }
    };
    let (ascent, descent) = metrics
        .or(std.map(|s| (f64::from(s.ascent), f64::from(s.descent))))
        .unwrap_or((800.0, -200.0));
    Ok(Font {
        name,
        codes: CodeSpace::OneByte,
        cids: CidMap::Identity,
        widths,
        matrix: Matrix::new(0.001, 0.0, 0.0, 0.001, 0.0, 0.0),
        ascent,
        descent,
        vertical: false,
    })
}

fn load_type3(pdf: &Pdf<'_>, dict: &Dict, name: String) -> Result<Font> {
    let arr = |key: &[u8]| -> Result<Vec<f64>> {
        Ok(match dict.get(key) {
            Some(o) => match pdf.resolve(o)? {
                Obj::Array(items) => items
                    .iter()
                    .map(|i| Ok(pdf.resolve(i)?.as_f64().unwrap_or(0.0)))
                    .collect::<Result<_>>()?,
                _ => Vec::new(),
            },
            None => Vec::new(),
        })
    };
    let m = arr(b"FontMatrix")?;
    let matrix = match m.as_slice() {
        [a, b, c, d, e, f] => Matrix::new(*a, *b, *c, *d, *e, *f),
        _ => return Err(font_err(&name, "Type 3 tanpa /FontMatrix")),
    };
    let first = dict
        .get(b"FirstChar")
        .and_then(Obj::as_i64)
        .and_then(|f| u32::try_from(f).ok())
        .unwrap_or(0);
    let widths = arr(b"Widths")?;
    let bbox = arr(b"FontBBox")?;
    let (ascent, descent) = match bbox.as_slice() {
        [_, y0, _, y1] if y1 > y0 => (*y1, *y0),
        _ => {
            // A zero box is allowed (§9.6.5: "all zeros ... no assumptions");
            // stand in one em, as measured by the matrix.
            let em = 1.0 / matrix.d.abs().max(1e-9);
            (0.8 * em, -0.2 * em)
        }
    };
    Ok(Font {
        name,
        codes: CodeSpace::OneByte,
        cids: CidMap::Identity,
        widths: Widths::Simple {
            first,
            widths,
            missing: 0.0,
        },
        matrix,
        ascent,
        descent,
        vertical: false,
    })
}

fn load_type0(pdf: &Pdf<'_>, dict: &Dict, name: String) -> Result<Font> {
    let enc = match dict.get(b"Encoding") {
        Some(o) => o.clone(),
        None => return Err(font_err(&name, "Type0 tanpa /Encoding")),
    };
    let (codes, cids, vertical) = match pdf.resolve(&enc)? {
        Obj::Name(n) if n == b"Identity-H" || n == b"Identity-V" => (
            CodeSpace::Ranges(vec![(vec![0, 0], vec![0xff, 0xff])]),
            CidMap::Identity,
            n == b"Identity-V",
        ),
        Obj::Name(n) => {
            return Err(font_err(
                &name,
                format!("CMap bawaan {} belum didukung", String::from_utf8_lossy(&n)),
            ))
        }
        _ => {
            let stream = pdf
                .stream_of(&enc)?
                .ok_or_else(|| font_err(&name, "/Encoding bukan nama maupun stream"))?;
            let data = decode_plain(Some(pdf), &stream.dict, &stream.data)?;
            let cmap = parse_cmap(&data, &name)?;
            let vertical = stream
                .dict
                .get(b"WMode")
                .and_then(Obj::as_i64)
                .map(|w| w == 1)
                .unwrap_or(cmap.vertical);
            (cmap.codes, cmap.cids, vertical)
        }
    };
    let desc_font = match dict.get(b"DescendantFonts") {
        Some(o) => match pdf.resolve(o)? {
            Obj::Array(items) => match items.first() {
                Some(f) => pdf.resolve_dict(f)?,
                None => None,
            },
            _ => None,
        },
        None => None,
    }
    .ok_or_else(|| font_err(&name, "Type0 tanpa /DescendantFonts"))?;

    let default = num(desc_font.get(b"DW")).unwrap_or(1000.0);
    let mut ranges = Vec::new();
    if let Some(w) = desc_font.get(b"W") {
        if let Obj::Array(items) = pdf.resolve(w)? {
            let items: Vec<Obj> = items
                .iter()
                .map(|i| pdf.resolve(i))
                .collect::<Result<_>>()?;
            let mut i = 0;
            while let Some(first) = items.get(i).and_then(Obj::as_i64) {
                let first = u32::try_from(first).unwrap_or(0);
                match items.get(i + 1) {
                    Some(Obj::Array(ws)) => {
                        for (k, w) in ws.iter().enumerate() {
                            let w = pdf.resolve(w)?.as_f64().unwrap_or(default);
                            let c = first + k as u32;
                            ranges.push((c, c, w));
                        }
                        i += 2;
                    }
                    Some(last) => {
                        let last = last.as_i64().and_then(|l| u32::try_from(l).ok());
                        let w = items.get(i + 2).and_then(Obj::as_f64);
                        match (last, w) {
                            (Some(last), Some(w)) => ranges.push((first, last, w)),
                            _ => return Err(font_err(&name, "/W rusak")),
                        }
                        i += 3;
                    }
                    None => break,
                }
            }
        }
    }
    let dw2 = match desc_font.get(b"DW2") {
        Some(o) => match pdf.resolve(o)? {
            Obj::Array(a) => [
                num(a.first()).unwrap_or(880.0),
                num(a.get(1)).unwrap_or(-1000.0),
            ],
            _ => [880.0, -1000.0],
        },
        None => [880.0, -1000.0],
    };
    let mut vranges = Vec::new();
    if let Some(w2) = desc_font.get(b"W2") {
        if let Obj::Array(items) = pdf.resolve(w2)? {
            let mut i = 0;
            while let Some(first) = items.get(i).and_then(Obj::as_i64) {
                let first = u32::try_from(first).unwrap_or(0);
                match items.get(i + 1) {
                    Some(Obj::Array(vs)) => {
                        for (k, chunk) in vs.chunks(3).enumerate() {
                            if let [a, b, c] = chunk {
                                let c0 = first + k as u32;
                                vranges.push((
                                    c0,
                                    c0,
                                    [
                                        a.as_f64().unwrap_or(0.0),
                                        b.as_f64().unwrap_or(0.0),
                                        c.as_f64().unwrap_or(0.0),
                                    ],
                                ));
                            }
                        }
                        i += 2;
                    }
                    Some(last) => {
                        let last = last.as_i64().and_then(|l| u32::try_from(l).ok());
                        let v: Vec<f64> = (i + 2..i + 5)
                            .filter_map(|k| items.get(k).and_then(Obj::as_f64))
                            .collect();
                        if let (Some(last), [a, b, c]) = (last, v.as_slice()) {
                            vranges.push((first, last, [*a, *b, *c]));
                        }
                        i += 5;
                    }
                    None => break,
                }
            }
        }
    }
    let desc = match desc_font.get(b"FontDescriptor") {
        Some(d) => pdf.resolve_dict(d)?,
        None => None,
    };
    let (ascent, descent) = descriptor_metrics(pdf, desc.as_ref())?.unwrap_or((880.0, -120.0));
    Ok(Font {
        name,
        codes,
        cids,
        widths: Widths::Cid {
            ranges,
            default,
            vertical: vranges,
            default_vertical: dw2,
        },
        matrix: Matrix::new(0.001, 0.0, 0.0, 0.001, 0.0, 0.0),
        ascent,
        descent,
        vertical,
    })
}

struct CMap {
    codes: CodeSpace,
    cids: CidMap,
    vertical: bool,
}

fn code_of(s: &[u8]) -> u32 {
    s.iter().fold(0u32, |acc, &b| acc << 8 | u32::from(b))
}

/// Reads an embedded CMap stream: codespace ranges, CID mappings, `/WMode`,
/// and `usecmap` of an Identity CMap.
fn parse_cmap(data: &[u8], font: &str) -> Result<CMap> {
    let mut lx = Lexer::new(data, 0);
    let mut toks: Vec<Token> = Vec::new();
    loop {
        match lx.next_token() {
            Ok(Some(t)) => toks.push(t),
            Ok(None) => break,
            // PostScript procedures and the like: skip the byte.
            Err(_) => lx.pos += 1,
        }
    }
    let mut spaces: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut ranges: Vec<(u32, u32, u32)> = Vec::new();
    let mut vertical = false;
    let mut identity = false;
    let mut i = 0;
    let kw = |t: Option<&Token>, k: &[u8]| matches!(t, Some(Token::Keyword(x)) if x == k);
    while let Some(t) = toks.get(i) {
        match t {
            Token::Keyword(k) if k == b"begincodespacerange" => {
                i += 1;
                while let (Some(Token::Str(lo)), Some(Token::Str(hi))) =
                    (toks.get(i), toks.get(i + 1))
                {
                    if lo.len() == hi.len() && !lo.is_empty() && lo.len() <= 4 {
                        spaces.push((lo.clone(), hi.clone()));
                    }
                    i += 2;
                }
            }
            Token::Keyword(k) if k == b"begincidrange" => {
                i += 1;
                while let (Some(Token::Str(lo)), Some(Token::Str(hi)), Some(Token::Int(cid))) =
                    (toks.get(i), toks.get(i + 1), toks.get(i + 2))
                {
                    ranges.push((code_of(lo), code_of(hi), u32::try_from(*cid).unwrap_or(0)));
                    i += 3;
                }
            }
            Token::Keyword(k) if k == b"begincidchar" => {
                i += 1;
                while let (Some(Token::Str(c)), Some(Token::Int(cid))) =
                    (toks.get(i), toks.get(i + 1))
                {
                    let c = code_of(c);
                    ranges.push((c, c, u32::try_from(*cid).unwrap_or(0)));
                    i += 2;
                }
            }
            Token::Name(n) if n == b"WMode" => {
                if let Some(Token::Int(w)) = toks.get(i + 1) {
                    vertical = *w == 1;
                }
                i += 1;
            }
            Token::Name(n) if kw(toks.get(i + 1), b"usecmap") => {
                if n == b"Identity-H" || n == b"Identity-V" {
                    identity = true;
                    spaces.push((vec![0, 0], vec![0xff, 0xff]));
                } else {
                    return Err(font_err(
                        font,
                        format!("usecmap {} belum didukung", String::from_utf8_lossy(n)),
                    ));
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    if spaces.is_empty() {
        return Err(font_err(font, "CMap tanpa codespacerange"));
    }
    // Later mappings override earlier ones for the same code, as in
    // PostScript; keep the last by walking in reverse when looking up.
    ranges.reverse();
    let cids = if ranges.is_empty() && identity {
        CidMap::Identity
    } else if identity {
        // Explicit mappings over an Identity base: codes outside them map to
        // themselves.
        let mut all = ranges;
        all.push((0, 0xffff, 0));
        CidMap::Ranges(all)
    } else {
        CidMap::Ranges(ranges)
    };
    Ok(CMap {
        codes: CodeSpace::Ranges(spaces),
        cids,
        vertical,
    })
}

/// Fonts loaded so far, by the object they came from.
#[derive(Debug, Default)]
pub struct FontCache {
    by_ref: BTreeMap<u32, std::rc::Rc<Font>>,
}

impl FontCache {
    pub fn get(&mut self, pdf: &Pdf<'_>, obj: &Obj) -> Result<std::rc::Rc<Font>> {
        if let Obj::Ref(r) = obj {
            if let Some(f) = self.by_ref.get(&r.num) {
                return Ok(f.clone());
            }
        }
        let dict = pdf
            .resolve_dict(obj)?
            .ok_or_else(|| RedactError::Syntax("sumber daya font bukan kamus".into()))?;
        let font = std::rc::Rc::new(load(pdf, &dict)?);
        if let Obj::Ref(r) = obj {
            self.by_ref.insert(r.num, font.clone());
        }
        Ok(font)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::tests::build;

    fn font_from(objects: &[&[u8]], dict_num: u32) -> Result<Font> {
        let src = build(objects);
        let pdf = Pdf::parse(&src).unwrap();
        let d = pdf.get(dict_num).unwrap();
        load(&pdf, d.dict().unwrap())
    }

    #[test]
    fn simple_widths_with_first_char_and_missing_width() {
        let f = font_from(
            &[
                b"<</Type/Catalog>>",
                b"<</Type/Font/Subtype/TrueType/BaseFont/X/FirstChar 65/Widths[600 700]/FontDescriptor 3 0 R>>",
                b"<</Type/FontDescriptor/MissingWidth 250/Ascent 900/Descent -100/FontFile2 1 0 R>>",
            ],
            2,
        )
        .unwrap();
        assert_eq!(f.width(65), 600.0);
        assert_eq!(f.width(66), 700.0);
        assert_eq!(f.width(67), 250.0);
        assert_eq!((f.ascent, f.descent), (900.0, -100.0));
        assert_eq!(f.split(b"AB").len(), 2);
    }

    /// Helvetica without /Widths: the AFM widths through WinAnsi, and a
    /// /Differences entry that renames one code.
    #[test]
    fn standard_fonts_without_widths_use_the_afm_metrics() {
        let f = font_from(
            &[
                b"<</Type/Catalog>>",
                b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding<</BaseEncoding/WinAnsiEncoding/Differences[65/W]>>>>",
            ],
            2,
        )
        .unwrap();
        assert_eq!(f.width(u32::from(b'a')), 556.0);
        assert_eq!(f.width(u32::from(b' ')), 278.0);
        assert_eq!(f.width(65), 944.0, "A dipetakan ke glyph W");
        // Arial is Helvetica's metric twin and is read as such.
        let arial = font_from(
            &[
                b"<</Type/Catalog>>",
                b"<</Type/Font/Subtype/TrueType/BaseFont/Arial,Bold/Encoding/WinAnsiEncoding>>",
            ],
            2,
        )
        .unwrap();
        assert_eq!(arial.width(u32::from(b'a')), 556.0);
        assert_eq!(arial.width(u32::from(b'M')), 833.0);
    }

    #[test]
    fn an_unknown_font_without_widths_is_refused() {
        let r = font_from(
            &[
                b"<</Type/Catalog>>",
                b"<</Type/Font/Subtype/TrueType/BaseFont/SomethingElse>>",
            ],
            2,
        );
        assert!(matches!(r, Err(RedactError::Font { .. })));
    }

    #[test]
    fn identity_h_takes_two_bytes_and_reads_w() {
        let f = font_from(
            &[
                b"<</Type/Catalog>>",
                b"<</Type/Font/Subtype/Type0/BaseFont/F/Encoding/Identity-H/DescendantFonts[3 0 R]>>",
                b"<</Type/Font/Subtype/CIDFontType2/DW 500/W[10[100 200] 20 30 300]>>",
            ],
            2,
        )
        .unwrap();
        let g = f.split(&[0, 10, 0, 11, 0, 25, 0, 99]);
        let codes: Vec<u32> = g.iter().map(|g| g.code).collect();
        assert_eq!(codes, vec![10, 11, 25, 99]);
        let w: Vec<f64> = codes.iter().map(|&c| f.width(c)).collect();
        assert_eq!(w, vec![100.0, 200.0, 300.0, 500.0]);
    }

    /// A mixed one- and two-byte codespace, as in Shift-JIS-like CMaps.
    #[test]
    fn embedded_cmaps_split_by_codespace() {
        let cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
            2 begincodespacerange <00> <80> <8140> <9FFC> endcodespacerange\n\
            1 begincidrange <8140> <817E> 633 endcidrange\n\
            1 begincidchar <41> 34 endcidchar\n\
            endcmap CMapName currentdict /CMap defineresource pop end end";
        let f = parse_cmap(cmap, "t").unwrap();
        let font = Font {
            name: "t".into(),
            codes: f.codes,
            cids: f.cids,
            widths: Widths::Cid {
                ranges: vec![(633, 633, 111.0), (634, 634, 222.0), (34, 34, 333.0)],
                default: 1000.0,
                vertical: vec![],
                default_vertical: [880.0, -1000.0],
            },
            matrix: Matrix::IDENTITY,
            ascent: 1.0,
            descent: 0.0,
            vertical: false,
        };
        let g = font.split(&[0x41, 0x81, 0x40, 0x81, 0x41]);
        let lens: Vec<usize> = g.iter().map(|g| g.len).collect();
        assert_eq!(lens, vec![1, 2, 2]);
        let w: Vec<f64> = g.iter().map(|g| font.width(g.code)).collect();
        assert_eq!(w, vec![333.0, 111.0, 222.0]);
    }

    #[test]
    fn type3_widths_are_in_glyph_space() {
        let f = font_from(
            &[
                b"<</Type/Catalog>>",
                b"<</Type/Font/Subtype/Type3/FontMatrix[0.01 0 0 0.01 0 0]/FontBBox[0 -20 80 90]/FirstChar 97/Widths[60]/CharProcs<<>>>>",
            ],
            2,
        )
        .unwrap();
        assert_eq!(f.width(97), 60.0);
        assert_eq!(f.matrix.a, 0.01);
        assert_eq!((f.ascent, f.descent), (90.0, -20.0));
    }
}
