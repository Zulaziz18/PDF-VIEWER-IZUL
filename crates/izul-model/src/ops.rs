//! Edits as data, and the undo stack over them (SPEC 8).
//!
//! Nothing mutates an annotation directly. Every change is an [`Op`] that knows
//! how to invert itself, applied through [`AnnotDoc::apply`], which is what
//! makes undo total rather than a list of special cases somebody has to
//! remember to extend. When a new kind of edit is added, the compiler asks for
//! its inverse; it cannot be forgotten.
//!
//! Two decisions worth stating, because both cost memory and both are
//! deliberate:
//!
//! * **`Replace` carries the whole object, before and after.** A diff would be
//!   smaller, and it would also be a second description of what an annotation
//!   is — one that can disagree with the first. For everything but a
//!   thousand-point ink stroke the object *is* smaller than a diff's
//!   bookkeeping, and correctness here is worth more than the bytes.
//! * **A gesture is one transaction.** Dragging an object emits one `Replace`
//!   when the pointer goes up, not one per frame. Undo steps that mean "the
//!   object moved" rather than "the object moved 0.3 points" are what a user
//!   expects, and it keeps the 200-step floor (SPEC 8) from being spent on a
//!   single drag.

use std::collections::BTreeMap;

use crate::annot::{AnnotId, AnnotObject};

/// One reversible edit.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Insert(Box<AnnotObject>),
    Delete(Box<AnnotObject>),
    Replace {
        before: Box<AnnotObject>,
        after: Box<AnnotObject>,
    },
}

impl Op {
    /// The edit that undoes this one.
    pub fn invert(&self) -> Op {
        match self {
            Op::Insert(obj) => Op::Delete(obj.clone()),
            Op::Delete(obj) => Op::Insert(obj.clone()),
            Op::Replace { before, after } => Op::Replace {
                before: after.clone(),
                after: before.clone(),
            },
        }
    }

    pub fn target(&self) -> AnnotId {
        match self {
            Op::Insert(o) | Op::Delete(o) => o.id,
            Op::Replace { after, .. } => after.id,
        }
    }
}

/// Why an edit could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// The object an edit refers to is not in the document. Always a bug on the
    /// caller's side, and worth failing loudly rather than ignoring: an undo
    /// stack whose steps silently no-op is one that puts the document in a
    /// state the user never asked for.
    NotFound(AnnotId),
    /// An insert collided with an existing id.
    AlreadyExists(AnnotId),
    /// The object is locked (SPEC 11.2's "kunci").
    Locked(AnnotId),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::NotFound(id) => write!(f, "anotasi {} tidak ada", id.0),
            EditError::AlreadyExists(id) => write!(f, "anotasi {} sudah ada", id.0),
            EditError::Locked(id) => write!(f, "anotasi {} terkunci", id.0),
        }
    }
}

impl std::error::Error for EditError {}

/// A group of edits that undo and redo as one step.
pub type Transaction = Vec<Op>;

/// The annotations of one document, and nothing else.
///
/// Deliberately not "the document": there is no page tree, no file, no PDFium
/// here. This is the layer the undo stack works on and the layer the display
/// list is built from, and keeping it free of everything else is what lets a
/// parity bug be reproduced in a unit test (SPEC 5).
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotDoc {
    /// Keyed and ordered by id so iteration is deterministic — the appearance
    /// streams of a page are written in this order, and a `HashMap` would make
    /// the saved file differ between runs for no reason.
    objects: BTreeMap<AnnotId, AnnotObject>,
    next_id: u64,
}

/// Hand-written rather than derived, and that is not a style choice: a derived
/// `Default` would start `next_id` at 0, and zero is the sentinel the command
/// layer uses for "this object has no id yet". A document created through
/// `Default` would then hand out that sentinel as a real id, and the first
/// annotation drawn in it would be treated as un-inserted forever. Caught by a
/// test rather than by a user, but only just.
impl Default for AnnotDoc {
    fn default() -> Self {
        AnnotDoc::new()
    }
}

impl AnnotDoc {
    pub fn new() -> Self {
        AnnotDoc {
            objects: BTreeMap::new(),
            next_id: 1,
        }
    }

