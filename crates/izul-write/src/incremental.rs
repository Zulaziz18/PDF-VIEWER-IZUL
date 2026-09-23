//! Reading the tail of a file PDFium has just written, and appending one
//! incremental update to it (ISO 32000-1 §7.5.6).
//!
//! The input is always `FPDF_SaveAsCopy` output with `FPDF_NO_INCREMENTAL`,
//! never a user's original file. Measured on the pinned PDFium build: that
//! output always ends in a classic cross-reference table and a direct trailer
//! (`/Size`, `/Root`, `/Info`, `/ID`), even when the source used cross-reference
//! streams and object streams, and every object sits at the offset the table
//! gives. That is the whole of what this module relies on, and every piece of
//! it is checked rather than assumed — a table or trailer that does not parse
//! is an error, never a guess.

use std::collections::{BTreeMap, HashMap};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SyntaxError {
    #[error("akhir berkas tidak dikenali: {0}")]
    Tail(&'static str),
    #[error("tabel xref rusak: {0}")]
    Xref(String),
    #[error("dokumen terenkripsi tidak dapat disunting-simpan")]
    Encrypted,
}

/// The trailer of the section being extended.
#[derive(Debug, Clone, PartialEq)]
pub struct Trailer {
    /// Byte offset of the `xref` keyword, for `/Prev`.
    pub startxref: usize,
    pub size: u32,
    pub root: String,
    pub info: Option<String>,
    pub id: Option<String>,
}

/// In-use objects of the final section: number -> (offset, generation).
pub type XrefTable = BTreeMap<u32, (usize, u16)>;

fn find_last(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

fn find_from(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn is_delim(b: u8) -> bool {
    is_ws(b) || matches!(b, b'/' | b'[' | b']' | b'<' | b'>' | b'(' | b')' | b'%')
}

/// The value following `/Key` in a dictionary's text, as raw text: an integer,
/// a reference `n g R`, or a bracketed array. Enough for a trailer.
fn value_after(dict: &str, key: &str) -> Option<String> {
    let bytes = dict.as_bytes();
    let pat = format!("/{key}");
    let mut from = 0usize;
    loop {
        let at = dict.get(from..)?.find(&pat)? + from;
        let end = at + pat.len();
        // `/Size` must not match `/SizeX`.
        if bytes.get(end).is_none_or(|b| is_delim(*b)) {
            let rest = dict.get(end..)?.trim_start();
            if rest.starts_with('[') {
                let close = rest.find(']')?;
                return Some(rest.get(..=close)?.to_string());
            }
            // Up to three tokens for a reference, one for a number.
            let tokens: Vec<&str> = rest
                .split(|c: char| c.is_whitespace() || "/[]<>".contains(c))
                .filter(|t| !t.is_empty())
                .take(3)
                .collect();
            return match tokens.as_slice() {
                [n, g, "R", ..] => Some(format!("{n} {g} R")),
                [n, ..] => Some((*n).to_string()),
                [] => None,
            };
        }
        from = end;
    }
}

/// Parses the final trailer and cross-reference table.
pub fn read_tail(pdf: &[u8]) -> Result<(Trailer, XrefTable), SyntaxError> {
    let sx = find_last(pdf, b"startxref").ok_or(SyntaxError::Tail("startxref tidak ada"))?;
    let after = pdf
        .get(sx + 9..)
        .ok_or(SyntaxError::Tail("startxref terpotong"))?;
    let digits: String = after
        .iter()
        .skip_while(|b| is_ws(**b))
        .take_while(|b| b.is_ascii_digit())
        .map(|b| *b as char)
        .collect();
    let startxref: usize = digits
        .parse()
        .map_err(|_| SyntaxError::Tail("offset startxref bukan angka"))?;
    if pdf.get(startxref..startxref + 4) != Some(b"xref") {
        // An xref *stream* here would mean a PDFium build that writes them;
        // this module was measured against one that does not, and says so
        // rather than misreading the file.
        return Err(SyntaxError::Tail("bukan tabel xref klasik"));
    }
    let trailer_at =
        find_from(pdf, startxref, b"trailer").ok_or(SyntaxError::Tail("trailer tidak ada"))?;

    // The table: subsection headers `start count`, then `count` entries.
    let table = std::str::from_utf8(pdf.get(startxref + 4..trailer_at).unwrap_or_default())
        .map_err(|_| SyntaxError::Xref("bukan teks".into()))?;
    let mut xref = XrefTable::new();
    let mut lines = table.lines().map(str::trim).filter(|l| !l.is_empty());
    while let Some(header) = lines.next() {
        let mut parts = header.split_whitespace();
        let (Some(start), Some(count), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(SyntaxError::Xref(format!("kepala subbagian: {header}")));
        };
        let start: u32 = start
            .parse()
            .map_err(|_| SyntaxError::Xref(header.into()))?;
        let count: u32 = count
            .parse()
            .map_err(|_| SyntaxError::Xref(header.into()))?;
        for i in 0..count {
            let entry = lines
                .next()
                .ok_or_else(|| SyntaxError::Xref("entri kurang".into()))?;
            let f: Vec<&str> = entry.split_whitespace().collect();
            let [off, gen, kind] = f.as_slice() else {
                return Err(SyntaxError::Xref(format!("entri: {entry}")));
            };
            if *kind == "n" {
                let off: usize = off.parse().map_err(|_| SyntaxError::Xref(entry.into()))?;
                let gen: u16 = gen.parse().map_err(|_| SyntaxError::Xref(entry.into()))?;
                xref.insert(start + i, (off, gen));
            }
        }
    }

    let dict_start = find_from(pdf, trailer_at, b"<<").ok_or(SyntaxError::Tail("kamus trailer"))?;
    let dict_end = find_from(pdf, dict_start, b"startxref").unwrap_or(pdf.len());
    let dict = String::from_utf8_lossy(pdf.get(dict_start..dict_end).unwrap_or_default());
    if value_after(&dict, "Encrypt").is_some() {
        return Err(SyntaxError::Encrypted);
    }
    let size: u32 = value_after(&dict, "Size")
        .and_then(|s| s.parse().ok())
        .ok_or(SyntaxError::Tail("/Size tidak ada"))?;
    let root = value_after(&dict, "Root").ok_or(SyntaxError::Tail("/Root tidak ada"))?;
    Ok((
        Trailer {
            startxref,
            size,
            root,
            info: value_after(&dict, "Info"),
            id: value_after(&dict, "ID"),
        },
        xref,
    ))
}

/// Decodes a PDF string token starting at `s[0]` (`(` or `<`) to text.
fn read_string(s: &[u8]) -> Option<String> {
    match s.first()? {
        b'(' => {
            let mut out = Vec::new();
            let mut depth = 0i32;
            let mut i = 1usize;
            while let Some(&b) = s.get(i) {
                match b {
                    b'\\' => {
                        let n = *s.get(i + 1)?;
                        out.push(match n {
                            b'n' => b'\n',
                            b'r' => b'\r',
                            b't' => b'\t',
                            other => other,
                        });
                        i += 2;
                        continue;
                    }
                    b'(' => depth += 1,
                    b')' if depth == 0 => break,
                    b')' => depth -= 1,
                    _ => {}
                }
                out.push(b);
                i += 1;
            }
            Some(decode_text(&out))
        }
        b'<' => {
            let end = s.iter().position(|b| *b == b'>')?;
            let hex: Vec<u8> = s
                .get(1..end)?
                .iter()
                .copied()
                .filter(|b| !is_ws(*b))
                .collect();
            let mut bytes = Vec::with_capacity(hex.len() / 2);
            for pair in hex.chunks(2) {
                let text = std::str::from_utf8(pair).ok()?;
                let padded = if text.len() == 1 {
                    format!("{text}0")
                } else {
                    text.to_string()
                };
                bytes.push(u8::from_str_radix(&padded, 16).ok()?);
            }
            Some(decode_text(&bytes))
        }
        _ => None,
    }
}

fn decode_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes
            .get(2..)
            .unwrap_or_default()
            .chunks_exact(2)
            .map(|c| {
                u16::from_be_bytes([
                    c.first().copied().unwrap_or(0),
                    c.get(1).copied().unwrap_or(0),
                ])
            })
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|b| *b as char).collect()
    }
}

