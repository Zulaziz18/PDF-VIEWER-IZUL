//! Rearranging a document's pages in place (SPEC 11.3, Phase 5).
//!
//! The page map the editor keeps (`izul_model::pages`) is applied here, to the
//! working copy, when a document is saved. **In place** is the point: building
//! a fresh document and importing pages into it would be simpler, and would
//! silently drop everything that belongs to the document rather than to a
//! page — the bookmarks, the metadata, the viewer preferences, the form. A
//! user who deleted one page must not find their outline gone.
//!
//! So the pages that stay are the file's own page objects, untouched; new
//! ones (copies, pages from other files, blank pages) are added at the end;
//! the ones not wanted are deleted; and the survivors are moved into order.
//! Bookmarks follow their pages because they point at page objects, not at
//! page numbers.

use std::collections::HashSet;
use std::os::raw::{c_int, c_ulong};

use crate::engine::Document;
use crate::error::{PdfError, Result};

/// Where one page of the arranged document comes from.
#[derive(Debug, Clone, Copy)]
pub enum ArrangeSource<'a> {
    /// Page `n` of the document being arranged.
    Own(u32),
    /// Page `n` of another open document.
    From(&'a Document, u32),
    /// A new empty page, in points.
    Blank { width: f32, height: f32 },
}

/// One page of the result: its source and quarter turns added to its own
/// `/Rotate`.
#[derive(Debug, Clone, Copy)]
pub struct Arranged<'a> {
    pub source: ArrangeSource<'a>,
    pub rotation: u8,
}

fn corrupt(detail: impl Into<String>) -> PdfError {
    PdfError::Corrupt {
        detail: detail.into(),
    }
}

