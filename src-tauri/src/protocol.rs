//! The `izul://` custom URI protocol for pixels (SPEC 6).
//!
//! Tile bytes must never travel through `invoke`. Tauri's command bridge
//! serialises payloads as JSON, so a 1 MiB BGRA tile would become several MiB of
//! base64 and a parse on the UI thread — the 16 ms frame budget is gone before
//! anything is drawn.
//!
//! Instead the frontend requests `izul://tile/{doc}/{slot}/{epoch}` and this
//! handler answers with the raw bytes straight out of the shared-memory ring.
//! The browser wraps the response in an `ImageBitmap`, which is the one copy we
//! cannot avoid because it hands the pixels to the compositor.

use std::sync::Arc;

use izul_ipc::message::{DocId, SlotRef};
use izul_ipc::TileRing;

/// A parsed `izul://tile/...` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileUri {
    pub doc: DocId,
    pub slot: u32,
    pub epoch: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UriError {
    #[error("skema harus izul://")]
    WrongScheme,
    #[error("jenis sumber daya tidak dikenali: {0}")]
    UnknownKind(String),
    #[error("jalur tidak lengkap")]
    Incomplete,
    #[error("komponen bukan angka: {0}")]
    NotANumber(String),
}

/// Parses `izul://tile/{doc}/{slot}/{epoch}`.
///
/// Strict on purpose. The handler reaches into shared memory, so a URI that is
/// merely *nearly* right must be rejected rather than coerced into something
/// plausible.
pub fn parse_tile_uri(uri: &str) -> Result<TileUri, UriError> {
    let rest = uri.strip_prefix("izul://").ok_or(UriError::WrongScheme)?;
    // Some webview builds normalise a custom scheme through a host component,
    // leaving a leading slash; accept both spellings and nothing else.
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let mut parts = rest.split('/');

    let kind = parts.next().unwrap_or_default();
    if kind != "tile" {
        return Err(UriError::UnknownKind(kind.to_string()));
    }
    let mut take = || -> Result<u64, UriError> {
        let raw = parts.next().ok_or(UriError::Incomplete)?;
        // Trim a query string or fragment the webview may append for cache
        // busting; anything else non-numeric is an error.
        let raw = raw.split(['?', '#']).next().unwrap_or(raw);
        if raw.is_empty() {
            return Err(UriError::Incomplete);
        }
        raw.parse::<u64>()
            .map_err(|_| UriError::NotANumber(raw.to_string()))
    };
    let doc = take()?;
    let slot = take()?;
    let epoch = take()?;
    if parts.next().is_some() {
        return Err(UriError::Incomplete);
    }
    Ok(TileUri {
        doc: DocId(doc),
        slot: u32::try_from(slot).map_err(|_| UriError::NotANumber(slot.to_string()))?,
        epoch: u32::try_from(epoch).map_err(|_| UriError::NotANumber(epoch.to_string()))?,
    })
}

/// Copies a tile out of the ring, releasing the slot immediately.
///
/// One copy happens here, into the response buffer the webview owns. It is not
/// avoidable: the slot has to go back to the worker, and the webview needs the
/// bytes to outlive that. Everything before this point — PDFium's render, the
/// transfer to the UI process — is copy-free.
pub fn read_tile(ring: &Arc<TileRing>, uri: TileUri, stride_hint: u32) -> Option<TileBytes> {
    let sref = SlotRef {
        slot: uri.slot,
        offset: 0,
        stride: stride_hint,
        width: 0,
        height: 0,
        epoch: uri.epoch,
    };
    let held = ring.acquire(&sref).ok()?;
    let (w, h, stride) = (held.width(), held.height(), held.stride());
    let used = stride as usize * h as usize;
    let bytes = held.bytes().get(..used)?.to_vec();
    Some(TileBytes {
        bytes,
        width: w,
        height: h,
        stride,
    })
}

/// A tile lifted out of shared memory, ready to hand to the webview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileBytes {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

impl TileBytes {
    /// Headers that let the frontend build an `ImageBitmap` without guessing.
    pub fn headers(&self) -> [(&'static str, String); 4] {
        [
            ("X-Izul-Width", self.width.to_string()),
            ("X-Izul-Height", self.height.to_string()),
            ("X-Izul-Stride", self.stride.to_string()),
            // BGRA is PDFium's native order; saying so explicitly means the
            // frontend never has to infer it from the byte count.
            ("X-Izul-Format", "BGRA8".to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_uri_parses() {
        assert_eq!(
            parse_tile_uri("izul://tile/7/12/3"),
            Ok(TileUri {
                doc: DocId(7),
                slot: 12,
                epoch: 3
            })
        );
    }

    #[test]
    fn a_leading_slash_from_the_webview_is_tolerated() {
        assert_eq!(
            parse_tile_uri("izul:///tile/7/12/3"),
            Ok(TileUri {
                doc: DocId(7),
                slot: 12,
                epoch: 3
            })
        );
    }

    #[test]
    fn a_cache_busting_query_is_ignored() {
        assert_eq!(
            parse_tile_uri("izul://tile/1/2/3?v=9"),
            Ok(TileUri {
                doc: DocId(1),
                slot: 2,
                epoch: 3
            })
        );
    }

    #[test]
    fn another_scheme_is_refused() {
        assert_eq!(
            parse_tile_uri("http://tile/1/2/3"),
            Err(UriError::WrongScheme)
        );
        assert_eq!(
            parse_tile_uri("file:///etc/passwd"),
            Err(UriError::WrongScheme)
        );
    }

    #[test]
    fn an_unknown_resource_kind_is_refused() {
        assert!(matches!(
            parse_tile_uri("izul://file/1/2/3"),
            Err(UriError::UnknownKind(_))
        ));
    }

    #[test]
    fn a_short_uri_is_refused() {
        assert_eq!(parse_tile_uri("izul://tile/1/2"), Err(UriError::Incomplete));
        assert_eq!(parse_tile_uri("izul://tile"), Err(UriError::Incomplete));
    }

    #[test]
    fn a_long_uri_is_refused() {
        assert_eq!(
            parse_tile_uri("izul://tile/1/2/3/4"),
            Err(UriError::Incomplete)
        );
    }

    #[test]
    fn path_traversal_never_looks_numeric() {
        // The handler indexes shared memory, so anything that is not plainly a
        // number must be rejected outright rather than coerced.
        for bad in [
            "izul://tile/../../etc/passwd",
            "izul://tile/1/../2/3",
            "izul://tile/1/2/-3",
            "izul://tile/0x10/2/3",
            "izul://tile/1/2/3abc",
            "izul://tile/ 1/2/3",
        ] {
            assert!(parse_tile_uri(bad).is_err(), "{bad} must not parse");
        }
    }

    #[test]
    fn a_component_too_large_for_a_slot_index_is_refused() {
        let uri = format!("izul://tile/1/{}/3", u64::from(u32::MAX) + 1);
        assert!(matches!(parse_tile_uri(&uri), Err(UriError::NotANumber(_))));
    }

    #[test]
    fn headers_describe_the_bitmap_exactly() {
        let t = TileBytes {
            bytes: vec![0; 16],
            width: 2,
            height: 2,
            stride: 8,
        };
        let h = t.headers();
        assert_eq!(h[0], ("X-Izul-Width", "2".to_string()));
        assert_eq!(h[2], ("X-Izul-Stride", "8".to_string()));
        assert_eq!(h[3], ("X-Izul-Format", "BGRA8".to_string()));
    }
}
