//! Image XObjects for the pictures image annotations draw.
//!
//! A baseline JPEG goes in exactly as it came, under `/DCTDecode`: every PDF
//! reader decodes JPEG itself, so re-encoding would only lose quality. It is
//! also what lets the worker hand the *original file* back when the document
//! is opened again (`FPDFImageObj_GetImageDataRaw` returns the stream as
//! stored).
//!
//! Everything else — PNG, GIF, WebP, and the rare JPEG that is not plain
//! greyscale or RGB — is decoded to 8-bit RGBA and written as a `/FlateDecode`
//! RGB image, with its alpha as a separate greyscale `/SMask` when any pixel is
//! not opaque. A transparent signature stays transparent.

use std::io::Write as _;

use flate2::write::ZlibEncoder;
use flate2::Compression;

/// One or two objects: the image, and its soft mask when it has one.
#[derive(Debug)]
pub struct ImageObjects {
    /// The image dictionary and stream, with `{SMASK}` standing in for the
    /// soft mask's reference until the caller has numbered it.
    pub image: Vec<u8>,
    pub smask: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

/// Placeholder the caller replaces with `N 0 R` once the mask has a number.
pub const SMASK_SLOT: &str = "{SMASK}";

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("gambar tidak dapat dibaca: {0}")]
    Decode(String),
}

pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(
        Vec::with_capacity(data.len() / 2 + 64),
        Compression::default(),
    );
    // Writing into a `Vec` cannot fail.
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

/// A stream object's body: `<<dict/Length n>>stream … endstream`.
pub fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<<{dict}/Length {}>>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// Width, height and component count of a baseline or progressive JPEG,
/// read from its first start-of-frame marker.
pub fn jpeg_info(bytes: &[u8]) -> Option<(u32, u32, u8)> {
    if bytes.get(..2) != Some(&[0xFF, 0xD8]) {
        return None;
    }
    let mut i = 2usize;
    while i + 4 <= bytes.len() {
        if bytes.get(i) != Some(&0xFF) {
            return None;
        }
        let marker = *bytes.get(i + 1)?;
        if marker == 0xFF {
            i += 1;
            continue;
        }
        let len = u16::from_be_bytes([*bytes.get(i + 2)?, *bytes.get(i + 3)?]) as usize;
        // SOF0..SOF15, minus DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let h = u16::from_be_bytes([*bytes.get(i + 5)?, *bytes.get(i + 6)?]) as u32;
            let w = u16::from_be_bytes([*bytes.get(i + 7)?, *bytes.get(i + 8)?]) as u32;
            let comps = *bytes.get(i + 9)?;
            return Some((w, h, comps));
        }
        i += 2 + len;
    }
    None
}

/// Builds the image object(s) for a stored image's original bytes.
pub fn image_objects(bytes: &[u8]) -> Result<ImageObjects, ImageError> {
    if let Some((w, h, comps)) = jpeg_info(bytes) {
        let space = match comps {
            1 => Some("/DeviceGray"),
            3 => Some("/DeviceRGB"),
            _ => None,
        };
        if let (Some(space), true) = (space, w > 0 && h > 0) {
            let dict = format!(
                "/Type/XObject/Subtype/Image/Width {w}/Height {h}/ColorSpace{space}/BitsPerComponent 8/Filter/DCTDecode"
            );
            return Ok(ImageObjects {
                image: stream(&dict, bytes),
                smask: None,
                width: w,
                height: h,
            });
        }
    }

    let decoded = image::load_from_memory(bytes).map_err(|e| ImageError::Decode(e.to_string()))?;
    let rgba = decoded.to_rgba8();
    let (w, h) = rgba.dimensions();
    let pixels = rgba.into_raw();
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    let mut alpha = Vec::with_capacity((w * h) as usize);
    for px in pixels.chunks_exact(4) {
        rgb.extend_from_slice(px.get(..3).unwrap_or(&[0, 0, 0]));
        alpha.push(*px.get(3).unwrap_or(&255));
    }
    let translucent = alpha.iter().any(|&a| a != 255);
    let mut dict = format!(
        "/Type/XObject/Subtype/Image/Width {w}/Height {h}/ColorSpace/DeviceRGB/BitsPerComponent 8/Filter/FlateDecode"
    );
    let smask = if translucent {
        dict.push_str(&format!("/SMask {SMASK_SLOT}"));
        Some(stream(
            &format!(
                "/Type/XObject/Subtype/Image/Width {w}/Height {h}/ColorSpace/DeviceGray/BitsPerComponent 8/Filter/FlateDecode"
            ),
            &deflate(&alpha),
        ))
    } else {
        None
    };
    Ok(ImageObjects {
        image: stream(&dict, &deflate(&rgb)),
        smask,
        width: w,
        height: h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, alpha: u8) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, alpha]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 100, 50]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_jpeg_is_passed_through_byte_for_byte() {
        let bytes = jpeg(7, 5);
        assert_eq!(jpeg_info(&bytes), Some((7, 5, 3)));
        let obj = image_objects(&bytes).unwrap();
        assert!(obj.smask.is_none());
        let text = String::from_utf8_lossy(&obj.image);
        assert!(text.contains("/Filter/DCTDecode") && text.contains("/Width 7/Height 5"));
        // The original bytes are inside the stream, unchanged.
        let at = obj
            .image
            .windows(bytes.len())
            .position(|w| w == bytes.as_slice());
        assert!(at.is_some());
    }

    #[test]
    fn an_opaque_png_has_no_mask_and_a_translucent_one_does() {
        let opaque = image_objects(&png(3, 2, 255)).unwrap();
        assert!(opaque.smask.is_none());
        assert!(!String::from_utf8_lossy(&opaque.image).contains("SMask"));

        let see_through = image_objects(&png(3, 2, 128)).unwrap();
        assert!(see_through.smask.is_some());
        assert!(String::from_utf8_lossy(&see_through.image).contains("/SMask {SMASK}"));
        assert_eq!((see_through.width, see_through.height), (3, 2));
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(image_objects(b"not an image").is_err());
        assert_eq!(jpeg_info(b"\xFF\xD8\xFF"), None);
    }
}
