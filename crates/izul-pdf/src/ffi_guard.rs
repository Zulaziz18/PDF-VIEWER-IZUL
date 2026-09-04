use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::error::{PdfError, Result};

/// Runs `f` with a panic boundary around it.
///
/// Every call into PDFium goes through here. PDFium parses untrusted input; a
/// malformed file can drive it into a state where our own Rust wrapper hits a
/// bounds check or a `PdfiumError` we mapped with an assertion. Letting that
/// unwind through the FFI frames is undefined behaviour, and letting it reach
/// the worker's main loop kills a process that may be serving other documents.
///
/// The invariant that makes `AssertUnwindSafe` sound here: a caught panic is
/// never recovered from at the document level. The caller converts it into
/// `PdfError::EnginePanic`, which the supervisor treats as a strike against the
/// document (two strikes and it is quarantined per SPEC 3.4). No PDFium handle
/// touched by the panicking call is used again.
pub fn guard<T>(operation: &'static str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => {
            let detail = panic_message(&payload);
            tracing::error!(operation, detail, "PDFium boundary caught a panic");
            Err(PdfError::EnginePanic { operation })
        }
    }
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> &str {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "<payload bukan string>"
    }
}
