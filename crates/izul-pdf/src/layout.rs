//! Reading a page for the Word conversion (7.1.0): characters with their
//! boxes and style, and the pictures placed on the page, in display space.
//! What becomes of them is `izul-docx`'s business; this module only reads.
//!
//! Measured on PDFium 7881, not assumed:
//!
//! * `FPDFText_GetFontSize` is the `Tf` operand only: a 12 pt run set with
//!   `Tf 1` and a 12× text matrix reads 1. The size drawn is that times the
//!   scale of `FPDFText_GetMatrix` (`the_size_is_the_size_drawn`) — and LaTeX
//!   and Word both write their text that way.
//! * `FPDFText_GetCharAngle` is **clockwise**: text set with the matrix
//!   `0 1 -1 0` (a quarter turn counter-clockwise) reads 3π/2, not π/2. A
//!   page's own `/Rotate` turns everything a further quarter clockwise per
//!   step for display, so the angle as seen is the sum of the two
//!   (`upright_means_upright_as_displayed`).
//! * Bold is taken from the weight, the font flags *or* the font name
//!   ("…-Bold", "…,Bold", "…Black"): embedded fonts often carry only one of
//!   the three.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::os::raw::{c_double, c_int, c_uint, c_ulong, c_void};

use izul_docx::{Glyph, PageLayout, Picture, Rect};
use pdfium_render::prelude::*;

use crate::engine::{Document, PageGeometry};
use crate::error::Result;
use crate::ffi_guard::guard;
use crate::geom::{PdfRectF, RotationQuarter};
use crate::text::TextPage;

const FPDF_PAGEOBJ_IMAGE: c_int = 3;
const FPDF_PAGEOBJ_FORM: c_int = 5;
/// `FXFONT_ITALIC` from `fpdf_text.h`'s font flags.
const FONT_FLAG_ITALIC: c_int = 1 << 6;
/// `FXFONT_FORCE_BOLD`.
const FONT_FLAG_BOLD: c_int = 1 << 18;
/// How far from upright, in radians, a character may lean and still be read
/// as part of the running text — italic is a skew, not a rotation, and does
/// not change the angle at all.
const UPRIGHT: f32 = 0.06;
/// For pictures re-encoded because their own bytes cannot go into Word as they
/// are. High: the picture has already been decoded once.
const JPEG_QUALITY: u8 = 92;
/// A picture smaller than this on the page (points, either side) is a rule,
/// a bullet or a speck, not a figure.
const MIN_PICTURE: f32 = 6.0;

/// The family part of a PDF font name: no subset tag, no style suffix.
/// "ABCDEF+TimesNewRomanPS-BoldMT" → "TimesNewRomanPS".
pub fn family_of(name: &str) -> String {
    let name = match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest,
        _ => name,
    };
    let base = name.split([',', '-']).next().unwrap_or(name);
    let base = base.trim_end_matches("MT").trim_end_matches("PS");
    if base.is_empty() {
        name.to_string()
    } else {
        base.to_string()
    }
}

/// Whether a font name says bold or italic, for fonts whose flags do not.
fn style_in_name(name: &str) -> (bool, bool) {
    let lower = name.to_ascii_lowercase();
    let bold = [
        "bold",
        "black",
        "heavy",
        "semibold",
        "demibold",
        "extrabold",
    ]
    .iter()
    .any(|w| lower.contains(w));
    let italic = lower.contains("italic") || lower.contains("oblique");
    (bold, italic)
}

impl Document {
    /// Everything [`izul_docx`] needs from one page.
    pub fn page_layout(&self, page: u32) -> Result<PageLayout> {
        self.check_page(page)?;
        let engine = self.engine();
        guard("page_layout", || {
            self.with_page(page, |p| {
                let geometry = PageGeometry::of_page(engine, p);
                let size = geometry.display_size(RotationQuarter::None);
                let mut out = PageLayout {
                    width: size.width,
                    height: size.height,
                    ..PageLayout::default()
                };
                read_glyphs(self, p, &geometry, &mut out)?;
                read_pictures(self, p, &geometry, &mut out);
                Ok(out)
            })
        })
    }
}

