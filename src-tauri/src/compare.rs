//! Visual differences between two pages, for compare mode (Phase 5, SPEC 10:
//! "perbedaan teks dan visual disorot").
//!
//! Text differences are found in the frontend from the text layers; this is
//! the other half, for what text cannot see — a changed figure, a moved
//! stamp, a scanned page. Both pages are rendered at preview resolution by
//! the ordinary tile pipeline (so a compare costs two cache lookups when the
//! pages are already on screen), laid over the same grid of cells in
//! *normalised* page space, and a cell is marked when its brightness or its
//! amount of ink differs. Normalised space is what lets two pages of
//! slightly different sizes or resolutions be compared at all.
//!
//! Deliberately coarse. A pixel diff marks every antialiasing difference
//! between two renderings of the same text; a cell diff marks the paragraph
//! that changed.

use serde::Serialize;

use crate::commands::AppState;
use crate::render::{Priority, TileKey, TileKind};

/// A BGRA bitmap as the render cache holds it.
#[derive(Debug, Clone, Copy)]
pub struct Bitmap<'a> {
    pub bytes: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

/// A marked region, as fractions of the page: x rightwards, y downwards.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct NormBox {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

/// Brightness and ink of one cell.
#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    mean: f32,
    ink: f32,
}

fn cells(bmp: Bitmap<'_>, cols: usize, rows: usize) -> Vec<Cell> {
    let mut out = vec![Cell::default(); cols * rows];
    if bmp.width == 0 || bmp.height == 0 {
        return out;
    }
    for (index, cell) in out.iter_mut().enumerate() {
        let (cx, cy) = (index % cols, index / cols);
        let x0 = (cx as u64 * u64::from(bmp.width) / cols as u64) as u32;
        let x1 =
            (((cx + 1) as u64 * u64::from(bmp.width)) / cols as u64).max(u64::from(x0) + 1) as u32;
        let y0 = (cy as u64 * u64::from(bmp.height) / rows as u64) as u32;
        let y1 =
            (((cy + 1) as u64 * u64::from(bmp.height)) / rows as u64).max(u64::from(y0) + 1) as u32;
        let (mut sum, mut dark, mut n) = (0f32, 0f32, 0f32);
        for y in y0..y1.min(bmp.height) {
            let row = (y * bmp.stride) as usize;
            for x in x0..x1.min(bmp.width) {
                let at = row + (x * 4) as usize;
                let Some(&[blue, green, red]) = bmp.bytes.get(at..at + 3) else {
                    continue;
                };
                // BGRA; Rec. 601 luma is plenty for "did this change".
                let l = 0.114 * f32::from(blue) + 0.587 * f32::from(green) + 0.299 * f32::from(red);
                sum += l;
                if l < 160.0 {
                    dark += 1.0;
                }
                n += 1.0;
            }
        }
        if n > 0.0 {
            *cell = Cell {
                mean: sum / n,
                ink: dark / n,
            };
        }
    }
    out
}

/// The regions where `a` and `b` differ, and the fraction of cells that do.
///
/// A cell differs when its mean brightness moves by more than `MEAN` levels
/// or its share of dark pixels by more than `INK`; each alone misses a case
/// the other catches — a thin line redrawn elsewhere in the cell keeps the
/// ink and moves nothing, a light grey fill keeps the ink and moves the mean.
pub fn diff(a: Bitmap<'_>, b: Bitmap<'_>, cols: usize, rows: usize) -> (Vec<NormBox>, f32) {
    const MEAN: f32 = 4.0;
    const INK: f32 = 0.02;
    let ca = cells(a, cols, rows);
    let cb = cells(b, cols, rows);
    let marked: Vec<bool> = ca
        .iter()
        .zip(&cb)
        .map(|(p, q)| (p.mean - q.mean).abs() > MEAN || (p.ink - q.ink).abs() > INK)
        .collect();
    let count = marked.iter().filter(|m| **m).count();
    let fraction = count as f32 / (cols * rows).max(1) as f32;

    // Runs per row, then runs with the same span on consecutive rows merge.
    let mut open: Vec<(usize, usize, usize, usize)> = Vec::new(); // (x0, x1, y0, y1) in cells
    let mut done: Vec<(usize, usize, usize, usize)> = Vec::new();
    for y in 0..rows {
        let mut runs = Vec::new();
        let mut x = 0;
        while x < cols {
            if marked.get(y * cols + x).copied().unwrap_or(false) {
                let start = x;
                while x < cols && marked.get(y * cols + x).copied().unwrap_or(false) {
                    x += 1;
                }
                runs.push((start, x));
            } else {
                x += 1;
            }
        }
        let mut next = Vec::new();
        for (x0, x1) in runs {
            if let Some(i) = open.iter().position(|r| r.0 == x0 && r.1 == x1 && r.3 == y) {
                let mut r = open.remove(i);
                r.3 = y + 1;
                next.push(r);
            } else {
                next.push((x0, x1, y, y + 1));
            }
        }
        done.append(&mut open);
        open = next;
    }
    done.append(&mut open);
    let boxes = done
        .into_iter()
        .map(|(x0, x1, y0, y1)| NormBox {
            x0: x0 as f32 / cols as f32,
            y0: y0 as f32 / rows as f32,
            x1: x1 as f32 / cols as f32,
            y1: y1 as f32 / rows as f32,
        })
        .collect();
    (boxes, fraction)
}

