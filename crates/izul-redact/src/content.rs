//! Content stream tokenizing (ISO 32000-1 §7.8.2, §8.9.7).
//!
//! A content stream is a flat list of operators, each preceded by its
//! operands. Every operator is kept with the byte range it came from, so the
//! rewriter can copy what it does not change exactly as it was written and
//! only re-serialise what it does.

use crate::error::{RedactError, Result};
use crate::object::{is_white, parse_from, Dict, Lexer, Obj, Token};

/// An inline image's dictionary (abbreviated keys as written) and data.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineImage {
    pub dict: Dict,
    pub data: Vec<u8>,
}

/// One operator with its operands.
#[derive(Debug, Clone, PartialEq)]
pub struct Op {
    pub operands: Vec<Obj>,
    pub operator: Vec<u8>,
    /// Bytes this operator was read from, operands included.
    pub start: usize,
    pub end: usize,
    pub inline: Option<InlineImage>,
}

impl Op {
    pub fn is(&self, name: &[u8]) -> bool {
        self.operator == name
    }

    pub fn num(&self, i: usize) -> f64 {
        self.operands.get(i).and_then(Obj::as_f64).unwrap_or(0.0)
    }
}

/// How many components a colour space named in an inline image has, when the
/// data length must be computed rather than searched for.
pub type Components<'f> = &'f dyn Fn(&Obj) -> Option<u32>;

/// Splits a content stream into operators.
pub fn parse(src: &[u8], components: Components<'_>) -> Result<Vec<Op>> {
    let mut lx = Lexer::new(src, 0);
    let mut ops = Vec::new();
    let mut operands: Vec<Obj> = Vec::new();
    let mut start: Option<usize> = None;
    loop {
        lx.skip_ws();
        let here = lx.pos;
        let tok = match lx.next_token() {
            Ok(Some(t)) => t,
            Ok(None) => break,
            Err(_) => {
                // A byte no reader can make sense of (a lone '>', say) is
                // skipped, as renderers skip it.
                lx.pos = here + 1;
                continue;
            }
        };
        match tok {
            Token::Keyword(k) if k == b"true" || k == b"false" || k == b"null" => {
                start.get_or_insert(here);
                operands.push(match k.as_slice() {
                    b"true" => Obj::Bool(true),
                    b"false" => Obj::Bool(false),
                    _ => Obj::Null,
                });
            }
            Token::Keyword(k) if k.len() == 1 && matches!(k.first(), Some(b'{' | b'}' | b')')) => {
                // Stray delimiters: skipped.
            }
            Token::Keyword(k) if k == b"BI" => {
                let begin = start.unwrap_or(here);
                let image = inline_image(&mut lx, components)?;
                ops.push(Op {
                    operands: std::mem::take(&mut operands),
                    operator: b"BI".to_vec(),
                    start: begin,
                    end: lx.pos,
                    inline: Some(image),
                });
                start = None;
            }
            Token::Keyword(k) => {
                ops.push(Op {
                    operands: std::mem::take(&mut operands),
                    operator: k,
                    start: start.unwrap_or(here),
                    end: lx.pos,
                    inline: None,
                });
                start = None;
            }
            other => {
                start.get_or_insert(here);
                match parse_from(&mut lx, other, false, 0) {
                    Ok(obj) => operands.push(obj),
                    Err(_) => {
                        // An operand that does not parse (an unclosed array at
                        // the end of the stream): drop what was collected.
                        operands.clear();
                        start = None;
                    }
                }
            }
        }
    }
    Ok(ops)
}

fn inline_image(lx: &mut Lexer<'_>, components: Components<'_>) -> Result<InlineImage> {
    let mut dict = Dict::new();
    loop {
        let Some(tok) = lx.next_token()? else {
            return Err(RedactError::Syntax("gambar inline tanpa ID".into()));
        };
        match tok {
            Token::Keyword(k) if k == b"ID" => break,
            Token::Name(key) => {
                let Some(vt) = lx.next_token()? else {
                    return Err(RedactError::Syntax("gambar inline terpotong".into()));
                };
                let v = parse_from(lx, vt, false, 0)?;
                dict.set(&key, v);
            }
            _ => return Err(RedactError::Syntax("kunci gambar inline bukan nama".into())),
        }
    }
    // One whitespace byte separates ID from the data.
    if lx.peek_byte().is_some_and(is_white) {
        lx.pos += 1;
    }
    let data_start = lx.pos;
    let end = data_end(lx.src, data_start, &dict, components)
        .ok_or_else(|| RedactError::Syntax("akhir gambar inline (EI) tidak ditemukan".into()))?;
    let data = lx.src.get(data_start..end).unwrap_or_default().to_vec();
    // Past the data: whitespace, then EI.
    lx.pos = end;
    lx.skip_ws();
    lx.pos += 2;
    Ok(InlineImage { dict, data })
}

