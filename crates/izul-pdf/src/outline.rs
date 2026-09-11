//! Document outline — the PDF's own bookmark tree (SPEC 11.1's sidebar).
//!
//! Two things make this less trivial than walking a tree:
//!
//! * a bookmark's destination can be direct (`/Dest`) or hidden behind a GoTo
//!   action, and a real-world file uses both spellings;
//! * the "tree" is a linked structure inside an untrusted file, so it can be
//!   cyclic. A naive walk hangs the worker, which the supervisor then kills as
//!   unresponsive — a corrupt file must not cost the user a tab.
//!
//! Both are handled here rather than left to the caller.

use std::collections::HashSet;
use std::os::raw::{c_ulong, c_void};

use pdfium_render::prelude::{FPDF_BOOKMARK, FPDF_BOOL, FPDF_DEST, FS_FLOAT};

use crate::engine::Document;
use crate::error::Result;
use crate::ffi_guard::guard;

/// `PDFACTION_GOTO` from `public/fpdf_doc.h`: a jump within this document.
const PDFACTION_GOTO: c_ulong = 1;

/// How deep a bookmark tree may nest before we stop descending.
///
/// Deeper than any hand-made outline and shallow enough that a malicious file
/// cannot drive us into a stack overflow.
const MAX_DEPTH: u32 = 32;

/// How many bookmarks we will read from one document.
///
/// A bound, not a judgement about what is reasonable: an outline this large is
/// already unusable as a sidebar, and the cap is what stops a generated file
/// with a million entries from freezing the worker.
const MAX_NODES: usize = 20_000;

/// One entry in the outline.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineNode {
    pub title: String,
    /// Target page, when the destination resolves to one in this document.
    pub page: Option<u32>,
    /// Target y in PDF points, when the destination names one. The viewport
    /// scrolls to it instead of to the page top.
    pub y: Option<f32>,
    pub children: Vec<OutlineNode>,
}

impl Document {
    /// The document's bookmark tree, empty when it has none.
    pub fn outline(&self) -> Result<Vec<OutlineNode>> {
        guard("outline", || {
            let mut seen = HashSet::new();
            let mut budget = MAX_NODES;
            Ok(self.siblings(std::ptr::null_mut(), 0, &mut seen, &mut budget))
        })
    }

    /// Collects the children of `parent` (null meaning the document root).
    fn siblings(
        &self,
        parent: FPDF_BOOKMARK,
        depth: u32,
        seen: &mut HashSet<usize>,
        budget: &mut usize,
    ) -> Vec<OutlineNode> {
        if depth >= MAX_DEPTH || *budget == 0 {
            return Vec::new();
        }
        let bindings = self.engine().bindings();
        let mut out = Vec::new();
        // SAFETY: `handle()` is a live document. A null `parent` asks PDFium for
        // the first top-level bookmark, which is the documented way to start.
        let mut node = unsafe { bindings.FPDFBookmark_GetFirstChild(self.handle(), parent) };
        while !node.is_null() {
            // A cyclic outline is a real shape in damaged files. Recording every
            // bookmark we have entered turns an infinite walk into a truncated
            // one.
            if !seen.insert(node as usize) || *budget == 0 {
                break;
            }
            *budget -= 1;
            let (page, y) = self.destination(node);
            out.push(OutlineNode {
                title: self.bookmark_title(node),
                page,
                y,
                children: self.siblings(node, depth + 1, seen, budget),
            });
            // SAFETY: `node` is a live bookmark handle from this document.
            node = unsafe { bindings.FPDFBookmark_GetNextSibling(self.handle(), node) };
        }
        out
    }

    /// A bookmark's title, decoded from PDFium's UTF-16LE buffer.
    fn bookmark_title(&self, node: FPDF_BOOKMARK) -> String {
        let bindings = self.engine().bindings();
        // SAFETY: passing a null buffer asks for the required length in bytes,
        // which is the documented two-call protocol.
        let bytes =
            unsafe { bindings.FPDFBookmark_GetTitle(node, std::ptr::null_mut(), 0) } as usize;
        // Below 2 bytes there is not even a terminator, so there is no title.
        if !(2..=(1 << 20)).contains(&bytes) {
            return String::new();
        }
        let mut buf = vec![0u16; bytes.div_ceil(2)];
        // SAFETY: `buf` holds at least `bytes` bytes, which is exactly what the
        // length query above asked for.
        let written = unsafe {
            bindings.FPDFBookmark_GetTitle(node, buf.as_mut_ptr() as *mut c_void, bytes as c_ulong)
        } as usize;
        // The buffer ends with a UTF-16 NUL that is not part of the title.
        let units = (written / 2).saturating_sub(1).min(buf.len());
        String::from_utf16_lossy(buf.get(..units).unwrap_or(&[]))
            .trim()
            .to_string()
    }

    /// Resolves a bookmark to a page and, when the destination names one, a y
    /// position in PDF points.
    fn destination(&self, node: FPDF_BOOKMARK) -> (Option<u32>, Option<f32>) {
        let bindings = self.engine().bindings();
        // SAFETY: `node` is a live bookmark of this live document.
        let mut dest = unsafe { bindings.FPDFBookmark_GetDest(self.handle(), node) };
        if dest.is_null() {
            // No direct destination: the bookmark may still carry a GoTo action,
            // which is how many producers write internal links.
            // SAFETY: as above.
            let action = unsafe { bindings.FPDFBookmark_GetAction(node) };
            if action.is_null() {
                return (None, None);
            }
            // SAFETY: `action` is a live action handle checked non-null above.
            if unsafe { bindings.FPDFAction_GetType(action) } != PDFACTION_GOTO {
                // Launch, remote-goto and URI actions all point outside this
                // document. SPEC 2 rules out following any of them.
                return (None, None);
            }
            // SAFETY: as above, and the action was confirmed to be a GoTo.
            dest = unsafe { bindings.FPDFAction_GetDest(self.handle(), action) };
            if dest.is_null() {
                return (None, None);
            }
        }
        (self.dest_page(dest), self.dest_y(dest))
    }

    fn dest_page(&self, dest: FPDF_DEST) -> Option<u32> {
        // SAFETY: `dest` is a live destination handle of this document.
        let index = unsafe {
            self.engine()
                .bindings()
                .FPDFDest_GetDestPageIndex(self.handle(), dest)
        };
        let page = u32::try_from(index).ok()?;
        (page < self.page_count()).then_some(page)
    }

    fn dest_y(&self, dest: FPDF_DEST) -> Option<f32> {
        let bindings = self.engine().bindings();
        let (mut has_x, mut has_y, mut has_zoom): (FPDF_BOOL, FPDF_BOOL, FPDF_BOOL) = (0, 0, 0);
        let (mut x, mut y, mut zoom): (FS_FLOAT, FS_FLOAT, FS_FLOAT) = (0.0, 0.0, 0.0);
        // SAFETY: `dest` is live and every out-parameter is a live local of the
        // type PDFium declares.
        let ok = unsafe {
            bindings.FPDFDest_GetLocationInPage(
                dest,
                &mut has_x,
                &mut has_y,
                &mut has_zoom,
                &mut x,
                &mut y,
                &mut zoom,
            )
        };
        if ok == 0 || has_y == 0 || !y.is_finite() {
            return None;
        }
        Some(y)
    }
}