#[derive(Debug, Clone, Serialize)]
pub struct VisualDiff {
    pub boxes: Vec<NormBox>,
    /// Share of the page that differs, 0 to 1.
    pub fraction: f32,
}

/// Longest edge of the renders compared, in pixels.
const EDGE: u32 = 512;
/// Cells across; down follows the page's shape.
const COLS: usize = 48;

/// Compares page `a_page` of `a_doc` with page `b_page` of `b_doc`, each at
/// its display rotation, so the boxes land in display space on both.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn compare_visual(
    state: tauri::State<'_, AppState>,
    a_doc: u64,
    a_page: u32,
    a_rotation: u8,
    b_doc: u64,
    b_page: u32,
    b_rotation: u8,
) -> Result<VisualDiff, String> {
    let key = |doc, page, rotation| TileKey {
        doc,
        page,
        rotation,
        ppp_milli: EDGE,
        col: 0,
        row: 0,
        kind: TileKind::Preview,
        invert: false,
    };
    let (ta, tb) = tokio::join!(
        state
            .render
            .fetch(key(a_doc, a_page, a_rotation), 0, Priority::Visible),
        state
            .render
            .fetch(key(b_doc, b_page, b_rotation), 0, Priority::Visible),
    );
    let ta = ta.map_err(|e| e.to_string())?;
    let tb = tb.map_err(|e| e.to_string())?;
    let rows = ((COLS as f32) * ta.height as f32 / ta.width.max(1) as f32)
        .round()
        .max(1.0) as usize;
    let (boxes, fraction) = diff(
        Bitmap {
            bytes: &ta.bytes,
            width: ta.width,
            height: ta.height,
            stride: ta.stride,
        },
        Bitmap {
            bytes: &tb.bytes,
            width: tb.width,
            height: tb.height,
            stride: tb.stride,
        },
        COLS,
        rows,
    );
    Ok(VisualDiff { boxes, fraction })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(w: u32, h: u32, ink: &[(u32, u32, u32, u32)]) -> Vec<u8> {
        let mut px = vec![255u8; (w * h * 4) as usize];
        for &(x0, y0, x1, y1) in ink {
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = ((y * w + x) * 4) as usize;
                    px[at..at + 3].copy_from_slice(&[0, 0, 0]);
                }
            }
        }
        px
    }

    fn bmp(bytes: &[u8], w: u32, h: u32) -> Bitmap<'_> {
        Bitmap {
            bytes,
            width: w,
            height: h,
            stride: w * 4,
        }
    }

    #[test]
    fn the_same_page_has_no_differences() {
        let a = page(100, 140, &[(10, 10, 90, 20)]);
        let (boxes, fraction) = diff(bmp(&a, 100, 140), bmp(&a, 100, 140), 10, 14);
        assert!(boxes.is_empty());
        assert_eq!(fraction, 0.0);
    }

    #[test]
    fn a_changed_block_is_one_box_where_it_is() {
        let a = page(100, 100, &[(10, 10, 90, 20)]);
        let b = page(100, 100, &[(10, 10, 90, 20), (50, 60, 70, 80)]);
        let (boxes, _) = diff(bmp(&a, 100, 100), bmp(&b, 100, 100), 10, 10);
        assert_eq!(
            boxes,
            vec![NormBox {
                x0: 0.5,
                y0: 0.6,
                x1: 0.7,
                y1: 0.8
            }]
        );
    }

    /// Normalised space: the same page rendered at two resolutions — which is
    /// what two different page sizes look like at one preview edge — is the
    /// same page.
    #[test]
    fn resolution_does_not_count_as_a_difference() {
        let a = page(100, 100, &[(20, 20, 60, 40)]);
        let b = page(200, 200, &[(40, 40, 120, 80)]);
        let (boxes, _) = diff(bmp(&a, 100, 100), bmp(&b, 200, 200), 10, 10);
        assert!(boxes.is_empty(), "{boxes:?}");
    }

    #[test]
    fn runs_merge_down_the_page() {
        let a = page(100, 100, &[]);
        let b = page(100, 100, &[(0, 0, 30, 30)]);
        let (boxes, fraction) = diff(bmp(&a, 100, 100), bmp(&b, 100, 100), 10, 10);
        assert_eq!(
            boxes,
            vec![NormBox {
                x0: 0.0,
                y0: 0.0,
                x1: 0.3,
                y1: 0.3
            }]
        );
        assert!((fraction - 0.09).abs() < 1e-6);
    }
}