/// Where an inline image's data ends.
fn data_end(src: &[u8], start: usize, dict: &Dict, components: Components<'_>) -> Option<usize> {
    let get = |long: &[u8], short: &[u8]| dict.get(long).or_else(|| dict.get(short));
    let ends_with_ei = |at: usize| {
        let mut p = at;
        while src.get(p).is_some_and(|&b| is_white(b)) {
            p += 1;
        }
        src.get(p..p + 2) == Some(b"EI") && src.get(p + 2).is_none_or(|&b| is_white(b) || b == b'%')
    };
    // PDF 2.0 gives the length outright.
    if let Some(len) = get(b"Length", b"L").and_then(Obj::as_i64) {
        let end = start + usize::try_from(len).ok()?;
        if ends_with_ei(end) {
            return Some(end);
        }
    }
    // Unfiltered data has a computable length.
    if get(b"Filter", b"F").is_none() {
        let w = usize::try_from(get(b"Width", b"W")?.as_i64()?).ok()?;
        let h = usize::try_from(get(b"Height", b"H")?.as_i64()?).ok()?;
        let mask = matches!(get(b"ImageMask", b"IM"), Some(Obj::Bool(true)));
        let bpc = if mask {
            1
        } else {
            usize::try_from(get(b"BitsPerComponent", b"BPC")?.as_i64()?).ok()?
        };
        let comps = if mask {
            1
        } else {
            components(get(b"ColorSpace", b"CS")?)? as usize
        };
        let end = start + (w * comps * bpc).div_ceil(8) * h;
        if ends_with_ei(end) {
            return Some(end);
        }
    }
    // Otherwise search: whitespace, EI, then whitespace or the end — and what
    // follows must read as content, so that "EI" inside compressed data is
    // not taken for the end.
    let mut p = start;
    while p + 2 <= src.len() {
        if src.get(p..p + 2) == Some(b"EI")
            && p > start
            && src.get(p - 1).is_some_and(|&b| is_white(b))
            && src.get(p + 2).is_none_or(|&b| is_white(b))
            && plausible_after(src, p + 2)
        {
            let mut end = p - 1;
            // A CR LF before EI is one end of line, not data.
            if end > start && src.get(end) == Some(&b'\n') && src.get(end - 1) == Some(&b'\r') {
                end -= 1;
            }
            return Some(end);
        }
        p += 1;
    }
    None
}

/// Whether the bytes after a candidate `EI` look like more content: a few
/// tokens that are ASCII and tokenize.
fn plausible_after(src: &[u8], at: usize) -> bool {
    let mut lx = Lexer::new(src, at);
    for _ in 0..3 {
        lx.skip_ws();
        let before = lx.pos;
        match lx.next_token() {
            Ok(None) => return true,
            Ok(Some(_)) => {
                if src
                    .get(before..lx.pos)
                    .is_some_and(|t| t.iter().any(|&b| b >= 0x80 || (b < 0x20 && !is_white(b))))
                {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &Obj) -> Option<u32> {
        None
    }

    #[test]
    fn operators_keep_their_operands_and_bytes() {
        let src = b"q 1 0 0 1 10 20 cm BT /F1 12 Tf [(A) -120 (B)] TJ ET Q";
        let ops = parse(src, &none).unwrap();
        let names: Vec<&[u8]> = ops.iter().map(|o| o.operator.as_slice()).collect();
        assert_eq!(
            names,
            vec![&b"q"[..], b"cm", b"BT", b"Tf", b"TJ", b"ET", b"Q"]
        );
        let tj = ops.iter().find(|o| o.is(b"TJ")).unwrap();
        assert_eq!(&src[tj.start..tj.end], b"[(A) -120 (B)] TJ");
        let cm = &ops[1];
        assert_eq!(cm.num(4), 10.0);
    }

    #[test]
    fn inline_image_with_computable_length_may_contain_ei() {
        // 2×1 RGB, unfiltered: six bytes that happen to spell "EI".
        let mut src = b"BI /W 2 /H 1 /BPC 8 /CS /RGB ID ".to_vec();
        src.extend_from_slice(b"a EI b");
        src.extend_from_slice(b"\nEI Q");
        let comps = |o: &Obj| match o.as_name() {
            Some(b"RGB") => Some(3),
            _ => None,
        };
        let ops = parse(&src, &comps).unwrap();
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].inline.as_ref().unwrap().data, b"a EI b");
        assert!(ops[1].is(b"Q"));
    }

    #[test]
    fn inline_image_with_a_filter_is_found_by_searching() {
        let src = b"BI /W 4 /H 4 /BPC 8 /CS /G /F /AHx ID 00FF00FF>\nEI 0 0 m";
        let ops = parse(src, &none).unwrap();
        let names: Vec<&[u8]> = ops.iter().map(|o| o.operator.as_slice()).collect();
        assert_eq!(names, vec![&b"BI"[..], b"m"]);
        assert_eq!(ops[0].inline.as_ref().unwrap().data, b"00FF00FF>");
    }

    #[test]
    fn junk_between_operators_is_skipped() {
        let ops = parse(b"q } > Q", &none).unwrap();
        assert_eq!(ops.len(), 2);
    }
}