impl Document {
    /// Makes this document's pages exactly `pages`, in that order.
    ///
    /// `own_again` is a second handle on the same file, needed only when one
    /// of the document's own pages appears more than once: PDFium imports
    /// between two documents, and this document's first use of a page keeps
    /// the original page object (and with it, the bookmarks pointing at it).
    pub fn arrange(&self, pages: &[Arranged<'_>], own_again: Option<&Document>) -> Result<()> {
        if pages.is_empty() {
            return Err(corrupt("dokumen harus punya setidaknya satu halaman"));
        }
        let bindings = self.engine().bindings();
        let original = self.page_count();
        // Page handles are cached by index, and every index is about to move.
        self.release_all_pages();

        // Where each entry is right now, in the document as it is being
        // changed. `None` until it exists.
        let mut at: Vec<usize> = Vec::with_capacity(pages.len());
        let mut kept: HashSet<u32> = HashSet::new();
        let mut count = original as usize;

        // 1. Add what is new at the end.
        for entry in pages {
            match entry.source {
                ArrangeSource::Own(p) if !kept.contains(&p) => {
                    if p >= original {
                        return Err(PdfError::PageOutOfRange {
                            page: p,
                            page_count: original,
                        });
                    }
                    kept.insert(p);
                    at.push(p as usize);
                }
                ArrangeSource::Own(p) => {
                    let again = own_again.ok_or_else(|| {
                        corrupt("salinan kedua dokumen diperlukan untuk menggandakan halaman")
                    })?;
                    import(self, again, p, count)?;
                    at.push(count);
                    count += 1;
                }
                ArrangeSource::From(src, p) => {
                    import(self, src, p, count)?;
                    at.push(count);
                    count += 1;
                }
                ArrangeSource::Blank { width, height } => {
                    // SAFETY: `handle` is live; the index is the current page
                    // count, which is always a valid insertion point; the page
                    // handle returned is closed at once, as the header asks.
                    let page = unsafe {
                        bindings.FPDFPage_New(
                            self.handle(),
                            count as c_int,
                            f64::from(width),
                            f64::from(height),
                        )
                    };
                    if page.is_null() {
                        return Err(corrupt("halaman kosong tidak dapat dibuat"));
                    }
                    // SAFETY: `page` was just returned non-null by FPDFPage_New.
                    unsafe { bindings.FPDF_ClosePage(page) };
                    at.push(count);
                    count += 1;
                }
            }
        }

        // 2. Delete the file's own pages nobody asked for, last first so the
        //    indices still to be deleted do not move.
        for p in (0..original).rev() {
            if kept.contains(&p) {
                continue;
            }
            // SAFETY: `handle` is live and `p` is below the current count —
            // pages are only ever removed at or above it from here on.
            unsafe { bindings.FPDFPage_Delete(self.handle(), p as c_int) };
            for pos in at.iter_mut() {
                if *pos > p as usize {
                    *pos -= 1;
                }
            }
            count -= 1;
        }
        if count != pages.len() {
            return Err(corrupt(format!(
                "susunan halaman tidak cocok: {count} ada, {} diminta",
                pages.len()
            )));
        }

        // 3. Move each into place. Everything before `i` is already final, so
        //    the page for `i` is always at `i` or after it.
        for i in 0..at.len() {
            let from = at.get(i).copied().unwrap_or(i);
            if from == i {
                continue;
            }
            let index = from as c_int;
            // SAFETY: `handle` is live; the one-element index buffer outlives
            // the call; both indices are below the page count.
            let ok =
                unsafe { bindings.FPDF_MovePages(self.handle(), &index, 1 as c_ulong, i as c_int) };
            if ok == 0 {
                return Err(corrupt(format!(
                    "halaman {from} tidak dapat dipindah ke {i}"
                )));
            }
            // The pages that were at i..from each moved one along.
            for pos in at.iter_mut() {
                if *pos >= i && *pos < from {
                    *pos += 1;
                }
            }
            if let Some(slot) = at.get_mut(i) {
                *slot = i;
            }
        }

        self.reload_page_tree();

        // 4. Turn what the map turned.
        for (i, entry) in pages.iter().enumerate() {
            if entry.rotation % 4 == 0 {
                continue;
            }
            self.with_page(i as u32, |page| {
                // SAFETY: `page` is a live handle held by the page cache.
                let own = unsafe { bindings.FPDFPage_GetRotation(page) }.max(0);
                let turned = (own + c_int::from(entry.rotation)) % 4;
                // SAFETY: as above; the value is 0..=3 as the API requires.
                unsafe { bindings.FPDFPage_SetRotation(page, turned) };
                Ok(())
            })?;
        }
        // A rotation is written when the page is saved or closed; drop the
        // handles so nothing holds a stale view of the tree.
        self.release_all_pages();
        Ok(())
    }
}

/// Copies page `page` of `src` into `dest` at index `at`.
fn import(dest: &Document, src: &Document, page: u32, at: usize) -> Result<()> {
    src.check_page(page)?;
    let index = page as c_int;
    // SAFETY: both documents are live; the index buffer outlives the call.
    let ok = unsafe {
        dest.engine().bindings().FPDF_ImportPagesByIndex(
            dest.handle(),
            src.handle(),
            &index,
            1 as c_ulong,
            at as c_int,
        )
    };
    if ok == 0 {
        return Err(corrupt(format!("halaman {} tidak dapat disalin", page + 1)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_and_lock;

    /// A PDF whose pages say "Halaman N", page 1 already turned 90 degrees,
    /// with one bookmark "Bab" pointing at page 2.
    fn numbered(n: usize) -> Vec<u8> {
        let first_page = 5usize;
        let mut objects: Vec<String> = vec![
            "<</Type/Catalog/Pages 2 0 R/Outlines 3 0 R>>".into(),
            String::new(), // pages, filled below
            "<</Type/Outlines/First 4 0 R/Last 4 0 R/Count 1>>".into(),
            format!(
                "<</Title(Bab)/Parent 3 0 R/Dest[{} 0 R/Fit]>>",
                first_page + 2 * 2
            ),
        ];
        let font = first_page + 2 * n;
        let mut kids = Vec::new();
        for i in 0..n {
            let page = first_page + 2 * i;
            kids.push(format!("{page} 0 R"));
            let rotate = if i == 1 { "/Rotate 90" } else { "" };
            objects.push(format!(
                "<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 400]{rotate}/Resources<</Font<</F1 {font} 0 R>>>>/Contents {} 0 R>>",
                page + 1
            ));
            let text = format!("BT /F1 24 Tf 40 300 Td (Halaman {i}) Tj ET");
            objects.push(format!(
                "<</Length {}>>stream\n{text}\nendstream",
                text.len()
            ));
        }
        objects.push("<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".into());
        objects[1] = format!("<</Type/Pages/Kids[{}]/Count {n}>>", kids.join(" "));

        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for off in offsets {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    fn texts(doc: &Document) -> Vec<String> {
        (0..doc.page_count())
            .map(|p| doc.page_text(p).unwrap().trim().to_string())
            .collect()
    }

    fn rotations(doc: &Document) -> Vec<i32> {
        let b = doc.engine().bindings();
        (0..doc.page_count())
            .map(|p| {
                doc.with_page(p, |h| {
                    // SAFETY: a live page handle from the cache.
                    Ok(unsafe { b.FPDFPage_GetRotation(h) })
                })
                .unwrap()
            })
            .collect()
    }

    fn own(p: u32) -> Arranged<'static> {
        Arranged {
            source: ArrangeSource::Own(p),
            rotation: 0,
        }
    }

    #[test]
    fn reorders_deletes_duplicates_and_adds_blank_and_foreign_pages() {
        let (engine, _pdfium) = engine_and_lock!();
        let bytes = numbered(4);
        let doc = engine
            .open_bytes(bytes.clone(), None::<&str>, None)
            .unwrap();
        doc.set_strip_izul(false);
        let again = engine.open_bytes(bytes, None::<&str>, None).unwrap();
        let other = engine.open_bytes(numbered(2), None::<&str>, None).unwrap();

        // [2, blank, 0, 0 again, other's 1 turned once]; pages 1 and 3 go.
        let plan = [
            own(2),
            Arranged {
                source: ArrangeSource::Blank {
                    width: 200.0,
                    height: 100.0,
                },
                rotation: 0,
            },
            own(0),
            own(0),
            Arranged {
                source: ArrangeSource::From(&other, 1),
                rotation: 1,
            },
        ];
        doc.arrange(&plan, Some(&again)).unwrap();
        assert_eq!(doc.page_count(), 5);

        // What a reader sees, after a real save and a real reopen.
        let saved = doc.save_to_vec().unwrap();
        let back = engine.open_bytes(saved, None::<&str>, None).unwrap();
        assert_eq!(
            texts(&back),
            vec!["Halaman 2", "", "Halaman 0", "Halaman 0", "Halaman 1"]
        );
        // The foreign page 1 carried its own /Rotate 90; one more turn is 180.
        assert_eq!(rotations(&back), vec![0, 0, 0, 0, 2]);
        let blank = back.page_size_fast(1).unwrap();
        assert_eq!((blank.width, blank.height), (200.0, 100.0));
    }

    /// The reason this is done in place: a bookmark points at a page object,
    /// and moving or deleting other pages must leave it pointing at the same
    /// page — now at a new number.
    #[test]
    fn bookmarks_follow_their_page() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine.open_bytes(numbered(4), None::<&str>, None).unwrap();
        doc.set_strip_izul(false);
        assert_eq!(doc.outline().unwrap()[0].page, Some(2));
        // Delete page 0, move page 2 to the front.
        doc.arrange(&[own(2), own(1), own(3)], None).unwrap();
        let back = engine
            .open_bytes(doc.save_to_vec().unwrap(), None::<&str>, None)
            .unwrap();
        let outline = back.outline().unwrap();
        assert_eq!(outline.len(), 1, "the outline survived");
        assert_eq!(outline[0].title, "Bab");
        assert_eq!(
            outline[0].page,
            Some(0),
            "still the page that says Halaman 2"
        );
        assert_eq!(texts(&back)[0], "Halaman 2");
    }

    #[test]
    fn turning_adds_to_the_pages_own_rotation() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine.open_bytes(numbered(2), None::<&str>, None).unwrap();
        doc.set_strip_izul(false);
        doc.arrange(
            &[
                Arranged {
                    source: ArrangeSource::Own(0),
                    rotation: 3,
                },
                Arranged {
                    source: ArrangeSource::Own(1),
                    rotation: 3,
                },
            ],
            None,
        )
        .unwrap();
        let back = engine
            .open_bytes(doc.save_to_vec().unwrap(), None::<&str>, None)
            .unwrap();
        // Page 1 was /Rotate 90 already: 1 + 3 is a full turn.
        assert_eq!(rotations(&back), vec![3, 0]);
    }

    #[test]
    fn a_duplicate_without_a_second_handle_is_refused_not_guessed() {
        let (engine, _pdfium) = engine_and_lock!();
        let doc = engine.open_bytes(numbered(2), None::<&str>, None).unwrap();
        assert!(doc.arrange(&[own(0), own(0)], None).is_err());
        assert!(doc.arrange(&[], None).is_err());
        assert!(doc.arrange(&[own(7)], None).is_err());
    }
}
