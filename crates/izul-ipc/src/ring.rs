//! Fixed-slot tile ring over shared memory (SPEC 6).
//!
//! The Phase 0 spike settled the shape of this. Rendering a full page as
//! 512x512 tiles costs 0.79x to 1.00x of rendering it in one call — clipping
//! lets PDFium skip work outside each tile, so tiling is free or slightly
//! cheaper. That means *every* render can be a tile, and the pixel channel
//! becomes a ring of identically sized slots instead of a variable-size
//! allocator. Bounded memory, no fragmentation, and cancellation granularity
//! that falls out for free.
//!
//! Layout of the region:
//!
//! ```text
//!   [ RingHeader ][ SlotHeader x slot_count ][ padding ][ slot data x slot_count ]
//! ```
//!
//! Coordination is by an atomic state word per slot. Only one side may touch a
//! slot's pixel bytes at a time, and which side that is follows from the state:
//!
//! ```text
//!   Free ---claim--> Writing ---publish--> Ready ---acquire--> Reading
//!     ^                                                            |
//!     +--------------------------- release ------------------------+
//! ```

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::message::SlotRef;

/// Edge length of a tile, in pixels.
///
/// 512 is what the SPEC names for high zoom, and the spike showed no reason to
/// use anything else lower down.
pub const TILE_EDGE: u32 = 512;

/// Bytes in one slot: a 512x512 BGRA tile is exactly 1 MiB.
pub const SLOT_BYTES: usize = (TILE_EDGE * TILE_EDGE) as usize * 4;

/// Identifies the layout so a stale region from a crashed worker of a different
/// build is rejected rather than misread.
const MAGIC: u32 = 0x495A_5552; // "IZUR"
const LAYOUT_VERSION: u32 = 1;

const STATE_FREE: u32 = 0;
const STATE_WRITING: u32 = 1;
const STATE_READY: u32 = 2;
const STATE_READING: u32 = 3;

#[repr(C, align(64))]
struct RingHeader {
    magic: AtomicU32,
    version: AtomicU32,
    slot_count: AtomicU32,
    slot_bytes: AtomicU32,
    /// Round-robin hint for the next claim. Purely an optimisation: correctness
    /// comes from the per-slot CAS, so a torn or stale value here only costs a
    /// little extra scanning.
    next_hint: AtomicU32,
}

#[repr(C, align(64))]
struct SlotHeader {
    state: AtomicU32,
    /// Bumped on every publish. A `SlotRef` whose epoch no longer matches
    /// describes a tile that has since been overwritten, so the reader can
    /// detect staleness instead of reading someone else's pixels.
    epoch: AtomicU32,
    width: AtomicU32,
    height: AtomicU32,
    stride: AtomicU32,
    page: AtomicU32,
    doc: AtomicU64,
    generation: AtomicU64,
}

const HEADER_BYTES: usize = std::mem::size_of::<RingHeader>();
const SLOT_HEADER_BYTES: usize = std::mem::size_of::<SlotHeader>();

/// Total region size needed for `slot_count` slots.
pub fn region_bytes(slot_count: u32) -> usize {
    let meta = HEADER_BYTES + SLOT_HEADER_BYTES * slot_count as usize;
    // Align the data area to 64 bytes so no tile's first scanline shares a cache
    // line with slot metadata the other process is spinning on.
    let data_start = meta.div_ceil(64) * 64;
    data_start + SLOT_BYTES * slot_count as usize
}

fn data_offset(slot_count: u32) -> usize {
    let meta = HEADER_BYTES + SLOT_HEADER_BYTES * slot_count as usize;
    meta.div_ceil(64) * 64
}

/// A view of the ring. Both processes construct one over the same region.
#[derive(Debug)]
pub struct TileRing {
    base: *mut u8,
    slot_count: u32,
    data_offset: usize,
}

// SAFETY: every access goes through atomics or through a slot the caller has
// exclusively claimed via the state machine, so the handle itself carries no
// thread affinity.
unsafe impl Send for TileRing {}
// SAFETY: as above — shared access is mediated by the per-slot atomic state.
unsafe impl Sync for TileRing {}

