//! The registry of open documents, and the session it writes to disk (SPEC 10).
//!
//! Phase 1 had one document and therefore no registry: the one `DocId` the
//! application had ever handed out was the document. Tabs change that in two
//! ways that are easy to get wrong separately.
//!
//! * A `DocId` no longer implies *which* file, because several tabs can hold
//!   the same path and the same path can sit in two panels. So the mapping
//!   `doc -> (path, file_id)` has to be kept somewhere the commands can read
//!   without asking the worker pool, which knows placement but not order.
//! * The arrangement itself is now state worth surviving a restart. That is
//!   what `sessions`/`session_tabs` are for, and this module is the only writer.
//!
//! Tab order lives here rather than in the frontend alone because the session
//! row is written from here: a store that had to be asked for its order on
//! every write would make closing a tab a round trip in the wrong direction.

use std::collections::HashMap;

use izul_store::files::FileId;
use izul_store::sessions::{SessionId, TabSlot};

/// One open document.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenDoc {
    pub doc: u64,
    pub path: String,
    pub file_id: i64,
    pub page_count: u32,
    /// Which of the (eventually four) split panels holds it. Phase 2 ships one.
    pub panel: u8,
    pub pinned: bool,
}

/// Every document the window currently holds, in tab order.
#[derive(Debug, Default)]
pub struct Workspace {
    /// Tab order *is* this vector's order. A separate `order` field would be a
    /// second source of truth, and the two would drift the first time a close
    /// and a reorder raced.
    order: Vec<u64>,
    docs: HashMap<u64, OpenDoc>,
    active: Option<u64>,
    session: Option<SessionId>,
}

impl Workspace {
    pub fn new(session: Option<SessionId>) -> Self {
        Workspace {
            session,
            ..Default::default()
        }
    }

    pub fn session(&self) -> Option<SessionId> {
        self.session
    }

    /// Adds a document at the end of the tab strip and makes it active.
    ///
    /// Opening is always what the user just did, so it always takes focus;
    /// restoring a session calls [`Self::set_active`] afterwards to put the
    /// focus back where it was instead.
    pub fn insert(&mut self, doc: OpenDoc) {
        let id = doc.doc;
        if self.docs.insert(id, doc).is_none() {
            self.order.push(id);
        }
        self.active = Some(id);
    }

    /// Removes a document, returning it, and hands focus to a neighbour.
    ///
    /// The neighbour is the tab that took the closed one's position, or the one
    /// before it when the last tab closed — which is what every editor does and
    /// what stops a close from leaving an empty window while tabs remain.
    pub fn remove(&mut self, doc: u64) -> Option<OpenDoc> {
        let removed = self.docs.remove(&doc)?;
        let index = self.order.iter().position(|d| *d == doc);
        if let Some(i) = index {
            self.order.remove(i);
            if self.active == Some(doc) {
                self.active = self.order.get(i).or_else(|| self.order.last()).copied();
            }
        }
        Some(removed)
    }

    /// Moves a tab to a new file — what "save as" does to the tab it saved.
    pub fn set_path(&mut self, doc: u64, path: String, file_id: i64) -> bool {
        match self.docs.get_mut(&doc) {
            Some(d) => {
                d.path = path;
                d.file_id = file_id;
                true
            }
            None => false,
        }
    }

