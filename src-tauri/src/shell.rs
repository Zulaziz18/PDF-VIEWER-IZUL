//! The taskbar jump list (SPEC 11.3, "jump list taskbar"; Phase 8).
//!
//! Windows builds the "Terbaru" category of an application's jump list from
//! the shell's recent documents for the file types it is registered to open
//! — which the installer does for `.pdf` (`bundle.fileAssociations`). Telling
//! the shell about each opened file is therefore the whole of it: one call,
//! no jump-list COM of our own to keep in step with our recent list.
//!
//! Not in the portable copy: it is meant to leave nothing behind on a
//! machine it was run from, and the shell's recent list is on that machine.

use std::path::Path;

/// Records `path` as recently opened, for the jump list.
pub fn add_to_recent(path: &Path) {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    if izul_store::is_portable(exe_dir.as_deref()) {
        return;
    }
    add(path);
}

#[cfg(windows)]
fn add(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{SHAddToRecentDocs, SHARD_PATHW};
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that lives until the call
    // returns; the shell copies what it keeps.
    unsafe { SHAddToRecentDocs(SHARD_PATHW.0 as u32, Some(wide.as_ptr().cast())) };
}

#[cfg(not(windows))]
fn add(_path: &Path) {}
