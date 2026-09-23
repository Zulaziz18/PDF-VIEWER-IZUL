//! Local state: sessions, recent files, reading positions, drafts, the
//! full-text index, and the thumbnail cache (SPEC 3.5, SPEC 7).
//!
//! Two databases, both in WAL mode. The split is deliberate: `cache.db` holds
//! only things that can be regenerated, so it can be deleted at any moment
//! without losing anything the user made, while `app.db` holds the things whose
//! loss would hurt.

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

pub mod db;
pub mod drafts;
pub mod error;
pub mod exports;
pub mod files;
pub mod identity;
pub mod prefs;
pub mod search;
pub mod sessions;

pub use db::{
    default_data_dir, open, open_memory, Which, APP_SCHEMA_VERSION, CACHE_SCHEMA_VERSION,
};
pub use error::{Result, StoreError};
pub use files::{FileId, FileRow, ReadingState, ViewMode};
pub use identity::{content_hash, FileStamp, PathHash};
pub use search::{Hit, IndexState};
pub use sessions::{SessionId, SessionTab, TabSlot};
