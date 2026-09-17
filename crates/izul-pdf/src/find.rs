//! Searching inside one page's text (SPEC 11.1's first tier).
//!
//! The FTS5 index in `izul-store` answers "which documents contain this", fast,
//! across the whole library. It cannot answer "where on the page is it", because
//! it stores text and not geometry. This module is the other half: PDFium's own
//! search, which knows the character positions and can therefore hand back the
//! rectangles to draw the highlight on.

use std::os::raw::{c_double, c_int, c_ulong};

use pdfium_render::prelude::{FPDF_SCHHANDLE, FPDF_WIDESTRING};

use crate::engine::{Document, PageGeometry};
use crate::error::Result;
use crate::geom::{PdfRectF, RotationQuarter};
use crate::sys::{FPDF_MATCHCASE, FPDF_MATCHWHOLEWORD};
use crate::text::TextPage;

/// How to match (SPEC 11.1).
///
/// The default is the forgiving one — case-insensitive, substrings allowed, no
/// cap — because that is what a reader typing into a search box expects before
/// touching any of the toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FindOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// Most matches to return from one page. `0` means no limit.
    ///
    /// A limit is worth having: a single-character search across a dense page
    /// can match thousands of times, and neither the wire nor the results panel
    /// gains anything from the tail of that list.
    pub max_hits: u32,
}

impl FindOptions {
    fn flags(self) -> c_ulong {
        let mut flags: c_ulong = 0;
        if self.case_sensitive {
            flags |= FPDF_MATCHCASE;
        }
        if self.whole_word {
            flags |= FPDF_MATCHWHOLEWORD;
        }
        flags
    }
}

/// One match on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// Index of the first matching character in the page's extracted text, so
    /// the caller can line a hit up with the text it already has.
    pub char_index: u32,
    pub char_count: u32,
    /// Boxes to highlight, in **display space** at the requested rotation — the
    /// same space the tiles are drawn in.
    ///
    /// More than one when the match runs across a line break, which is why this
    /// is a list and not a single rectangle: drawing one box around a match that
    /// wrapped would paint over the whole block between its two halves.
    pub rects: Vec<PdfRectF>,
}

/// Owns one `FPDF_SCHHANDLE`, closing it on drop.
///
/// A guard rather than a plain call at the end: every early return in the search
/// loop — a corrupt page, a hit limit, an error reading a rectangle — must still
/// release the context, and a `Drop` impl is the only version of that which
/// cannot be forgotten when the loop grows another branch.
struct Search<'d> {
    raw: FPDF_SCHHANDLE,
    doc: &'d Document,
}

impl Drop for Search<'_> {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: `raw` is a non-null handle from `FPDFText_FindStart`,
            // closed exactly once, here.
            unsafe { self.doc.engine().bindings().FPDFText_FindClose(self.raw) }
        }
    }
}

impl Document {
    /// Finds `needle` on one page, with the boxes to highlight it.
    ///
    /// An empty needle returns no matches rather than asking PDFium what an
    /// empty search means: the answer differs between builds, and a search box
    /// that the user has just cleared must simply stop highlighting.
    pub fn find_on_page(
        &self,
        page: u32,
        needle: &str,
        opts: FindOptions,
        rotation: RotationQuarter,
    ) -> Result<Vec<Match>> {
        self.check_page(page)?;
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let engine = self.engine();
        crate::ffi_guard::guard("find_on_page", || {
            self.with_page(page, |p| {
                let geometry = PageGeometry::of_page(engine, p);
                let tp = TextPage::load(self, p)?;
                let char_count = tp.count();
                if char_count <= 0 {
                    return Ok(Vec::new());
                }

                // UTF-16 with an explicit NUL, kept alive for the whole search.
                // PDFium copies the needle into its own context, but holding the
                // buffer until `FindClose` costs nothing and removes the question
                // entirely. Both targets are little-endian, which is what
                // `FPDF_WIDESTRING` wants.
                let needle_utf16: Vec<u16> =
                    needle.encode_utf16().chain(std::iter::once(0)).collect();

                let bindings = engine.bindings();
                // SAFETY: `tp.raw()` is a live text page owned by `tp`, and
                // `needle_utf16` is NUL-terminated and outlives the handle.
                let raw = unsafe {
                    bindings.FPDFText_FindStart(
                        tp.raw(),
                        needle_utf16.as_ptr() as FPDF_WIDESTRING,
                        opts.flags(),
                        0,
                    )
                };
                let search = Search { raw, doc: self };
                if search.raw.is_null() {
                    return Ok(Vec::new());
                }

                let mut out = Vec::new();
                // A match must start at a distinct character index, so the page's
                // character count bounds the number of iterations. The loop is
                // written against that bound rather than trusting `FindNext` to
                // always advance: a malformed page must not be able to hang a
                // worker, and the supervisor would kill it six seconds later.
                for _ in 0..=char_count {
                    // SAFETY: `search.raw` is a live, non-null search context.
                    if unsafe { bindings.FPDFText_FindNext(search.raw) } == 0 {
                        break;
                    }
                    // SAFETY: as above.
                    let index = unsafe { bindings.FPDFText_GetSchResultIndex(search.raw) };
                    // SAFETY: as above.
                    let count = unsafe { bindings.FPDFText_GetSchCount(search.raw) };
                    if index < 0 || count <= 0 {
                        continue;
                    }
                    out.push(Match {
                        char_index: index as u32,
                        char_count: count as u32,
                        rects: tp.rects_of(index, count, &geometry, rotation),
                    });
                    if opts.max_hits != 0 && out.len() as u32 >= opts.max_hits {
                        break;
                    }
                }
                Ok(out)
            })
        })
    }
}