/// Where PDFium put a placeholder.
///
/// Measured: an annotation `FPDFPage_CreateAnnot` makes on a page PDFium did
/// not otherwise rewrite is written **inline**, as a direct dictionary inside
/// the page's `/Annots` array — not as an object of its own. Treating the page
/// object as if it were the annotation replaced the whole page with an
/// annotation dictionary, and the page no longer loaded; the round-trip test in
/// `izul-pdf` caught it. So both shapes are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// The placeholder is object `num` itself.
    Object { num: u32, gen: u16 },
    /// The placeholder is the dictionary at `span` (absolute byte offsets,
    /// `<<` to `>>` inclusive) inside object `container`, whose body — the
    /// bytes between `obj` and `endobj` — is `body`.
    Inline {
        container: u32,
        gen: u16,
        span: (usize, usize),
        body: (usize, usize),
    },
}

/// What a scan of one object's top-level dictionary found.
struct DictScan {
    /// Every dictionary closed during the scan, `(start, end_exclusive)`.
    spans: Vec<(usize, usize)>,
    /// Start of every `/NM` key.
    nms: Vec<usize>,
    /// One past the top-level dictionary's closing `>>`.
    end: usize,
}

/// Scans the dictionary starting at `at` (which must be `<<`), skipping
/// strings and comments so that brackets inside them do not count. Never
/// reads past the top-level dictionary — a stream's bytes are not touched.
fn scan_dict(pdf: &[u8], at: usize) -> Option<DictScan> {
    if pdf.get(at..at + 2) != Some(b"<<") {
        return None;
    }
    let mut stack: Vec<usize> = Vec::new();
    let mut spans = Vec::new();
    let mut nms = Vec::new();
    let mut i = at;
    while let Some(&b) = pdf.get(i) {
        match b {
            b'<' if pdf.get(i + 1) == Some(&b'<') => {
                stack.push(i);
                i += 2;
            }
            b'>' if pdf.get(i + 1) == Some(&b'>') => {
                let start = stack.pop()?;
                spans.push((start, i + 2));
                i += 2;
                if stack.is_empty() {
                    return Some(DictScan { spans, nms, end: i });
                }
            }
            b'<' => {
                // Hex string.
                i += pdf.get(i..)?.iter().position(|c| *c == b'>')? + 1;
            }
            b'(' => {
                let mut depth = 0i32;
                i += 1;
                loop {
                    match *pdf.get(i)? {
                        b'\\' => i += 2,
                        b'(' => {
                            depth += 1;
                            i += 1;
                        }
                        b')' if depth == 0 => {
                            i += 1;
                            break;
                        }
                        b')' => {
                            depth -= 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'%' => {
                i += pdf
                    .get(i..)?
                    .iter()
                    .position(|c| *c == b'\n' || *c == b'\r')?;
            }
            b'/' => {
                if pdf.get(i..i + 3) == Some(b"/NM") && pdf.get(i + 3).is_none_or(|c| is_delim(*c))
                {
                    nms.push(i);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// Finds where PDFium put the placeholder for each wanted id.
///
/// Walks the table from the highest object number down, because the
/// placeholders — or the pages holding them — are among the objects PDFium
/// touched last; the walk stops as soon as every id is found. Only each
/// object's top-level dictionary is scanned, never a stream's bytes.
pub fn find_placeholders(pdf: &[u8], xref: &XrefTable, ids: &[u64]) -> HashMap<u64, Placement> {
    let mut wanted: HashMap<String, u64> = ids
        .iter()
        .map(|id| (format!("{}{id}", crate::metadata::NM_PREFIX), *id))
        .collect();
    let mut found = HashMap::new();
    for (&num, &(offset, gen)) in xref.iter().rev() {
        if wanted.is_empty() {
            break;
        }
        let Some(head_end) = find_from(pdf, offset, b"obj").map(|p| p + 3) else {
            continue;
        };
        let Some(body_start) = pdf
            .get(head_end..)
            .and_then(|r| r.iter().position(|b| !is_ws(*b)))
            .map(|p| p + head_end)
        else {
            continue;
        };
        let Some(scan) = scan_dict(pdf, body_start) else {
            continue;
        };
        if scan.nms.is_empty() {
            continue;
        }
        // A stream object's dictionary is followed by `stream`; ours never
        // are, and rewriting one would mean copying its data.
        let after = pdf
            .get(scan.end..)
            .and_then(|r| r.iter().position(|b| !is_ws(*b)))
            .map(|p| p + scan.end);
        if after.is_some_and(|a| pdf.get(a..a + 6) == Some(b"stream")) {
            continue;
        }
        let Some(body_end) = find_from(pdf, scan.end, b"endobj") else {
            continue;
        };
        for nm in &scan.nms {
            let Some(value_at) = pdf
                .get(nm + 3..)
                .and_then(|r| r.iter().position(|b| !is_ws(*b)))
                .map(|p| p + nm + 3)
            else {
                continue;
            };
            let Some(value) = pdf.get(value_at..).and_then(read_string) else {
                continue;
            };
            let Some(id) = wanted.remove(&value) else {
                continue;
            };
            // The innermost dictionary around this `/NM` is the annotation.
            let span = scan
                .spans
                .iter()
                .filter(|(s, e)| *s < *nm && *e > *nm)
                .max_by_key(|(s, _)| *s)
                .copied();
            let placement = match span {
                Some(span) if span.0 == body_start && span.1 == scan.end => {
                    Placement::Object { num, gen }
                }
                Some(span) => Placement::Inline {
                    container: num,
                    gen,
                    span,
                    body: (body_start, body_end),
                },
                None => continue,
            };
            found.insert(id, placement);
        }
    }
    found
}

/// Objects to append, and the arithmetic of the cross-reference section.
#[derive(Debug)]
pub struct Update {
    next: u32,
    objects: BTreeMap<u32, (u16, Vec<u8>)>,
}

impl Update {
    pub fn new(trailer: &Trailer) -> Self {
        Update {
            next: trailer.size,
            objects: BTreeMap::new(),
        }
    }

    /// A new object number.
    pub fn reserve(&mut self) -> u32 {
        let n = self.next;
        self.next += 1;
        n
    }

    /// Sets (or redefines) object `num` with generation `gen`. `body` is what
    /// goes between `obj` and `endobj`.
    pub fn put(&mut self, num: u32, gen: u16, body: Vec<u8>) {
        self.objects.insert(num, (gen, body));
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// `pdf` with the update appended.
    pub fn finish(self, mut pdf: Vec<u8>, trailer: &Trailer) -> Vec<u8> {
        if !pdf.ends_with(b"\n") && !pdf.ends_with(b"\r") {
            pdf.push(b'\n');
        }
        let mut offsets: BTreeMap<u32, (usize, u16)> = BTreeMap::new();
        for (num, (gen, body)) in &self.objects {
            offsets.insert(*num, (pdf.len(), *gen));
            pdf.extend_from_slice(format!("{num} {gen} obj\n").as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref_at = pdf.len();
        pdf.extend_from_slice(b"xref\n");
        // Contiguous runs become one subsection each.
        let nums: Vec<u32> = offsets.keys().copied().collect();
        let mut i = 0usize;
        while let Some(&start) = nums.get(i) {
            let mut end = i;
            while nums
                .get(end + 1)
                .is_some_and(|n| *n == nums.get(end).copied().unwrap_or(0) + 1)
            {
                end += 1;
            }
            pdf.extend_from_slice(format!("{start} {}\n", end - i + 1).as_bytes());
            for n in nums.get(i..=end).unwrap_or_default() {
                let (off, gen) = offsets.get(n).copied().unwrap_or((0, 0));
                // Exactly twenty bytes per entry, EOL included (§7.5.4).
                pdf.extend_from_slice(format!("{off:010} {gen:05} n\r\n").as_bytes());
            }
            i = end + 1;
        }
        let size = self.next.max(trailer.size);
        let mut dict = format!("<</Size {size}/Root {}", trailer.root);
        if let Some(info) = &trailer.info {
            dict.push_str(&format!("/Info {info}"));
        }
        if let Some(id) = &trailer.id {
            dict.push_str(&format!("/ID{id}"));
        }
        dict.push_str(&format!("/Prev {}>>", trailer.startxref));
        pdf.extend_from_slice(format!("trailer\n{dict}\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
        pdf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny file shaped exactly like PDFium's output: CRLF, classic table.
    fn pdfium_like() -> Vec<u8> {
        let objs = [
            "<</Type/Catalog/Pages 2 0 R>>",
            "<</Type/Pages/Kids[3 0 R]/Count 1>>",
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Annots[4 0 R 5 0 R]>>",
            "<</NM(izul-7)/Rect[0 0 1 1]/Subtype/Square/Type/Annot>>",
            "<</NM<FEFF00690007A0075006C002D0038>/Subtype/Square>>",
            "<</NM <FEFF0069007A0075006C002D0039> /Subtype/Square>>",
        ];
        let mut pdf = b"%PDF-1.7\r\n".to_vec();
        let mut offs = Vec::new();
        for (i, body) in objs.iter().enumerate() {
            offs.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\r\n{body}\r\nendobj\r\n", i + 1).as_bytes());
        }
        let x = pdf.len();
        pdf.extend_from_slice(b"xref\r\n0 7\r\n0000000000 65535 f\r\n");
        for o in offs {
            pdf.extend_from_slice(format!("{o:010} 00000 n\r\n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\r\n<</Root 1 0 R /Size 7/ID[<AA><BB>]>>\r\nstartxref\r\n{x}\r\n%%EOF\r\n"
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn reads_pdfiums_tail() {
        let pdf = pdfium_like();
        let (t, xref) = read_tail(&pdf).unwrap();
        assert_eq!(t.size, 7);
        assert_eq!(t.root, "1 0 R");
        assert_eq!(t.info, None);
        assert_eq!(t.id.as_deref(), Some("[<AA><BB>]"));
        assert_eq!(xref.len(), 6);
        assert!(pdf[xref[&4].0..].starts_with(b"4 0 obj"));
    }

    #[test]
    fn finds_placeholders_by_literal_and_hex_names() {
        let pdf = pdfium_like();
        let (_, xref) = read_tail(&pdf).unwrap();
        // The hex one spells "iz" + garbage, so it must *not* match 8.
        let found = find_placeholders(&pdf, &xref, &[7, 8, 9]);
        assert_eq!(found.get(&7), Some(&Placement::Object { num: 4, gen: 0 }));
        assert_eq!(found.get(&8), None);
        assert_eq!(
            found.get(&9),
            Some(&Placement::Object { num: 6, gen: 0 }),
            "UTF-16 hex names are decoded"
        );
    }

    #[test]
    fn finds_a_placeholder_written_inline_in_the_page() {
        // Exactly what PDFium wrote in the round-trip test that caught this.
        let body = "<</Annots[<</NM(izul-5)/Subtype/Square/Type/Annot>> <</NM(other)/T(a >> b)>>]/Contents 4 0 R /Type/Page>>";
        let mut pdf = b"%PDF-1.7\r\n".to_vec();
        let off = pdf.len();
        pdf.extend_from_slice(format!("3 0 obj\r\n{body}\r\nendobj\r\n").as_bytes());
        let x = pdf.len();
        pdf.extend_from_slice(format!("xref\r\n3 1\r\n{off:010} 00000 n\r\ntrailer\r\n<</Root 1 0 R/Size 4>>\r\nstartxref\r\n{x}\r\n%%EOF").as_bytes());
        let (_, xref) = read_tail(&pdf).unwrap();
        let found = find_placeholders(&pdf, &xref, &[5]);
        let Some(Placement::Inline {
            container,
            span,
            body: (bs, be),
            ..
        }) = found.get(&5).copied()
        else {
            panic!("{found:?}");
        };
        assert_eq!(container, 3);
        assert_eq!(
            &pdf[span.0..span.1],
            b"<</NM(izul-5)/Subtype/Square/Type/Annot>>"
        );
        assert_eq!(&pdf[bs..be].trim_ascii_end(), &body.as_bytes());
    }

    #[test]
    fn an_update_chains_to_the_previous_section() {
        let pdf = pdfium_like();
        let (t, _) = read_tail(&pdf).unwrap();
        let mut up = Update::new(&t);
        let fresh = up.reserve();
        assert_eq!(fresh, 7);
        up.put(
            4,
            0,
            b"<</Subtype/Square/NM(izul-7)/Contents(baru)>>".to_vec(),
        );
        up.put(fresh, 0, b"<</Type/Font>>".to_vec());
        let out = up.finish(pdf.clone(), &t);
        assert!(out.starts_with(&pdf), "the original bytes stay untouched");

        // Reading the result back finds the new section, and its entries point
        // at the new definitions.
        let (t2, xref2) = read_tail(&out).unwrap();
        assert_eq!(t2.size, 8);
        assert_eq!(t2.root, "1 0 R");
        let tail = String::from_utf8_lossy(&out[t.startxref..]);
        assert!(tail.contains(&format!("/Prev {}", t.startxref)));
        assert!(out[xref2[&4].0..].starts_with(b"4 0 obj\n<</Subtype/Square"));
        assert!(out[xref2[&7].0..].starts_with(b"7 0 obj\n<</Type/Font>>"));
        // 4 and 7 are not contiguous, so they are two subsections.
        let section = String::from_utf8_lossy(&out[t2.startxref..]);
        assert!(
            section.contains("\n4 1\n") && section.contains("\n7 1\n"),
            "{section}"
        );
    }

    #[test]
    fn every_entry_is_twenty_bytes() {
        let pdf = pdfium_like();
        let (t, _) = read_tail(&pdf).unwrap();
        let mut up = Update::new(&t);
        for _ in 0..3 {
            let n = up.reserve();
            up.put(n, 0, b"null".to_vec());
        }
        let out = up.finish(pdf, &t);
        let (t2, _) = read_tail(&out).unwrap();
        let section = &out[t2.startxref..];
        let text = String::from_utf8_lossy(section);
        let entries: Vec<&str> = text.lines().filter(|l| l.ends_with(" n")).collect();
        assert_eq!(entries.len(), 3);
        for e in entries {
            assert_eq!(e.len(), 18, "{e:?}");
            assert!(
                text.contains(&format!("{e}\r\n")),
                "two-byte EOL after {e:?}"
            );
        }
    }

    #[test]
    fn an_encrypted_trailer_is_refused() {
        let pdf = pdfium_like();
        let text = String::from_utf8_lossy(&pdf).replace("/Size 7", "/Size 7/Encrypt 9 0 R");
        assert_eq!(
            read_tail(text.as_bytes()).map(|_| ()),
            Err(SyntaxError::Encrypted)
        );
    }

    #[test]
    fn a_file_without_a_table_is_an_error_not_a_guess() {
        assert!(read_tail(b"%PDF-1.7\nstartxref\n0\n%%EOF").is_err());
        assert!(read_tail(b"garbage").is_err());
    }
}
