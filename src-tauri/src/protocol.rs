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
//!   http://izul.localhost/tile/{doc}/{page}/{rotation}/{scale}/{col}/{row}/{kind}?g={gen}&p={pri}
//! ```
//!
//! That is the form the frontend actually sends, and the only one WebView2 on
//! Windows will `fetch()` at all — a bare custom scheme is not fetchable
//! there. `wry` translates `http://izul.localhost/x` back to `izul://x`
//! before this handler ever runs, but only for `http`, because this app does
//! not opt into `useHttpsScheme` in `tauri.conf.json`. The parser below still
//! accepts `izul://` (what macOS and Linux fetch directly) and
//! `https://izul.localhost/` (harmless to keep tolerant), but `http://` is the
//! one that matters on the platform this ships for.
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

/// An `izul://image/{doc}/{ref}` request: the pixels of an inserted image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageUri {
    pub doc: u64,
    pub image: u32,
}

/// Strips whichever of the four spellings of our scheme this webview used.
///
/// Shared with [`parse_tile_uri`] rather than repeated, because the `localhost`
/// authority `wry` leaves behind on Windows was missed once already and cost a
/// session of blank pages (see the Phase 1 notes in CLAUDE.md).
fn strip_scheme(uri: &str) -> Result<&str, UriError> {
    let rest = uri
        .strip_prefix("http://izul.localhost/")
        .or_else(|| uri.strip_prefix("izul://"))
        .or_else(|| uri.strip_prefix("https://izul.localhost/"))
        .ok_or(UriError::WrongScheme)?;
    let rest = rest.strip_prefix("localhost/").unwrap_or(rest);
    Ok(rest.strip_prefix('/').unwrap_or(rest))
}

pub fn parse_image_uri(uri: &str) -> Result<ImageUri, UriError> {
    let rest = strip_scheme(uri)?;
    let path = rest.split('?').next().unwrap_or(rest);
    let path = path.split('#').next().unwrap_or(path);
    let mut parts = path.split('/');
    if parts.next() != Some("image") {
        return Err(UriError::UnknownKind(
            path.split('/').next().unwrap_or_default().to_string(),
        ));
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
    let image = take()?;
    if parts.next().is_some() {
        return Err(UriError::Incomplete);
    }
    Ok(ImageUri {
        doc,
        image: u32::try_from(image).map_err(|_| UriError::NotANumber(image.to_string()))?,
    })
}

/// An `izul://print/{job}/{index}` request: one page prepared for printing
/// (Phase 8, `printing.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrintUri {
    pub job: u64,
    pub index: u32,
}

pub fn parse_print_uri(uri: &str) -> Result<PrintUri, UriError> {
    let rest = strip_scheme(uri)?;
    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut parts = path.split('/');
    if parts.next() != Some("print") {
        return Err(UriError::UnknownKind(
            path.split('/').next().unwrap_or_default().to_string(),
        ));
    }
    let mut take = || -> Result<u64, UriError> {
        let raw = parts.next().ok_or(UriError::Incomplete)?;
        raw.parse::<u64>()
            .map_err(|_| UriError::NotANumber(raw.to_string()))
    };
    let job = take()?;
    let index = take()?;
    if parts.next().is_some() {
        return Err(UriError::Incomplete);
    }
    Ok(PrintUri {
        job,
        index: u32::try_from(index).map_err(|_| UriError::NotANumber(index.to_string()))?,
    })
}

