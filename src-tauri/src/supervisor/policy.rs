//! Pool sizing, restart backoff, and the poison rule (SPEC 3.4).
//!
//! Pure decision logic, deliberately separated from the process plumbing. These
//! are the rules that are easy to get subtly wrong and impossible to test if
//! they only exist inline in a spawn loop.

use std::time::Duration;

/// Number of worker processes.
///
/// `min(cores / 2, 8)`, floor 2. Half the cores because each worker renders on
/// one thread and the UI process still needs room to composite; capped at 8
/// because PDFium's own memory per process is the binding constraint well
/// before core count is.
pub fn pool_size(logical_cores: usize) -> usize {
    (logical_cores / 2).clamp(2, 8)
}

/// Heartbeat interval. The supervisor pings every worker this often.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);

/// A worker silent for longer than this is considered hung and is killed.
///
/// Three missed beats rather than two: a single long render on a pathological
/// page can legitimately block the worker's loop, and killing a worker that was
/// about to answer costs the user more than waiting one more interval.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(6);

/// Strikes before a document is quarantined (SPEC 3.4).
pub const POISON_THRESHOLD: u32 = 2;

/// Delay before restarting a worker that died, by consecutive failure count.
///
/// Exponential with a ceiling. The ceiling matters: a worker that dies because
/// the machine is out of memory will keep dying, and an unbounded backoff would
/// leave the user's tabs dead forever with no explanation.
pub fn restart_backoff(consecutive_failures: u32) -> Duration {
    const BASE_MS: u64 = 250;
    const CEILING: Duration = Duration::from_secs(30);
    let shift = consecutive_failures.min(8);
    Duration::from_millis(BASE_MS.saturating_mul(1u64 << shift)).min(CEILING)
}

/// Memory ceiling for one worker's Job Object.
///
/// Generous, because a legitimately large document plus PDFium's image cache can
/// run to hundreds of megabytes, and a worker killed mid-render looks to the
/// user like a crash. The point of the cap is to stop a hostile file from
/// exhausting the machine, not to run workers lean.
pub const WORKER_MEMORY_CAP_BYTES: u64 = 3 * 1024 * 1024 * 1024;

/// Tile slots per worker ring. 256 slots of 1 MiB each is 256 MiB of shared
/// memory per worker: enough for several screens of tiles in flight, bounded so
/// eight workers cannot claim more than 2 GiB between them.
pub const RING_SLOTS: u32 = 256;

/// Where a document should be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Share a worker with other documents, chosen by load.
    Pooled(usize),
    /// Alone, read-only, with a bounded image cache. For quarantined documents.
    Isolated,
}

/// Picks the worker for a new document.
///
/// Least-loaded wins, except that the worker serving the document the user is
/// actively looking at is avoided when there is any alternative: SPEC 3.4 asks
/// for the active document not to share a worker with heavy background work,
/// and the cheapest way to honour that is to not pile onto its worker.
pub fn place(loads: &[usize], active_worker: Option<usize>, poisoned: bool) -> Placement {
    if poisoned {
        return Placement::Isolated;
    }
    if loads.is_empty() {
        return Placement::Isolated;
    }
    let best_avoiding_active = loads
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != active_worker)
        .min_by_key(|(i, load)| (**load, *i));
    let chosen = best_avoiding_active.or_else(|| {
        loads
            .iter()
            .enumerate()
            .min_by_key(|(i, load)| (**load, *i))
    });
    match chosen {
        Some((i, _)) => Placement::Pooled(i),
        None => Placement::Isolated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_size_is_half_the_cores_within_bounds() {
        assert_eq!(pool_size(1), 2, "floor of 2 even on a single core");
        assert_eq!(pool_size(2), 2);
        assert_eq!(pool_size(4), 2);
        assert_eq!(pool_size(8), 4);
        assert_eq!(pool_size(16), 8);
        assert_eq!(pool_size(64), 8, "capped at 8");
    }

    #[test]
    fn backoff_grows_then_stops_growing() {
        let a = restart_backoff(0);
        let b = restart_backoff(1);
        let c = restart_backoff(2);
        assert!(a < b && b < c, "{a:?} {b:?} {c:?}");
        assert_eq!(
            restart_backoff(20),
            Duration::from_secs(30),
            "must reach a ceiling"
        );
        assert!(
            restart_backoff(100) <= Duration::from_secs(30),
            "and never exceed it"
        );
    }

    #[test]
    fn backoff_starts_short_enough_to_feel_instant() {
        assert!(restart_backoff(0) <= Duration::from_millis(500));
    }

    #[test]
    fn heartbeat_timeout_allows_at_least_two_missed_beats() {
        assert!(HEARTBEAT_TIMEOUT >= HEARTBEAT_INTERVAL * 3);
    }

    #[test]
    fn a_poisoned_document_is_always_isolated() {
        assert_eq!(place(&[0, 0, 0], None, true), Placement::Isolated);
        assert_eq!(place(&[5, 1, 9], Some(1), true), Placement::Isolated);
    }

    #[test]
    fn placement_picks_the_least_loaded_worker() {
        assert_eq!(place(&[3, 1, 7], None, false), Placement::Pooled(1));
    }

    #[test]
    fn placement_avoids_the_worker_serving_the_active_document() {
        // Worker 1 is least loaded but is serving what the user is looking at.
        assert_eq!(place(&[3, 1, 2], Some(1), false), Placement::Pooled(2));
    }

    #[test]
    fn placement_uses_the_active_worker_when_it_is_the_only_one() {
        assert_eq!(place(&[4], Some(0), false), Placement::Pooled(0));
    }

    #[test]
    fn placement_breaks_ties_deterministically() {
        assert_eq!(place(&[2, 2, 2], None, false), Placement::Pooled(0));
    }

    #[test]
    fn an_empty_pool_forces_isolation() {
        assert_eq!(place(&[], None, false), Placement::Isolated);
    }
}