    /// The next free id. Monotonic and never reused, because the undo stack
    /// refers to objects by id: reusing one would let an undo resurrect the
    /// wrong object.
    pub fn fresh_id(&mut self) -> AnnotId {
        let id = AnnotId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Places an object read back from a saved file, keeping its id.
    ///
    /// Not an [`Op`]: opening a file is not an edit, and an undo that removed
    /// the annotations the file already had would be undoing something the
    /// user never did. `next_id` moves past the object so a fresh id can never
    /// collide with it. An object whose id is already taken is refused — two
    /// objects with one id would make every later undo ambiguous.
    pub fn import(&mut self, obj: AnnotObject) -> Result<(), EditError> {
        if obj.id.0 == 0 || self.objects.contains_key(&obj.id) {
            return Err(EditError::AlreadyExists(obj.id));
        }
        self.next_id = self.next_id.max(obj.id.0.saturating_add(1));
        self.objects.insert(obj.id, obj);
        Ok(())
    }

    pub fn get(&self, id: AnnotId) -> Option<&AnnotObject> {
        self.objects.get(&id)
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Every object, by ascending id.
    pub fn iter(&self) -> impl Iterator<Item = &AnnotObject> {
        self.objects.values()
    }

    /// The objects of one page, in paint order: `z` first, then id so that two
    /// objects at the same depth still have a stable order.
    pub fn page(&self, page: u32) -> Vec<&AnnotObject> {
        let mut out: Vec<&AnnotObject> = self.objects.values().filter(|o| o.page == page).collect();
        out.sort_by_key(|o| (o.z, o.id));
        out
    }

    /// Applies one edit. Returns the edit that undoes it.
    pub fn apply(&mut self, op: Op) -> Result<Op, EditError> {
        let inverse = op.invert();
        match op {
            Op::Insert(obj) => {
                if self.objects.contains_key(&obj.id) {
                    return Err(EditError::AlreadyExists(obj.id));
                }
                self.next_id = self.next_id.max(obj.id.0 + 1);
                self.objects.insert(obj.id, *obj);
            }
            Op::Delete(obj) => {
                match self.objects.get(&obj.id) {
                    None => return Err(EditError::NotFound(obj.id)),
                    Some(existing) if existing.locked => return Err(EditError::Locked(obj.id)),
                    Some(_) => {}
                }
                self.objects.remove(&obj.id);
            }
            Op::Replace { after, .. } => {
                match self.objects.get(&after.id) {
                    None => return Err(EditError::NotFound(after.id)),
                    // A locked object can still be *unlocked*, which is the one
                    // edit that must get through — otherwise locking is a
                    // one-way door.
                    Some(existing) if existing.locked && after.locked => {
                        return Err(EditError::Locked(after.id))
                    }
                    Some(_) => {}
                }
                self.objects.insert(after.id, *after);
            }
        }
        Ok(inverse)
    }
}

/// Undo and redo for one document (SPEC 8).
#[derive(Debug, Clone)]
pub struct CommandStack {
    /// Inverses, newest last.
    undo: Vec<Transaction>,
    /// Re-applications, newest last.
    redo: Vec<Transaction>,
    limit: usize,
}

/// SPEC 8's floor: at least 200 steps per document.
pub const DEFAULT_UNDO_LIMIT: usize = 200;

impl Default for CommandStack {
    fn default() -> Self {
        CommandStack::new(DEFAULT_UNDO_LIMIT)
    }
}

impl CommandStack {
    pub fn new(limit: usize) -> Self {
        CommandStack {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: limit.max(1),
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Applies a transaction and records how to undo it.
    ///
    /// All-or-nothing: an edit that fails halfway is rolled back before the
    /// error is returned, so a failed multi-select move cannot leave half the
    /// selection moved.
    pub fn commit(&mut self, doc: &mut AnnotDoc, ops: Transaction) -> Result<(), EditError> {
        if ops.is_empty() {
            return Ok(());
        }
        let mut inverses: Transaction = Vec::with_capacity(ops.len());
        for op in ops {
            match doc.apply(op) {
                Ok(inv) => inverses.push(inv),
                Err(e) => {
                    // Unwind what did land, newest first.
                    for undo_op in inverses.into_iter().rev() {
                        let _ = doc.apply(undo_op);
                    }
                    return Err(e);
                }
            }
        }
        inverses.reverse();
        self.undo.push(inverses);
        // A new edit invalidates the redo branch: the future the user was
        // stepping back into no longer exists.
        self.redo.clear();
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        Ok(())
    }

    pub fn undo(&mut self, doc: &mut AnnotDoc) -> Result<bool, EditError> {
        let Some(ops) = self.undo.pop() else {
            return Ok(false);
        };
        let mut inverses: Transaction = Vec::with_capacity(ops.len());
        for op in ops {
            inverses.push(doc.apply(op)?);
        }
        inverses.reverse();
        self.redo.push(inverses);
        Ok(true)
    }

    pub fn redo(&mut self, doc: &mut AnnotDoc) -> Result<bool, EditError> {
        let Some(ops) = self.redo.pop() else {
            return Ok(false);
        };
        let mut inverses: Transaction = Vec::with_capacity(ops.len());
        for op in ops {
            inverses.push(doc.apply(op)?);
        }
        inverses.reverse();
        self.undo.push(inverses);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annot::{AnnotKind, AnnotPayload, ShapeStyle};
    use crate::geom::PdfRectF;

    fn shape(doc: &mut AnnotDoc, x: f32) -> AnnotObject {
        let id = doc.fresh_id();
        AnnotObject::new(
            id,
            0,
            AnnotKind::Rect,
            PdfRectF::new(x, 0.0, x + 10.0, 10.0),
            AnnotPayload::Shape {
                style: ShapeStyle::default(),
            },
        )
    }

    #[test]
    fn insert_then_undo_leaves_the_document_as_it_was() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        let obj = shape(&mut doc, 0.0);
        let id = obj.id;
        stack
            .commit(&mut doc, vec![Op::Insert(Box::new(obj))])
            .expect("commit");
        assert_eq!(doc.len(), 1);
        assert!(stack.undo(&mut doc).expect("undo"));
        assert!(doc.is_empty());
        assert!(stack.redo(&mut doc).expect("redo"));
        assert!(doc.get(id).is_some());
    }

    #[test]
    fn a_move_undoes_to_the_old_position() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        let obj = shape(&mut doc, 0.0);
        let id = obj.id;
        stack
            .commit(&mut doc, vec![Op::Insert(Box::new(obj.clone()))])
            .expect("commit");

        let mut moved = obj.clone();
        moved.translate(25.0, 5.0);
        stack
            .commit(
                &mut doc,
                vec![Op::Replace {
                    before: Box::new(obj.clone()),
                    after: Box::new(moved),
                }],
            )
            .expect("commit");
        assert_eq!(doc.get(id).map(|o| o.rect.left), Some(25.0));
        stack.undo(&mut doc).expect("undo");
        assert_eq!(doc.get(id).map(|o| o.rect.left), Some(0.0));
        stack.redo(&mut doc).expect("redo");
        assert_eq!(doc.get(id).map(|o| o.rect.left), Some(25.0));
    }

    /// Moving five selected objects is one thing the user did, so it is one
    /// press of Ctrl+Z.
    #[test]
    fn a_multi_select_move_is_one_undo_step() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        let objs: Vec<AnnotObject> = (0..5).map(|i| shape(&mut doc, i as f32 * 20.0)).collect();
        stack
            .commit(
                &mut doc,
                objs.iter()
                    .cloned()
                    .map(|o| Op::Insert(Box::new(o)))
                    .collect(),
            )
            .expect("commit");

        let moved: Transaction = objs
            .iter()
            .map(|o| {
                let mut after = o.clone();
                after.translate(0.0, 100.0);
                Op::Replace {
                    before: Box::new(o.clone()),
                    after: Box::new(after),
                }
            })
            .collect();
        stack.commit(&mut doc, moved).expect("commit");
        assert!(doc.iter().all(|o| o.rect.bottom == 100.0));
        assert_eq!(
            stack.undo_depth(),
            2,
            "sisip dan geser, bukan sepuluh langkah"
        );
        stack.undo(&mut doc).expect("undo");
        assert!(doc.iter().all(|o| o.rect.bottom == 0.0), "semuanya kembali");
    }

    /// A transaction that fails partway must not leave half the selection moved.
    #[test]
    fn a_failed_transaction_rolls_back_what_already_landed() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        let a = shape(&mut doc, 0.0);
        stack
            .commit(&mut doc, vec![Op::Insert(Box::new(a.clone()))])
            .expect("commit");

        let mut moved = a.clone();
        moved.translate(10.0, 0.0);
        let ghost = shape(&mut doc, 500.0); // never inserted
        let result = stack.commit(
            &mut doc,
            vec![
                Op::Replace {
                    before: Box::new(a.clone()),
                    after: Box::new(moved),
                },
                Op::Delete(Box::new(ghost)),
            ],
        );
        assert!(matches!(result, Err(EditError::NotFound(_))), "{result:?}");
        assert_eq!(
            doc.get(a.id).map(|o| o.rect.left),
            Some(0.0),
            "yang sempat berhasil harus dibatalkan"
        );
        assert_eq!(stack.undo_depth(), 1, "transaksi gagal tidak masuk riwayat");
    }

    #[test]
    fn a_new_edit_throws_away_the_redo_branch() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        let a = shape(&mut doc, 0.0);
        stack
            .commit(&mut doc, vec![Op::Insert(Box::new(a))])
            .expect("commit");
        stack.undo(&mut doc).expect("undo");
        assert!(stack.can_redo());
        let b = shape(&mut doc, 50.0);
        stack
            .commit(&mut doc, vec![Op::Insert(Box::new(b))])
            .expect("commit");
        assert!(
            !stack.can_redo(),
            "masa depan yang ditinggalkan tidak kembali"
        );
    }

