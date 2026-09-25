//! PDF objects: the value type, a lexer shared with content streams, a parser,
//! and a serialiser (ISO 32000-1 §7.2–7.3).
//!
//! The parser is written for the file PDFium has just saved *and* for the
//! content streams inside it, which are the user's own bytes, untouched by
//! PDFium. So it is strict about structure it cannot guess (an unterminated
//! dictionary is an error, never a best effort) and lenient only where every
//! reader is lenient too (a stray delimiter between operators is skipped by
//! the content tokenizer, not here).

use crate::error::{RedactError, Result};

/// An indirect reference, `num gen R`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ref {
    pub num: u32,
    pub gen: u16,
}

/// A dictionary, keys in the order they were read.
///
/// Order is kept because nothing requires otherwise and a rewritten object
/// that reads the same as the original is easier to check by eye.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dict(pub Vec<(Vec<u8>, Obj)>);

impl Dict {
    pub fn new() -> Self {
        Dict(Vec::new())
    }

    pub fn get(&self, key: &[u8]) -> Option<&Obj> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut Obj> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn set(&mut self, key: &[u8], value: Obj) {
        match self.get_mut(key) {
            Some(slot) => *slot = value,
            None => self.0.push((key.to_vec(), value)),
        }
    }

    pub fn remove(&mut self, key: &[u8]) -> Option<Obj> {
        let at = self.0.iter().position(|(k, _)| k == key)?;
        Some(self.0.remove(at).1)
    }

    pub fn contains(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }

    /// The value of a name-valued key.
    pub fn name(&self, key: &[u8]) -> Option<&[u8]> {
        match self.get(key)? {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }
}

/// A direct PDF value.
#[derive(Debug, Clone, PartialEq)]
pub enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Array(Vec<Obj>),
    Dict(Dict),
    Ref(Ref),
}