fn rect_of(geometry: &PageGeometry, r: PdfRectF) -> Rect {
    let d = geometry.to_display(r, RotationQuarter::None);
    Rect::new(d.left, d.bottom, d.right, d.top)
}

fn read_glyphs(
    doc: &Document,
    page: FPDF_PAGE,
    geometry: &PageGeometry,
    out: &mut PageLayout,
) -> Result<()> {
    let tp = TextPage::load(doc, page)?;
    let bindings = tp.bindings();
    let raw = tp.raw();
    let turn = geometry.intrinsic.degrees() as f32 / 90.0 * FRAC_PI_2;
    let mut name_buf = vec![0u8; 256];
    for i in 0..tp.count() {
        // SAFETY: `i` is in `0..count` and `raw` is a live text page; every
        // out-parameter is a live local of the type the header declares.
        unsafe {
            let Some(ch) = char::from_u32(bindings.FPDFText_GetUnicode(raw, i)) else {
                continue;
            };
            let generated = bindings.FPDFText_IsGenerated(raw, i) == 1;
            let mut fs = FS_RECTF {
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            };
            if bindings.FPDFText_GetLooseCharBox(raw, i, &mut fs) == 0 && !generated {
                continue;
            }
            let rect = rect_of(
                geometry,
                PdfRectF::new(fs.left, fs.bottom, fs.right, fs.top),
            );
            if !generated {
                let angle = bindings.FPDFText_GetCharAngle(raw, i);
                if angle >= 0.0 {
                    let lean = (angle + turn).rem_euclid(TAU);
                    if lean > UPRIGHT && lean < TAU - UPRIGHT {
                        out.skipped_turned += 1;
                        continue;
                    }
                }
            }
            let (mut ox, mut oy): (c_double, c_double) = (0.0, 0.0);
            let baseline = if bindings.FPDFText_GetCharOrigin(raw, i, &mut ox, &mut oy) != 0 {
                let o = rect_of(
                    geometry,
                    PdfRectF::new(ox as f32, oy as f32, ox as f32, oy as f32),
                );
                o.bottom
            } else {
                rect.bottom
            };
            // The `Tf` operand times the scale of the character's matrix: a
            // run set with `Tf 1` and a 12× text matrix is 12 pt on the page.
            let operand = bindings.FPDFText_GetFontSize(raw, i) as f32;
            let mut m = FS_MATRIX {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 0.0,
                f: 0.0,
            };
            let scale = if bindings.FPDFText_GetMatrix(raw, i, &mut m) != 0 {
                (m.a * m.d - m.b * m.c).abs().sqrt()
            } else {
                1.0
            };
            let size = operand
                * if scale.is_finite() && scale > 0.0 {
                    scale
                } else {
                    1.0
                };
            let mut flags: c_int = 0;
            let len = bindings.FPDFText_GetFontInfo(
                raw,
                i,
                name_buf.as_mut_ptr() as *mut c_void,
                name_buf.len() as c_ulong,
                &mut flags,
            ) as usize;
            let name = if len > 1 && len <= name_buf.len() {
                String::from_utf8_lossy(name_buf.get(..len - 1).unwrap_or(&[])).to_string()
            } else {
                String::new()
            };
            let weight = bindings.FPDFText_GetFontWeight(raw, i);
            let (name_bold, name_italic) = style_in_name(&name);
            let bold = weight >= 600 || flags & FONT_FLAG_BOLD != 0 || name_bold;
            let italic = flags & FONT_FLAG_ITALIC != 0 || name_italic;
            let (mut r, mut g, mut b, mut a): (c_uint, c_uint, c_uint, c_uint) = (0, 0, 0, 255);
            let color =
                if bindings.FPDFText_GetFillColor(raw, i, &mut r, &mut g, &mut b, &mut a) != 0 {
                    [r.min(255) as u8, g.min(255) as u8, b.min(255) as u8]
                } else {
                    [0, 0, 0]
                };
            let family = family_of(&name);
            let font = match out.fonts.iter().position(|f| *f == family) {
                Some(k) => k,
                None => {
                    out.fonts.push(family);
                    out.fonts.len() - 1
                }
            };
            out.glyphs.push(Glyph {
                ch,
                rect,
                baseline,
                size: if size.is_finite() && size > 0.0 {
                    size
                } else {
                    rect.height()
                },
                bold,
                italic,
                color,
                font: u16::try_from(font).unwrap_or(0),
                generated,
            });
        }
    }
    Ok(())
}

