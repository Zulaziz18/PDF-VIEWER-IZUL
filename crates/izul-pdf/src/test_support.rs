//! Scaffolding shared by this crate's tests.
//!
//! It exists for two reasons, both of them correctness rather than convenience.
//!
//! **PDFium may be initialised once per process**, and `Engine::load_from`
//! enforces that by refusing the second call. Every test module that kept its
//! own `OnceLock<Engine>` therefore worked alone and lost the race when run
//! beside another, leaving its tests to skip *silently* — a suite that reports
//! success while proving nothing. One engine for the whole binary removes the
//! race rather than documenting it.
//!
//! **PDFium's `FPDF_*` entry points share process-global state.** The shipped
//! application honours that: `izul-worker` runs a current-thread Tokio runtime
//! precisely so rendering never happens on two threads, and [`crate::Document`]
//! holds a `RefCell`, so the type system stops it being shared. `cargo test`
//! honours nothing of the sort — it runs test functions on a thread pool — so
//! any test that holds a document must hold [`pdfium_lock`] while it does.
//! Without it the suite dies with SIGSEGV once enough tests touch documents at
//! the same time, which is how this module came to be written.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::engine::Engine;

/// Repository root, for reaching `vendor/` and `test-fixtures/`.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The one engine this test binary is allowed to have.
///
/// `None` when PDFium has not been vendored, which is what lets a docs-only
/// change be tested without running `vendor/pdfium/fetch.sh` first.
pub fn engine() -> Option<&'static Engine> {
    static ENGINE: OnceLock<Option<&'static Engine>> = OnceLock::new();
    *ENGINE.get_or_init(|| {
        let lib = if cfg!(windows) {
            root().join("vendor/pdfium/win-x64/bin/pdfium.dll")
        } else {
            root().join("vendor/pdfium/linux-x64/lib/libpdfium.so")
        };
        lib.exists().then(|| Engine::load_from(&lib).ok()).flatten()
    })
}

/// Serialises PDFium use across the test binary's threads.
///
/// Hold it for as long as a [`crate::Document`] is alive. Poisoning is ignored
/// on purpose: one failed assertion should report itself, not turn every later
/// test into a lock error that hides it.
pub fn pdfium_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Skips rather than fails when a prerequisite has not been fetched — unless the
/// caller insisted it be there, which is what CI does. A test that silently
/// passes because its fixture is missing guards nothing, so the escape hatch
/// exists only to keep `vendor/pdfium/fetch.sh` optional on a developer machine.
#[macro_export]
macro_rules! require_fixture {
    ($what:expr, $missing:expr) => {
        if $missing {
            if std::env::var_os("IZUL_REQUIRE_FIXTURES").is_some() {
                panic!(concat!($what, " tidak ada dan IZUL_REQUIRE_FIXTURES diset"));
            }
            eprintln!(concat!("LEWATI: ", $what, " belum diambil"));
            return;
        }
    };
}

/// The engine, or an early return, plus the lock that makes using it safe.
///
/// Returns the guard so the caller binds it: `let (engine, _pdfium) = ...`.
/// Dropping it immediately would defeat the point, which is why it is handed
/// back rather than taken inside.
#[macro_export]
macro_rules! engine_and_lock {
    () => {{
        $crate::require_fixture!("PDFium", $crate::test_support::engine().is_none());
        match $crate::test_support::engine() {
            Some(e) => (e, $crate::test_support::pdfium_lock()),
            None => return,
        }
    }};
}

/// Path to the ten-page viewer fixture, or an early return.
///
/// `bench/make_fixtures.py viewer` builds it: a bookmark tree, pages that are
/// not all one size, one page carrying its own `/Rotate`, and a heading reading
/// `Bagian N — halaman M` at the top of every page.
#[macro_export]
macro_rules! viewer_fixture {
    () => {{
        let path = $crate::test_support::root().join("test-fixtures/viewer-10p.pdf");
        $crate::require_fixture!("test-fixtures/viewer-10p.pdf", !path.exists());
        path
    }};
}
