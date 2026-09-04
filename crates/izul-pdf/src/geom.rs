//! Geometry re-exported from `izul-model`, plus the page-level types that only
//! make sense next to PDFium.
//!
//! The coordinate types themselves live in `izul-model` so that the model, this
//! crate, and the IPC layer all speak about the same `PdfRectF` rather than
//! three structurally identical ones that need converting at every boundary.

pub use izul_model::geom::{Matrix, PdfPointF, PdfRectF, RotationQuarter};

use serde::{Deserialize, Serialize};

/// Page dimensions in points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

impl PageSize {
    /// Scale factor that fits this page into `px` pixels of width.
    pub fn scale_for_width(&self, px: f32) -> f32 {
        if self.width > 0.0 {
            px / self.width
        } else {
            1.0
        }
    }

    /// Scale factor that fits the whole page inside `w` by `h` pixels.
    pub fn scale_to_fit(&self, w: f32, h: f32) -> f32 {
        if self.width <= 0.0 || self.height <= 0.0 {
            return 1.0;
        }
        (w / self.width).min(h / self.height)
    }

    pub fn rotated(&self, r: RotationQuarter) -> PageSize {
        if r.swaps_axes() {
            PageSize {
                width: self.height,
                height: self.width,
            }
        } else {
            *self
        }
    }
}
