//! The page map: which pages a document has, in which order, from where, and
//! turned how far (SPEC 11.3, Phase 5).
//!
//! Page operations are edits like any other — they undo with Ctrl+Z, they are
//! lost if the user says "don't save", they are written when the user saves —
//! so they live next to the annotations, as data, on the same undo stack.
//! Nothing touches the PDF until the file is written: the viewer draws the map,
//! and the worker applies it to the working copy at save time.
//!
//! **Annotations follow their page.** An object's `page` is always a display
//! index — the position in this map — so every structural edit carries the
//! renumbering of the objects it moves, the objects it removes with a deleted
//! page, and the copies it makes for a duplicated one, in the same [`Op`]. One
//! step, one undo; there is no state in which a page has moved and its
//! highlights have not.
//!
//! `None` for the map means the file's own pages in the file's own order, which
//! is every document until its first page operation. Keeping that case explicit
//! rather than writing out `0..n` means an untouched document never pays for a
//! map, and a save knows at a glance there is nothing to rearrange.

use serde::{Deserialize, Serialize};

use crate::annot::AnnotObject;
use crate::ops::{AnnotDoc, EditError, Op};

/// Index into a document's table of page sources. Source 0 is the document's
/// own file; others are files pages were merged or copied in from.
pub type SourceId = u32;

/// The document's own file.
pub const OWN_FILE: SourceId = 0;

/// One page of the map.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PageEntry {
    /// Page `page` (zero-based) of source `source`.
    Page {
        source: SourceId,
        page: u32,
        /// Quarter turns clockwise on top of the page's own `/Rotate`.
        rotation: u8,
    },
    /// A new empty page, in points.
    Blank {
        width: f32,
        height: f32,
        rotation: u8,
    },
}

impl PageEntry {
    pub fn own(page: u32) -> Self {
        PageEntry::Page {
            source: OWN_FILE,
            page,
            rotation: 0,
        }
    }

    pub fn rotation(&self) -> u8 {
        match *self {
            PageEntry::Page { rotation, .. } | PageEntry::Blank { rotation, .. } => rotation,
        }
    }

    /// The same page turned `quarters` further (negative is anticlockwise).
    pub fn rotated(self, quarters: i32) -> Self {
        let turn = |r: u8| (i32::from(r) + quarters).rem_euclid(4) as u8;
        match self {
            PageEntry::Page {
                source,
                page,
                rotation,
            } => PageEntry::Page {
                source,
                page,
                rotation: turn(rotation),
            },
            PageEntry::Blank {
                width,
                height,
                rotation,
            } => PageEntry::Blank {
                width,
                height,
                rotation: turn(rotation),
            },
        }
    }
}

/// The map as it is: the explicit one, or the file's own pages in order.
pub fn resolve(map: Option<&[PageEntry]>, file_pages: u32) -> Vec<PageEntry> {
    match map {
        Some(m) => m.to_vec(),
        None => (0..file_pages).map(PageEntry::own).collect(),
    }
}

/// Whether a map is the file's own pages, in order, unturned.
pub fn is_identity(map: &[PageEntry], file_pages: u32) -> bool {
    map.len() == file_pages as usize
        && map
            .iter()
            .enumerate()
            .all(|(i, e)| *e == PageEntry::own(i as u32))
}

/// A structural edit, as the user asks for it. Page numbers are display
/// indices, zero-based.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PageCommand {
    Delete {
        pages: Vec<u32>,
    },
    /// Moves `pages` (kept in their current relative order) so that they sit
    /// just before the page that is at display index `before` now; `before`
    /// equal to the page count means "to the end".
    Move {
        pages: Vec<u32>,
        before: u32,
    },
    Rotate {
        pages: Vec<u32>,
        quarters: i32,
    },
    /// `count` empty pages of the given size, before display index `at`.
    InsertBlank {
        at: u32,
        count: u32,
        width: f32,
        height: f32,
    },
    /// A copy of each page — annotations included — right after the last of
    /// them.
    Duplicate {
        pages: Vec<u32>,
    },
    /// Pages from elsewhere — a merged file, pages dragged in from another
    /// document — before display index `at`. `objects[i].page` is an index
    /// into `entries`; the objects get fresh ids here.
    Insert {
        at: u32,
        entries: Vec<PageEntry>,
        objects: Vec<AnnotObject>,
    },
}

