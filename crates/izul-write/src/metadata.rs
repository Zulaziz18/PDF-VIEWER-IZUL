//! The key that makes a saved annotation editable again (SPEC 8: "objek hidup
//! setelah simpan-buka").
//!
//! Every annotation this application writes carries `/IzulObj`: the whole
//! [`AnnotObject`] as JSON, behind a version tag. Other readers ignore a key
//! they do not know and draw the appearance stream; this application finds the
//! key, rebuilds the object, and the annotation is live again — its ink points,
//! its text, its arrow head, not a picture of them.
//!
//! The JSON is kept ASCII (every other character becomes `\uXXXX`), so the
//! value is written as a compact literal string rather than as UTF-16 hex.
//! Images are **not** in here: their pixels are already in the appearance
//! stream's image XObject, and the worker reads them back from there through
//! PDFium on open. Carrying them twice would double the file and would put
//! megabyte strings in a dictionary, past Acrobat's documented 32 767-byte
//! string limit (ISO 32000-1, Annex C).

use izul_model::annot::AnnotObject;
use serde::{Deserialize, Serialize};

/// The dictionary key, without the slash.
pub const KEY: &str = "IzulObj";

/// Prefix of every `/NM` this application writes. The worker creates its
/// placeholders with it and this crate finds them by it.
pub const NM_PREFIX: &str = "izul-";

/// Version of the stored shape. Raised when `AnnotObject` changes in a way old
/// files cannot be read into; a file with a newer version is shown, not edited.
pub const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Stored {
    izul: u32,
    obj: AnnotObject,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MetadataError {
    #[error("metadata anotasi tidak dapat dibaca: {0}")]
    Malformed(String),
    #[error("anotasi ditulis versi aplikasi yang lebih baru (format {0})")]
    TooNew(u32),
}

/// The `/IzulObj` value for an object, as ASCII-only JSON.
pub fn encode(obj: &AnnotObject) -> String {
    let stored = Stored {
        izul: VERSION,
        obj: obj.clone(),
    };
    // Serialising a plain data struct cannot fail; an empty string would be
    // read back as malformed and the annotation shown but not editable, which
    // is the safe direction to be wrong in.
    let json = serde_json::to_string(&stored).unwrap_or_default();
    ascii_json(&json)
}

/// Rebuilds the object from a `/IzulObj` value.
pub fn decode(value: &str) -> Result<AnnotObject, MetadataError> {
    #[derive(Deserialize)]
    struct Version {
        izul: u32,
    }
    let version: Version =
        serde_json::from_str(value).map_err(|e| MetadataError::Malformed(e.to_string()))?;
    if version.izul > VERSION {
        return Err(MetadataError::TooNew(version.izul));
    }
    let stored: Stored =
        serde_json::from_str(value).map_err(|e| MetadataError::Malformed(e.to_string()))?;
    Ok(stored.obj)
}

/// Replaces every non-ASCII character in a JSON text with its `\uXXXX`
/// escape. Valid only outside string delimiters' own syntax, which is fine:
/// JSON never needs a non-ASCII character anywhere but inside a string, where
/// the escape means the same thing.
fn ascii_json(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for ch in json.chars() {
        if ch.is_ascii() {
            out.push(ch);
        } else {
            let mut units = [0u16; 2];
            for unit in ch.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use izul_model::annot::{AnnotId, AnnotKind, AnnotPayload, FontSpec, TextAlign};
    use izul_model::display::Rgba;
    use izul_model::geom::PdfRectF;

    fn text(t: &str) -> AnnotObject {
        AnnotObject::new(
            AnnotId(9),
            2,
            AnnotKind::FreeText,
            PdfRectF::new(10.0, 10.0, 200.0, 60.0),
            AnnotPayload::FreeText {
                text: t.into(),
                font: FontSpec::default(),
                color: Rgba::BLACK,
                align: TextAlign::Left,
                line_spacing: 1.2,
                background: None,
                border: None,
            },
        )
    }

    #[test]
    fn round_trips_and_stays_ascii() {
        let obj = text("Catatan — é, 漢字, مرحبا, 😀 (kurung)");
        let value = encode(&obj);
        assert!(value.is_ascii(), "{value}");
        assert_eq!(decode(&value).unwrap(), obj);
    }

    #[test]
    fn a_newer_format_is_refused_by_name() {
        let value = encode(&text("x")).replacen("\"izul\":1", "\"izul\":99", 1);
        assert_eq!(decode(&value), Err(MetadataError::TooNew(99)));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(matches!(decode("{"), Err(MetadataError::Malformed(_))));
        assert!(matches!(
            decode("{\"izul\":1}"),
            Err(MetadataError::Malformed(_))
        ));
    }
}
