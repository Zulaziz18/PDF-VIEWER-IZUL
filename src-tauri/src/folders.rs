//! Local folders for the home screen's "PC Ini / Desktop / Dokumen" navigation
//! (SPEC 12, revised 2026-09-23).
//!
//! Only what a file picker would show: sub-folders and PDF files, nothing
//! hidden, nothing read beyond the directory listing and each entry's
//! metadata. Opening a file from here goes through `open_document` like every
//! other path, so this module grants no access the picker did not already
//! have — it just draws the same listing inside the window.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One row of a folder listing.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FolderEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// Bytes; `None` for folders.
    pub size: Option<u64>,
    /// Seconds since the Unix epoch, like every other timestamp the store keeps.
    pub modified: Option<i64>,
}

/// A place the navigation offers without the user having to find it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct KnownFolder {
    /// `desktop`, `documents`, `downloads` or `drive`.
    pub kind: &'static str,
    pub path: String,
}

/// Caps a listing so that opening a folder of fifty thousand files cannot
/// stall the window. The home screen is navigation, not a file manager.
pub const MAX_ENTRIES: usize = 2000;

fn is_hidden(path: &Path, name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN: u32 = 0x2;
        const SYSTEM: u32 = 0x4;
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            return meta.file_attributes() & (HIDDEN | SYSTEM) != 0;
        }
    }
    #[cfg(not(windows))]
    let _ = path;
    false
}

fn seconds(time: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    time.ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// Sub-folders and PDFs directly inside `dir`: folders first, then files, each
/// group by name without regard to case — the order Explorer uses.
pub fn list(dir: &Path) -> std::io::Result<Vec<FolderEntry>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if is_hidden(&path, &name) {
            continue;
        }
        // Follows links deliberately: a shortcut to a folder of PDFs on another
        // drive is exactly what a user expects to be able to walk into.
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let is_pdf = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        let wanted = meta.is_dir() || (meta.is_file() && is_pdf);
        if !wanted {
            continue;
        }
        out.push(FolderEntry {
            name,
            path: path.to_string_lossy().to_string(),
            is_dir: meta.is_dir(),
            size: meta.is_file().then_some(meta.len()),
            modified: seconds(meta.modified()),
        });
        if out.len() >= MAX_ENTRIES {
            break;
        }
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

/// The drives that exist on this machine, `C:\` and so on.
///
/// Probing each letter is what Explorer's own "This PC" amounts to without a
/// COM call. A and B are skipped: they are floppy letters, and probing an
/// empty floppy controller can take seconds.
pub fn drives() -> Vec<PathBuf> {
    if cfg!(windows) {
        (b'C'..=b'Z')
            .map(|letter| PathBuf::from(format!("{}:\\", letter as char)))
            .filter(|p| p.exists())
            .collect()
    } else {
        vec![PathBuf::from("/")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("izul-folders-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn lists_folders_first_then_pdfs_and_nothing_else() {
        let dir = scratch("order");
        std::fs::create_dir(dir.join("zeta")).expect("dir");
        std::fs::create_dir(dir.join("Alpha")).expect("dir");
        std::fs::write(dir.join("b.PDF"), b"%PDF-1.7").expect("file");
        std::fs::write(dir.join("a.pdf"), b"%PDF-1.7\n").expect("file");
        std::fs::write(dir.join("notes.txt"), b"x").expect("file");
        std::fs::write(dir.join(".hidden.pdf"), b"x").expect("file");

        let names: Vec<_> = list(&dir)
            .expect("list")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["Alpha", "zeta", "a.pdf", "b.PDF"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_carry_size_and_time_and_folders_carry_no_size() {
        let dir = scratch("meta");
        std::fs::create_dir(dir.join("sub")).expect("dir");
        std::fs::write(dir.join("doc.pdf"), b"%PDF-1.7\n").expect("file");
        let entries = list(&dir).expect("list");
        let folder = entries.iter().find(|e| e.is_dir).expect("folder");
        let file = entries.iter().find(|e| !e.is_dir).expect("file");
        assert_eq!(folder.size, None);
        assert_eq!(file.size, Some(9));
        assert!(
            file.modified.is_some_and(|m| m > 1_600_000_000),
            "{:?}",
            file.modified
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_an_empty_list() {
        let dir = scratch("missing").join("nope");
        assert!(list(&dir).is_err());
    }

    #[test]
    fn there_is_always_at_least_one_drive() {
        assert!(!drives().is_empty());
    }
}