/// The pictures on the page, including those inside form XObjects — which
/// is where a flattened annotation's picture ends up, so the pictures a user
/// inserted come across too.
fn read_pictures(doc: &Document, page: FPDF_PAGE, geometry: &PageGeometry, out: &mut PageLayout) {
    let bindings = doc.engine().bindings();
    // SAFETY: `page` is live; object handles come from it and are used before
    // anything changes the page.
    let count = unsafe { bindings.FPDFPage_CountObjects(page) };
    let objects: Vec<FPDF_PAGEOBJECT> = (0..count)
        // SAFETY: as above.
        .map(|i| unsafe { bindings.FPDFPage_GetObject(page, i) })
        .collect();
    visit(doc, page, geometry, out, &objects, None, 0);
}

/// Forms nest; past this depth a document is either pathological or hostile.
const MAX_FORM_DEPTH: u32 = 8;

fn visit(
    doc: &Document,
    page: FPDF_PAGE,
    geometry: &PageGeometry,
    out: &mut PageLayout,
    objects: &[FPDF_PAGEOBJECT],
    // The matrix from this level's space to the page's, `None` on the page.
    to_page: Option<FS_MATRIX>,
    depth: u32,
) {
    let bindings = doc.engine().bindings();
    for &object in objects {
        if object.is_null() {
            continue;
        }
        // SAFETY: `object` is a live page object of `page` or of one of its
        // forms; every out-parameter is a live local.
        let kind = unsafe { bindings.FPDFPageObj_GetType(object) };
        if kind == FPDF_PAGEOBJ_FORM && depth < MAX_FORM_DEPTH {
            let mut m = identity();
            // SAFETY: as above.
            if unsafe { bindings.FPDFPageObj_GetMatrix(object, &mut m) } == 0 {
                continue;
            }
            let inner = match to_page {
                Some(outer) => multiply(&m, &outer),
                None => m,
            };
            // SAFETY: as above; child handles come from this form object.
            let n = unsafe { bindings.FPDFFormObj_CountObjects(object) };
            let children: Vec<FPDF_PAGEOBJECT> = (0..n.max(0) as c_ulong)
                // SAFETY: as above.
                .map(|k| unsafe { bindings.FPDFFormObj_GetObject(object, k) })
                .collect();
            visit(doc, page, geometry, out, &children, Some(inner), depth + 1);
            continue;
        }
        if kind != FPDF_PAGEOBJ_IMAGE {
            continue;
        }
        let (mut l, mut b, mut r, mut t) = (0f32, 0f32, 0f32, 0f32);
        // SAFETY: as above.
        if unsafe { bindings.FPDFPageObj_GetBounds(object, &mut l, &mut b, &mut r, &mut t) } == 0 {
            continue;
        }
        let bounds = match &to_page {
            Some(m) => transform_rect(m, PdfRectF::new(l, b, r, t)),
            None => PdfRectF::new(l, b, r, t),
        };
        let rect = rect_of(geometry, bounds);
        if rect.width() < MIN_PICTURE || rect.height() < MIN_PICTURE {
            continue;
        }
        match picture_bytes(doc, page, object) {
            Some((mime, bytes, width_px, height_px)) => out.pictures.push(Picture {
                rect,
                mime: mime.to_string(),
                bytes,
                width_px,
                height_px,
            }),
            None => out.skipped_pictures += 1,
        }
    }
}

