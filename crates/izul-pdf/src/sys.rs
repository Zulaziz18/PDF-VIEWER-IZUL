//! The handful of plain integer constants from PDFium's public headers that
//! `pdfium-render` does not re-export.
//!
//! These are values from `public/fpdfview.h`, not layout-sensitive types, so
//! reproducing them here costs nothing in safety. The struct types we need
//! (`FS_MATRIX`, `FS_RECTF`) *are* re-exported and we use those directly rather
//! than declaring our own, so no layout is assumed anywhere in this crate.
//!
//! Pinned against PDFium `chromium/7881`; see `vendor/pdfium/fetch.sh`.

use std::os::raw::{c_int, c_ulong};

/// `FPDFBitmap_BGRA` — 4 bytes per pixel, byte order blue, green, red, alpha.
pub const FPDF_BITMAP_BGRA: c_int = 4;

/// `FPDF_ANNOT` — include annotations and form fields in the render.
pub const FPDF_ANNOT: c_int = 0x01;

/// `FPDF_LCD_TEXT` — subpixel text antialiasing.
pub const FPDF_LCD_TEXT: c_int = 0x02;

/// `FPDF_RENDER_LIMITEDIMAGECACHE` — cap PDFium's internal image cache. Set for
/// documents in the quarantine path, where bounded memory matters more than speed.
pub const FPDF_RENDER_LIMITEDIMAGECACHE: c_int = 0x0200;

/// `FPDF_RENDER_NO_SMOOTHIMAGE` — skip image smoothing, for the low-resolution
/// first tier of the two-tier render.
pub const FPDF_RENDER_NO_SMOOTHIMAGE: c_int = 0x0400;

/// `FPDF_MATCHCASE` — the search must match letter case exactly.
///
/// From `public/fpdf_text.h`. `FPDFText_FindStart` takes its flags as
/// `unsigned long`, so these are typed to match rather than as `c_int`.
pub const FPDF_MATCHCASE: c_ulong = 0x0001;

/// `FPDF_MATCHWHOLEWORD` — the match must be bounded by non-word characters.
pub const FPDF_MATCHWHOLEWORD: c_ulong = 0x0002;
