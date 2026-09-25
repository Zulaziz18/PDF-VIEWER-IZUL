//! Images under a marked area (ISO 32000-1 §8.9).
//!
//! An image wholly inside an area is simply not drawn any more. One that is
//! only partly inside keeps its pixels outside the area; the pixels inside are
//! set to zero in every component — a constant, so nothing of what was there
//! survives — and the image is written again as a new object used only by
//! this page: a JPEG as a JPEG again, with its own quantisation tables and
//! subsampling (so the rest of it loses next to nothing and the file does not
//! grow — as Flate over decoded pixels, a 145 MB scan became 469 MB), and
//! everything else as Flate. Other pages that draw the same image
//! keep the original, which is correct: redaction is of *this* place.
//!
//! What cannot be decoded here (`JPXDecode`, `JBIG2Decode`, `CCITTFaxDecode`)
//! cannot have part of it cleared, so an image in one of those formats that an
//! area touches is removed from the page entirely, and the report says so.
//! Removing more than asked is a visible loss; removing less would be a leak.

use crate::error::{RedactError, Result};
use crate::file::{Pdf, Stream};
use crate::filters::{self, deflate};
use crate::geom::{covered_fraction, quad, Matrix, Rect};
use crate::object::{Dict, Obj};

/// What to do with one image.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageFate {
    /// No area touches it.
    Keep,
    /// Take the drawing out of the content stream.
    Remove { unsupported: bool },
    /// Draw this instead: new samples, Flate-encoded, and the dictionary to go
    /// with them (for an inline image, the abbreviated one).
    Replace {
        dict: Dict,
        data: Vec<u8>,
        /// Soft mask and explicit mask, redacted the same way, to be written
        /// as new objects and referenced from `dict`.
        smask: Option<Stream>,
        mask: Option<Stream>,
    },
}

/// Colour components of a colour space, for knowing how samples are laid
/// out. `None` for spaces an image cannot use.
pub fn components(pdf: Option<&Pdf<'_>>, cs: &Obj, resources: Option<&Dict>) -> Option<u32> {
    let resolve = |o: &Obj| -> Option<Obj> {
        match pdf {
            Some(p) => p.resolve(o).ok(),
            None => Some(o.clone()),
        }
    };
    match resolve(cs)? {
        Obj::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"G" | b"CalGray" | b"Indexed" | b"I" => Some(1),
            b"DeviceRGB" | b"RGB" | b"CalRGB" | b"Lab" => Some(3),
            b"DeviceCMYK" | b"CMYK" => Some(4),
            other => {
                let res = resources?;
                let spaces = resolve(res.get(b"ColorSpace")?)?;
                let named = spaces.as_dict()?.get(other)?.clone();
                // A name that resolves to itself would loop.
                if named == Obj::Name(other.to_vec()) {
                    return None;
                }
                components(pdf, &named, None)
            }
        },
        Obj::Array(items) => {
            let family = items.first()?.as_name()?.to_vec();
            match family.as_slice() {
                b"Indexed" | b"I" | b"Separation" | b"CalGray" => Some(1),
                b"CalRGB" | b"Lab" => Some(3),
                b"DeviceN" => Some(u32::try_from(resolve(items.get(1)?)?.as_array()?.len()).ok()?),
                b"ICCBased" => {
                    let n = match (pdf, items.get(1)?) {
                        (Some(p), r @ Obj::Ref(_)) => {
                            p.resolve_dict(r).ok()??.get(b"N")?.as_i64()?
                        }
                        (_, Obj::Dict(d)) => d.get(b"N")?.as_i64()?,
                        _ => return None,
                    };
                    u32::try_from(n).ok()
                }
                other => components(pdf, &Obj::Name(other.to_vec()), resources),
            }
        }
        _ => None,
    }
}

/// Reads a key that inline images may abbreviate.
fn key<'d>(d: &'d Dict, long: &[u8], short: &[u8]) -> Option<&'d Obj> {
    d.get(long).or_else(|| d.get(short))
}

/// The layout of an image's samples.
#[derive(Debug, Clone, Copy)]
struct Layout {
    w: usize,
    h: usize,
    comps: usize,
    bpc: usize,
}

