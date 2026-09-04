//! The `izul` custom URI protocol for pixels (SPEC 6).
//!
//! Tile bytes must never travel through `invoke`. Tauri's command bridge
//! serialises payloads as JSON, so a 1 MiB BGRA tile would become several MiB of
//! base64 and a parse on the UI thread — the 16 ms frame budget is gone before
//! anything is drawn.
//!
//! Instead the frontend requests a tile *by identity* and this handler answers
//! with the bytes: from the cache when they are resident, and otherwise after
//! scheduling the render and waiting for it. One round trip, not two, and the
//! same URI is a cache key on both sides of the boundary.
//!
//! ```text
//!   izul://tile/{doc}/{page}/{rotation}/{scale}/{col}/{row}/{kind}?g={gen}&p={pri}
//! ```
//!
//! * `rotation` — quarter turns clockwise, 0..3.
//! * `scale` — pixels per PDF point in thousandths; for a preview, the
//!   bitmap's maximum edge in pixels instead.
//! * `kind` — `sharp`, `fast` or `preview`.
//! * `g` — the layout epoch the request belongs to. A request from an epoch
//!   the user has already left is answered 409 rather than rendered.
//! * `p` — priority: 2 visible, 1 preview, 0 prefetch.
//!
//! Everything in the path is an integer, and the parser is strict, because this
//! handler reaches into shared memory: a URI that is merely *nearly* right must
//! be rejected rather than coerced into something plausible.

use crate::render::{Priority, TileKey, TileKind};

/// A parsed tile request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileUri {
    pub key: TileKey,
    pub generation: u64,
    pub priority: Priority,
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
    #[error("tingkat ubin tidak dikenali: {0}")]
    UnknownTier(String),
    #[error("rotasi harus 0..3, bukan {0}")]
    BadRotation(u64),
    #[error("skala di luar jangkauan: {0}")]
    BadScale(u64),
}

/// Largest scale we accept, in thousandths of a pixel per point.
///
/// SPEC 11.1 caps zoom at 1600 %; with a 4x display scale that is 64 pixels per
/// point. Anything past it is a malformed request, and the cap is what stops
/// one from asking for a page the size of a stadium.
const MAX_PPP_MILLI: u64 = 64_000;

pub fn parse_tile_uri(uri: &str) -> Result<TileUri, UriError> {
    // Three spellings reach this handler for the same request. `izul://...`
    // is what macOS and Linux's webviews will fetch directly. WebView2 on
    // Windows refuses to `fetch()` a bare custom scheme at all — only
    // `http`/`https` are fetchable there — so the frontend addresses tiles as
    // `https://izul.localhost/...`, and Tauri delivers that here exactly as
    // it arrived. `http://izul.localhost/...` is accepted alongside it because
    // that is the form Tauri falls back to on Android, which this application
    // does not ship on but which costs nothing to also accept.
    let rest = uri
        .strip_prefix("izul://")
        .or_else(|| uri.strip_prefix("https://izul.localhost/"))
        .or_else(|| uri.strip_prefix("http://izul.localhost/"))
        .ok_or(UriError::WrongScheme)?;
    // Some webview builds normalise a custom scheme through a host component,
    // leaving a leading slash; accept both spellings and nothing else.
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    // The query is read separately: only the path identifies the tile.
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, q),
        None => (rest, ""),
    };
    let path = path.split('#').next().unwrap_or(path);
    let mut parts = path.split('/');

    let kind = parts.next().unwrap_or_default();
    if kind != "tile" {
        return Err(UriError::UnknownKind(kind.to_string()));
    }
    let mut take = || -> Result<u64, UriError> {
        let raw = parts.next().ok_or(UriError::Incomplete)?;
        if raw.is_empty() {
            return Err(UriError::Incomplete);
        }
        raw.parse::<u64>()
            .map_err(|_| UriError::NotANumber(raw.to_string()))
    };
    let doc = take()?;
    let page = take()?;
    let rotation = take()?;
    let scale = take()?;
    let col = take()?;
    let row = take()?;
    let tier = parts.next().ok_or(UriError::Incomplete)?;
    let tier = tier.split('#').next().unwrap_or(tier);
    if parts.next().is_some() {
        return Err(UriError::Incomplete);
    }

    if rotation > 3 {
        return Err(UriError::BadRotation(rotation));
    }
    if scale == 0 || scale > MAX_PPP_MILLI {
        return Err(UriError::BadScale(scale));
    }
    let kind = match tier {
        "sharp" => TileKind::Sharp,
        "fast" => TileKind::Fast,
        "preview" => TileKind::Preview,
        other => return Err(UriError::UnknownTier(other.to_string())),
    };

    let num = |v: u64, what: &str| -> Result<u32, UriError> {
        u32::try_from(v).map_err(|_| UriError::NotANumber(format!("{what}={v}")))
    };
    let small = |v: u64, what: &str| -> Result<u16, UriError> {
        u16::try_from(v).map_err(|_| UriError::NotANumber(format!("{what}={v}")))
    };

    Ok(TileUri {
        key: TileKey {
            doc,
            page: num(page, "page")?,
            rotation: rotation as u8,
            ppp_milli: num(scale, "scale")?,
            col: small(col, "col")?,
            row: small(row, "row")?,
            kind,
        },
        generation: query_value(query, "g").unwrap_or(0),
        priority: Priority::from_u8(query_value(query, "p").unwrap_or(2) as u8),
    })
}

