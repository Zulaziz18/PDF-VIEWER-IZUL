//! Stream filters (ISO 32000-1 §7.4).
//!
//! Decoding is needed for two things: content streams, which must be read to
//! find the glyphs, and images, whose pixels under a marked area must be
//! replaced. Encoding is only ever Flate: whatever is rewritten is written back
//! compressed, and nothing else needs to be produced.
//!
//! An image codec (`DCTDecode`, `JPXDecode`, `JBIG2Decode`, `CCITTFaxDecode`)
//! ends the chain: the general filters before it are applied and the codec is
//! reported for the image code to deal with, or to refuse.

use std::io::{Read, Write};

use crate::error::{RedactError, Result};
use crate::file::Pdf;
use crate::object::{Dict, Obj};

/// A stream's data with its general-purpose filters removed.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub data: Vec<u8>,
    /// An image codec still to apply, with its parameters.
    pub codec: Option<(Vec<u8>, Option<Dict>)>,
}

const IMAGE_CODECS: [&[u8]; 4] = [
    b"DCTDecode",
    b"JPXDecode",
    b"JBIG2Decode",
    b"CCITTFaxDecode",
];

/// The filters and their parameters, abbreviations expanded (inline images
/// use `/F`, `/DP` and short filter names).
pub fn chain(pdf: Option<&Pdf<'_>>, dict: &Dict) -> Result<Vec<(Vec<u8>, Option<Dict>)>> {
    let resolve = |o: &Obj| -> Result<Obj> {
        match pdf {
            Some(p) => p.resolve(o),
            None => Ok(o.clone()),
        }
    };
    let filter = dict.get(b"Filter").or_else(|| dict.get(b"F"));
    let parms = dict.get(b"DecodeParms").or_else(|| dict.get(b"DP"));
    let names: Vec<Vec<u8>> = match filter.map(&resolve).transpose()? {
        None | Some(Obj::Null) => Vec::new(),
        Some(Obj::Name(n)) => vec![n],
        Some(Obj::Array(items)) => items
            .iter()
            .map(|i| match resolve(i)? {
                Obj::Name(n) => Ok(n),
                _ => Err(RedactError::Filter("nama filter bukan nama".into())),
            })
            .collect::<Result<_>>()?,
        Some(_) => return Err(RedactError::Filter("/Filter bukan nama".into())),
    };
    let parms: Vec<Option<Dict>> = match parms.map(&resolve).transpose()? {
        Some(Obj::Dict(d)) => vec![Some(d)],
        Some(Obj::Array(items)) => items
            .iter()
            .map(|i| {
                Ok(match resolve(i)? {
                    Obj::Dict(d) => Some(d),
                    _ => None,
                })
            })
            .collect::<Result<_>>()?,
        _ => Vec::new(),
    };
    Ok(names
        .into_iter()
        .enumerate()
        .map(|(i, n)| (expand(&n), parms.get(i).cloned().flatten()))
        .collect())
}

fn expand(name: &[u8]) -> Vec<u8> {
    let full: &[u8] = match name {
        b"AHx" => b"ASCIIHexDecode",
        b"A85" => b"ASCII85Decode",
        b"LZW" => b"LZWDecode",
        b"Fl" => b"FlateDecode",
        b"RL" => b"RunLengthDecode",
        b"CCF" => b"CCITTFaxDecode",
        b"DCT" => b"DCTDecode",
        other => other,
    };
    full.to_vec()
}

/// Applies every general filter in the chain.
pub fn decode(pdf: Option<&Pdf<'_>>, dict: &Dict, data: &[u8]) -> Result<Decoded> {
    let filters = chain(pdf, dict)?;
    let mut cur = data.to_vec();
    let mut iter = filters.into_iter().peekable();
    while let Some((name, parms)) = iter.next() {
        if IMAGE_CODECS.contains(&name.as_slice()) {
            if iter.peek().is_some() {
                return Err(RedactError::Filter(format!(
                    "filter sesudah {}",
                    String::from_utf8_lossy(&name)
                )));
            }
            return Ok(Decoded {
                data: cur,
                codec: Some((name, parms)),
            });
        }
        cur = match name.as_slice() {
            b"FlateDecode" => predict(inflate(&cur)?, parms.as_ref())?,
            b"LZWDecode" => {
                let early = parms
                    .as_ref()
                    .and_then(|p| p.get(b"EarlyChange"))
                    .and_then(Obj::as_i64)
                    .unwrap_or(1);
                predict(lzw(&cur, early != 0)?, parms.as_ref())?
            }
            b"ASCIIHexDecode" => ascii_hex(&cur),
            b"ASCII85Decode" => ascii85(&cur)?,
            b"RunLengthDecode" => run_length(&cur),
            b"Crypt" => {
                // Only the identity crypt filter can appear in an unencrypted
                // file.
                cur
            }
            other => {
                return Err(RedactError::Filter(format!(
                    "filter {} tidak didukung",
                    String::from_utf8_lossy(other)
                )))
            }
        };
    }
    Ok(Decoded {
        data: cur,
        codec: None,
    })
}