    pub fn get(&self, doc: u64) -> Option<&OpenDoc> {
        self.docs.get(&doc)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn active(&self) -> Option<u64> {
        self.active
    }

    /// Focuses a document. A document that is not open is ignored rather than
    /// clearing the focus: a stale click must not blank the window.
    pub fn set_active(&mut self, doc: u64) -> bool {
        if self.docs.contains_key(&doc) {
            self.active = Some(doc);
            true
        } else {
            false
        }
    }

    pub fn set_pinned(&mut self, doc: u64, pinned: bool) -> bool {
        match self.docs.get_mut(&doc) {
            Some(d) => {
                d.pinned = pinned;
                true
            }
            None => false,
        }
    }

    /// Rearranges the strip to `order`.
    ///
    /// Ids that are not open are dropped and open ids the caller forgot are
    /// appended in their existing order, so a frontend working from a slightly
    /// stale list can never make a tab disappear from the registry while the
    /// document stays open in a worker.
    pub fn reorder(&mut self, order: &[u64]) {
        let mut next: Vec<u64> = Vec::with_capacity(self.order.len());
        for id in order {
            if self.docs.contains_key(id) && !next.contains(id) {
                next.push(*id);
            }
        }
        for id in &self.order {
            if !next.contains(id) {
                next.push(*id);
            }
        }
        self.order = next;
    }

    /// The tabs in order.
    pub fn tabs(&self) -> Vec<&OpenDoc> {
        self.order
            .iter()
            .filter_map(|id| self.docs.get(id))
            .collect()
    }

    pub fn ids(&self) -> Vec<u64> {
        self.order.clone()
    }

    /// The rows to write to `session_tabs`.
    ///
    /// A document with no row in `files` cannot be restored — there would be no
    /// path to reopen — so it is left out rather than written with a bogus id.
    pub fn slots(&self) -> Vec<TabSlot> {
        self.tabs()
            .iter()
            .enumerate()
            .filter(|(_, d)| d.file_id > 0)
            .map(|(i, d)| TabSlot {
                file_id: FileId(d.file_id),
                panel: d.panel,
                tab_order: i as u32,
                pinned: d.pinned,
                is_active: self.active == Some(d.doc),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: u64, path: &str) -> OpenDoc {
        OpenDoc {
            doc: id,
            path: path.to_string(),
            file_id: id as i64,
            page_count: 1,
            panel: 0,
            pinned: false,
        }
    }

    #[test]
    fn opening_appends_and_focuses() {
        let mut w = Workspace::new(None);
        w.insert(doc(1, "/a.pdf"));
        w.insert(doc(2, "/b.pdf"));
        assert_eq!(w.ids(), vec![1, 2]);
        assert_eq!(w.active(), Some(2));
    }

    /// Closing the focused tab must land on a neighbour, not on nothing: a
    /// window with three tabs open and an empty viewport is the bug this
    /// prevents.
    #[test]
    fn closing_the_active_tab_focuses_its_neighbour() {
        let mut w = Workspace::new(None);
        for i in 1..=3 {
            w.insert(doc(i, "/x.pdf"));
        }
        w.set_active(2);
        w.remove(2);
        assert_eq!(w.active(), Some(3), "tab yang menggantikan posisinya");
        w.set_active(3);
        w.remove(3);
        assert_eq!(w.active(), Some(1), "tab terakhir: mundur satu");
        w.remove(1);
        assert_eq!(w.active(), None);
        assert!(w.is_empty());
    }

    #[test]
    fn closing_an_inactive_tab_leaves_the_focus_alone() {
        let mut w = Workspace::new(None);
        for i in 1..=3 {
            w.insert(doc(i, "/x.pdf"));
        }
        w.set_active(3);
        w.remove(1);
        assert_eq!(w.active(), Some(3));
    }

    /// A frontend that reorders from a list it fetched a moment ago must not be
    /// able to drop a tab out of the registry: the document would stay open in
    /// a worker with nothing left pointing at it.
    #[test]
    fn a_stale_reorder_never_loses_a_document() {
        let mut w = Workspace::new(None);
        for i in 1..=3 {
            w.insert(doc(i, "/x.pdf"));
        }
        w.reorder(&[3, 99, 3]);
        assert_eq!(w.ids(), vec![3, 1, 2], "yang tak disebut ikut di belakang");
        assert_eq!(w.len(), 3);
    }

    #[test]
    fn the_saved_slots_carry_order_and_focus() {
        let mut w = Workspace::new(None);
        for i in 1..=3 {
            w.insert(doc(i, "/x.pdf"));
        }
        w.set_active(1);
        w.set_pinned(2, true);
        let slots = w.slots();
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0].tab_order, 0);
        assert_eq!(slots[2].tab_order, 2);
        assert!(slots[0].is_active, "yang aktif ditandai");
        assert!(!slots[1].is_active);
        assert!(slots[1].pinned);
    }

    /// A document we never managed to record in `files` has no path to come
    /// back from, so writing it as a tab would restore a row that joins to
    /// nothing.
    #[test]
    fn a_document_without_a_file_row_is_not_saved() {
        let mut w = Workspace::new(None);
        let mut d = doc(1, "/a.pdf");
        d.file_id = 0;
        w.insert(d);
        w.insert(doc(2, "/b.pdf"));
        assert_eq!(w.slots().len(), 1);
    }
}