impl TextPage<'_> {
    /// The highlight boxes covering `count` characters from `start`, in display
    /// space.
    fn rects_of(
        &self,
        start: c_int,
        count: c_int,
        geometry: &PageGeometry,
        rotation: RotationQuarter,
    ) -> Vec<PdfRectF> {
        let bindings = self.bindings();
        // SAFETY: `raw()` is a live text page; `start` and `count` came from a
        // search result on this same page.
        let n = unsafe { bindings.FPDFText_CountRects(self.raw(), start, count) };
        let mut rects = Vec::with_capacity(n.max(0) as usize);
        for i in 0..n {
            let mut left: c_double = 0.0;
            let mut top: c_double = 0.0;
            let mut right: c_double = 0.0;
            let mut bottom: c_double = 0.0;
            // SAFETY: `i` is in `0..n` from `FPDFText_CountRects`, and the four
            // out-parameters are live and correctly typed.
            let ok = unsafe {
                bindings.FPDFText_GetRect(
                    self.raw(),
                    i,
                    &mut left,
                    &mut top,
                    &mut right,
                    &mut bottom,
                )
            };
            if ok == 0 {
                continue;
            }
            // PDFium reports these in the page's own user space with `top` as
            // the larger y, the same convention `PdfRectF` keeps.
            let rect = PdfRectF::new(left as f32, bottom as f32, right as f32, top as f32);
            rects.push(geometry.to_display(rect, rotation));
        }
        rects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{engine_and_lock, viewer_fixture};

    /// The fixture opened, with the PDFium lock held for as long as the document
    /// lives. The guard is returned rather than dropped here: `cargo test` runs
    /// these on a thread pool, and PDFium tolerates exactly one at a time.
    macro_rules! doc_or_skip {
        () => {{
            let (engine, pdfium) = engine_and_lock!();
            let path = viewer_fixture!();
            match engine.open(&path, None) {
                Ok(d) => (d, pdfium),
                Err(e) => panic!("buka fixture: {e}"),
            }
        }};
    }

    fn opts() -> FindOptions {
        FindOptions::default()
    }

    #[test]
    fn a_word_on_the_page_is_found_with_a_box_to_highlight() {
        let (doc, _pdfium) = doc_or_skip!();
        let hits = doc
            .find_on_page(0, "Bagian", opts(), RotationQuarter::None)
            .expect("cari");
        assert!(!hits.is_empty(), "judul halaman memuat kata itu");
        let first = hits.first().expect("ada");
        assert_eq!(first.char_count, 6, "sepanjang kata yang dicari");
        assert!(
            !first.rects.is_empty(),
            "sebuah kecocokan tanpa kotak tidak bisa disorot"
        );
        for r in &first.rects {
            assert!(
                r.right > r.left && r.top > r.bottom,
                "kotak sorot harus punya luas: {r:?}"
            );
        }
    }

    #[test]
    fn the_reported_index_points_at_the_match_in_the_page_text() {
        // The index is what lets the UI line a hit up with the text layer it
        // already has, so it has to agree with `page_text`, not merely be
        // plausible.
        let (doc, _pdfium) = doc_or_skip!();
        let text = doc.page_text(0).expect("teks");
        let hits = doc
            .find_on_page(0, "halaman", opts(), RotationQuarter::None)
            .expect("cari");
        let hit = hits.first().expect("ada kecocokan");

        let utf16: Vec<u16> = text.encode_utf16().collect();
        let start = hit.char_index as usize;
        let end = start + hit.char_count as usize;
        let slice = utf16.get(start..end).expect("rentang di dalam teks");
        assert_eq!(
            String::from_utf16_lossy(slice).to_lowercase(),
            "halaman",
            "indeks harus menunjuk ke kata yang sama di teks halaman"
        );
    }

    #[test]
    fn case_sensitivity_is_honoured() {
        let (doc, _pdfium) = doc_or_skip!();
        let insensitive = doc
            .find_on_page(0, "BAGIAN", opts(), RotationQuarter::None)
            .expect("cari");
        assert!(
            !insensitive.is_empty(),
            "bawaannya abaikan besar-kecil huruf"
        );

        let sensitive = doc
            .find_on_page(
                0,
                "BAGIAN",
                FindOptions {
                    case_sensitive: true,
                    ..opts()
                },
                RotationQuarter::None,
            )
            .expect("cari");
        assert!(
            sensitive.is_empty(),
            "dokumen menulis \"Bagian\", bukan \"BAGIAN\""
        );
    }

    #[test]
    fn whole_word_rejects_a_prefix_that_is_part_of_a_longer_word() {
        let (doc, _pdfium) = doc_or_skip!();
        let partial = doc
            .find_on_page(0, "Bagia", opts(), RotationQuarter::None)
            .expect("cari");
        assert!(!partial.is_empty(), "tanpa whole-word, awalan cocok");

        let whole = doc
            .find_on_page(
                0,
                "Bagia",
                FindOptions {
                    whole_word: true,
                    ..opts()
                },
                RotationQuarter::None,
            )
            .expect("cari");
        assert!(
            whole.is_empty(),
            "\"Bagia\" bukan kata utuh di dalam \"Bagian\""
        );
    }

    #[test]
    fn a_word_that_is_not_there_returns_nothing_rather_than_failing() {
        let (doc, _pdfium) = doc_or_skip!();
        let hits = doc
            .find_on_page(
                0,
                "zzzxxqqwv-tidak-mungkin-ada",
                opts(),
                RotationQuarter::None,
            )
            .expect("cari");
        assert!(hits.is_empty());
    }

    #[test]
    fn an_empty_needle_finds_nothing() {
        // A cleared search box must stop highlighting, and PDFium's own answer
        // to an empty needle differs between builds.
        let (doc, _pdfium) = doc_or_skip!();
        assert!(doc
            .find_on_page(0, "", opts(), RotationQuarter::None)
            .expect("cari")
            .is_empty());
    }

    #[test]
    fn the_hit_limit_caps_the_result_and_zero_means_no_limit() {
        let (doc, _pdfium) = doc_or_skip!();
        let all = doc
            .find_on_page(0, "a", opts(), RotationQuarter::None)
            .expect("cari");
        assert!(
            all.len() > 2,
            "halaman teks padat pasti memuat banyak huruf a: {}",
            all.len()
        );

        let capped = doc
            .find_on_page(
                0,
                "a",
                FindOptions {
                    max_hits: 2,
                    ..opts()
                },
                RotationQuarter::None,
            )
            .expect("cari");
        assert_eq!(capped.len(), 2);
    }

    #[test]
    fn a_page_out_of_range_is_an_error_not_a_silent_empty_list() {
        let (doc, _pdfium) = doc_or_skip!();
        assert!(doc
            .find_on_page(9_999, "Bagian", opts(), RotationQuarter::None)
            .is_err());
    }

    #[test]
    fn boxes_come_back_in_display_space_for_a_page_with_its_own_rotate() {
        // Page 2 of the fixture carries `/Rotate 90`. Phase 1 lost two sessions
        // to coordinate spaces being assumed rather than measured, so this
        // checks the boxes against the page's display size rather than trusting
        // that `to_display` was called.
        let (doc, _pdfium) = doc_or_skip!();
        let size = doc
            .page_display_size(2, RotationQuarter::None)
            .expect("ukuran");
        let hits = doc
            .find_on_page(2, "Bagian", opts(), RotationQuarter::None)
            .expect("cari");
        let hit = hits.first().expect("ada kecocokan");
        for r in &hit.rects {
            assert!(
                r.left >= -1.0
                    && r.bottom >= -1.0
                    && r.right <= size.width + 1.0
                    && r.top <= size.height + 1.0,
                "kotak {r:?} keluar dari halaman {size:?} — ruang koordinatnya salah"
            );
        }
    }

    #[test]
    fn rotating_the_view_moves_the_boxes_but_keeps_the_same_matches() {
        let (doc, _pdfium) = doc_or_skip!();
        let upright = doc
            .find_on_page(0, "Bagian", opts(), RotationQuarter::None)
            .expect("cari");
        let turned = doc
            .find_on_page(0, "Bagian", opts(), RotationQuarter::Cw90)
            .expect("cari");

        assert_eq!(
            upright.len(),
            turned.len(),
            "memutar tampilan tidak mengubah isi teks"
        );
        assert_eq!(
            upright.first().map(|h| h.char_index),
            turned.first().map(|h| h.char_index)
        );
        let a = upright.first().and_then(|h| h.rects.first());
        let b = turned.first().and_then(|h| h.rects.first());
        assert_ne!(a, b, "tapi kotaknya harus pindah mengikuti rotasi");
    }
}