impl Layout {
    fn row_bytes(&self) -> usize {
        (self.w * self.comps * self.bpc).div_ceil(8)
    }

    /// Zeroes every component of pixel `(x, y)`, row 0 at the top.
    fn clear(&self, buf: &mut [u8], x: usize, y: usize) {
        let row = y * self.row_bytes();
        let bits = self.comps * self.bpc;
        let first = x * bits;
        if bits.is_multiple_of(8) {
            let at = row + first / 8;
            if let Some(px) = buf.get_mut(at..at + bits / 8) {
                px.fill(0);
            }
            return;
        }
        for bit in first..first + bits {
            if let Some(b) = buf.get_mut(row + bit / 8) {
                *b &= !(0x80u8 >> (bit % 8));
            }
        }
    }
}

/// The pixels of an image whose squares overlap an area: the area is taken
/// back into the image's own pixel grid through the inverse of the matrix it
/// is drawn with, so only the pixels near it are tested.
fn covered_pixels(l: &Layout, ctm: &Matrix, areas: &[Rect]) -> Vec<(usize, usize)> {
    // Pixel (x, y) occupies [x/w, (x+1)/w] × [1 - (y+1)/h, 1 - y/h] of the
    // unit square the image is painted into.
    let Some(inv) = ctm.invert() else {
        return Vec::new();
    };
    let (w, h) = (l.w as f64, l.h as f64);
    let mut out = Vec::new();
    for area in areas {
        let q = quad(area, &inv);
        let xs = q.iter().map(|p| p.0 * w);
        let ys = q.iter().map(|p| (1.0 - p.1) * h);
        let (x0, x1) = xs.fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(v), b.max(v)));
        let (y0, y1) = ys.fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(v), b.max(v)));
        let x0 = x0.floor().clamp(0.0, w) as usize;
        let x1 = x1.ceil().clamp(0.0, w) as usize;
        let y0 = y0.floor().clamp(0.0, h) as usize;
        let y1 = y1.ceil().clamp(0.0, h) as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let px = Rect::new(
                    x as f64 / w,
                    1.0 - (y + 1) as f64 / h,
                    (x + 1) as f64 / w,
                    1.0 - y as f64 / h,
                );
                if covered_fraction(&quad(&px, ctm), area) > 1e-9 {
                    out.push((x, y));
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Whether the unit square under `ctm` overlaps any area, and whether one
/// area holds it entirely.
pub fn placement(ctm: &Matrix, areas: &[Rect]) -> (bool, bool) {
    let q = quad(&Rect::new(0.0, 0.0, 1.0, 1.0), ctm);
    let touched = areas.iter().any(|a| covered_fraction(&q, a) > 1e-9);
    let inside = areas.iter().any(|a| covered_fraction(&q, a) > 1.0 - 1e-9);
    (touched, inside)
}

/// Samples, ready to edit, and whether they came out of a JPEG.
/// The samples, and — for a JPEG — how it was encoded.
fn samples(
    pdf: Option<&Pdf<'_>>,
    dict: &Dict,
    data: &[u8],
    l: &Layout,
) -> Result<Option<(Vec<u8>, Option<JpegParams>)>> {
    let decoded = filters::decode(pdf, dict, data)?;
    let (mut raw, params) = match decoded.codec {
        None => (decoded.data, None),
        Some((codec, _)) if codec == b"DCTDecode" => {
            let params = jpeg_params(&decoded.data);
            (jpeg(&decoded.data, l)?, params)
        }
        Some(_) => return Ok(None),
    };
    raw.resize(l.row_bytes() * l.h, 0);
    Ok(Some((raw, params)))
}

/// What a JPEG was encoded with: quantisation tables in natural order (luma,
/// then chroma when there is colour) and the luma sampling factors.
#[derive(Debug, Clone, PartialEq)]
struct JpegParams {
    luma: [u16; 64],
    chroma: Option<[u16; 64]>,
    h: u8,
    v: u8,
}

/// Zigzag position → natural (row-major) position (ITU-T T.81 Figure A.6).
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Reads the DQT and SOF segments of a baseline or progressive JPEG. `None`
/// for anything it does not follow; the caller then writes Flate instead.
fn jpeg_params(data: &[u8]) -> Option<JpegParams> {
    let mut tables: [Option<[u16; 64]>; 4] = [None; 4];
    let mut comps: Vec<(u8, u8)> = Vec::new(); // (sampling byte, table)
    let mut i = 2;
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    while i + 4 <= data.len() {
        if *data.get(i)? != 0xFF {
            return None;
        }
        let marker = *data.get(i + 1)?;
        if marker == 0xFF {
            i += 1;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*data.get(i + 2)?, *data.get(i + 3)?]));
        let body = data.get(i + 4..i + 2 + len)?;
        match marker {
            0xDB => {
                let mut at = 0;
                while at < body.len() {
                    let pq = body.get(at)? >> 4;
                    let tq = usize::from(body.get(at)? & 0x0F);
                    at += 1;
                    let mut t = [0u16; 64];
                    for &nat in &ZIGZAG {
                        let v = if pq == 0 {
                            u16::from(*body.get(at)?)
                        } else {
                            u16::from_be_bytes([*body.get(at)?, *body.get(at + 1)?])
                        };
                        at += if pq == 0 { 1 } else { 2 };
                        *t.get_mut(nat)? = v;
                    }
                    *tables.get_mut(tq)? = Some(t);
                }
            }
            0xC0..=0xC2 => {
                let n = usize::from(*body.get(5)?);
                for c in 0..n {
                    comps.push((*body.get(6 + c * 3 + 1)?, *body.get(6 + c * 3 + 2)?));
                }
            }
            0xDA => break,
            _ => {}
        }
        i += 2 + len;
    }
    let &(hv, lt) = comps.first()?;
    let luma = (*tables.get(usize::from(lt))?)?;
    let chroma = match comps.get(1) {
        Some(&(_, ct)) => Some((*tables.get(usize::from(ct))?)?),
        None => None,
    };
    Some(JpegParams {
        luma,
        chroma,
        h: hv >> 4,
        v: hv & 0x0F,
    })
}

