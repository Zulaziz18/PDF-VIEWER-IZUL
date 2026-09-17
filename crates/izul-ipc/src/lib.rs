//! IPC between the UI process and the sandboxed PDFium workers (SPEC 6).
//!
//! Two channels, because commands and pixels want opposite things:
//!
//! * the **command channel** ([`transport`], [`codec`], [`message`]) is a named
//!   pipe carrying small `postcard` frames, each tagged with a `request_id` and
//!   a `generation`;
//! * the **pixel channel** ([`shm`], [`ring`]) is a shared-memory ring of
//!   fixed-size tile slots that PDFium renders into directly, so a bitmap is
//!   written once and never copied on its way to the screen.
//!
//! Both sides of both channels are built from this crate, which is why the wire
//! format can afford to be non-self-describing.

// SPEC 0 bans `unwrap`, `expect` and `panic` in production code, and the
// workspace lints deny them. Test code is the exception on purpose: inside a
// test, `expect` *is* the failure report, and rewriting every assertion into
// error propagation would make the tests harder to read without making anything
// safer. The relaxation is scoped to `cfg(test)`, so it can never reach a
// shipped build.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod codec;
pub mod message;
pub mod ring;
pub mod shm;
pub mod transport;

pub use codec::{read_frame, write_frame, CodecError, MAX_FRAME_BYTES};
pub use message::{
    CharBoxWire, DocId, Envelope, ErrorKind, Generation, OutlineEntry, RenderQuality, Request,
    RequestEnvelope, RequestId, Response, ResponseEnvelope, SearchHitWire, SearchOptions, SlotRef,
    PROTOCOL_VERSION,
};
pub use ring::{region_bytes, ReadSlot, RingError, TileRing, WriteSlot, SLOT_BYTES, TILE_EDGE};
pub use shm::SharedRegion;
pub use transport::{connect, ChannelName, Listener};
