//! The tile cache (SPEC 9).
//!
//! Bitmaps live in the UI process, not in the workers, for three reasons: they
//! survive a worker crash, they are shared between documents that happen to
//! land on different workers, and a tile the user scrolls back to costs a
//! `memcpy` instead of a round trip and a re-render.
//!
//! The budget is in bytes rather than entries because entries are not
//! comparable: a 512x512 sharp tile is 1 MiB and a page preview is a few tens
//! of kilobytes. Counting entries would let a thousand previews evict the
//! screen the user is looking at.
//!
//! Pure data structure — no PDFium, no I/O, no locks. The locking is one level
//! up, in [`crate::render::RenderService`], so the eviction rule can be tested
//! for what it is: an ordering problem.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::Serialize;

/// Which tier a cached bitmap belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TileKind {
    /// Whole page at thumbnail resolution: SPEC 9's first tier, and the
    /// sidebar's thumbnail. One bitmap serving both is deliberate.
    Preview,
    /// A tile rendered without smoothing, for the moment a scroll is moving.
    Fast,
    /// A tile with everything on. What the user ends up looking at.
    Sharp,
}

/// Identity of one cached bitmap.
///
/// SPEC 9 requires the key to cover scale and DPI. It covers them as their
/// product, `ppp_milli` — pixels per PDF point, in thousandths — because that
/// product is exactly what determines the pixels. Keying on zoom and DPI
/// separately would store the same bitmap twice whenever a 125 % window on a
/// 2x monitor met a 250 % window on a 1x one.
///
/// Deliberately *not* keyed by generation: a generation changes on every zoom
/// step, and including it would throw the cache away each time the user turned
/// the wheel — the exact moment it is most valuable. Generation governs
/// scheduling, not identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub doc: u64,
    pub page: u32,
    /// Total rotation applied for display, in quarter turns.
    pub rotation: u8,
    /// Pixels per PDF point, in thousandths. For a preview this is the
    /// bitmap's requested maximum edge in pixels instead, since a preview is
    /// sized to fit rather than to a scale.
    pub ppp_milli: u32,
    pub col: u16,
    pub row: u16,
    pub kind: TileKind,
}