    /// SPEC 8 sets the floor at 200; the oldest step is what falls off.
    #[test]
    fn the_stack_keeps_two_hundred_steps_and_drops_the_oldest() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        assert_eq!(DEFAULT_UNDO_LIMIT, 200);
        for i in 0..250 {
            let obj = shape(&mut doc, i as f32);
            stack
                .commit(&mut doc, vec![Op::Insert(Box::new(obj))])
                .expect("commit");
        }
        assert_eq!(stack.undo_depth(), 200);
        for _ in 0..200 {
            assert!(stack.undo(&mut doc).expect("undo"));
        }
        assert!(!stack.can_undo());
        // The 50 oldest inserts are past the horizon and stay in the document,
        // which is the honest consequence of a bounded stack.
        assert_eq!(doc.len(), 50);
    }

    #[test]
    fn a_locked_object_refuses_to_be_deleted_but_can_be_unlocked() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        let mut obj = shape(&mut doc, 0.0);
        obj.locked = true;
        stack
            .commit(&mut doc, vec![Op::Insert(Box::new(obj.clone()))])
            .expect("commit");
        let err = stack.commit(&mut doc, vec![Op::Delete(Box::new(obj.clone()))]);
        assert!(matches!(err, Err(EditError::Locked(_))), "{err:?}");

        let mut unlocked = obj.clone();
        unlocked.locked = false;
        stack
            .commit(
                &mut doc,
                vec![Op::Replace {
                    before: Box::new(obj.clone()),
                    after: Box::new(unlocked.clone()),
                }],
            )
            .expect("membuka kunci harus boleh");
        stack
            .commit(&mut doc, vec![Op::Delete(Box::new(unlocked))])
            .expect("setelah dibuka boleh dihapus");
        assert!(doc.is_empty());
    }

    #[test]
    fn ids_are_never_reused_even_after_an_undo() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        let a = shape(&mut doc, 0.0);
        let first = a.id;
        stack
            .commit(&mut doc, vec![Op::Insert(Box::new(a))])
            .expect("commit");
        stack.undo(&mut doc).expect("undo");
        let b = shape(&mut doc, 0.0);
        assert_ne!(
            b.id, first,
            "id yang dipakai ulang membuat undo salah sasaran"
        );
    }

    #[test]
    fn a_page_is_painted_in_z_order_then_by_id() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        let mut ops = Vec::new();
        for (i, z) in [3, 1, 2, 1].into_iter().enumerate() {
            let mut o = shape(&mut doc, i as f32);
            o.z = z;
            o.page = if i == 3 { 1 } else { 0 };
            ops.push(Op::Insert(Box::new(o)));
        }
        stack.commit(&mut doc, ops).expect("commit");
        let order: Vec<i32> = doc.page(0).iter().map(|o| o.z).collect();
        assert_eq!(order, vec![1, 2, 3]);
        assert_eq!(doc.page(1).len(), 1, "halaman lain tidak ikut");
    }

    #[test]
    fn iteration_order_is_stable_so_saved_files_do_not_churn() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        let mut ops = Vec::new();
        for i in 0..10 {
            ops.push(Op::Insert(Box::new(shape(&mut doc, i as f32))));
        }
        stack.commit(&mut doc, ops).expect("commit");
        let a: Vec<AnnotId> = doc.iter().map(|o| o.id).collect();
        let b: Vec<AnnotId> = doc.iter().map(|o| o.id).collect();
        assert_eq!(a, b);
        assert!(a.windows(2).all(|w| w[0] < w[1]));
    }

    /// Zero is the command layer's "no id yet"; a document must never hand it
    /// out, however it was created.
    #[test]
    fn a_default_document_never_hands_out_the_zero_id() {
        let mut from_new = AnnotDoc::new();
        let mut from_default = AnnotDoc::default();
        assert_eq!(from_new.fresh_id(), from_default.fresh_id());
        assert_ne!(from_default.fresh_id().0, 0);
        assert_eq!(AnnotDoc::default(), AnnotDoc::new());
    }

    #[test]
    fn an_empty_transaction_is_not_an_undo_step() {
        let mut doc = AnnotDoc::new();
        let mut stack = CommandStack::default();
        stack.commit(&mut doc, Vec::new()).expect("commit");
        assert!(!stack.can_undo());
    }
}