#[derive(Debug, thiserror::Error)]
pub enum RingError {
    #[error("wilayah terlalu kecil: butuh {needed} byte, tersedia {available}")]
    RegionTooSmall { needed: usize, available: usize },
    #[error("tata letak ring tidak dikenali (magic {magic:#x}, versi {version})")]
    BadLayout { magic: u32, version: u32 },
    #[error("tidak ada slot kosong")]
    NoFreeSlot,
    #[error("slot {slot} di luar jangkauan (ring punya {count})")]
    SlotOutOfRange { slot: u32, count: u32 },
    #[error("slot {slot} sudah ditimpa (epoch {expected} != {actual})")]
    Stale {
        slot: u32,
        expected: u32,
        actual: u32,
    },
    #[error("slot {slot} tidak dalam keadaan {expected}")]
    WrongState { slot: u32, expected: &'static str },
}

impl TileRing {
    /// Initialises a fresh ring in `region`. Called by the side that created the
    /// shared memory — the supervisor, so that a worker crash never leaves a
    /// half-initialised header behind.
    ///
    /// # Safety
    /// `base` must point at a zeroed, writable mapping of at least
    /// [`region_bytes(slot_count)`] bytes, and no other `TileRing` may be
    /// initialising the same region concurrently.
    pub unsafe fn initialise(
        base: *mut u8,
        region_len: usize,
        slot_count: u32,
    ) -> Result<Self, RingError> {
        let needed = region_bytes(slot_count);
        if region_len < needed {
            return Err(RingError::RegionTooSmall {
                needed,
                available: region_len,
            });
        }
        // SAFETY: the caller guarantees `base` is a writable mapping large
        // enough for the header, and `RingHeader` is `repr(C)` with only atomic
        // fields, so a zeroed region is already a valid instance of it.
        let header = unsafe { &*(base as *const RingHeader) };
        header.slot_count.store(slot_count, Ordering::Relaxed);
        header
            .slot_bytes
            .store(SLOT_BYTES as u32, Ordering::Relaxed);
        header.version.store(LAYOUT_VERSION, Ordering::Relaxed);
        header.next_hint.store(0, Ordering::Relaxed);
        // Magic last, with release ordering: a reader that sees the magic is
        // guaranteed to see every field written above it.
        header.magic.store(MAGIC, Ordering::Release);
        Ok(Self {
            base,
            slot_count,
            data_offset: data_offset(slot_count),
        })
    }

    /// Attaches to a ring another process initialised.
    ///
    /// # Safety
    /// `base` must point at a writable mapping of at least `region_len` bytes
    /// that stays mapped for the life of the returned `TileRing`.
    pub unsafe fn attach(base: *mut u8, region_len: usize) -> Result<Self, RingError> {
        if region_len < HEADER_BYTES {
            return Err(RingError::RegionTooSmall {
                needed: HEADER_BYTES,
                available: region_len,
            });
        }
        // SAFETY: as in `initialise`; the header is at offset 0.
        let header = unsafe { &*(base as *const RingHeader) };
        let magic = header.magic.load(Ordering::Acquire);
        let version = header.version.load(Ordering::Relaxed);
        if magic != MAGIC || version != LAYOUT_VERSION {
            return Err(RingError::BadLayout { magic, version });
        }
        let slot_count = header.slot_count.load(Ordering::Relaxed);
        let needed = region_bytes(slot_count);
        if region_len < needed {
            return Err(RingError::RegionTooSmall {
                needed,
                available: region_len,
            });
        }
        Ok(Self {
            base,
            slot_count,
            data_offset: data_offset(slot_count),
        })
    }

    pub fn slot_count(&self) -> u32 {
        self.slot_count
    }

    fn slot(&self, i: u32) -> Result<&SlotHeader, RingError> {
        if i >= self.slot_count {
            return Err(RingError::SlotOutOfRange {
                slot: i,
                count: self.slot_count,
            });
        }
        // SAFETY: `i` is bounds checked, and the slot header array starts
        // immediately after the ring header within the mapping the caller
        // guaranteed at construction.
        Ok(unsafe {
            &*(self.base.add(HEADER_BYTES + SLOT_HEADER_BYTES * i as usize) as *const SlotHeader)
        })
    }

    fn data_ptr(&self, i: u32) -> *mut u8 {
        // SAFETY: `i` is checked by every caller before reaching here, and the
        // data area was sized for `slot_count` slots at construction.
        unsafe { self.base.add(self.data_offset + SLOT_BYTES * i as usize) }
    }

