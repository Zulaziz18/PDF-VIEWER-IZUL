//! The render pipeline in the UI process (SPEC 9).
//!
//! Two parts, in the order a tile passes through them:
//!
//! * [`cache`] — an LRU of rendered bitmaps with a byte budget, keyed by
//!   content, so scrolling back is free and a zoom does not throw away what it
//!   just built;
//! * [`scheduler`] — priority, coalescing and cancellation between the
//!   viewport's requests and the workers.
//!
//! The bitmaps deliberately live on this side rather than in the workers: they
//! survive a worker crash, they are shared across documents that landed on
//! different workers, and only this side knows what is on screen.
//!
//! ## Why this is a crate and not a module of the UI process
//!
//! SPEC 5 lists the crates, and this is not one of them — the deviation is
//! deliberate and reported. The pipeline is where the performance targets in
//! SPEC 13 are met or missed, and a phase that has to *prove* them with
//! benchmarks needs to drive the real cache, the real priority queue and the
//! real cancellation from a harness, not from inside a windowed application.
//! Keeping it inside `src-tauri` would have made the benchmark either a
//! re-implementation of the thing being measured, or impossible. Everything
//! Tauri-shaped stays in the UI process: this crate reaches a worker through
//! the [`scheduler::TileBackend`] trait and knows nothing else about its host.

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

pub mod cache;
pub mod scheduler;

pub use cache::{CacheStats, CachedTile, TileCache, TileKey, TileKind};
pub use scheduler::{
    tile_source, DocInfo, Priority, RenderError, RenderService, RenderStats, TileBackend, TILE_EDGE,
};

/// Default bitmap cache budget: 2 GiB, as SPEC 9 names.
pub const DEFAULT_CACHE_BUDGET_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// Reads the cache budget, in bytes.
///
/// SPEC 9 asks for the budget to be configurable. `IZUL_TILE_CACHE_MB` is the
/// override; the preference stored in `app.db` is what the settings UI will
/// write in Phase 8. An unparseable or absurd value falls back to the default
/// rather than leaving the cache in a state nobody intended.
pub fn cache_budget_bytes(pref_mb: Option<u64>) -> usize {
    let from_env = std::env::var("IZUL_TILE_CACHE_MB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    match from_env.or(pref_mb) {
        // 64 MiB is roughly one screen of tiles plus previews: below it the
        // cache stops being a cache. 16 GiB is past any machine we target.
        Some(mb) if (64..=16_384).contains(&mb) => mb as usize * 1024 * 1024,
        Some(mb) => {
            tracing::warn!(mb, "anggaran cache di luar jangkauan, memakai bawaan");
            DEFAULT_CACHE_BUDGET_BYTES
        }
        None => DEFAULT_CACHE_BUDGET_BYTES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_budget_is_the_two_gigabytes_the_spec_names() {
        assert_eq!(DEFAULT_CACHE_BUDGET_BYTES, 2 * 1024 * 1024 * 1024);
        assert_eq!(cache_budget_bytes(None), DEFAULT_CACHE_BUDGET_BYTES);
    }

    #[test]
    fn a_stored_preference_is_honoured() {
        assert_eq!(cache_budget_bytes(Some(512)), 512 * 1024 * 1024);
    }

    #[test]
    fn an_absurd_preference_falls_back_rather_than_crippling_the_cache() {
        assert_eq!(cache_budget_bytes(Some(1)), DEFAULT_CACHE_BUDGET_BYTES);
        assert_eq!(
            cache_budget_bytes(Some(1_000_000)),
            DEFAULT_CACHE_BUDGET_BYTES
        );
    }
}