/// The cleared samples as a JPEG made the way the original was. `None` when
/// that cannot be done faithfully (CMYK, a size or sampling the encoder does
/// not take).
fn encode_jpeg(buf: &[u8], l: &Layout, p: &JpegParams) -> Option<Vec<u8>> {
    use jpeg_encoder::{ColorType, Encoder, QuantizationTableType, SamplingFactor};
    let color = match (l.comps, l.bpc) {
        (1, 8) => ColorType::Luma,
        (3, 8) => ColorType::Rgb,
        _ => return None,
    };
    let w = u16::try_from(l.w).ok()?;
    let h = u16::try_from(l.h).ok()?;
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, 100);
    let chroma = p.chroma.unwrap_or(p.luma);
    enc.set_quantization_tables(
        QuantizationTableType::Custom(Box::new(p.luma)),
        QuantizationTableType::Custom(Box::new(chroma)),
    );
    if l.comps == 3 {
        enc.set_sampling_factor(SamplingFactor::from_factors(p.h, p.v)?);
    }
    enc.encode(buf, w, h, color).ok()?;
    Some(out)
}

/// Decodes a JPEG to samples in the colour space the PDF declares: grey, RGB
/// (the JPEG's YCbCr converted, as a DCTDecode filter outputs) or the four
/// stored CMYK values, uninverted — an inverting `/Decode` stays with the
/// dictionary and keeps doing its job.
fn jpeg(data: &[u8], l: &Layout) -> Result<Vec<u8>> {
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let out = match l.comps {
        1 => ColorSpace::Luma,
        3 => ColorSpace::RGB,
        4 => ColorSpace::CMYK,
        n => return Err(RedactError::Filter(format!("JPEG dengan {n} komponen"))),
    };
    let opts = DecoderOptions::default().jpeg_set_out_colorspace(out);
    let mut dec =
        zune_jpeg::JpegDecoder::new_with_options(zune_core::bytestream::ZCursor::new(data), opts);
    let pixels = dec
        .decode()
        .map_err(|e| RedactError::Filter(format!("JPEG: {e:?}")))?;
    if l.bpc != 8 {
        return Err(RedactError::Filter("JPEG bukan 8 bit".into()));
    }
    Ok(pixels)
}