    /// Claims a free slot for writing.
    ///
    /// Returns [`RingError::NoFreeSlot`] when every slot is occupied. The caller
    /// — the worker — treats that as backpressure and reclaims, rather than
    /// blocking a render thread on the UI's consumption rate.
    pub fn claim(&self) -> Result<WriteSlot<'_>, RingError> {
        // SAFETY: the header is at offset 0 of a mapping validated at construction.
        let header = unsafe { &*(self.base as *const RingHeader) };
        let start = header.next_hint.fetch_add(1, Ordering::Relaxed);
        for probe in 0..self.slot_count {
            let i = (start.wrapping_add(probe)) % self.slot_count;
            let slot = self.slot(i)?;
            if slot
                .state
                .compare_exchange(
                    STATE_FREE,
                    STATE_WRITING,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Ok(WriteSlot {
                    ring: self,
                    index: i,
                });
            }
        }
        Err(RingError::NoFreeSlot)
    }

    /// Claims a slot, reclaiming the oldest published-but-unread one if none is
    /// free.
    ///
    /// A `Ready` slot the UI has not taken is by definition a tile it has not
    /// drawn. When the ring is full, the newest tile is worth more than the
    /// oldest unread one — the user is looking at where they scrolled *to* —
    /// so overwriting is the right call, and the epoch bump means the UI can
    /// still tell that the old `SlotRef` went stale.
    pub fn claim_or_reclaim(&self) -> Result<WriteSlot<'_>, RingError> {
        if let Ok(slot) = self.claim() {
            return Ok(slot);
        }
        let mut oldest: Option<(u64, u32)> = None;
        for i in 0..self.slot_count {
            let slot = self.slot(i)?;
            if slot.state.load(Ordering::Acquire) == STATE_READY {
                let gen = slot.generation.load(Ordering::Relaxed);
                if oldest.is_none_or(|(g, _)| gen < g) {
                    oldest = Some((gen, i));
                }
            }
        }
        let (_, i) = oldest.ok_or(RingError::NoFreeSlot)?;
        let slot = self.slot(i)?;
        slot.state
            .compare_exchange(
                STATE_READY,
                STATE_WRITING,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .map_err(|_| RingError::NoFreeSlot)?;
        Ok(WriteSlot {
            ring: self,
            index: i,
        })
    }

    /// Takes a published slot for reading, verifying it is the tile the
    /// `SlotRef` describes and not a later one that reused the slot.
    pub fn acquire(&self, r: &SlotRef) -> Result<ReadSlot<'_>, RingError> {
        let slot = self.slot(r.slot)?;
        let epoch = slot.epoch.load(Ordering::Acquire);
        if epoch != r.epoch {
            return Err(RingError::Stale {
                slot: r.slot,
                expected: r.epoch,
                actual: epoch,
            });
        }
        slot.state
            .compare_exchange(
                STATE_READY,
                STATE_READING,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .map_err(|_| RingError::WrongState {
                slot: r.slot,
                expected: "Ready",
            })?;
        // Re-check after the claim: a reclaim could have won the race between
        // the epoch load and the CAS.
        let epoch_now = slot.epoch.load(Ordering::Acquire);
        if epoch_now != r.epoch {
            slot.state.store(STATE_READY, Ordering::Release);
            return Err(RingError::Stale {
                slot: r.slot,
                expected: r.epoch,
                actual: epoch_now,
            });
        }
        Ok(ReadSlot {
            ring: self,
            index: r.slot,
        })
    }
}

/// An exclusively claimed slot, writable until published.
#[derive(Debug)]
pub struct WriteSlot<'r> {
    ring: &'r TileRing,
    index: u32,
}