fn identity() -> FS_MATRIX {
    FS_MATRIX {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    }
}

/// `first` then `second`, in PDF's row-vector convention.
fn multiply(first: &FS_MATRIX, second: &FS_MATRIX) -> FS_MATRIX {
    FS_MATRIX {
        a: first.a * second.a + first.b * second.c,
        b: first.a * second.b + first.b * second.d,
        c: first.c * second.a + first.d * second.c,
        d: first.c * second.b + first.d * second.d,
        e: first.e * second.a + first.f * second.c + second.e,
        f: first.e * second.b + first.f * second.d + second.f,
    }
}

fn transform_rect(m: &FS_MATRIX, r: PdfRectF) -> PdfRectF {
    let pts = [
        (r.left, r.bottom),
        (r.right, r.bottom),
        (r.left, r.top),
        (r.right, r.top),
    ];
    let mapped: Vec<(f32, f32)> = pts
        .iter()
        .map(|&(x, y)| (m.a * x + m.c * y + m.e, m.b * x + m.d * y + m.f))
        .collect();
    let xs = mapped.iter().map(|p| p.0);
    let ys = mapped.iter().map(|p| p.1);
    PdfRectF::new(
        xs.clone().fold(f32::INFINITY, f32::min),
        ys.clone().fold(f32::INFINITY, f32::min),
        xs.fold(f32::NEG_INFINITY, f32::max),
        ys.fold(f32::NEG_INFINITY, f32::max),
    )
}

/// The bytes of one image object: the JPEG itself when it is one, otherwise
/// its pixels with the soft mask applied, as PNG. The object's matrix is
/// changed to read the pixels at their own size (see `izul::extract_image`)
/// and put back before returning, so the page draws as it did.
fn picture_bytes(
    doc: &Document,
    page: FPDF_PAGE,
    object: FPDF_PAGEOBJECT,
) -> Option<(&'static str, Vec<u8>, u32, u32)> {
    let bindings = doc.engine().bindings();
    let (mut w, mut h): (c_uint, c_uint) = (0, 0);
    // SAFETY: `object` is a live image object of `page`; buffers are sized
    // from the lengths PDFium reports, and the matrix is restored on every
    // path that changed it.
    unsafe {
        if bindings.FPDFImageObj_GetImagePixelSize(object, &mut w, &mut h) == 0 || w == 0 || h == 0
        {
            return None;
        }
        // Only a stream whose one filter is DCTDecode *is* a JPEG file. With
        // a filter in front of it (reportlab writes [/ASCII85Decode
        // /DCTDecode]) the raw bytes are ASCII85 text, and Word shows nothing.
        if bindings.FPDFImageObj_GetImageFilterCount(object) == 1
            && crate::izul::filter_is_dct(bindings, object)
        {
            let len = bindings.FPDFImageObj_GetImageDataRaw(object, std::ptr::null_mut(), 0);
            if len > 0 {
                let mut bytes = vec![0u8; len as usize];
                let wrote = bindings.FPDFImageObj_GetImageDataRaw(
                    object,
                    bytes.as_mut_ptr() as *mut c_void,
                    len,
                );
                // A CMYK JPEG is a valid PDF image and an invalid Word one;
                // those go through the PNG path below instead.
                if wrote == len
                    && bytes.starts_with(&[0xFF, 0xD8, 0xFF])
                    && !crate::izul::jpeg_is_cmyk(&bytes)
                {
                    return Some(("image/jpeg", bytes, w, h));
                }
            }
        }
        let mut before = FS_MATRIX {
            a: 0.0,
            b: 0.0,
            c: 0.0,
            d: 0.0,
            e: 0.0,
            f: 0.0,
        };
        if bindings.FPDFPageObj_GetMatrix(object, &mut before) == 0 {
            return None;
        }
        let natural = FS_MATRIX {
            a: w as f32,
            b: 0.0,
            c: 0.0,
            d: h as f32,
            e: 0.0,
            f: 0.0,
        };
        bindings.FPDFPageObj_SetMatrix(object, &natural);
        let pixels = crate::izul::rendered_rgba(doc, page, object);
        bindings.FPDFPageObj_SetMatrix(object, &before);
        encode(pixels?).map(|(mime, bytes)| (mime, bytes, w, h))
    }
}

