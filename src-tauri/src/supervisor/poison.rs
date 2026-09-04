//! Tracking documents that take workers down with them (SPEC 3.4).
//!
//! A document that crashes a worker twice is quarantined: reopened alone, in a
//! read-only mode with a bounded image cache, and never again allowed to share a
//! worker with anything else. Two strikes rather than one because a worker can
//! also die for reasons that have nothing to do with the document it happened to
//! be holding — the machine running out of memory, or another document on the
//! same worker being the real culprit.

use std::collections::HashMap;

use super::policy::POISON_THRESHOLD;

/// Identity used for quarantine decisions.
///
/// The *path* is deliberately not the key: a user who copies a malformed file to
/// a new name has the same file, and quarantine should follow the content. The
/// caller supplies a content hash from `izul-store`.
pub type DocKey = [u8; 32];

#[derive(Debug, Default)]
pub struct PoisonList {
    strikes: HashMap<DocKey, u32>,
}

impl PoisonList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that a worker died while holding these documents.
    ///
    /// Every document resident on the worker gets a strike, because we cannot
    /// tell from a dead process which one was responsible. That is why the
    /// threshold is two: an innocent document caught alongside a bad one needs a
    /// second, independent crash before it is quarantined.
    pub fn worker_died_holding(&mut self, docs: &[DocKey]) {
        for d in docs {
            *self.strikes.entry(*d).or_insert(0) += 1;
        }
    }

    pub fn strikes(&self, doc: &DocKey) -> u32 {
        self.strikes.get(doc).copied().unwrap_or(0)
    }

    pub fn is_quarantined(&self, doc: &DocKey) -> bool {
        self.strikes(doc) >= POISON_THRESHOLD
    }

    pub fn quarantined_count(&self) -> usize {
        self.strikes
            .values()
            .filter(|s| **s >= POISON_THRESHOLD)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> DocKey {
        [b; 32]
    }

    #[test]
    fn one_crash_is_not_enough_to_quarantine() {
        let mut p = PoisonList::new();
        p.worker_died_holding(&[key(1)]);
        assert_eq!(p.strikes(&key(1)), 1);
        assert!(
            !p.is_quarantined(&key(1)),
            "one crash can be the machine, not the file"
        );
    }

    #[test]
    fn two_crashes_quarantine_the_document() {
        let mut p = PoisonList::new();
        p.worker_died_holding(&[key(1)]);
        p.worker_died_holding(&[key(1)]);
        assert!(p.is_quarantined(&key(1)));
        assert_eq!(p.quarantined_count(), 1);
    }

    #[test]
    fn every_document_on_a_dead_worker_takes_a_strike() {
        let mut p = PoisonList::new();
        p.worker_died_holding(&[key(1), key(2), key(3)]);
        for b in 1..=3 {
            assert_eq!(p.strikes(&key(b)), 1);
            assert!(!p.is_quarantined(&key(b)));
        }
    }

    #[test]
    fn a_bystander_needs_a_second_independent_crash() {
        let mut p = PoisonList::new();
        // Both documents were on the worker that died.
        p.worker_died_holding(&[key(1), key(2)]);
        // The real culprit is reopened alone and kills its worker again.
        p.worker_died_holding(&[key(1)]);
        assert!(p.is_quarantined(&key(1)));
        assert!(
            !p.is_quarantined(&key(2)),
            "the bystander must not be quarantined"
        );
    }

    #[test]
    fn an_unknown_document_is_clean() {
        let p = PoisonList::new();
        assert_eq!(p.strikes(&key(7)), 0);
        assert!(!p.is_quarantined(&key(7)));
    }
}