impl Obj {
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Obj::Int(i) => Some(i as f64),
            Obj::Real(r) => Some(r),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Obj::Int(i) => Some(i),
            // Some writers put `12.0` where an integer belongs; every reader
            // accepts it.
            Obj::Real(r) if r.fract() == 0.0 && r.abs() < 1e15 => Some(r as i64),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Obj::Dict(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Obj]> {
        match self {
            Obj::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_ref(&self) -> Option<Ref> {
        match *self {
            Obj::Ref(r) => Some(r),
            _ => None,
        }
    }

    pub fn name(n: &str) -> Obj {
        Obj::Name(n.as_bytes().to_vec())
    }
}

// ---------------------------------------------------------------------------
// Lexing
// ---------------------------------------------------------------------------

pub fn is_white(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

pub fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

pub fn is_regular(b: u8) -> bool {
    !is_white(b) && !is_delim(b)
}

/// One token.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Int(i64),
    Real(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    ArrayOpen,
    ArrayClose,
    DictOpen,
    DictClose,
    /// Anything else made of regular characters: `true`, `obj`, `R`, and
    /// every content-stream operator.
    Keyword(Vec<u8>),
}

/// A cursor over bytes.
#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    pub src: &'a [u8],
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a [u8], pos: usize) -> Self {
        Lexer { src, pos }
    }

    pub fn peek_byte(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    /// Skips whitespace and comments.
    pub fn skip_ws(&mut self) {
        while let Some(b) = self.peek_byte() {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(c) = self.peek_byte() {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn err(&self, what: &str) -> RedactError {
        RedactError::Syntax(format!("{what} pada byte {}", self.pos))
    }

    /// The next token, or `None` at the end.
    pub fn next_token(&mut self) -> Result<Option<Token>> {
        self.skip_ws();
        let Some(b) = self.peek_byte() else {
            return Ok(None);
        };
        match b {
            b'[' => {
                self.pos += 1;
                Ok(Some(Token::ArrayOpen))
            }
            b']' => {
                self.pos += 1;
                Ok(Some(Token::ArrayClose))
            }
            b'<' => {
                if self.src.get(self.pos + 1) == Some(&b'<') {
                    self.pos += 2;
                    Ok(Some(Token::DictOpen))
                } else {
                    self.hex_string().map(|s| Some(Token::Str(s)))
                }
            }
            b'>' => {
                if self.src.get(self.pos + 1) == Some(&b'>') {
                    self.pos += 2;
                    Ok(Some(Token::DictClose))
                } else {
                    Err(self.err("'>' tunggal"))
                }
            }
            b'(' => self.literal_string().map(|s| Some(Token::Str(s))),
            b'/' => {
                self.pos += 1;
                Ok(Some(Token::Name(self.name_body())))
            }
            b')' | b'{' | b'}' => {
                // Stray delimiters: PostScript procedures never appear in the
                // objects and streams this crate reads, and every reader skips
                // them. Report them as a keyword so a caller can decide.
                self.pos += 1;
                Ok(Some(Token::Keyword(vec![b])))
            }
            _ => {
                let start = self.pos;
                while self.peek_byte().is_some_and(is_regular) {
                    self.pos += 1;
                }
                let word = self.src.get(start..self.pos).unwrap_or_default();
                Ok(Some(classify(word)))
            }
        }
    }

    fn name_body(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(b) = self.peek_byte() {
            if !is_regular(b) {
                break;
            }
            if b == b'#' {
                let hex = self.src.get(self.pos + 1..self.pos + 3);
                if let Some(v) = hex.and_then(hex_pair) {
                    out.push(v);
                    self.pos += 3;
                    continue;
                }
            }
            out.push(b);
            self.pos += 1;
        }
        out
    }

    fn hex_string(&mut self) -> Result<Vec<u8>> {
        self.pos += 1; // '<'
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        loop {
            let Some(b) = self.peek_byte() else {
                return Err(self.err("string heksadesimal tidak ditutup"));
            };
            self.pos += 1;
            if b == b'>' {
                break;
            }
            if is_white(b) {
                continue;
            }
            let Some(v) = hex_digit(b) else {
                return Err(self.err("karakter bukan heksadesimal"));
            };
            match high.take() {
                Some(h) => out.push(h << 4 | v),
                None => high = Some(v),
            }
        }
        // An odd final digit is followed by an implicit 0 (§7.3.4.3).
        if let Some(h) = high {
            out.push(h << 4);
        }
        Ok(out)
    }

    fn literal_string(&mut self) -> Result<Vec<u8>> {
        self.pos += 1; // '('
        let mut out = Vec::new();
        let mut depth = 1usize;
        loop {
            let Some(b) = self.peek_byte() else {
                return Err(self.err("string tidak ditutup"));
            };
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                b'\\' => {
                    let Some(e) = self.peek_byte() else {
                        return Err(self.err("escape di akhir berkas"));
                    };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'(' | b')' | b'\\' => out.push(e),
                        b'\r' => {
                            // Line continuation; \r\n counts as one end of line.
                            if self.peek_byte() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.peek_byte() {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xff) as u8);
                        }
                        // An unknown escape is the character itself.
                        other => out.push(other),
                    }
                }
                b'\r' => {
                    // An unescaped end of line is read as a single \n.
                    if self.peek_byte() == Some(b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                other => out.push(other),
            }
        }
        Ok(out)
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex_pair(p: &[u8]) -> Option<u8> {
    let [h, l] = *p else { return None };
    Some(hex_digit(h)? << 4 | hex_digit(l)?)
}

/// A run of regular characters: a number when it reads as one.
fn classify(word: &[u8]) -> Token {
    if let Some(n) = parse_number(word) {
        return n;
    }
    Token::Keyword(word.to_vec())
}

fn parse_number(word: &[u8]) -> Option<Token> {
    let first = *word.first()?;
    if !(first.is_ascii_digit() || matches!(first, b'+' | b'-' | b'.')) {
        return None;
    }
    let text = std::str::from_utf8(word).ok()?;
    let body = text.trim_start_matches(['+', '-']);
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    // "--5" and "1.2.3" are malformed; readers take what they can, and so do
    // we: the leading sign run collapses, and a second point ends the number.
    let negative = text
        .bytes()
        .take_while(|b| matches!(b, b'+' | b'-'))
        .any(|b| b == b'-');
    if !body.contains('.') {
        return Some(match body.parse::<i64>() {
            Ok(v) => Token::Int(if negative { -v } else { v }),
            // Too large for i64: keep it as a real, as other readers do.
            Err(_) => {
                let v: f64 = body.parse().ok()?;
                Token::Real(if negative { -v } else { v })
            }
        });
    }
    let mut parts = body.splitn(3, '.');
    let int = parts.next().unwrap_or("");
    let frac = parts.next().unwrap_or("");
    let joined = format!("{}.{}", if int.is_empty() { "0" } else { int }, frac);
    let v: f64 = joined.parse().ok()?;
    Some(Token::Real(if negative { -v } else { v }))
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Nesting limit for arrays and dictionaries; deeper is hostile, not real.
const MAX_DEPTH: usize = 256;

/// Parses one object starting at the lexer's position.
///
/// `refs` says whether `n g R` is recognised — true in a file, false in a
/// content stream, where `R` is not an operator and two integers followed by
/// something called `R` would be a coincidence.
pub fn parse_object(lx: &mut Lexer<'_>, refs: bool) -> Result<Obj> {
    let Some(tok) = lx.next_token()? else {
        return Err(lx.err("objek diharapkan, berkas habis"));
    };
    parse_from(lx, tok, refs, 0)
}

pub fn parse_from(lx: &mut Lexer<'_>, tok: Token, refs: bool, depth: usize) -> Result<Obj> {
    if depth > MAX_DEPTH {
        return Err(lx.err("objek bersarang terlalu dalam"));
    }
    Ok(match tok {
        Token::Int(i) => {
            if refs {
                if let Some(r) = try_ref(lx, i) {
                    return Ok(Obj::Ref(r));
                }
            }
            Obj::Int(i)
        }
        Token::Real(r) => Obj::Real(r),
        Token::Name(n) => Obj::Name(n),
        Token::Str(s) => Obj::Str(s),
        Token::ArrayOpen => {
            let mut items = Vec::new();
            loop {
                let Some(t) = lx.next_token()? else {
                    return Err(lx.err("array tidak ditutup"));
                };
                if t == Token::ArrayClose {
                    break;
                }
                items.push(parse_from(lx, t, refs, depth + 1)?);
            }
            Obj::Array(items)
        }
        Token::DictOpen => {
            let mut dict = Dict::new();
            loop {
                let Some(t) = lx.next_token()? else {
                    return Err(lx.err("kamus tidak ditutup"));
                };
                match t {
                    Token::DictClose => break,
                    Token::Name(key) => {
                        let Some(vt) = lx.next_token()? else {
                            return Err(lx.err("nilai kamus hilang"));
                        };
                        if vt == Token::DictClose {
                            // `/Key >>`: a key with no value is ignored by
                            // every reader.
                            break;
                        }
                        let v = parse_from(lx, vt, refs, depth + 1)?;
                        // A null value is the same as an absent key (§7.3.7).
                        if v != Obj::Null {
                            dict.set(&key, v);
                        }
                    }
                    _ => return Err(lx.err("kunci kamus bukan nama")),
                }
            }
            Obj::Dict(dict)
        }
        Token::Keyword(k) => match k.as_slice() {
            b"true" => Obj::Bool(true),
            b"false" => Obj::Bool(false),
            b"null" => Obj::Null,
            _ => {
                return Err(RedactError::Syntax(format!(
                    "kata kunci tak terduga '{}' pada byte {}",
                    String::from_utf8_lossy(&k),
                    lx.pos
                )))
            }
        },
        Token::ArrayClose | Token::DictClose => return Err(lx.err("penutup tak terduga")),
    })
}

/// After an integer: is this `num gen R`? Restores the position when not.
fn try_ref(lx: &mut Lexer<'_>, num: i64) -> Option<Ref> {
    let save = lx.pos;
    let found = (|| {
        let Ok(Some(Token::Int(gen))) = lx.next_token() else {
            return None;
        };
        let Ok(Some(Token::Keyword(k))) = lx.next_token() else {
            return None;
        };
        if k != b"R" {
            return None;
        }
        Some(Ref {
            num: u32::try_from(num).ok()?,
            gen: u16::try_from(gen).ok()?,
        })
    })();
    if found.is_none() {
        lx.pos = save;
    }
    found
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// A number the way PDF wants it: no exponent, no trailing zeros.
pub fn fmt_real(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let mut s = format!("{v:.6}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

pub fn write_name(out: &mut Vec<u8>, name: &[u8]) {
    out.push(b'/');
    for &b in name {
        if b.is_ascii_graphic() && is_regular(b) && b != b'#' {
            out.push(b);
        } else {
            out.push(b'#');
            push_hex(out, b);
        }
    }
}

/// Strings are written as hexadecimal: no escaping rules, no end-of-line
/// normalisation for a reader to apply, and binary-safe.
pub fn write_string(out: &mut Vec<u8>, s: &[u8]) {
    out.push(b'<');
    for &b in s {
        push_hex(out, b);
    }
    out.push(b'>');
}

pub fn push_hex(out: &mut Vec<u8>, b: u8) {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    out.push(DIGITS.get(usize::from(b >> 4)).copied().unwrap_or(b'0'));
    out.push(DIGITS.get(usize::from(b & 15)).copied().unwrap_or(b'0'));
}

pub fn write_obj(out: &mut Vec<u8>, obj: &Obj) {
    match obj {
        Obj::Null => out.extend_from_slice(b"null"),
        Obj::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Obj::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Obj::Real(r) => out.extend_from_slice(fmt_real(*r).as_bytes()),
        Obj::Name(n) => write_name(out, n),
        Obj::Str(s) => write_string(out, s),
        Obj::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b' ');
                }
                write_obj(out, item);
            }
            out.push(b']');
        }
        Obj::Dict(d) => write_dict(out, d),
        Obj::Ref(r) => out.extend_from_slice(format!("{} {} R", r.num, r.gen).as_bytes()),
    }
}

pub fn write_dict(out: &mut Vec<u8>, d: &Dict) {
    out.extend_from_slice(b"<<");
    for (k, v) in &d.0 {
        write_name(out, k);
        out.push(b' ');
        write_obj(out, v);
    }
    out.extend_from_slice(b">>");
}

pub fn to_bytes(obj: &Obj) -> Vec<u8> {
    let mut out = Vec::new();
    write_obj(&mut out, obj);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Obj {
        parse_object(&mut Lexer::new(s.as_bytes(), 0), true).unwrap()
    }

    #[test]
    fn numbers_names_and_refs() {
        assert_eq!(parse("42"), Obj::Int(42));
        assert_eq!(parse("-.5"), Obj::Real(-0.5));
        assert_eq!(parse("+3."), Obj::Real(3.0));
        assert_eq!(parse("/A#20B"), Obj::Name(b"A B".to_vec()));
        assert_eq!(parse("12 0 R"), Obj::Ref(Ref { num: 12, gen: 0 }));
        assert_eq!(
            parse("[1 2 R 3]"),
            Obj::Array(vec![Obj::Ref(Ref { num: 1, gen: 2 }), Obj::Int(3)])
        );
        // Two integers not followed by R stay two integers.
        assert_eq!(parse("[1 2 3]").as_array().map(<[Obj]>::len), Some(3));
    }

    #[test]
    fn strings_literal_and_hex() {
        assert_eq!(parse("(a\\(b\\)c)"), Obj::Str(b"a(b)c".to_vec()));
        assert_eq!(parse("(nest (ed) ok)"), Obj::Str(b"nest (ed) ok".to_vec()));
        assert_eq!(parse("(\\101\\60)"), Obj::Str(b"A0".to_vec()));
        assert_eq!(parse("(a\\\nb)"), Obj::Str(b"ab".to_vec()));
        assert_eq!(parse("(a\r\nb)"), Obj::Str(b"a\nb".to_vec()));
        assert_eq!(parse("<48 65 6C6C6F>"), Obj::Str(b"Hello".to_vec()));
        assert_eq!(parse("<ABC>"), Obj::Str(vec![0xAB, 0xC0]));
    }

    #[test]
    fn dictionaries_drop_null_values() {
        let d = parse("<</A 1 /B null /C <</D (x)>>>>");
        let d = d.as_dict().unwrap();
        assert_eq!(d.get(b"A"), Some(&Obj::Int(1)));
        assert!(!d.contains(b"B"));
        assert!(d.get(b"C").and_then(Obj::as_dict).is_some());
    }

    #[test]
    fn written_objects_read_back_the_same() {
        let src = "<</Type/Page/Name/A#20B/S(bin\\000ary)/N -1.25/R 3 0 R/K[true false null]>>";
        let obj = parse(src);
        let again = parse(std::str::from_utf8(&to_bytes(&obj)).unwrap());
        assert_eq!(obj, again);
    }

    #[test]
    fn reals_have_no_exponent_and_no_trailing_zeros() {
        assert_eq!(fmt_real(1.5), "1.5");
        assert_eq!(fmt_real(2.0), "2");
        assert_eq!(fmt_real(1e-7), "0");
        assert_eq!(fmt_real(-0.0000001), "0");
        assert_eq!(fmt_real(123456.123456789), "123456.123457");
    }

    #[test]
    fn unterminated_structures_are_errors() {
        for bad in ["<</A 1", "[1 2", "(abc", "<41"] {
            assert!(
                parse_object(&mut Lexer::new(bad.as_bytes(), 0), true).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn deep_nesting_is_refused() {
        let s = "[".repeat(1000) + &"]".repeat(1000);
        assert!(parse_object(&mut Lexer::new(s.as_bytes(), 0), true).is_err());
    }
}
