//! Recognising a file across moves and edits (SPEC 7).
//!
//! Three facts are kept about every file: a hash of its path, its size, and its
//! mtime. Together they answer the two questions the app keeps needing to ask.
//!
//! * *Is this the same file I had open?* — the path hash.
//! * *Has it changed since I last looked?* — size and mtime.
//!
//! The second question is why a draft carries `base_hash`: replaying recorded
//! operations onto a document that has changed underneath them would corrupt it
//! silently, which is worse than losing the draft.

use std::path::Path;

/// Stable identity for a filesystem path.
///
/// The path is normalised before hashing: Windows compares paths
/// case-insensitively, so `C:\Docs\A.pdf` and `c:\docs\a.pdf` must map to one
/// row in `files`. Unix is case-sensitive, so no folding happens there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathHash(pub [u8; 32]);

impl PathHash {
    pub fn of(path: &Path) -> Self {
        let s = path.to_string_lossy();
        let normalised = normalise(&s);
        PathHash(blake3::hash(normalised.as_bytes()).into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn from_slice(b: &[u8]) -> Option<Self> {
        <[u8; 32]>::try_from(b).ok().map(PathHash)
    }
}

fn normalise(path: &str) -> String {
    let unified = path.replace('\\', "/");
    if cfg!(windows) {
        unified.to_lowercase()
    } else {
        unified
    }
}

/// Size and modification time, as observed when the file was last read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    /// Seconds since the Unix epoch. Second resolution is deliberate: some
    /// filesystems and network shares do not preserve anything finer, and a
    /// stamp that spuriously differs would invalidate every cache entry.
    pub mtime: i64,
}

impl FileStamp {
    pub fn of(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Ok(FileStamp {
            size: meta.len(),
            mtime,
        })
    }

    /// True when the file on disk no longer matches what we recorded.
    pub fn differs_from(&self, other: &FileStamp) -> bool {
        self != other
    }
}

/// Hash of a document's bytes, used as `drafts.base_hash`.
pub fn content_hash(bytes: &[u8]) -> [u8; 32] {
    blake3::hash(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn separator_style_does_not_change_identity() {
        let a = PathHash::of(&PathBuf::from(r"C:\Docs\Report.pdf"));
        let b = PathHash::of(&PathBuf::from("C:/Docs/Report.pdf"));
        assert_eq!(a, b);
    }

    #[test]
    fn different_paths_hash_differently() {
        assert_ne!(
            PathHash::of(&PathBuf::from("/a/report.pdf")),
            PathHash::of(&PathBuf::from("/b/report.pdf"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_folds_case() {
        assert_eq!(
            PathHash::of(&PathBuf::from(r"C:\Docs\A.pdf")),
            PathHash::of(&PathBuf::from(r"c:\docs\a.pdf"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_keeps_case_significant() {
        assert_ne!(
            PathHash::of(&PathBuf::from("/docs/A.pdf")),
            PathHash::of(&PathBuf::from("/docs/a.pdf"))
        );
    }

    #[test]
    fn hash_round_trips_through_bytes() {
        let h = PathHash::of(&PathBuf::from("/x/y.pdf"));
        assert_eq!(PathHash::from_slice(h.as_bytes()), Some(h));
        assert_eq!(PathHash::from_slice(&[0u8; 8]), None);
    }

    #[test]
    fn a_changed_stamp_is_detected() {
        let a = FileStamp {
            size: 100,
            mtime: 5,
        };
        assert!(!a.differs_from(&FileStamp {
            size: 100,
            mtime: 5
        }));
        assert!(a.differs_from(&FileStamp {
            size: 101,
            mtime: 5
        }));
        assert!(a.differs_from(&FileStamp {
            size: 100,
            mtime: 6
        }));
    }

    #[test]
    fn content_hash_distinguishes_documents() {
        assert_ne!(content_hash(b"%PDF-1.7 a"), content_hash(b"%PDF-1.7 b"));
        assert_eq!(content_hash(b"same"), content_hash(b"same"));
    }
}