/// Decodes a stream that must not end in an image codec (a content stream).
pub fn decode_plain(pdf: Option<&Pdf<'_>>, dict: &Dict, data: &[u8]) -> Result<Vec<u8>> {
    let d = decode(pdf, dict, data)?;
    if let Some((codec, _)) = d.codec {
        return Err(RedactError::Filter(format!(
            "{} pada stream yang bukan gambar",
            String::from_utf8_lossy(&codec)
        )));
    }
    Ok(d.data)
}

/// zlib, falling back to a raw deflate stream; a truncated stream yields what
/// it holds, as it does in every renderer.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let zlib = flate2::read::ZlibDecoder::new(data).read_to_end(&mut out);
    if zlib.is_ok() || !out.is_empty() {
        return Ok(out);
    }
    out.clear();
    let raw = flate2::read::DeflateDecoder::new(data).read_to_end(&mut out);
    if raw.is_ok() || !out.is_empty() {
        return Ok(out);
    }
    if data.is_empty() {
        return Ok(out);
    }
    Err(RedactError::Filter("data Flate rusak".into()))
}

pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing into a Vec cannot fail.
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

fn lzw(data: &[u8], early_change: bool) -> Result<Vec<u8>> {
    let mut dec = if early_change {
        weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
    } else {
        weezl::decode::Decoder::new(weezl::BitOrder::Msb, 8)
    };
    let mut out = Vec::new();
    let result = dec.into_stream(&mut out).decode_all(data);
    match result.status {
        Ok(_) => Ok(out),
        // As with Flate: what decoded before the damage is what readers show.
        Err(_) if !out.is_empty() => Ok(out),
        Err(e) => Err(RedactError::Filter(format!("LZW: {e}"))),
    }
}

fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut high: Option<u8> = None;
    for &b in data {
        if b == b'>' {
            break;
        }
        let v = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => continue,
        };
        match high.take() {
            Some(h) => out.push(h << 4 | v),
            None => high = Some(v),
        }
    }
    if let Some(h) = high {
        out.push(h << 4);
    }
    out
}

fn ascii85(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut group = [0u32; 5];
    let mut n = 0usize;
    let body = data.strip_prefix(b"<~").unwrap_or(data);
    for &b in body {
        match b {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0, 0, 0, 0]),
            b'!'..=b'u' => {
                if let Some(slot) = group.get_mut(n) {
                    *slot = u32::from(b - b'!');
                }
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u64, |acc, &d| acc * 85 + u64::from(d));
                    let v = u32::try_from(v)
                        .map_err(|_| RedactError::Filter("ASCII85 melampaui 32 bit".into()))?;
                    out.extend_from_slice(&v.to_be_bytes());
                    n = 0;
                }
            }
            _ if crate::object::is_white(b) => {}
            _ => return Err(RedactError::Filter("karakter ASCII85 tidak sah".into())),
        }
    }
    if n > 1 {
        for slot in group.iter_mut().skip(n) {
            *slot = 84;
        }
        let v = group.iter().fold(0u64, |acc, &d| acc * 85 + u64::from(d));
        let bytes = (v as u32).to_be_bytes();
        out.extend_from_slice(bytes.get(..n - 1).unwrap_or_default());
    }
    Ok(out)
}

fn run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&len) = data.get(i) {
        i += 1;
        match len {
            128 => break,
            0..=127 => {
                let n = usize::from(len) + 1;
                out.extend_from_slice(data.get(i..i + n).unwrap_or_default());
                i += n;
            }
            _ => {
                if let Some(&b) = data.get(i) {
                    out.extend(std::iter::repeat_n(b, 257 - usize::from(len)));
                }
                i += 1;
            }
        }
    }
    out
}

fn int_parm(parms: Option<&Dict>, key: &[u8], default: i64) -> i64 {
    parms
        .and_then(|p| p.get(key))
        .and_then(Obj::as_i64)
        .unwrap_or(default)
}