impl WriteSlot<'_> {
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The slot's pixel bytes.
    ///
    /// Exclusive for as long as this `WriteSlot` exists: the slot is in
    /// `Writing`, which no other participant will transition out of.
    pub fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: the slot is in `Writing`, claimed by this handle alone, so no
        // other process or thread may touch these bytes. The data area was sized
        // for `SLOT_BYTES` per slot at construction.
        unsafe { std::slice::from_raw_parts_mut(self.ring.data_ptr(self.index), SLOT_BYTES) }
    }

    /// Publishes the tile and returns the reference to send over the command
    /// channel.
    pub fn publish(
        self,
        doc: u64,
        page: u32,
        generation: u64,
        width: u32,
        height: u32,
        stride: u32,
    ) -> Result<SlotRef, RingError> {
        let slot = self.ring.slot(self.index)?;
        let epoch = slot.epoch.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        slot.width.store(width, Ordering::Relaxed);
        slot.height.store(height, Ordering::Relaxed);
        slot.stride.store(stride, Ordering::Relaxed);
        slot.page.store(page, Ordering::Relaxed);
        slot.doc.store(doc, Ordering::Relaxed);
        slot.generation.store(generation, Ordering::Relaxed);
        let index = self.index;
        let offset = (self.ring.data_offset + SLOT_BYTES * index as usize) as u64;
        // Release: a reader that sees `Ready` sees the pixels and the metadata.
        slot.state.store(STATE_READY, Ordering::Release);
        // Published: the slot now belongs to the reader, so `Drop` must not
        // hand it back to the free pool.
        std::mem::forget(self);
        Ok(SlotRef {
            slot: index,
            offset,
            stride,
            width,
            height,
            epoch,
        })
    }
}

impl Drop for WriteSlot<'_> {
    /// An abandoned claim — a cancelled render, or a panic caught at the FFI
    /// boundary — returns the slot to the pool rather than leaking it.
    fn drop(&mut self) {
        if let Ok(slot) = self.ring.slot(self.index) {
            slot.state.store(STATE_FREE, Ordering::Release);
        }
    }
}

/// A published slot, readable until released.
#[derive(Debug)]
pub struct ReadSlot<'r> {
    ring: &'r TileRing,
    index: u32,
}