fn layout(pdf: Option<&Pdf<'_>>, dict: &Dict, resources: Option<&Dict>) -> Option<Layout> {
    let w = usize::try_from(key(dict, b"Width", b"W")?.as_i64()?).ok()?;
    let h = usize::try_from(key(dict, b"Height", b"H")?.as_i64()?).ok()?;
    let mask = matches!(key(dict, b"ImageMask", b"IM"), Some(Obj::Bool(true)));
    let (comps, bpc) = if mask {
        (1, 1)
    } else {
        let cs = key(dict, b"ColorSpace", b"CS")?;
        let bpc = key(dict, b"BitsPerComponent", b"BPC")
            .and_then(Obj::as_i64)
            .unwrap_or(8);
        (
            components(pdf, cs, resources)? as usize,
            usize::try_from(bpc).ok()?,
        )
    };
    if w == 0 || h == 0 || !matches!(bpc, 1 | 2 | 4 | 8 | 16) || w.saturating_mul(h) > 400_000_000 {
        return None;
    }
    Some(Layout { w, h, comps, bpc })
}

/// Redacts one image drawn with `ctm`.
///
/// `inline` selects the abbreviated keys and hex encoding an inline image
/// needs (its data sits inside the content stream, where raw binary could
/// contain `EI` and end it early for a reader that searches).
pub fn redact(
    pdf: Option<&Pdf<'_>>,
    dict: &Dict,
    data: &[u8],
    ctm: &Matrix,
    areas: &[Rect],
    resources: Option<&Dict>,
    inline: bool,
) -> Result<ImageFate> {
    let (touched, inside) = placement(ctm, areas);
    if !touched {
        return Ok(ImageFate::Keep);
    }
    if inside {
        return Ok(ImageFate::Remove { unsupported: false });
    }
    let Some(l) = layout(pdf, dict, resources) else {
        return Ok(ImageFate::Remove { unsupported: true });
    };
    let Some((mut buf, jpeg_made)) = samples(pdf, dict, data, &l)? else {
        return Ok(ImageFate::Remove { unsupported: true });
    };
    let pixels = covered_pixels(&l, ctm, areas);
    if pixels.len() == l.w * l.h {
        return Ok(ImageFate::Remove { unsupported: false });
    }
    for &(x, y) in &pixels {
        l.clear(&mut buf, x, y);
    }

    let mut out = dict.clone();
    for k in [
        &b"Filter"[..],
        b"F",
        b"DecodeParms",
        b"DP",
        b"Length",
        b"L",
        b"Metadata",
    ] {
        out.remove(k);
    }
    let bytes = if inline {
        out.set(b"F", Obj::name("AHx"));
        let mut hex = Vec::with_capacity(buf.len() * 2 + 1);
        for b in &buf {
            crate::object::push_hex(&mut hex, *b);
        }
        hex.push(b'>');
        hex
    } else if let Some(j) = jpeg_made.as_ref().and_then(|p| encode_jpeg(&buf, &l, p)) {
        out.set(b"Filter", Obj::name("DCTDecode"));
        j
    } else {
        out.set(b"Filter", Obj::name("FlateDecode"));
        deflate(&buf)
    };
    if !matches!(key(dict, b"ImageMask", b"IM"), Some(Obj::Bool(true))) {
        out.set(
            if inline {
                &b"BPC"[..]
            } else {
                b"BitsPerComponent"
            },
            Obj::Int(l.bpc as i64),
        );
    }

    // The masks are images of their own, drawn into the same unit square.
    let side = |k: &[u8]| -> Result<Option<Stream>> {
        let Some(p) = pdf else { return Ok(None) };
        let Some(r) = dict.get(k) else {
            return Ok(None);
        };
        let Some(s) = p.stream_of(r)? else {
            return Ok(None);
        };
        match redact(pdf, &s.dict, &s.data, ctm, areas, None, false)? {
            ImageFate::Replace { dict, data, .. } => Ok(Some(Stream { dict, data })),
            // A mask no area reaches stays as it is.
            ImageFate::Keep => Ok(None),
            // A mask that cannot be decoded is dropped with its image.
            ImageFate::Remove { .. } => Err(RedactError::Unsupported("mask gambar".into())),
        }
    };
    let smask = side(b"SMask");
    let mask = side(b"Mask");
    let (smask, mask) = match (smask, mask) {
        (Ok(s), Ok(m)) => (s, m),
        _ => return Ok(ImageFate::Remove { unsupported: true }),
    };
    Ok(ImageFate::Replace {
        dict: out,
        data: bytes,
        smask,
        mask,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(w: i64, h: i64) -> Dict {
        let mut d = Dict::new();
        d.set(b"Width", Obj::Int(w));
        d.set(b"Height", Obj::Int(h));
        d.set(b"ColorSpace", Obj::name("DeviceGray"));
        d.set(b"BitsPerComponent", Obj::Int(8));
        d
    }

    /// A 4×4 grey image on 0..40 × 0..40; an area over its right half clears
    /// columns 2 and 3 and nothing else.
    #[test]
    fn a_partly_covered_image_loses_exactly_the_covered_pixels() {
        let ctm = Matrix::new(40.0, 0.0, 0.0, 40.0, 0.0, 0.0);
        let data = vec![200u8; 16];
        let fate = redact(
            None,
            &gray(4, 4),
            &data,
            &ctm,
            &[Rect::new(20.0, -5.0, 100.0, 100.0)],
            None,
            false,
        )
        .unwrap();
        let ImageFate::Replace { dict, data, .. } = fate else {
            panic!("{fate:?}")
        };
        let raw = filters::decode_plain(None, &dict, &data).unwrap();
        for row in raw.chunks(4) {
            assert_eq!(row, [200, 200, 0, 0]);
        }
    }

    /// A colour JPEG, partly covered, comes back as a JPEG made with the same
    /// tables and subsampling: the covered half dark, the other half as it
    /// was to within rounding, and no bigger than a JPEG — where decoded
    /// pixels under Flate made a 145 MB scan 469 MB.
    #[test]
    fn a_jpeg_stays_a_jpeg_with_its_own_tables() {
        let (w, h) = (64usize, 64usize);
        let mut px = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&[(x * 4) as u8, (y * 4) as u8, 180]);
            }
        }
        let mut src = Vec::new();
        let mut enc = jpeg_encoder::Encoder::new(&mut src, 80);
        enc.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
        enc.encode(&px, w as u16, h as u16, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        let mut d = gray(64, 64);
        d.set(b"ColorSpace", Obj::name("DeviceRGB"));
        d.set(b"Filter", Obj::name("DCTDecode"));
        let ctm = Matrix::new(64.0, 0.0, 0.0, 64.0, 0.0, 0.0);
        // The right half, x 32..64.
        let area = [Rect::new(32.0, -1.0, 100.0, 100.0)];
        let fate = redact(None, &d, &src, &ctm, &area, None, false).unwrap();
        let ImageFate::Replace { dict, data, .. } = fate else {
            panic!("{fate:?}")
        };
        assert_eq!(dict.name(b"Filter"), Some(&b"DCTDecode"[..]));
        assert_eq!(jpeg_params(&data), jpeg_params(&src));
        assert!(jpeg_params(&src).is_some_and(|p| (p.h, p.v) == (2, 2)));
        assert!(
            data.len() < src.len() * 2,
            "{} vs {}",
            data.len(),
            src.len()
        );
        let l = Layout {
            w,
            h,
            comps: 3,
            bpc: 8,
        };
        let before = jpeg(&src, &l).unwrap();
        let after = jpeg(&data, &l).unwrap();
        let mut worst_kept = 0u8;
        let mut worst_cleared = 0u8;
        for y in 0..h {
            for x in 0..w {
                for c in 0..3 {
                    let i = (y * w + x) * 3 + c;
                    // One 16-pixel MCU either side of the edge mixes both.
                    if x < 16 {
                        worst_kept = worst_kept.max(before[i].abs_diff(after[i]));
                    } else if x >= 48 {
                        worst_cleared = worst_cleared.max(after[i]);
                    }
                }
            }
        }
        assert!(worst_kept <= 6, "kept half moved by {worst_kept}");
        assert!(worst_cleared <= 6, "cleared half still has {worst_cleared}");
    }

    #[test]
    fn a_jpegs_tables_are_read_in_natural_order() {
        // Annex K luma table at quality 50 is the table itself.
        let mut src = Vec::new();
        let enc = jpeg_encoder::Encoder::new(&mut src, 50);
        enc.encode(&[128u8; 64], 8, 8, jpeg_encoder::ColorType::Luma)
            .unwrap();
        let p = jpeg_params(&src).unwrap();
        assert_eq!(&p.luma[..8], &[16, 11, 10, 16, 24, 40, 51, 61]);
        assert_eq!(p.luma[8], 12);
        assert!(p.chroma.is_none());
    }

    #[test]
    fn an_image_inside_an_area_is_removed_and_one_outside_kept() {
        let ctm = Matrix::new(10.0, 0.0, 0.0, 10.0, 5.0, 5.0);
        let big = [Rect::new(0.0, 0.0, 50.0, 50.0)];
        let far = [Rect::new(100.0, 100.0, 150.0, 150.0)];
        let d = gray(2, 2);
        assert_eq!(
            redact(None, &d, &[1; 4], &ctm, &big, None, false).unwrap(),
            ImageFate::Remove { unsupported: false }
        );
        assert_eq!(
            redact(None, &d, &[1; 4], &ctm, &far, None, false).unwrap(),
            ImageFate::Keep
        );
    }

    /// Row 0 of an image is its *top*: an area over the top of the drawn
    /// image clears the first rows of data, not the last.
    #[test]
    fn rows_count_from_the_top() {
        let ctm = Matrix::new(20.0, 0.0, 0.0, 20.0, 0.0, 0.0);
        let fate = redact(
            None,
            &gray(2, 2),
            &[9; 4],
            &ctm,
            &[Rect::new(-1.0, 12.0, 30.0, 30.0)],
            None,
            false,
        )
        .unwrap();
        let ImageFate::Replace { dict, data, .. } = fate else {
            panic!()
        };
        assert_eq!(
            filters::decode_plain(None, &dict, &data).unwrap(),
            [0, 0, 9, 9]
        );
    }

    #[test]
    fn one_bit_masks_clear_single_bits() {
        let l = Layout {
            w: 10,
            h: 1,
            comps: 1,
            bpc: 1,
        };
        let mut buf = vec![0xff, 0xff];
        l.clear(&mut buf, 0, 0);
        l.clear(&mut buf, 9, 0);
        assert_eq!(buf, [0x7f, 0xbf]);
    }

    #[test]
    fn a_jpx_image_that_is_touched_is_removed_whole() {
        let mut d = gray(4, 4);
        d.set(b"Filter", Obj::name("JPXDecode"));
        let ctm = Matrix::new(40.0, 0.0, 0.0, 40.0, 0.0, 0.0);
        let fate = redact(
            None,
            &d,
            b"....",
            &ctm,
            &[Rect::new(30.0, 30.0, 50.0, 50.0)],
            None,
            false,
        )
        .unwrap();
        assert_eq!(fate, ImageFate::Remove { unsupported: true });
    }

    #[test]
    fn inline_images_are_rewritten_as_hex() {
        let mut d = Dict::new();
        d.set(b"W", Obj::Int(2));
        d.set(b"H", Obj::Int(1));
        d.set(b"CS", Obj::name("G"));
        d.set(b"BPC", Obj::Int(8));
        let ctm = Matrix::new(20.0, 0.0, 0.0, 10.0, 0.0, 0.0);
        let fate = redact(
            None,
            &d,
            &[0x45, 0x49],
            &ctm,
            &[Rect::new(0.0, 0.0, 10.0, 10.0)],
            None,
            true,
        )
        .unwrap();
        let ImageFate::Replace { dict, data, .. } = fate else {
            panic!()
        };
        assert_eq!(dict.name(b"F"), Some(&b"AHx"[..]));
        assert_eq!(data, b"0049>");
    }
}
