//! Reading the file PDFium wrote, and writing the redacted one.
//!
//! **Input.** Always PDFium's own full save (`FPDF_SaveAsCopy` without
//! `FPDF_INCREMENTAL`), never a user's original. Measured on the pinned build:
//! that save writes a classic cross-reference table with its trailer, and
//! unpacks object streams into ordinary objects — a file saved by pikepdf with
//! object streams came back with none. So only the classic form is read here,
//! and anything else is a refusal rather than a guess.
//!
//! **Output.** A complete new file, never an incremental update. An
//! incremental update *appends*: every object it replaces is still in the
//! file, byte for byte, for anyone who reads past the last cross-reference
//! table. For redaction that would be the whole failure. So the output holds
//! only the objects still reachable from the trailer; an old content stream
//! that was replaced is simply not written. Objects that did not change are
//! copied verbatim — their bytes are PDFium's, and re-serialising them would
//! only add a way to get something wrong.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{RedactError, Result};
use crate::object::{
    parse_from, parse_object, write_dict, write_obj, Dict, Lexer, Obj, Ref, Token,
};

/// A stream: its dictionary and its data as stored (still encoded).
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub dict: Dict,
    pub data: Vec<u8>,
}

/// What an object number holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Stored {
    Obj(Obj),
    Stream(Stream),
}