impl ReadSlot<'_> {
    /// The tile's pixel bytes. Exclusive while this handle exists.
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: the slot is in `Reading`, claimed by this handle alone, so the
        // writer will not touch it until we release.
        unsafe { std::slice::from_raw_parts(self.ring.data_ptr(self.index), SLOT_BYTES) }
    }

    pub fn width(&self) -> u32 {
        self.ring
            .slot(self.index)
            .map(|s| s.width.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    pub fn height(&self) -> u32 {
        self.ring
            .slot(self.index)
            .map(|s| s.height.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    pub fn stride(&self) -> u32 {
        self.ring
            .slot(self.index)
            .map(|s| s.stride.load(Ordering::Relaxed))
            .unwrap_or(0)
    }
}

impl Drop for ReadSlot<'_> {
    fn drop(&mut self) {
        if let Ok(slot) = self.ring.slot(self.index) {
            slot.state.store(STATE_FREE, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shm::SharedRegion;

    fn ring(tag: &str, slots: u32) -> (SharedRegion, TileRing) {
        let name = format!(
            "ringtest-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let len = region_bytes(slots);
        let region = SharedRegion::create(&name, len).expect("create region");
        // SAFETY: freshly created, zeroed, correctly sized, and not shared yet.
        let ring = unsafe { TileRing::initialise(region.as_ptr(), len, slots) }.expect("init");
        (region, ring)
    }

    #[test]
    fn round_trip_one_tile() {
        let (_r, ring) = ring("roundtrip", 4);
        let mut w = ring.claim().expect("claim");
        w.bytes()[..4].copy_from_slice(&[1, 2, 3, 4]);
        let sref = w.publish(7, 3, 99, 512, 512, 2048).expect("publish");

        let read = ring.acquire(&sref).expect("acquire");
        assert_eq!(&read.bytes()[..4], &[1, 2, 3, 4]);
        assert_eq!(read.width(), 512);
        assert_eq!(read.stride(), 2048);
    }

    #[test]
    fn slot_is_reusable_after_release() {
        let (_r, ring) = ring("reuse", 1);
        for i in 0..5u8 {
            let mut w = ring.claim().expect("claim");
            w.bytes()[0] = i;
            let sref = w.publish(1, 0, i as u64, 512, 512, 2048).expect("publish");
            let read = ring.acquire(&sref).expect("acquire");
            assert_eq!(read.bytes()[0], i);
        }
    }

    #[test]
    fn full_ring_refuses_a_plain_claim() {
        let (_r, ring) = ring("full", 2);
        let a = ring.claim().expect("claim 1");
        let b = ring.claim().expect("claim 2");
        assert!(matches!(ring.claim(), Err(RingError::NoFreeSlot)));
        drop(a);
        drop(b);
    }

    #[test]
    fn abandoned_claim_returns_the_slot() {
        let (_r, ring) = ring("abandon", 1);
        drop(ring.claim().expect("claim"));
        assert!(
            ring.claim().is_ok(),
            "an abandoned claim must not leak the slot"
        );
    }

    #[test]
    fn reclaim_takes_the_oldest_unread_tile() {
        let (_r, ring) = ring("reclaim", 2);
        let mut w = ring.claim().expect("claim");
        w.bytes()[0] = 0xAA;
        let old = w
            .publish(1, 0, /* generation */ 1, 512, 512, 2048)
            .expect("publish");
        let mut w2 = ring.claim().expect("claim");
        w2.bytes()[0] = 0xBB;
        let newer = w2
            .publish(1, 1, /* generation */ 9, 512, 512, 2048)
            .expect("publish");

        // Ring is full of Ready slots; the next claim must take generation 1.
        let mut w3 = ring.claim_or_reclaim().expect("reclaim");
        assert_eq!(w3.index(), old.slot, "must reclaim the oldest generation");
        w3.bytes()[0] = 0xCC;
        let replacement = w3.publish(1, 2, 10, 512, 512, 2048).expect("publish");

        // The newer tile is untouched; the reclaimed one now reports stale.
        assert!(
            ring.acquire(&old).is_err(),
            "the overwritten SlotRef must not read as live"
        );
        assert_eq!(
            ring.acquire(&newer).expect("newer still live").bytes()[0],
            0xBB
        );
        assert_eq!(
            ring.acquire(&replacement).expect("replacement").bytes()[0],
            0xCC
        );
    }

    #[test]
    fn stale_slot_ref_is_rejected() {
        let (_r, ring) = ring("stale", 1);
        let w = ring.claim().expect("claim");
        let first = w.publish(1, 0, 1, 512, 512, 2048).expect("publish");
        drop(ring.acquire(&first).expect("acquire")); // consume, freeing the slot

        let w = ring.claim().expect("claim");
        let _second = w.publish(1, 1, 2, 512, 512, 2048).expect("publish");

        match ring.acquire(&first) {
            Err(RingError::Stale { .. }) => {}
            other => panic!("expected Stale, got {other:?}"),
        };
    }

    #[test]
    fn attach_reads_the_same_ring() {
        let name = format!(
            "attach-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let len = region_bytes(4);
        let a = SharedRegion::create(&name, len).expect("create");
        // SAFETY: fresh, zeroed, correctly sized region.
        let ring_a = unsafe { TileRing::initialise(a.as_ptr(), len, 4) }.expect("init");
        let b = SharedRegion::open(&name, len).expect("open");
        // SAFETY: `b` maps the same initialised region, and stays mapped here.
        let ring_b = unsafe { TileRing::attach(b.as_ptr(), len) }.expect("attach");
        assert_eq!(ring_b.slot_count(), 4);

        let mut w = ring_a.claim().expect("claim");
        w.bytes()[..3].copy_from_slice(&[9, 8, 7]);
        let sref = w.publish(42, 5, 1, 512, 512, 2048).expect("publish");
        assert_eq!(
            &ring_b.acquire(&sref).expect("acquire").bytes()[..3],
            &[9, 8, 7]
        );
    }

    #[test]
    fn attach_rejects_an_uninitialised_region() {
        let name = format!(
            "bad-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let len = region_bytes(2);
        let r = SharedRegion::create(&name, len).expect("create");
        // SAFETY: the region is mapped and correctly sized; it is simply not
        // initialised as a ring, which is exactly what we are asserting.
        match unsafe { TileRing::attach(r.as_ptr(), len) } {
            Err(RingError::BadLayout { .. }) => {}
            other => panic!("expected BadLayout, got {other:?}"),
        }
    }

    #[test]
    fn region_sizing_accounts_for_headers_and_alignment() {
        for slots in [1u32, 2, 7, 64, 256] {
            let need = region_bytes(slots);
            assert!(need >= SLOT_BYTES * slots as usize + HEADER_BYTES);
            assert_eq!(
                data_offset(slots) % 64,
                0,
                "data area must be cache-line aligned"
            );
        }
    }
}
