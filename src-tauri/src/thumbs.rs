//! Cover images for the recent-files list (SPEC 11.3, SPEC 12).
//!
//! A list of file names is a list of file names; SPEC 12 asks the empty state
//! to be designed rather than left as a grey rectangle, and a reader recognises
//! the cover of a document long before they read its name.
//!
//! The awkward part is *when* a cover can be made. Rendering one means opening
//! the PDF, which means a worker, which is the one thing a recent-files list
//! must not need — a list of twenty files would open twenty documents just to
//! draw a panel nobody has clicked yet. So a cover is captured at the one
//! moment the document is already open and already rendered, which is when the
//! user opens it, and written to disk next to the databases. A file that has
//! never been opened in this application has no cover and gets a placeholder,
//! which is honest and costs nothing.
//!
//! PNG rather than the raw BGRA the tile protocol serves: a cover is 200 px on
//! its longest edge, so the encode is under a millisecond, and a data URL drops
//! straight into an `<img>` without the frontend decoding anything by hand.

use std::path::{Path, PathBuf};

use izul_store::PathHash;

/// Longest edge of a cover, in pixels.
///
/// The list draws them at 96 px wide (144 px on a 1.5x display); 200 keeps a
/// 2x display sharp without storing anything anyone would call an image cache.
pub const COVER_MAX_EDGE: u32 = 200;

/// Where covers live.
pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("thumbs")
}

/// The file a cover for `path` would be stored in.
///
/// Named by the path hash, which folds case on Windows, so the same document
/// reached as `C:\Docs\A.pdf` and `c:\docs\a.pdf` shares one cover instead of
/// rendering two.
pub fn file_for(data_dir: &Path, path: &Path) -> PathBuf {
    let hash = PathHash::of(path);
    let mut name = String::with_capacity(64);
    for byte in hash.as_bytes().iter().take(16) {
        name.push_str(&format!("{byte:02x}"));
    }
    name.push_str(".png");
    dir(data_dir).join(name)
}

/// Encodes a BGRA bitmap as a PNG.
///
/// PDFium renders BGRA and `image` wants RGBA, so the swizzle happens here —
/// once, on a bitmap of at most 200x200, off the UI thread.
pub fn encode_png(bgra: &[u8], width: u32, height: u32, stride: u32) -> Result<Vec<u8>, String> {
    let (w, h) = (width as usize, height as usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        let from = y * stride as usize;
        for x in 0..w {
            let s = from + x * 4;
            let d = (y * w + x) * 4;
            let (b, g, r, a) = match bgra.get(s..s + 4) {
                Some([b, g, r, a]) => (*b, *g, *r, *a),
                _ => return Err("bitmap terpotong".into()),
            };
            match rgba.get_mut(d..d + 4) {
                Some(px) => px.copy_from_slice(&[r, g, b, a]),
                None => return Err("bitmap terpotong".into()),
            }
        }
    }
    let buffer: image::RgbaImage = image::ImageBuffer::from_raw(width, height, rgba)
        .ok_or_else(|| "ukuran bitmap tidak konsisten".to_string())?;
    let mut out = std::io::Cursor::new(Vec::new());
    buffer
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| format!("png: {e}"))?;
    Ok(out.into_inner())
}

/// Writes a cover for `path`, creating the folder if needed.
pub fn store(data_dir: &Path, path: &Path, png: &[u8]) -> Result<(), String> {
    let target = file_for(data_dir, path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&target, png).map_err(|e| format!("{}: {e}", target.display()))
}

/// The cover for `path` as a `data:` URL, or `None` when there is none.
///
/// A data URL rather than a file path because the webview cannot read the
/// user's data folder — and should not be able to. `img-src data:` is already
/// in the CSP for exactly this.
pub fn load_data_url(data_dir: &Path, path: &Path) -> Option<String> {
    let bytes = std::fs::read(file_for(data_dir, path)).ok()?;
    Some(format!("data:image/png;base64,{}", base64(&bytes)))
}

/// Minimal base64, so a cover does not pull a dependency in for six lines.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0) as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        let idx = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, v) in idx.iter().enumerate() {
            if i > chunk.len() {
                out.push('=');
            } else {
                // `v` is masked to 0..63 and the table has 64 entries, so the
                // fallback is unreachable; it is here because the workspace
                // lints refuse a bare index, and a wrong character would be
                // visible in the test above anyway.
                out.push(char::from(
                    ALPHABET.get(*v as usize).copied().unwrap_or(b'='),
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checked against a known-good encoder rather than against itself: a
    /// base64 that is wrong in a consistent way would satisfy a round-trip test
    /// and still produce an image the webview refuses.
    #[test]
    fn base64_matches_the_standard_encoding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    /// Windows compares paths case-insensitively, and a cover rendered for one
    /// spelling must be found under the other.
    #[test]
    fn a_covers_name_follows_the_paths_identity() {
        let dir = PathBuf::from("/data");
        let a = file_for(&dir, Path::new(r"C:\Docs\Report.pdf"));
        let b = file_for(&dir, Path::new("C:/Docs/Report.pdf"));
        assert_eq!(a, b);
        let other = file_for(&dir, Path::new("C:/Docs/Other.pdf"));
        assert_ne!(a, other);
    }

    #[test]
    fn a_missing_cover_is_none_rather_than_an_error() {
        let dir = std::env::temp_dir().join("izul-thumbs-test-none");
        assert!(load_data_url(&dir, Path::new("/nowhere/x.pdf")).is_none());
    }

    /// The swizzle is the part that is easy to get backwards, and a cover with
    /// red and blue swapped looks plausible enough to ship.
    #[test]
    fn bgra_becomes_rgba_in_the_encoded_image() {
        // One pure-red pixel, written the way PDFium writes it: B, G, R, A.
        let png = encode_png(&[0x00, 0x00, 0xff, 0xff], 1, 1, 4).expect("png");
        let decoded = image::load_from_memory(&png)
            .expect("baca kembali")
            .to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [0xff, 0x00, 0x00, 0xff]);
    }

    /// Rows of a PDFium bitmap are padded to a stride; reading them as if they
    /// were tight produces a sheared image.
    #[test]
    fn padded_rows_are_honoured() {
        // 1px wide, 2px tall, stride 8: one pixel of padding per row.
        let bgra = [
            0x00, 0x00, 0xff, 0xff, 0x11, 0x11, 0x11, 0x11, // red, then padding
            0xff, 0x00, 0x00, 0xff, 0x22, 0x22, 0x22, 0x22, // blue, then padding
        ];
        let png = encode_png(&bgra, 1, 2, 8).expect("png");
        let decoded = image::load_from_memory(&png)
            .expect("baca kembali")
            .to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [0xff, 0x00, 0x00, 0xff]);
        assert_eq!(decoded.get_pixel(0, 1).0, [0x00, 0x00, 0xff, 0xff]);
    }

    #[test]
    fn a_truncated_bitmap_is_refused_rather_than_read_past() {
        assert!(encode_png(&[0, 0, 0], 1, 1, 4).is_err());
    }
}