/// A photograph as JPEG — as PNG it is several times the size, and the
/// conversion makes one out of every JPEG that sits behind a second filter —
/// and anything with transparency, or with few colours (a chart, a diagram,
/// a logo, whose edges JPEG would smear), as PNG.
fn encode(pixels: image::RgbaImage) -> Option<(&'static str, Vec<u8>)> {
    let opaque = pixels.pixels().all(|p| p.0[3] == 255);
    let mut colours = std::collections::HashSet::new();
    let mut photographic = false;
    for p in pixels.pixels() {
        if colours.insert(p.0) && colours.len() > 256 {
            photographic = true;
            break;
        }
    }
    let opaque = opaque && photographic;
    let mut out = Vec::new();
    if opaque {
        let rgb = image::DynamicImage::ImageRgba8(pixels).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
            .encode_image(&rgb)
            .ok()?;
        Some(("image/jpeg", out))
    } else {
        pixels
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .ok()?;
        Some(("image/png", out))
    }
}

#[cfg(test)]
mod tests {
    use crate::engine_and_lock;

    fn jpeg() -> Vec<u8> {
        let img = image::RgbImage::from_fn(40, 30, |x, y| {
            image::Rgb([(x * 6) as u8, (y * 8) as u8, 120])
        });
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .encode_image(&img)
            .expect("jpeg");
        out
    }

    fn ascii85(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in data.chunks(4) {
            let mut block = [0u8; 4];
            block[..chunk.len()].copy_from_slice(chunk);
            let mut n = u32::from_be_bytes(block);
            let mut digits = [0u8; 5];
            for d in digits.iter_mut().rev() {
                *d = (n % 85) as u8 + b'!';
                n /= 85;
            }
            out.extend_from_slice(&digits[..chunk.len() + 1]);
        }
        out.extend_from_slice(b"~>");
        out
    }

    /// One page: red 18 pt Helvetica-Bold "Judul", a sideways "Miring", and
    /// a picture whose stream is `image` under `filter`.
    fn page_pdf(rotate: i32, filter: &str, image: &[u8]) -> Vec<u8> {
        let content = "BT /F1 18 Tf 1 0 0 rg 40 350 Td (Judul) Tj ET \
                       BT /F1 1 Tf 0 0 0 rg 12 0 0 12 40 320 Tm (Skala) Tj ET \
                       BT /F1 12 Tf 0 0 0 rg 0 1 -1 0 280 100 Tm (Miring) Tj ET \
                       q 200 0 0 150 50 100 cm /Im1 Do Q";
        let mut objects: Vec<Vec<u8>> = vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
            format!(
                "<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 400]/Rotate {rotate}\
                 /Resources<</Font<</F1 6 0 R>>/XObject<</Im1 5 0 R>>>>/Contents 4 0 R>>"
            )
            .into_bytes(),
            format!(
                "<</Length {}>>\nstream\n{content}\nendstream",
                content.len() + 1
            )
            .into_bytes(),
        ];
        let mut img = format!(
            "<</Type/XObject/Subtype/Image/Width 40/Height 30/ColorSpace/DeviceRGB\
             /BitsPerComponent 8/Filter{filter}/Length {}>>\nstream\n",
            image.len()
        )
        .into_bytes();
        img.extend_from_slice(image);
        img.extend_from_slice(b"\nendstream");
        objects.push(img);
        objects.push(b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica-Bold>>".to_vec());

        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for off in &offsets {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    fn text_of(layout: &izul_docx::PageLayout) -> String {
        layout
            .glyphs
            .iter()
            .filter(|g| !g.generated)
            .map(|g| g.ch)
            .collect()
    }

    #[test]
    fn reads_size_weight_colour_and_baseline() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine
            .open_bytes(page_pdf(0, "/DCTDecode", &jpeg()), None, None)
            .expect("buka");
        let layout = doc.page_layout(0).expect("tata letak");
        let first = layout.glyphs.first().expect("ada huruf");
        assert_eq!(first.ch, 'J');
        assert!((first.size - 18.0).abs() < 0.01, "{}", first.size);
        assert!(first.bold, "Helvetica-Bold");
        assert!(!first.italic);
        assert_eq!(first.color, [255, 0, 0]);
        assert!((first.baseline - 350.0).abs() < 0.01, "{}", first.baseline);
        assert_eq!(
            layout
                .fonts
                .get(usize::from(first.font))
                .map(String::as_str),
            Some("Helvetica")
        );
    }

    /// `Tf 1` with a 12× text matrix is 12 pt on the page, and Word needs
    /// the 12.
    #[test]
    fn the_size_is_the_size_drawn() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine
            .open_bytes(page_pdf(0, "/DCTDecode", &jpeg()), None, None)
            .expect("buka");
        let layout = doc.page_layout(0).expect("tata letak");
        let s = layout.glyphs.iter().find(|g| g.ch == 'S').expect("Skala");
        assert!((s.size - 12.0).abs() < 0.01, "{}", s.size);
    }