impl Stored {
    /// The dictionary of a dictionary or of a stream.
    pub fn dict(&self) -> Option<&Dict> {
        match self {
            Stored::Obj(Obj::Dict(d)) => Some(d),
            Stored::Stream(s) => Some(&s.dict),
            Stored::Obj(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    offset: usize,
    gen: u16,
}

/// A parsed file with pending changes.
#[derive(Debug)]
pub struct Pdf<'a> {
    src: &'a [u8],
    xref: BTreeMap<u32, Entry>,
    pub trailer: Dict,
    changed: BTreeMap<u32, Stored>,
    next: u32,
}

/// How many references a chain may follow before it is called a loop.
const MAX_CHAIN: usize = 32;

fn find_last(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

impl<'a> Pdf<'a> {
    pub fn parse(src: &'a [u8]) -> Result<Self> {
        let tail_from = src.len().saturating_sub(2048);
        let tail = src.get(tail_from..).unwrap_or_default();
        let at = find_last(tail, b"startxref")
            .ok_or_else(|| RedactError::Syntax("startxref tidak ada".into()))?;
        let mut lx = Lexer::new(src, tail_from + at + b"startxref".len());
        let Some(Token::Int(offset)) = lx.next_token()? else {
            return Err(RedactError::Syntax("offset startxref tidak terbaca".into()));
        };
        let mut xref = BTreeMap::new();
        let mut trailer: Option<Dict> = None;
        let mut next_section = usize::try_from(offset).ok();
        let mut seen = BTreeSet::new();
        while let Some(off) = next_section.take() {
            if !seen.insert(off) || seen.len() > 64 {
                return Err(RedactError::Syntax("rantai /Prev berputar".into()));
            }
            let t = read_section(src, off, &mut xref)?;
            if let Some(prev) = t.get(b"Prev").and_then(Obj::as_i64) {
                next_section = usize::try_from(prev).ok();
            }
            if trailer.is_none() {
                trailer = Some(t);
            }
        }
        let trailer = trailer.ok_or_else(|| RedactError::Syntax("trailer tidak ada".into()))?;
        if trailer.contains(b"Encrypt") {
            return Err(RedactError::Encrypted);
        }
        let size = trailer.get(b"Size").and_then(Obj::as_i64).unwrap_or(0);
        let max_used = xref.keys().next_back().copied().unwrap_or(0);
        let next = u32::try_from(size)
            .unwrap_or(0)
            .max(max_used.saturating_add(1));
        Ok(Pdf {
            src,
            xref,
            trailer,
            changed: BTreeMap::new(),
            next,
        })
    }

    /// The object stored under `num`.
    pub fn get(&self, num: u32) -> Result<Stored> {
        if let Some(s) = self.changed.get(&num) {
            return Ok(s.clone());
        }
        let entry = self.xref.get(&num).ok_or(RedactError::Missing(num))?;
        let (stored, _) = self.load_at(num, *entry)?;
        Ok(stored)
    }

    fn load_at(&self, num: u32, entry: Entry) -> Result<(Stored, usize)> {
        let mut lx = Lexer::new(self.src, entry.offset);
        let head = (lx.next_token()?, lx.next_token()?, lx.next_token()?);
        match head {
            (Some(Token::Int(n)), Some(Token::Int(_)), Some(Token::Keyword(k)))
                if k == b"obj" && i64::from(num) == n => {}
            _ => {
                return Err(RedactError::Syntax(format!(
                    "objek {num} tidak ada di offset {}",
                    entry.offset
                )))
            }
        }
        let obj = parse_object(&mut lx, true)?;
        let after_obj = lx.pos;
        let mut peek = lx.clone();
        match peek.next_token()? {
            Some(Token::Keyword(k)) if k == b"stream" => {
                let Obj::Dict(dict) = obj else {
                    return Err(RedactError::Syntax(format!("stream {num} tanpa kamus")));
                };
                // The data starts after the end of line following `stream`.
                let mut start = peek.pos;
                if self.src.get(start) == Some(&b'\r') {
                    start += 1;
                }
                if self.src.get(start) == Some(&b'\n') {
                    start += 1;
                }
                let declared = self.length_of(&dict)?;
                let end = match declared {
                    Some(len) if self.ends_stream(start + len) => start + len,
                    _ => self.search_endstream(start).ok_or_else(|| {
                        RedactError::Syntax(format!("endstream objek {num} tidak ditemukan"))
                    })?,
                };
                let data = self
                    .src
                    .get(start..end)
                    .ok_or_else(|| RedactError::Syntax(format!("stream {num} terpotong")))?
                    .to_vec();
                Ok((Stored::Stream(Stream { dict, data }), end))
            }
            _ => Ok((Stored::Obj(obj), after_obj)),
        }
    }

    fn length_of(&self, dict: &Dict) -> Result<Option<usize>> {
        let len = match dict.get(b"Length") {
            Some(Obj::Ref(r)) => match self.get(r.num)? {
                Stored::Obj(o) => o.as_i64(),
                Stored::Stream(_) => None,
            },
            Some(o) => o.as_i64(),
            None => None,
        };
        Ok(len.and_then(|l| usize::try_from(l).ok()))
    }

    /// Whether `endstream` follows `at`, after at most one end of line.
    fn ends_stream(&self, at: usize) -> bool {
        let mut p = at;
        for _ in 0..2 {
            if matches!(self.src.get(p), Some(b'\r' | b'\n')) {
                p += 1;
            }
        }
        self.src.get(p..p + 9).is_some_and(|w| w == b"endstream")
    }

    fn search_endstream(&self, from: usize) -> Option<usize> {
        let rest = self.src.get(from..)?;
        let at = rest.windows(9).position(|w| w == b"endstream")?;
        let mut end = from + at;
        // The end of line before `endstream` is not data.
        if end > from && self.src.get(end - 1) == Some(&b'\n') {
            end -= 1;
        }
        if end > from && self.src.get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
        Some(end)
    }

    /// Follows references until a direct value. A stream resolves to its
    /// dictionary.
    pub fn resolve(&self, obj: &Obj) -> Result<Obj> {
        let mut cur = obj.clone();
        for _ in 0..MAX_CHAIN {
            let Obj::Ref(r) = cur else { return Ok(cur) };
            cur = match self.get(r.num) {
                Ok(Stored::Obj(o)) => o,
                Ok(Stored::Stream(s)) => return Ok(Obj::Dict(s.dict)),
                // A reference to nothing is null (§7.3.10).
                Err(RedactError::Missing(_)) => return Ok(Obj::Null),
                Err(e) => return Err(e),
            };
        }
        Err(RedactError::Syntax(
            "rantai referensi terlalu panjang".into(),
        ))
    }

    pub fn resolve_dict(&self, obj: &Obj) -> Result<Option<Dict>> {
        Ok(match self.resolve(obj)? {
            Obj::Dict(d) => Some(d),
            _ => None,
        })
    }

    /// The stream a value refers to, if it refers to one.
    pub fn stream_of(&self, obj: &Obj) -> Result<Option<Stream>> {
        let Obj::Ref(r) = obj else { return Ok(None) };
        Ok(match self.get(r.num) {
            Ok(Stored::Stream(s)) => Some(s),
            Ok(Stored::Obj(Obj::Ref(inner))) => self.stream_of(&Obj::Ref(inner))?,
            Ok(Stored::Obj(_)) | Err(RedactError::Missing(_)) => None,
            Err(e) => return Err(e),
        })
    }

    pub fn set(&mut self, num: u32, value: Stored) {
        self.changed.insert(num, value);
    }

    pub fn add(&mut self, value: Stored) -> Ref {
        let num = self.next;
        self.next += 1;
        self.changed.insert(num, value);
        Ref { num, gen: 0 }
    }

    /// The document catalog.
    pub fn catalog(&self) -> Result<(Ref, Dict)> {
        let r = self
            .trailer
            .get(b"Root")
            .and_then(Obj::as_ref)
            .ok_or_else(|| RedactError::Syntax("trailer tanpa /Root".into()))?;
        let d = self
            .resolve_dict(&Obj::Ref(r))?
            .ok_or_else(|| RedactError::Syntax("/Root bukan kamus".into()))?;
        Ok((r, d))
    }

    /// Every page object, in document order.
    pub fn pages(&self) -> Result<Vec<Ref>> {
        let (_, cat) = self.catalog()?;
        let root = cat
            .get(b"Pages")
            .and_then(Obj::as_ref)
            .ok_or_else(|| RedactError::Syntax("katalog tanpa /Pages".into()))?;
        let mut out = Vec::new();
        let mut visited = BTreeSet::new();
        self.walk_pages(root, &mut out, &mut visited, 0)?;
        Ok(out)
    }

    fn walk_pages(
        &self,
        node: Ref,
        out: &mut Vec<Ref>,
        visited: &mut BTreeSet<u32>,
        depth: usize,
    ) -> Result<()> {
        if depth > 64 || !visited.insert(node.num) {
            return Err(RedactError::Syntax("pohon halaman berputar".into()));
        }
        let Some(dict) = self.resolve_dict(&Obj::Ref(node))? else {
            return Ok(());
        };
        let kids = dict.get(b"Kids").and_then(Obj::as_array);
        match (dict.name(b"Type"), kids) {
            (Some(b"Page"), _) | (None, None) => out.push(node),
            (_, Some(kids)) => {
                for kid in kids {
                    if let Obj::Ref(r) = kid {
                        self.walk_pages(*r, out, visited, depth + 1)?;
                    }
                }
            }
            (_, None) => out.push(node),
        }
        Ok(())
    }

    /// An inheritable page attribute (§7.7.3.4): the page's own, or the
    /// nearest ancestor's.
    pub fn inherited(&self, page: &Dict, key: &[u8]) -> Result<Option<Obj>> {
        if let Some(v) = page.get(key) {
            return Ok(Some(v.clone()));
        }
        let mut parent = page.get(b"Parent").cloned();
        for _ in 0..64 {
            let Some(p) = parent.take() else { break };
            let Some(d) = self.resolve_dict(&p)? else {
                break;
            };
            if let Some(v) = d.get(key) {
                return Ok(Some(v.clone()));
            }
            parent = d.get(b"Parent").cloned();
        }
        Ok(None)
    }

    /// Writes the whole file: only what the trailer still reaches, unchanged
    /// objects byte for byte.
    pub fn write(&self) -> Result<Vec<u8>> {
        let reachable = self.reachable()?;
        let mut out = Vec::with_capacity(self.src.len());
        let header_end = self
            .src
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
            .unwrap_or(0);
        let header = self.src.get(..header_end).unwrap_or_default();
        if header.starts_with(b"%PDF-") {
            out.extend_from_slice(header);
        } else {
            out.extend_from_slice(b"%PDF-1.7");
        }
        out.extend_from_slice(b"\n%\xE2\xE3\xCF\xD3\n");

        let mut offsets: BTreeMap<u32, (usize, u16)> = BTreeMap::new();
        for &num in &reachable {
            let start = out.len();
            if let Some(stored) = self.changed.get(&num) {
                out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
                write_stored(&mut out, stored);
                out.extend_from_slice(b"\nendobj\n");
                offsets.insert(num, (start, 0));
            } else if let Some(entry) = self.xref.get(&num) {
                let (_, end) = self.load_at(num, *entry)?;
                // Copy through `endobj`, which follows the object (or the
                // stream's `endstream`).
                let rest = self.src.get(end..).unwrap_or_default();
                let tail = rest
                    .windows(6)
                    .position(|w| w == b"endobj")
                    .ok_or_else(|| RedactError::Syntax(format!("endobj {num} tidak ada")))?;
                let bytes = self
                    .src
                    .get(entry.offset..end + tail + 6)
                    .ok_or_else(|| RedactError::Syntax(format!("objek {num} terpotong")))?;
                out.extend_from_slice(bytes);
                out.push(b'\n');
                offsets.insert(num, (start, entry.gen));
            }
        }

        let size = offsets.keys().next_back().map_or(1, |n| n + 1);
        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        for num in 0..size {
            match offsets.get(&num) {
                Some((off, gen)) => {
                    out.extend_from_slice(format!("{off:010} {gen:05} n\r\n").as_bytes());
                }
                None => out.extend_from_slice(b"0000000000 65535 f\r\n"),
            }
        }
        let mut trailer = Dict::new();
        trailer.set(b"Size", Obj::Int(i64::from(size)));
        for key in [&b"Root"[..], b"Info", b"ID"] {
            if let Some(v) = self.trailer.get(key) {
                trailer.set(key, v.clone());
            }
        }
        out.extend_from_slice(b"trailer\n");
        write_dict(&mut out, &trailer);
        out.extend_from_slice(format!("\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
        Ok(out)
    }

    /// Object numbers reachable from the trailer, sorted.
    pub fn reachable(&self) -> Result<BTreeSet<u32>> {
        let mut seen = BTreeSet::new();
        let mut todo: Vec<u32> = Vec::new();
        let push_refs = |obj: &Obj, todo: &mut Vec<u32>| collect_refs(obj, todo);
        for key in [&b"Root"[..], b"Info"] {
            if let Some(v) = self.trailer.get(key) {
                push_refs(v, &mut todo);
            }
        }
        while let Some(num) = todo.pop() {
            if !seen.insert(num) {
                continue;
            }
            match self.get(num) {
                Ok(Stored::Obj(o)) => push_refs(&o, &mut todo),
                Ok(Stored::Stream(s)) => push_refs(&Obj::Dict(s.dict), &mut todo),
                Err(RedactError::Missing(_)) => {
                    seen.remove(&num);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(seen)
    }
}

fn collect_refs(obj: &Obj, out: &mut Vec<u32>) {
    match obj {
        Obj::Ref(r) => out.push(r.num),
        Obj::Array(items) => items.iter().for_each(|i| collect_refs(i, out)),
        Obj::Dict(d) => d.0.iter().for_each(|(_, v)| collect_refs(v, out)),
        _ => {}
    }
}

fn write_stored(out: &mut Vec<u8>, stored: &Stored) {
    match stored {
        Stored::Obj(o) => write_obj(out, o),
        Stored::Stream(s) => {
            let mut dict = s.dict.clone();
            dict.set(b"Length", Obj::Int(s.data.len() as i64));
            write_dict(out, &dict);
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(&s.data);
            out.extend_from_slice(b"\nendstream");
        }
    }
}

/// Reads one classic cross-reference section at `off` into `xref` (entries
/// already present, from a later section, win) and returns its trailer.
fn read_section(src: &[u8], off: usize, xref: &mut BTreeMap<u32, Entry>) -> Result<Dict> {
    let mut lx = Lexer::new(src, off);
    match lx.next_token()? {
        Some(Token::Keyword(k)) if k == b"xref" => {}
        _ => {
            return Err(RedactError::Unsupported(
                "tabel referensi silang bukan bentuk klasik".into(),
            ))
        }
    }
    loop {
        match lx.next_token()? {
            Some(Token::Keyword(k)) if k == b"trailer" => {
                let t = lx
                    .next_token()?
                    .ok_or_else(|| RedactError::Syntax("trailer kosong".into()))?;
                return match parse_from(&mut lx, t, true, 0)? {
                    Obj::Dict(d) => Ok(d),
                    _ => Err(RedactError::Syntax("trailer bukan kamus".into())),
                };
            }
            Some(Token::Int(start)) => {
                let Some(Token::Int(count)) = lx.next_token()? else {
                    return Err(RedactError::Syntax("subbagian xref rusak".into()));
                };
                let start = u32::try_from(start)
                    .map_err(|_| RedactError::Syntax("nomor objek negatif".into()))?;
                for i in 0..u32::try_from(count).unwrap_or(0) {
                    let (Some(Token::Int(o)), Some(Token::Int(g)), Some(Token::Keyword(kind))) =
                        (lx.next_token()?, lx.next_token()?, lx.next_token()?)
                    else {
                        return Err(RedactError::Syntax("entri xref rusak".into()));
                    };
                    let num = start + i;
                    if kind == b"n" && o > 0 {
                        xref.entry(num).or_insert(Entry {
                            offset: usize::try_from(o).unwrap_or(0),
                            gen: u16::try_from(g).unwrap_or(0),
                        });
                    }
                }
            }
            _ => return Err(RedactError::Syntax("bagian xref rusak".into())),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tiny well-formed file with the given objects (1-based), in the shape
    /// PDFium writes: classic xref, trailer, startxref.
    pub(crate) fn build(objects: &[&[u8]]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offs = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offs.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f\r\n");
        for o in offs {
            out.extend_from_slice(format!("{o:010} 00000 n\r\n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    fn sample() -> Vec<u8> {
        build(&[
            b"<</Type/Catalog/Pages 2 0 R>>",
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 100 100]/Contents 4 0 R>>",
            b"<</Length 5 0 R>>\nstream\nBT ET\nendstream",
            b"6",
            b"(orphan)",
        ])
    }

    #[test]
    fn reads_objects_and_indirect_stream_lengths() {
        let src = sample();
        let pdf = Pdf::parse(&src).unwrap();
        let Stored::Stream(s) = pdf.get(4).unwrap() else {
            panic!("bukan stream")
        };
        assert_eq!(s.data, b"BT ET\n");
        assert_eq!(pdf.pages().unwrap(), vec![Ref { num: 3, gen: 0 }]);
    }

    /// The orphan is not written, and a replaced stream's old bytes are gone.
    #[test]
    fn writes_only_what_is_reachable_and_nothing_replaced() {
        let src = sample();
        let mut pdf = Pdf::parse(&src).unwrap();
        pdf.set(
            4,
            Stored::Stream(Stream {
                dict: Dict::new(),
                data: b"q Q".to_vec(),
            }),
        );
        let out = pdf.write().unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(!text.contains("orphan"), "{text}");
        assert!(!text.contains("BT ET"), "{text}");
        assert!(text.contains("q Q"));
        let again = Pdf::parse(&out).unwrap();
        let Stored::Stream(s) = again.get(4).unwrap() else {
            panic!("bukan stream")
        };
        assert_eq!(s.data, b"q Q");
        assert!(again.get(6).is_err(), "objek yatim tidak ditulis");
        assert_eq!(again.pages().unwrap().len(), 1);
    }

    #[test]
    fn an_xref_stream_is_refused_not_guessed() {
        let mut src = b"%PDF-1.5\n1 0 obj\n<</Type/XRef>>\nstream\n\nendstream\nendobj\n".to_vec();
        src.extend_from_slice(b"startxref\n9\n%%EOF\n");
        assert!(matches!(Pdf::parse(&src), Err(RedactError::Unsupported(_))));
    }

    #[test]
    fn encrypted_files_are_refused() {
        let mut src = sample();
        let at = find_last(&src, b"/Root 1 0 R").unwrap();
        src.splice(at..at, b"/Encrypt 5 0 R".iter().copied());
        assert!(matches!(Pdf::parse(&src), Err(RedactError::Encrypted)));
    }
}