/// Why a page command was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum PageError {
    /// A page number past the end.
    OutOfRange(u32),
    /// Deleting every page: a PDF must have at least one.
    WouldBeEmpty,
    /// Nothing to do — an empty selection.
    Nothing,
    Edit(EditError),
}

impl std::fmt::Display for PageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PageError::OutOfRange(p) => write!(f, "halaman {} tidak ada", p + 1),
            PageError::WouldBeEmpty => {
                write!(f, "dokumen harus menyisakan setidaknya satu halaman")
            }
            PageError::Nothing => write!(f, "tidak ada halaman yang dipilih"),
            PageError::Edit(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PageError {}

impl From<EditError> for PageError {
    fn from(e: EditError) -> Self {
        PageError::Edit(e)
    }
}

/// Sorted, deduplicated, and checked against the page count.
fn selection(pages: &[u32], count: usize) -> Result<Vec<u32>, PageError> {
    let mut out = pages.to_vec();
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(PageError::Nothing);
    }
    if let Some(&bad) = out.iter().find(|&&p| p as usize >= count) {
        return Err(PageError::OutOfRange(bad));
    }
    Ok(out)
}

/// Works out the one [`Op`] that performs `cmd` on `doc`, whose file has
/// `file_pages` pages. Pure apart from handing out fresh ids for copies.
pub fn plan(doc: &mut AnnotDoc, file_pages: u32, cmd: PageCommand) -> Result<Op, PageError> {
    let before = doc.page_map().map(<[PageEntry]>::to_vec);
    let current = resolve(before.as_deref(), file_pages);
    let n = current.len();

    // Each command is a new list of entries plus, for every new position, the
    // old position it came from (`None` for a page that did not exist before).
    // Annotations are renumbered from that, which is what keeps them on their
    // page whatever happened around it.
    let mut after: Vec<PageEntry> = Vec::with_capacity(n + 1);
    let mut origin: Vec<Option<u32>> = Vec::with_capacity(n + 1);
    // Copies: (new position, old position) — the page's annotations are
    // duplicated onto the new one.
    let mut copies: Vec<(u32, u32)> = Vec::new();
    let mut added: Vec<AnnotObject> = Vec::new();

    match cmd {
        PageCommand::Delete { pages } => {
            let sel = selection(&pages, n)?;
            if sel.len() >= n {
                return Err(PageError::WouldBeEmpty);
            }
            for (i, e) in current.iter().enumerate() {
                if sel.binary_search(&(i as u32)).is_err() {
                    after.push(*e);
                    origin.push(Some(i as u32));
                }
            }
        }
        PageCommand::Move {
            pages,
            before: target,
        } => {
            let sel = selection(&pages, n)?;
            if target as usize > n {
                return Err(PageError::OutOfRange(target));
            }
            let rest: Vec<u32> = (0..n as u32)
                .filter(|p| sel.binary_search(p).is_err())
                .collect();
            let at = rest.iter().filter(|&&p| p < target).count();
            let (head, tail) = rest.split_at(at.min(rest.len()));
            let order: Vec<u32> = head
                .iter()
                .chain(sel.iter())
                .chain(tail.iter())
                .copied()
                .collect();
            for old in order {
                // `sel` and `rest` were both checked against `n` above.
                let entry = current
                    .get(old as usize)
                    .ok_or(PageError::OutOfRange(old))?;
                after.push(*entry);
                origin.push(Some(old));
            }
        }
        PageCommand::Rotate { pages, quarters } => {
            let sel = selection(&pages, n)?;
            if quarters.rem_euclid(4) == 0 {
                return Err(PageError::Nothing);
            }
            for (i, e) in current.iter().enumerate() {
                let turned = sel.binary_search(&(i as u32)).is_ok();
                after.push(if turned { e.rotated(quarters) } else { *e });
                origin.push(Some(i as u32));
            }
        }
        PageCommand::InsertBlank {
            at,
            count,
            width,
            height,
        } => {
            if at as usize > n {
                return Err(PageError::OutOfRange(at));
            }
            if count == 0 {
                return Err(PageError::Nothing);
            }
            for (i, e) in current.iter().enumerate() {
                if i == at as usize {
                    for _ in 0..count {
                        after.push(PageEntry::Blank {
                            width,
                            height,
                            rotation: 0,
                        });
                        origin.push(None);
                    }
                }
                after.push(*e);
                origin.push(Some(i as u32));
            }
            if at as usize == n {
                for _ in 0..count {
                    after.push(PageEntry::Blank {
                        width,
                        height,
                        rotation: 0,
                    });
                    origin.push(None);
                }
            }
        }
        PageCommand::Duplicate { pages } => {
            let sel = selection(&pages, n)?;
            let last = *sel.last().ok_or(PageError::Nothing)?;
            for (i, e) in current.iter().enumerate() {
                after.push(*e);
                origin.push(Some(i as u32));
                if i as u32 == last {
                    for &p in &sel {
                        let entry = current.get(p as usize).ok_or(PageError::OutOfRange(p))?;
                        copies.push((after.len() as u32, p));
                        after.push(*entry);
                        origin.push(None);
                    }
                }
            }
        }
        PageCommand::Insert {
            at,
            entries,
            objects,
        } => {
            if at as usize > n {
                return Err(PageError::OutOfRange(at));
            }
            if entries.is_empty() {
                return Err(PageError::Nothing);
            }
            for (i, e) in current.iter().enumerate() {
                if i == at as usize {
                    for x in &entries {
                        after.push(*x);
                        origin.push(None);
                    }
                }
                after.push(*e);
                origin.push(Some(i as u32));
            }
            if at as usize == n {
                for x in &entries {
                    after.push(*x);
                    origin.push(None);
                }
            }
            for mut obj in objects {
                if obj.page as usize >= entries.len() {
                    return Err(PageError::OutOfRange(obj.page));
                }
                obj.id = doc.fresh_id();
                obj.page += at;
                added.push(obj);
            }
        }
    }

    // Old position → new position, for the pages that stayed.
    let mut new_of: Vec<Option<u32>> = vec![None; n];
    for (new, old) in origin.iter().enumerate() {
        if let Some(old) = old {
            if let Some(slot) = new_of.get_mut(*old as usize) {
                *slot = Some(new as u32);
            }
        }
    }
    let mut moves = Vec::new();
    let mut removed = Vec::new();
    for obj in doc.iter() {
        match new_of.get(obj.page as usize).copied().flatten() {
            Some(new) if new != obj.page => moves.push((obj.id, obj.page, new)),
            Some(_) => {}
            None => removed.push(obj.clone()),
        }
    }
    for (new, old) in copies {
        let originals: Vec<AnnotObject> = doc.page(old).into_iter().cloned().collect();
        for mut copy in originals {
            copy.id = doc.fresh_id();
            copy.page = new;
            added.push(copy);
        }
    }

    // A map that has come back to the file's own order is stored as `None`,
    // so that undoing every page edit leaves nothing to rearrange at save.
    let after = if is_identity(&after, file_pages) {
        None
    } else {
        Some(after)
    };
    Ok(Op::Pages {
        before,
        after,
        moves,
        removed,
        added,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annot::{AnnotId, AnnotKind, AnnotPayload, ShapeStyle};
    use crate::geom::PdfRectF;
    use crate::ops::CommandStack;

    fn rect_on(doc: &mut AnnotDoc, page: u32) -> AnnotId {
        let id = doc.fresh_id();
        let obj = AnnotObject::new(
            id,
            page,
            AnnotKind::Rect,
            PdfRectF::new(10.0, 10.0, 50.0, 50.0),
            AnnotPayload::Shape {
                style: ShapeStyle::default(),
            },
        );
        doc.import(obj).unwrap();
        id
    }

    fn pages_of(doc: &AnnotDoc) -> Vec<(u64, u32)> {
        doc.iter().map(|o| (o.id.0, o.page)).collect()
    }

    /// Source pages of the map, `None` for a blank page.
    fn order(doc: &AnnotDoc, file_pages: u32) -> Vec<Option<u32>> {
        resolve(doc.page_map(), file_pages)
            .iter()
            .map(|e| match e {
                PageEntry::Page { page, .. } => Some(*page),
                PageEntry::Blank { .. } => None,
            })
            .collect()
    }

    fn run(doc: &mut AnnotDoc, stack: &mut CommandStack, n: u32, cmd: PageCommand) {
        let op = plan(doc, n, cmd).unwrap();
        stack.commit(doc, vec![op]).unwrap();
    }

    #[test]
    fn deleting_a_page_takes_its_annotations_and_renumbers_the_rest() {
        let mut doc = AnnotDoc::new();
        let a = rect_on(&mut doc, 0);
        let b = rect_on(&mut doc, 1);
        let c = rect_on(&mut doc, 3);
        let original = doc.clone();
        let mut stack = CommandStack::default();
        run(
            &mut doc,
            &mut stack,
            4,
            PageCommand::Delete { pages: vec![1] },
        );
        assert_eq!(order(&doc, 4), vec![Some(0), Some(2), Some(3)]);
        assert_eq!(pages_of(&doc), vec![(a.0, 0), (c.0, 2)]);
        assert!(doc.get(b).is_none());
        // One undo puts back the page *and* what was on it, where it was.
        stack.undo(&mut doc).unwrap();
        assert_eq!(doc, original);
        assert!(doc.page_map().is_none(), "back to the file's own order");
    }

    #[test]
    fn moving_pages_keeps_their_order_and_their_annotations() {
        let mut doc = AnnotDoc::new();
        let on0 = rect_on(&mut doc, 0);
        let on4 = rect_on(&mut doc, 4);
        let mut stack = CommandStack::default();
        // Drag pages 1 and 3 to before page 0.
        run(
            &mut doc,
            &mut stack,
            5,
            PageCommand::Move {
                pages: vec![3, 1],
                before: 0,
            },
        );
        assert_eq!(
            order(&doc, 5),
            vec![Some(1), Some(3), Some(0), Some(2), Some(4)]
        );
        assert_eq!(pages_of(&doc), vec![(on0.0, 2), (on4.0, 4)]);
        // And to the end.
        run(
            &mut doc,
            &mut stack,
            5,
            PageCommand::Move {
                pages: vec![0],
                before: 5,
            },
        );
        assert_eq!(
            order(&doc, 5),
            vec![Some(3), Some(0), Some(2), Some(4), Some(1)]
        );
        assert_eq!(pages_of(&doc), vec![(on0.0, 1), (on4.0, 3)]);
        stack.undo(&mut doc).unwrap();
        stack.undo(&mut doc).unwrap();
        assert_eq!(pages_of(&doc), vec![(on0.0, 0), (on4.0, 4)]);
        assert!(doc.page_map().is_none());
    }

    /// A move onto itself is not an edit worth an undo step, but it must not
    /// break anything either.
    #[test]
    fn moving_a_page_to_where_it_is_changes_nothing() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        run(
            &mut doc,
            &mut stack,
            3,
            PageCommand::Move {
                pages: vec![1],
                before: 1,
            },
        );
        run(
            &mut doc,
            &mut stack,
            3,
            PageCommand::Move {
                pages: vec![1],
                before: 2,
            },
        );
        assert!(doc.page_map().is_none());
    }

    #[test]
    fn rotating_turns_the_entry_and_four_turns_are_no_turn() {
        let mut doc = AnnotDoc::new();
        let on1 = rect_on(&mut doc, 1);
        let mut stack = CommandStack::default();
        run(
            &mut doc,
            &mut stack,
            3,
            PageCommand::Rotate {
                pages: vec![1],
                quarters: 1,
            },
        );
        let map = resolve(doc.page_map(), 3);
        assert_eq!(map[1].rotation(), 1);
        assert_eq!(map[0].rotation(), 0);
        assert_eq!(
            pages_of(&doc),
            vec![(on1.0, 1)],
            "annotations stay in page space"
        );
        run(
            &mut doc,
            &mut stack,
            3,
            PageCommand::Rotate {
                pages: vec![1],
                quarters: -3,
            },
        );
        assert_eq!(resolve(doc.page_map(), 3)[1].rotation(), 2);
        run(
            &mut doc,
            &mut stack,
            3,
            PageCommand::Rotate {
                pages: vec![1],
                quarters: 2,
            },
        );
        assert!(
            doc.page_map().is_none(),
            "a full turn is the file's own page again"
        );
    }

    #[test]
    fn duplicating_copies_the_annotations_with_new_ids() {
        let mut doc = AnnotDoc::new();
        let on1 = rect_on(&mut doc, 1);
        let mut stack = CommandStack::default();
        run(
            &mut doc,
            &mut stack,
            3,
            PageCommand::Duplicate { pages: vec![1, 0] },
        );
        assert_eq!(
            order(&doc, 3),
            vec![Some(0), Some(1), Some(0), Some(1), Some(2)]
        );
        let objs = pages_of(&doc);
        assert_eq!(objs.len(), 2);
        assert!(objs.contains(&(on1.0, 1)));
        let copy = objs.iter().find(|(id, _)| *id != on1.0).unwrap();
        assert_eq!(copy.1, 3, "the copy sits on the copy of its page");
        assert!(copy.0 > on1.0, "a fresh id, never a reused one");
        stack.undo(&mut doc).unwrap();
        assert_eq!(pages_of(&doc), vec![(on1.0, 1)]);
    }

    #[test]
    fn blank_pages_go_where_they_are_asked_for() {
        let mut doc = AnnotDoc::new();
        let on1 = rect_on(&mut doc, 1);
        let mut stack = CommandStack::default();
        run(
            &mut doc,
            &mut stack,
            2,
            PageCommand::InsertBlank {
                at: 1,
                count: 2,
                width: 595.0,
                height: 842.0,
            },
        );
        assert_eq!(order(&doc, 2), vec![Some(0), None, None, Some(1)]);
        assert_eq!(pages_of(&doc), vec![(on1.0, 3)]);
        run(
            &mut doc,
            &mut stack,
            2,
            PageCommand::InsertBlank {
                at: 4,
                count: 1,
                width: 100.0,
                height: 100.0,
            },
        );
        assert_eq!(order(&doc, 2), vec![Some(0), None, None, Some(1), None]);
    }

    #[test]
    fn inserted_pages_bring_their_annotations_with_fresh_ids() {
        let mut doc = AnnotDoc::new();
        let mine = rect_on(&mut doc, 0);
        let mut other = AnnotDoc::new();
        let theirs = rect_on(&mut other, 1);
        let incoming = other.get(theirs).unwrap().clone();
        let mut stack = CommandStack::default();
        let entries = vec![
            PageEntry::Page {
                source: 1,
                page: 4,
                rotation: 0,
            },
            PageEntry::Page {
                source: 1,
                page: 5,
                rotation: 1,
            },
        ];
        run(
            &mut doc,
            &mut stack,
            2,
            PageCommand::Insert {
                at: 1,
                entries: entries.clone(),
                objects: vec![incoming],
            },
        );
        let map = resolve(doc.page_map(), 2);
        assert_eq!(map[1..3], entries[..]);
        let objs = pages_of(&doc);
        assert_eq!(objs.len(), 2);
        let (id, page) = *objs.iter().find(|(id, _)| *id != mine.0).unwrap();
        assert_eq!(page, 2, "second inserted page, at 1 + 1");
        assert_ne!(id, mine.0);
    }

    #[test]
    fn refusals() {
        let mut doc = AnnotDoc::new();
        assert_eq!(
            plan(&mut doc, 2, PageCommand::Delete { pages: vec![0, 1] }),
            Err(PageError::WouldBeEmpty)
        );
        assert_eq!(
            plan(&mut doc, 2, PageCommand::Delete { pages: vec![2] }),
            Err(PageError::OutOfRange(2))
        );
        assert_eq!(
            plan(&mut doc, 2, PageCommand::Delete { pages: vec![] }),
            Err(PageError::Nothing)
        );
        assert_eq!(
            plan(
                &mut doc,
                2,
                PageCommand::Move {
                    pages: vec![0],
                    before: 3
                }
            ),
            Err(PageError::OutOfRange(3))
        );
    }

    #[test]
    fn a_locked_annotation_leaves_with_its_page() {
        let mut doc = AnnotDoc::new();
        let id = rect_on(&mut doc, 2);
        let mut locked = doc.get(id).unwrap().clone();
        locked.locked = true;
        doc = {
            let mut d = AnnotDoc::new();
            d.import(locked).unwrap();
            d
        };
        let mut stack = CommandStack::default();
        run(
            &mut doc,
            &mut stack,
            3,
            PageCommand::Move {
                pages: vec![2],
                before: 0,
            },
        );
        assert_eq!(pages_of(&doc), vec![(id.0, 0)]);
        run(
            &mut doc,
            &mut stack,
            3,
            PageCommand::Delete { pages: vec![0] },
        );
        assert!(doc.is_empty());
        stack.undo(&mut doc).unwrap();
        assert!(doc.get(id).unwrap().locked);
    }

    #[test]
    fn a_page_op_that_does_not_fit_the_document_changes_nothing() {
        let mut doc = AnnotDoc::new();
        let id = rect_on(&mut doc, 0);
        let snapshot = doc.clone();
        let bad = Op::Pages {
            before: None,
            after: Some(vec![PageEntry::own(1)]),
            // Claims the object is on page 5; it is on page 0.
            moves: vec![(id, 5, 1)],
            removed: vec![],
            added: vec![],
        };
        assert!(doc.apply(bad).is_err());
        assert_eq!(doc, snapshot);
    }
}