    #[test]
    fn sideways_text_is_left_out_and_counted() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine
            .open_bytes(page_pdf(0, "/DCTDecode", &jpeg()), None, None)
            .expect("buka");
        let layout = doc.page_layout(0).expect("tata letak");
        assert_eq!(text_of(&layout), "JudulSkala");
        assert_eq!(layout.skipped_turned, 6, "Miring");
    }

    /// The same sideways text on a page turned a quarter for display reads
    /// upright on screen — and the upright title now reads sideways. This
    /// pins which way `FPDFText_GetCharAngle` turns against `/Rotate`.
    #[test]
    fn upright_means_upright_as_displayed() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine
            .open_bytes(page_pdf(90, "/DCTDecode", &jpeg()), None, None)
            .expect("buka");
        let layout = doc.page_layout(0).expect("tata letak");
        assert_eq!(text_of(&layout), "Miring");
        assert_eq!(layout.skipped_turned, 10, "Judul, Skala");
        assert!((layout.width - 400.0).abs() < 0.01, "lebar tampilan");
    }

    #[test]
    fn a_lone_dct_stream_comes_back_as_the_jpeg_itself() {
        let (engine, _pdfium) = engine_and_lock!();
        let original = jpeg();
        let doc = engine
            .open_bytes(page_pdf(0, "/DCTDecode", &original), None, None)
            .expect("buka");
        let layout = doc.page_layout(0).expect("tata letak");
        let pic = layout.pictures.first().expect("gambar");
        assert_eq!(pic.mime, "image/jpeg");
        assert_eq!(pic.bytes, original);
        assert_eq!((pic.width_px, pic.height_px), (40, 30));
        assert!(
            (pic.rect.width() - 200.0).abs() < 0.01 && (pic.rect.height() - 150.0).abs() < 0.01
        );
    }

    /// reportlab writes its JPEGs as [/ASCII85Decode /DCTDecode]. The raw
    /// stream is then ASCII85 text, and taken as a JPEG it showed as nothing
    /// in Word (found by the LibreOffice proof). It is decoded and encoded
    /// again — as JPEG, being opaque.
    #[test]
    fn a_jpeg_behind_another_filter_comes_back_decoded() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine
            .open_bytes(
                page_pdf(0, "[/ASCII85Decode/DCTDecode]", &ascii85(&jpeg())),
                None,
                None,
            )
            .expect("buka");
        let layout = doc.page_layout(0).expect("tata letak");
        let pic = layout.pictures.first().expect("gambar");
        assert_eq!(pic.mime, "image/jpeg");
        assert!(
            pic.bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
            "berkas JPEG sungguhan"
        );
        let decoded = image::load_from_memory(&pic.bytes).expect("JPEG terbaca");
        assert_eq!((decoded.width(), decoded.height()), (40, 30));
    }

    #[test]
    fn reading_a_picture_leaves_the_page_as_it_draws() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine
            .open_bytes(
                page_pdf(0, "[/ASCII85Decode/DCTDecode]", &ascii85(&jpeg())),
                None,
                None,
            )
            .expect("buka");
        let before = doc.page_layout(0).expect("pertama");
        let again = doc.page_layout(0).expect("kedua");
        assert_eq!(
            before.pictures.first().map(|p| p.rect),
            again.pictures.first().map(|p| p.rect),
            "matriks gambar dikembalikan"
        );
    }

    /// A picture inside a form XObject — where a flattened annotation's
    /// picture lives — placed with `cm` and offset again by the form's own
    /// `/Matrix`.
    fn form_pdf() -> Vec<u8> {
        let image = jpeg();
        let page = "q 1 0 0 1 50 100 cm /Fm1 Do Q";
        let form = "q 200 0 0 150 0 0 cm /Im1 Do Q";
        let mut objects: Vec<Vec<u8>> = vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 400]/Resources<</XObject<</Fm1 6 0 R>>>>/Contents 4 0 R>>"
                .to_vec(),
            format!("<</Length {}>>\nstream\n{page}\nendstream", page.len() + 1).into_bytes(),
        ];
        let mut img = format!(
            "<</Type/XObject/Subtype/Image/Width 40/Height 30/ColorSpace/DeviceRGB\
             /BitsPerComponent 8/Filter/DCTDecode/Length {}>>\nstream\n",
            image.len()
        )
        .into_bytes();
        img.extend_from_slice(&image);
        img.extend_from_slice(b"\nendstream");
        objects.push(img);
        objects.push(
            format!(
                "<</Type/XObject/Subtype/Form/BBox[0 0 300 300]/Matrix[1 0 0 1 10 0]\
                 /Resources<</XObject<</Im1 5 0 R>>>>/Length {}>>\nstream\n{form}\nendstream",
                form.len() + 1
            )
            .into_bytes(),
        );
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for off in &offsets {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn a_picture_inside_a_form_is_found_where_it_is_drawn() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine.open_bytes(form_pdf(), None, None).expect("buka");
        let layout = doc.page_layout(0).expect("tata letak");
        let pic = layout.pictures.first().expect("gambar di dalam form");
        let r = pic.rect;
        // 50 from cm, 10 more from the form's /Matrix; 100 up; 200 × 150.
        assert!(
            (r.left - 60.0).abs() < 0.5 && (r.bottom - 100.0).abs() < 0.5,
            "{r:?}"
        );
        assert!(
            (r.width() - 200.0).abs() < 0.5 && (r.height() - 150.0).abs() < 0.5,
            "{r:?}"
        );
        assert_eq!(pic.mime, "image/jpeg");
    }

    #[test]
    fn a_chart_with_few_colours_stays_png() {
        let px = image::RgbaImage::from_fn(40, 40, |x, _| {
            if x < 20 {
                image::Rgba([255, 255, 255, 255])
            } else {
                image::Rgba([30, 60, 160, 255])
            }
        });
        assert_eq!(super::encode(px).expect("enkode").0, "image/png");
    }

    #[test]
    fn a_transparent_picture_stays_png() {
        let mut px = image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]));
        px.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        let (mime, bytes) = super::encode(px).expect("enkode");
        assert_eq!(mime, "image/png");
        assert!(bytes.starts_with(b"\x89PNG"));
    }

    #[test]
    fn family_names_lose_subset_tags_and_styles() {
        assert_eq!(
            super::family_of("ABCDEF+TimesNewRomanPS-BoldMT"),
            "TimesNewRoman"
        );
        assert_eq!(super::family_of("Helvetica-Bold"), "Helvetica");
        assert_eq!(super::family_of("Arial,Bold"), "Arial");
        assert_eq!(super::family_of("Calibri"), "Calibri");
    }
}