/// Reads one `key=value` pair out of a query string, ignoring anything else.
fn query_value(query: &str, key: &str) -> Option<u64> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == key)
        .and_then(|(_, v)| v.parse::<u64>().ok())
}

/// Headers that let the frontend build an `ImageBitmap` without guessing.
pub fn tile_headers(width: u32, height: u32, stride: u32) -> [(&'static str, String); 4] {
    [
        ("X-Izul-Width", width.to_string()),
        ("X-Izul-Height", height.to_string()),
        ("X-Izul-Stride", stride.to_string()),
        // BGRA is PDFium's native order; saying so explicitly means the
        // frontend never has to infer it from the byte count.
        ("X-Izul-Format", "BGRA8".to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(uri: &str) -> TileKey {
        parse_tile_uri(uri).expect("parses").key
    }

    #[test]
    fn a_well_formed_uri_parses() {
        let u = parse_tile_uri("izul://tile/7/3/0/1500/2/1/sharp?g=9&p=2").expect("parses");
        assert_eq!(
            u.key,
            TileKey {
                doc: 7,
                page: 3,
                rotation: 0,
                ppp_milli: 1500,
                col: 2,
                row: 1,
                kind: TileKind::Sharp,
            }
        );
        assert_eq!(u.generation, 9);
        assert_eq!(u.priority, Priority::Visible);
    }

    #[test]
    fn a_leading_slash_from_the_webview_is_tolerated() {
        assert_eq!(key("izul:///tile/1/0/0/1000/0/0/sharp").doc, 1);
    }

    #[test]
    fn the_windows_form_is_what_actually_gets_fetched_there() {
        // WebView2 will not `fetch()` a bare custom scheme, so the frontend
        // addresses tiles this way on every platform. If this parse breaks, a
        // Windows build renders nothing and looks fine everywhere else.
        assert_eq!(
            key("https://izul.localhost/tile/1/0/0/1000/0/0/sharp"),
            key("izul://tile/1/0/0/1000/0/0/sharp")
        );
    }

    #[test]
    fn the_android_form_is_tolerated_too() {
        assert_eq!(
            key("http://izul.localhost/tile/1/0/0/1000/0/0/sharp"),
            key("izul://tile/1/0/0/1000/0/0/sharp")
        );
    }

    #[test]
    fn every_tier_is_recognised_and_nothing_else_is() {
        assert_eq!(key("izul://tile/1/0/0/1000/0/0/fast").kind, TileKind::Fast);
        assert_eq!(
            key("izul://tile/1/0/0/256/0/0/preview").kind,
            TileKind::Preview
        );
        assert!(matches!(
            parse_tile_uri("izul://tile/1/0/0/1000/0/0/blurry"),
            Err(UriError::UnknownTier(_))
        ));
    }

    #[test]
    fn the_query_defaults_to_the_visible_tier_of_generation_zero() {
        let u = parse_tile_uri("izul://tile/1/0/0/1000/0/0/sharp").expect("parses");
        assert_eq!(u.generation, 0);
        assert_eq!(u.priority, Priority::Visible);
    }

    #[test]
    fn priority_survives_the_round_trip() {
        for (p, expect) in [
            (0, Priority::Prefetch),
            (1, Priority::Preview),
            (2, Priority::Visible),
        ] {
            let uri = format!("izul://tile/1/0/0/1000/0/0/sharp?g=1&p={p}");
            assert_eq!(parse_tile_uri(&uri).expect("parses").priority, expect);
        }
    }

    #[test]
    fn a_cache_busting_fragment_is_ignored() {
        assert_eq!(key("izul://tile/1/0/0/1000/0/0/sharp#x").page, 0);
    }

    #[test]
    fn another_scheme_is_refused() {
        assert_eq!(
            parse_tile_uri("http://tile/1/0/0/1000/0/0/sharp"),
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
            parse_tile_uri("izul://file/1/0/0/1000/0/0/sharp"),
            Err(UriError::UnknownKind(_))
        ));
    }

    #[test]
    fn a_short_or_long_uri_is_refused() {
        assert_eq!(
            parse_tile_uri("izul://tile/1/0/0/1000/0/0"),
            Err(UriError::Incomplete)
        );
        assert_eq!(parse_tile_uri("izul://tile"), Err(UriError::Incomplete));
        assert_eq!(
            parse_tile_uri("izul://tile/1/0/0/1000/0/0/sharp/9"),
            Err(UriError::Incomplete)
        );
    }

    #[test]
    fn path_traversal_never_looks_numeric() {
        for bad in [
            "izul://tile/../../etc/passwd",
            "izul://tile/1/../2/0/1000/0/0/sharp",
            "izul://tile/1/0/0/1000/0/-1/sharp",
            "izul://tile/0x10/0/0/1000/0/0/sharp",
            "izul://tile/1/0/0/1000/0/0abc/sharp",
            "izul://tile/ 1/0/0/1000/0/0/sharp",
        ] {
            assert!(parse_tile_uri(bad).is_err(), "{bad} must not parse");
        }
    }

    #[test]
    fn an_impossible_rotation_or_scale_is_refused() {
        assert_eq!(
            parse_tile_uri("izul://tile/1/0/4/1000/0/0/sharp"),
            Err(UriError::BadRotation(4))
        );
        assert_eq!(
            parse_tile_uri("izul://tile/1/0/0/0/0/0/sharp"),
            Err(UriError::BadScale(0))
        );
        // 1600 % zoom on a 4x display is 64 px per point; past that a request
        // is asking for a page nobody could display.
        assert!(matches!(
            parse_tile_uri("izul://tile/1/0/0/999999/0/0/sharp"),
            Err(UriError::BadScale(_))
        ));
    }

    #[test]
    fn a_component_too_large_for_its_field_is_refused() {
        let uri = format!("izul://tile/1/0/0/1000/{}/0/sharp", u64::from(u32::MAX));
        assert!(matches!(parse_tile_uri(&uri), Err(UriError::NotANumber(_))));
    }

    #[test]
    fn headers_describe_the_bitmap_exactly() {
        let h = tile_headers(2, 2, 8);
        assert_eq!(h[0], ("X-Izul-Width", "2".to_string()));
        assert_eq!(h[2], ("X-Izul-Stride", "8".to_string()));
        assert_eq!(h[3], ("X-Izul-Format", "BGRA8".to_string()));
    }
}
