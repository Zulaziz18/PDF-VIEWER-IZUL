//! `version.json` as the single source of version truth (SPEC 0).
//!
//! Baked in at compile time rather than read from disk: a portable build must
//! report its own version even if someone deletes the file next to it, and the
//! title bar must never disagree with the About box.

use serde::{Deserialize, Serialize};

const VERSION_JSON: &str = include_str!("../../version.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub name: String,
    pub codename: String,
    pub version: String,
    pub phase: String,
    #[serde(rename = "phaseName")]
    pub phase_name: String,
    #[serde(rename = "buildChannel")]
    pub build_channel: String,
    #[serde(rename = "pdfiumVersion")]
    pub pdfium_version: String,
    #[serde(rename = "pdfiumTag")]
    pub pdfium_tag: String,
    #[serde(rename = "minWindows")]
    pub min_windows: String,
}

/// Parses the embedded manifest.
///
/// A failure here is a build-configuration error, not a run-time condition, so
/// the caller gets a `Result` and the app refuses to start rather than showing
/// an invented version.
pub fn info() -> Result<VersionInfo, serde_json::Error> {
    serde_json::from_str(VERSION_JSON)
}

/// Text for the title bar: "PDF Studio Izul 7.0.0-alpha.0 (Atlas)".
pub fn title_bar_text(v: &VersionInfo) -> String {
    format!("{} {} ({})", v.name, v.version, v.codename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_manifest_parses() {
        let v = info().expect("version.json must parse");
        assert!(!v.version.is_empty());
        assert!(!v.pdfium_version.is_empty());
    }

    #[test]
    fn the_cargo_version_matches_version_json() {
        // Two places state the version; if they ever disagree, the title bar and
        // the installer disagree, and the user is the one who finds out.
        let v = info().expect("parse");
        assert_eq!(v.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_pinned_pdfium_matches_the_pdf_crate() {
        // The vendored binary, the bindings feature, and this manifest must all
        // name one PDFium build.
        let v = info().expect("parse");
        assert_eq!(v.pdfium_version, "151.0.7881.0");
        assert!(v.pdfium_tag.ends_with("7881"));
    }

    #[test]
    fn title_bar_text_is_assembled_from_the_manifest() {
        let v = info().expect("parse");
        let t = title_bar_text(&v);
        assert!(t.contains(&v.version));
        assert!(t.contains(&v.codename));
    }
}