/// A bitmap in the cache.
///
/// `bytes` is an `Arc<[u8]>` so a cache hit hands out the same allocation to
/// every waiter instead of copying it per request.
#[derive(Debug, Clone)]
pub struct CachedTile {
    pub bytes: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

impl CachedTile {
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
    pub bytes: usize,
    pub budget_bytes: usize,
}

#[derive(Debug)]
struct Entry {
    tile: CachedTile,
    /// Position in `order`. Bumped on every hit.
    seq: u64,
}

/// Least-recently-used cache with a byte budget.
#[derive(Debug)]
pub struct TileCache {
    entries: HashMap<TileKey, Entry>,
    /// Access order, oldest first. A `BTreeMap` rather than an intrusive list
    /// because it keeps eviction O(log n) with no unsafe pointers, and the map
    /// is touched once per tile, not once per pixel.
    order: BTreeMap<u64, TileKey>,
    next_seq: u64,
    bytes: usize,
    budget: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl TileCache {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: BTreeMap::new(),
            next_seq: 0,
            bytes: 0,
            budget: budget_bytes,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Looks a tile up, counting the hit or miss and marking it as used.
    pub fn get(&mut self, key: &TileKey) -> Option<CachedTile> {
        let seq = self.next_seq;
        let Some(entry) = self.entries.get_mut(key) else {
            self.misses += 1;
            return None;
        };
        self.next_seq += 1;
        self.order.remove(&entry.seq);
        entry.seq = seq;
        self.order.insert(seq, *key);
        self.hits += 1;
        Some(entry.tile.clone())
    }

    pub fn insert(&mut self, key: TileKey, tile: CachedTile) {
        let size = tile.byte_len();
        // A single tile larger than the whole budget would evict everything and
        // then itself. Refusing it keeps the cache useful instead of empty.
        if size > self.budget {
            return;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.order.remove(&old.seq);
            self.bytes = self.bytes.saturating_sub(old.tile.byte_len());
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.bytes += size;
        self.order.insert(seq, key);
        self.entries.insert(key, Entry { tile, seq });
        self.evict_to_budget();
    }

    fn evict_to_budget(&mut self) {
        while self.bytes > self.budget {
            let Some((&seq, &key)) = self.order.iter().next() else {
                break;
            };
            self.order.remove(&seq);
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(entry.tile.byte_len());
                self.evictions += 1;
            }
        }
    }

    /// Drops everything belonging to one document. Called when a tab closes, so
    /// a closed document cannot hold on to a gigabyte of bitmaps.
    pub fn forget_document(&mut self, doc: u64) -> usize {
        let doomed: Vec<TileKey> = self
            .entries
            .keys()
            .filter(|k| k.doc == doc)
            .copied()
            .collect();
        let count = doomed.len();
        for key in doomed {
            if let Some(entry) = self.entries.remove(&key) {
                self.order.remove(&entry.seq);
                self.bytes = self.bytes.saturating_sub(entry.tile.byte_len());
            }
        }
        count
    }

    /// Drops every tile of one page: its content changed (a form field was
    /// filled, Phase 7) and no bitmap of it is true any more, previews
    /// included.
    pub fn forget_page(&mut self, doc: u64, page: u32) -> usize {
        let doomed: Vec<TileKey> = self
            .entries
            .keys()
            .filter(|k| k.doc == doc && k.page == page)
            .copied()
            .collect();
        let count = doomed.len();
        for key in doomed {
            if let Some(entry) = self.entries.remove(&key) {
                self.order.remove(&entry.seq);
                self.bytes = self.bytes.saturating_sub(entry.tile.byte_len());
            }
        }
        count
    }

    /// Drops the sharp tiles of a document but keeps its previews.
    ///
    /// This is what an inactive tab costs: SPEC 10 asks for full-resolution
    /// bitmaps to be released while thumbnails and position survive, and the
    /// preview tier is both the thumbnail and the guarantee that returning to
    /// the tab shows a page rather than a white rectangle.
    pub fn trim_document(&mut self, doc: u64) -> usize {
        let doomed: Vec<TileKey> = self
            .entries
            .keys()
            .filter(|k| k.doc == doc && k.kind != TileKind::Preview)
            .copied()
            .collect();
        let count = doomed.len();
        for key in doomed {
            if let Some(entry) = self.entries.remove(&key) {
                self.order.remove(&entry.seq);
                self.bytes = self.bytes.saturating_sub(entry.tile.byte_len());
            }
        }
        count
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            entries: self.entries.len(),
            bytes: self.bytes,
            budget_bytes: self.budget,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(doc: u64, page: u32, col: u16) -> TileKey {
        TileKey {
            doc,
            page,
            rotation: 0,
            ppp_milli: 1000,
            col,
            row: 0,
            kind: TileKind::Sharp,
        }
    }

    fn tile(bytes: usize) -> CachedTile {
        CachedTile {
            bytes: vec![7u8; bytes].into(),
            width: 512,
            height: 512,
            stride: 2048,
        }
    }

    #[test]
    fn a_stored_tile_comes_back() {
        let mut c = TileCache::new(1024);
        c.insert(key(1, 0, 0), tile(100));
        let got = c.get(&key(1, 0, 0)).expect("hit");
        assert_eq!(got.byte_len(), 100);
        assert_eq!(c.stats().hits, 1);
    }

    #[test]
    fn forgetting_a_page_keeps_the_other_pages_and_documents() {
        let mut c = TileCache::new(4096);
        c.insert(key(1, 0, 0), tile(10));
        c.insert(key(1, 0, 1), tile(10));
        c.insert(key(1, 1, 0), tile(10));
        c.insert(key(2, 0, 0), tile(10));
        assert_eq!(c.forget_page(1, 0), 2);
        assert!(c.get(&key(1, 0, 0)).is_none());
        assert!(c.get(&key(1, 1, 0)).is_some());
        assert!(c.get(&key(2, 0, 0)).is_some());
        assert_eq!(c.stats().bytes, 20);
    }

    #[test]
    fn a_missing_tile_counts_as_a_miss() {
        let mut c = TileCache::new(1024);
        assert!(c.get(&key(1, 0, 0)).is_none());
        assert_eq!(c.stats().misses, 1);
        assert_eq!(c.stats().hits, 0);
    }

    #[test]
    fn scale_is_part_of_identity() {
        // The same tile at two zoom levels is two different bitmaps. Sharing a
        // key would show the user a blurry page after a zoom and never correct
        // it, which is precisely the bug SPEC 9's "cache key includes scale"
        // exists to prevent.
        let mut c = TileCache::new(1024);
        let mut a = key(1, 0, 0);
        let mut b = a;
        a.ppp_milli = 1000;
        b.ppp_milli = 2000;
        c.insert(a, tile(10));
        assert!(c.get(&b).is_none());
        assert!(c.get(&a).is_some());
    }

    #[test]
    fn rotation_and_kind_are_part_of_identity() {
        let mut c = TileCache::new(1024);
        let base = key(1, 0, 0);
        c.insert(base, tile(10));
        let rotated = TileKey {
            rotation: 1,
            ..base
        };
        let preview = TileKey {
            kind: TileKind::Preview,
            ..base
        };
        assert!(c.get(&rotated).is_none());
        assert!(c.get(&preview).is_none());
    }

    #[test]
    fn the_budget_is_respected_and_the_oldest_goes_first() {
        let mut c = TileCache::new(250);
        c.insert(key(1, 0, 0), tile(100));
        c.insert(key(1, 1, 0), tile(100));
        c.insert(key(1, 2, 0), tile(100)); // 300 > 250, so one must go
        assert!(c.stats().bytes <= 250);
        assert!(c.get(&key(1, 0, 0)).is_none(), "oldest must be evicted");
        assert!(c.get(&key(1, 2, 0)).is_some(), "newest must survive");
        assert_eq!(c.stats().evictions, 1);
    }

    #[test]
    fn a_hit_makes_a_tile_young_again() {
        let mut c = TileCache::new(250);
        c.insert(key(1, 0, 0), tile(100));
        c.insert(key(1, 1, 0), tile(100));
        // Touch the oldest; the next insert must evict the other one instead.
        assert!(c.get(&key(1, 0, 0)).is_some());
        c.insert(key(1, 2, 0), tile(100));
        assert!(c.get(&key(1, 0, 0)).is_some(), "recently used must survive");
        assert!(c.get(&key(1, 1, 0)).is_none());
    }

    #[test]
    fn re_inserting_a_key_does_not_double_count_its_bytes() {
        let mut c = TileCache::new(1000);
        c.insert(key(1, 0, 0), tile(100));
        c.insert(key(1, 0, 0), tile(100));
        assert_eq!(c.stats().bytes, 100);
        assert_eq!(c.stats().entries, 1);
    }

    #[test]
    fn a_tile_larger_than_the_budget_is_refused_rather_than_emptying_the_cache() {
        let mut c = TileCache::new(150);
        c.insert(key(1, 0, 0), tile(100));
        c.insert(key(1, 1, 0), tile(500));
        assert!(c.get(&key(1, 0, 0)).is_some(), "existing tile must survive");
        assert!(c.get(&key(1, 1, 0)).is_none());
    }

    #[test]
    fn closing_a_document_frees_its_bitmaps() {
        let mut c = TileCache::new(10_000);
        c.insert(key(1, 0, 0), tile(100));
        c.insert(key(1, 1, 0), tile(100));
        c.insert(key(2, 0, 0), tile(100));
        assert_eq!(c.forget_document(1), 2);
        assert_eq!(c.stats().bytes, 100);
        assert!(c.get(&key(2, 0, 0)).is_some());
    }

    #[test]
    fn trimming_a_tab_keeps_its_previews() {
        let mut c = TileCache::new(10_000);
        let sharp = key(1, 0, 0);
        let preview = TileKey {
            kind: TileKind::Preview,
            ..sharp
        };
        c.insert(sharp, tile(1000));
        c.insert(preview, tile(50));
        assert_eq!(c.trim_document(1), 1);
        assert!(c.get(&sharp).is_none());
        assert!(c.get(&preview).is_some(), "the thumbnail must survive");
    }

    #[test]
    fn stats_report_what_the_status_bar_shows() {
        let mut c = TileCache::new(1000);
        c.insert(key(1, 0, 0), tile(100));
        let _ = c.get(&key(1, 0, 0));
        let _ = c.get(&key(1, 9, 0));
        let s = c.stats();
        assert_eq!((s.hits, s.misses, s.entries, s.bytes), (1, 1, 1, 100));
        assert_eq!(s.budget_bytes, 1000);
    }
}