pub fn parse_tile_uri(uri: &str) -> Result<TileUri, UriError> {
    // Four spellings reach this handler for the same request, and which one
    // arrives is decided by the webview, not by us.
    //
    // WebView2 on Windows cannot `fetch()` a bare custom scheme at all — only
    // `http`/`https` are fetchable there — so the frontend addresses tiles as
    // `http://izul.localhost/...`. `wry` intercepts that and hands it over as
    // `izul://localhost/...`: its `revert_uri_work_around` simply replaces the
    // literal `http://izul.` with `izul://`, so the `localhost` that was part
    // of the virtual host is left behind as the custom scheme's authority.
    // That is the shape Windows actually delivers, and it is the form every
    // real tile request takes there.
    //
    // macOS and Linux fetch the custom scheme directly and give `izul://...`
    // with no authority at all. The `http`/`https` forms are accepted too, in
    // case a request ever reaches us before wry's translation.
    let rest = uri
        .strip_prefix("http://izul.localhost/")
        .or_else(|| uri.strip_prefix("izul://"))
        .or_else(|| uri.strip_prefix("https://izul.localhost/"))
        .ok_or(UriError::WrongScheme)?;
    // The authority, where the webview left one. Stripped only here, at the
    // very front, so a path segment further in that happens to read
    // `localhost` is still the nonsense it looks like.
    let rest = rest.strip_prefix("localhost/").unwrap_or(rest);
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
            invert: query_value(query, "inv") == Some(1),
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

/// The tile headers the frontend reads, named once so the list below and the
/// `Access-Control-Expose-Headers` value cannot drift apart.
pub const TILE_HEADER_NAMES: [&str; 4] = [
    "X-Izul-Width",
    "X-Izul-Height",
    "X-Izul-Stride",
    "X-Izul-Format",
];

/// Headers that let the frontend build an `ImageBitmap` without guessing.
pub fn tile_headers(width: u32, height: u32, stride: u32) -> [(&'static str, String); 4] {
    [
        (TILE_HEADER_NAMES[0], width.to_string()),
        (TILE_HEADER_NAMES[1], height.to_string()),
        (TILE_HEADER_NAMES[2], stride.to_string()),
        // BGRA is PDFium's native order; saying so explicitly means the
        // frontend never has to infer it from the byte count.
        (TILE_HEADER_NAMES[3], "BGRA8".to_string()),
    ]
}

/// Cross-origin headers, required on **every** response this protocol makes.
///
/// The page and the tiles do not share an origin: the document runs at
/// `http://localhost:5173` under `tauri dev` and at the app's own origin in a
/// packaged build, while tiles come from `http://izul.localhost`. A response
/// without `Access-Control-Allow-Origin` is therefore discarded by the browser
/// before any code of ours sees it — `fetch` rejects with a bare
/// `TypeError: Failed to fetch` and DevTools reports zero response headers,
/// which is indistinguishable from the request never having been answered.
/// Tauri's own IPC and asset protocols set these for exactly this reason; a
/// hand-registered protocol has to do it itself.
///
/// `Access-Control-Expose-Headers` is the second half and just as necessary:
/// on a cross-origin response the browser hides every header that is not named
/// there, so `X-Izul-Width` would read back as `null` and a tile that arrived
/// perfectly would be rejected for having no dimensions.
pub fn cors_headers() -> [(&'static str, String); 2] {
    [
        // `*` rather than the window's origin: it is what Tauri's own IPC
        // protocol uses, the response carries no credentials, and the origin
        // differs between a dev run and a packaged build.
        ("Access-Control-Allow-Origin", "*".to_string()),
        (
            "Access-Control-Expose-Headers",
            TILE_HEADER_NAMES.join(", "),
        ),
    ]
}

#[cfg(test)]
mod tests {

    /// The Windows spelling, which is the one that actually arrives there: wry
    /// leaves `localhost` behind as the custom scheme's authority, and reading
    /// it as the first path segment is what made every tile 400 in Phase 1.
    #[test]
    fn dark_modes_inversion_is_read_from_the_query_and_is_its_own_tile() {
        let light = key("izul://tile/7/3/0/1500/2/1/sharp?g=9&p=2");
        let dark = key("izul://tile/7/3/0/1500/2/1/sharp?g=9&p=2&inv=1");
        assert!(!light.invert);
        assert!(dark.invert);
        assert_ne!(
            light, dark,
            "ubin gelap dan terang tidak boleh berbagi entri cache"
        );
        assert!(!key("izul://tile/7/3/0/1500/2/1/sharp?inv=0").invert);
    }

    #[test]
    fn a_print_uri_parses_in_every_spelling_and_nothing_else_does() {
        for uri in [
            "izul://print/4/12",
            "izul://localhost/print/4/12",
            "http://izul.localhost/print/4/12?v=1",
            "https://izul.localhost/print/4/12",
        ] {
            assert_eq!(
                parse_print_uri(uri),
                Ok(PrintUri { job: 4, index: 12 }),
                "{uri}"
            );
        }
        assert!(parse_print_uri("izul://print/4").is_err());
        assert!(parse_print_uri("izul://print/4/x").is_err());
        assert!(parse_print_uri("izul://print/4/1/2").is_err());
        assert!(parse_print_uri("izul://image/4/1").is_err());
        assert!(parse_print_uri("izul://print/../../1").is_err());
    }

    #[test]
    fn an_image_uri_parses_in_every_spelling() {
        let expected = ImageUri { doc: 3, image: 7 };
        for uri in [
            "izul://image/3/7",
            "izul://localhost/image/3/7",
            "http://izul.localhost/image/3/7",
            "https://izul.localhost/image/3/7",
        ] {
            assert_eq!(parse_image_uri(uri), Ok(expected), "{uri}");
        }
    }

    #[test]
    fn an_image_uri_is_strict_about_what_it_accepts() {
        assert!(
            parse_image_uri("izul://image/3").is_err(),
            "kurang satu bagian"
        );
        assert!(
            parse_image_uri("izul://image/3/7/8").is_err(),
            "kelebihan bagian"
        );
        assert!(parse_image_uri("izul://image/x/7").is_err(), "bukan angka");
        assert!(parse_image_uri("izul://tile/3/7").is_err(), "jenis lain");
        assert!(parse_image_uri("file:///etc/passwd").is_err(), "skema lain");
    }

    /// The two routes must not answer for each other, or a malformed tile URI
    /// could be served as an image and vice versa.
    #[test]
    fn the_two_routes_do_not_overlap() {
        let tile = "izul://tile/1/0/0/256/0/0/preview?g=2&p=0";
        assert!(parse_tile_uri(tile).is_ok());
        assert!(parse_image_uri(tile).is_err());
        let image = "izul://image/1/0";
        assert!(parse_image_uri(image).is_ok());
        assert!(parse_tile_uri(image).is_err());
    }
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
                invert: false,
            }
        );
        assert_eq!(u.generation, 9);
        assert_eq!(u.priority, Priority::Visible);
    }

    #[test]
    fn a_leading_slash_from_the_webview_is_tolerated() {
        assert_eq!(key("izul:///tile/1/0/0/1000/0/0/sharp").doc, 1);
    }

    /// The URI Windows actually delivers, copied from a user's log.
    ///
    /// `wry` rewrites `http://izul.localhost/x` by replacing the literal
    /// `http://izul.` with `izul://` (see its `revert_uri_work_around`), so the
    /// `localhost` that was part of the virtual host survives as the custom
    /// scheme's authority. Every tile request on Windows arrived in this shape
    /// and every one was refused — 1400 of them in one session — because the
    /// parser read `localhost` as the resource kind.
    #[test]
    fn the_authority_wry_leaves_behind_is_not_part_of_the_path() {
        let uri = "izul://localhost/tile/1/0/0/256/0/0/preview?g=2&p=0";
        let parsed = parse_tile_uri(uri).expect("uri dari log pengguna harus terurai");
        assert_eq!(parsed.key.doc, 1);
        assert_eq!(parsed.key.page, 0);
        assert_eq!(parsed.key.ppp_milli, 256);
        assert_eq!(parsed.key.kind, TileKind::Preview);
        assert_eq!(parsed.generation, 2);
    }

    /// The same request in all four spellings names the same tile. Which one
    /// arrives depends on the webview, and no caller should have to care.
    #[test]
    fn every_spelling_of_the_same_tile_parses_to_the_same_key() {
        let tail = "tile/7/3/1/1500/2/4/sharp?g=9&p=2";
        let forms = [
            format!("izul://localhost/{tail}"),
            format!("izul://{tail}"),
            format!("http://izul.localhost/{tail}"),
            format!("https://izul.localhost/{tail}"),
        ];
        let mut keys = Vec::new();
        for form in &forms {
            let parsed = parse_tile_uri(form).unwrap_or_else(|e| panic!("{form}: {e}"));
            keys.push(parsed.key);
        }
        for (form, key) in forms.iter().zip(keys.iter()) {
            assert_eq!(*key, keys[0], "{form} mengurai jadi ubin yang berbeda");
        }
    }

    /// `localhost` is an authority, not a resource kind — but only where an
    /// authority can appear. A path segment that happens to say `localhost`
    /// deeper in is still nonsense and must stay refused.
    #[test]
    fn localhost_is_only_stripped_where_the_authority_belongs() {
        assert!(parse_tile_uri("izul://localhost/localhost/1/0/0/256/0/0/preview").is_err());
        assert!(parse_tile_uri("izul://localhost").is_err());
        assert!(parse_tile_uri("izul://localhost/").is_err());
    }

    #[test]
    fn the_windows_form_is_what_actually_gets_fetched_there() {
        // WebView2 will not `fetch()` a bare custom scheme, so the frontend
        // addresses tiles this way — and specifically as `http`, not `https`,
        // because this app does not set `useHttpsScheme`. If this parse
        // breaks, a Windows build renders nothing and looks fine everywhere
        // else, which is exactly the failure this guards against.
        assert_eq!(
            key("http://izul.localhost/tile/1/0/0/1000/0/0/sharp"),
            key("izul://tile/1/0/0/1000/0/0/sharp")
        );
    }

    #[test]
    fn the_https_form_is_tolerated_even_though_nothing_sends_it() {
        // Kept accepted in case a future window opts into `useHttpsScheme`;
        // costs nothing to allow and saves a debugging session if it changes.
        assert_eq!(
            key("https://izul.localhost/tile/1/0/0/1000/0/0/sharp"),
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

    #[test]
    fn every_response_may_cross_the_origin_boundary() {
        // Without this the browser discards the answer before the viewport
        // sees it, and the failure looks exactly like a request that was
        // never answered at all.
        let cors = cors_headers();
        assert_eq!(cors[0], ("Access-Control-Allow-Origin", "*".to_string()));
    }

    #[test]
    fn every_header_the_viewport_reads_is_exposed_to_it() {
        // A cross-origin response hides any header not named here, so a tile
        // would arrive intact and then be rejected for having no dimensions.
        let exposed = cors_headers()[1].1.clone();
        for (name, _) in tile_headers(1, 1, 4) {
            assert!(
                exposed.contains(name),
                "{name} is read by src/viewport/tileSource.ts but not exposed"
            );
        }
    }
}