/// Undoes a PNG or TIFF predictor (§7.4.4.4).
fn predict(data: Vec<u8>, parms: Option<&Dict>) -> Result<Vec<u8>> {
    let predictor = int_parm(parms, b"Predictor", 1);
    if predictor <= 1 {
        return Ok(data);
    }
    let colors = usize::try_from(int_parm(parms, b"Colors", 1).clamp(1, 32)).unwrap_or(1);
    let bpc = usize::try_from(int_parm(parms, b"BitsPerComponent", 8).clamp(1, 16)).unwrap_or(8);
    let columns = usize::try_from(int_parm(parms, b"Columns", 1).max(1)).unwrap_or(1);
    let bpp = (colors * bpc).div_ceil(8).max(1);
    let row_len = (colors * bpc * columns).div_ceil(8);
    if predictor == 2 {
        if bpc != 8 {
            return Err(RedactError::Filter("prediktor TIFF selain 8 bit".into()));
        }
        let mut out = data;
        for row in out.chunks_mut(row_len) {
            for i in bpp..row.len() {
                let left = row.get(i - bpp).copied().unwrap_or(0);
                if let Some(v) = row.get_mut(i) {
                    *v = v.wrapping_add(left);
                }
            }
        }
        return Ok(out);
    }
    // PNG: each row carries its own filter type byte.
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row_len];
    for chunk in data.chunks(row_len + 1) {
        let Some((&kind, raw)) = chunk.split_first() else {
            break;
        };
        let mut row = raw.to_vec();
        row.resize(row_len, 0);
        for i in 0..row_len {
            let a = if i >= bpp {
                row.get(i - bpp).copied().unwrap_or(0)
            } else {
                0
            };
            let b = prev.get(i).copied().unwrap_or(0);
            let c = if i >= bpp {
                prev.get(i - bpp).copied().unwrap_or(0)
            } else {
                0
            };
            let add = match kind {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => paeth(a, b, c),
                _ => return Err(RedactError::Filter("jenis filter PNG tidak dikenal".into())),
            };
            if let Some(v) = row.get_mut(i) {
                *v = v.wrapping_add(add);
            }
        }
        out.extend_from_slice(&row);
        prev = row;
    }
    Ok(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let pa = (p - i16::from(a)).abs();
    let pb = (p - i16::from(b)).abs();
    let pc = (p - i16::from(c)).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(pairs: &[(&str, Obj)]) -> Dict {
        let mut d = Dict::new();
        for (k, v) in pairs {
            d.set(k.as_bytes(), v.clone());
        }
        d
    }

    #[test]
    fn flate_round_trips() {
        let data = b"BT /F1 12 Tf (rahasia) Tj ET".repeat(20);
        let d = dict(&[("Filter", Obj::name("FlateDecode"))]);
        assert_eq!(decode_plain(None, &d, &deflate(&data)).unwrap(), data);
    }

    #[test]
    fn chained_ascii85_then_flate() {
        // "Man " in ASCII85 is "9jqo^".
        let d = dict(&[("Filter", Obj::name("ASCII85Decode"))]);
        assert_eq!(decode_plain(None, &d, b"9jqo^~>").unwrap(), b"Man ");
        assert_eq!(decode_plain(None, &d, b"z~>").unwrap(), [0, 0, 0, 0]);
        // A final partial group: "Ma" is "9jn".
        assert_eq!(decode_plain(None, &d, b"9jn~>").unwrap(), b"Ma");
    }

    #[test]
    fn hex_and_run_length() {
        let d = dict(&[("Filter", Obj::name("AHx"))]);
        assert_eq!(decode_plain(None, &d, b"48 69 7>").unwrap(), b"Hip");
        let d = dict(&[("Filter", Obj::name("RunLengthDecode"))]);
        assert_eq!(
            decode_plain(None, &d, &[1, b'a', b'b', 254, b'z', 128]).unwrap(),
            b"abzzz"
        );
    }

    #[test]
    fn lzw_matches_the_specifications_example() {
        // §7.4.4.2, Example 2: "-----A---B" encodes to these bytes.
        let enc = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
        let d = dict(&[("Filter", Obj::name("LZWDecode"))]);
        assert_eq!(decode_plain(None, &d, &enc).unwrap(), b"-----A---B");
    }

    #[test]
    fn png_up_predictor() {
        // Two rows of three bytes; the second is "up" from the first.
        let raw = [0u8, 1, 2, 3, 2, 1, 1, 1];
        let parms = dict(&[("Predictor", Obj::Int(12)), ("Columns", Obj::Int(3))]);
        let d = dict(&[
            ("Filter", Obj::name("FlateDecode")),
            ("DecodeParms", Obj::Dict(parms)),
        ]);
        assert_eq!(
            decode_plain(None, &d, &deflate(&raw)).unwrap(),
            [1, 2, 3, 2, 3, 4]
        );
    }

    #[test]
    fn an_image_codec_ends_the_chain_and_is_reported() {
        let d = dict(&[(
            "Filter",
            Obj::Array(vec![Obj::name("ASCIIHexDecode"), Obj::name("DCTDecode")]),
        )]);
        let got = decode(None, &d, b"FFD8>").unwrap();
        assert_eq!(got.data, [0xFF, 0xD8]);
        assert_eq!(got.codec.map(|c| c.0), Some(b"DCTDecode".to_vec()));
        assert!(decode_plain(None, &d, b"FFD8>").is_err());
    }

    #[test]
    fn unknown_filters_are_refused() {
        let d = dict(&[("Filter", Obj::name("Rot13Decode"))]);
        assert!(decode(None, &d, b"x").is_err());
    }
}
