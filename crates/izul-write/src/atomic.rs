//! Replacing a file so that it is never half-written (SPEC 8: "simpan
//! atomik — tulis ke berkas sementara di volume yang sama → verifikasi bisa
//! dibuka ulang → ganti nama").
//!
//! The sequence, and what each step protects against:
//!
//! 1. **Write a temporary file beside the target** — same folder, hence same
//!    volume, so the final step is a rename and not a copy. The original is not
//!    opened for writing at any point.
//! 2. **Flush it to the disk** (`sync_all`). Without this, a power cut after
//!    the rename can leave the *new* name pointing at blocks the disk never
//!    received: a file of the right size full of zeros.
//! 3. **Verify** — the caller reopens the temporary file (the worker does, with
//!    PDFium) before anything is replaced.
//! 4. **Rename over the target** in one call. On NTFS and on every Unix file
//!    system a same-volume rename is atomic: an observer sees the old file or
//!    the new one, never a mixture. On Windows it is `MoveFileExW` with
//!    `MOVEFILE_WRITE_THROUGH`, which does not return until the rename is on
//!    disk; on Unix the folder is flushed afterwards for the same reason.
//!
//! Power lost before step 4: the original is untouched and a stray temporary
//! file remains, which [`sweep_stale`] removes on the next save into that
//! folder. Power lost after step 4: the new file is complete, because step 2
//! finished before step 4 began.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Every temporary file this module creates starts with this.
pub const TEMP_PREFIX: &str = ".izul-simpan-";
const TEMP_SUFFIX: &str = ".tmp";

/// A written, flushed, not yet committed replacement for a file.
///
/// Dropping it without [`PendingWrite::commit`] deletes the temporary file and
/// leaves the target exactly as it was.
#[derive(Debug)]
pub struct PendingWrite {
    temp: PathBuf,
    target: PathBuf,
    committed: bool,
}

fn temp_beside(target: &Path) -> io::Result<PathBuf> {
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok(dir.join(format!(
        "{TEMP_PREFIX}{}-{nonce}{TEMP_SUFFIX}",
        std::process::id()
    )))
}

impl PendingWrite {
    /// Steps 1 and 2: writes `bytes` to a temporary file beside `target` and
    /// flushes it to disk.
    pub fn write(target: &Path, bytes: &[u8]) -> io::Result<Self> {
        Self::write_with(target, |file| file.write_all(bytes))
    }

    /// As [`PendingWrite::write`], with the writing done by `fill`. Split out
    /// so a test can fail in the middle of a write, which is what a full disk
    /// or a yanked USB stick does.
    pub fn write_with(
        target: &Path,
        fill: impl FnOnce(&mut File) -> io::Result<()>,
    ) -> io::Result<Self> {
        let temp = temp_beside(target)?;
        let pending = PendingWrite {
            temp: temp.clone(),
            target: target.to_path_buf(),
            committed: false,
        };
        // `create_new`: never follow or reuse something that already sits at
        // the temporary name.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        fill(&mut file)?;
        file.sync_all()?;
        drop(file);
        Ok(pending)
    }

    /// The temporary file, for the caller to verify before committing.
    pub fn temp_path(&self) -> &Path {
        &self.temp
    }

    /// Step 4: atomically replaces the target with the temporary file.
    pub fn commit(mut self) -> io::Result<()> {
        replace(&self.temp, &self.target)?;
        self.committed = true;
        sync_parent(&self.target);
        Ok(())
    }
}

impl Drop for PendingWrite {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

#[cfg(windows)]
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let wide = |p: &Path| -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let (a, b) = (wide(from), wide(to));
    // SAFETY: both buffers are NUL-terminated UTF-16 strings that outlive the
    // call; `MoveFileExW` only reads them.
    unsafe {
        MoveFileExW(
            PCWSTR(a.as_ptr()),
            PCWSTR(b.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| io::Error::from_raw_os_error(e.code().0 & 0xFFFF))
}

#[cfg(not(windows))]
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// Makes the rename itself durable on Unix, where a rename lives in the
/// folder's own metadata. Windows has no folder handle to flush; there
/// `MOVEFILE_WRITE_THROUGH` did the equivalent.
fn sync_parent(target: &Path) {
    #[cfg(unix)]
    if let Some(dir) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = target;
}

/// Removes temporary files an interrupted save left in `dir`, older than
/// `min_age` — old enough that no save still running could own them.
pub fn sweep_stale(dir: &Path, min_age: Duration) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(TEMP_PREFIX) && name.ends_with(TEMP_SUFFIX)) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age >= min_age);
        if old && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("izul-atomic-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn temps(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(TEMP_PREFIX))
            .collect()
    }

    #[test]
    fn a_committed_write_replaces_the_file_and_leaves_nothing_behind() {
        let dir = scratch("commit");
        let target = dir.join("dok.pdf");
        fs::write(&target, b"lama").unwrap();
        let pending = PendingWrite::write(&target, b"baru").unwrap();
        assert_eq!(fs::read(pending.temp_path()).unwrap(), b"baru");
        assert_eq!(
            fs::read(&target).unwrap(),
            b"lama",
            "nothing replaced before commit"
        );
        pending.commit().unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"baru");
        assert!(temps(&dir).is_empty());
    }

    #[test]
    fn abandoning_after_verification_fails_keeps_the_original() {
        let dir = scratch("abandon");
        let target = dir.join("dok.pdf");
        fs::write(&target, b"asli").unwrap();
        let pending = PendingWrite::write(&target, b"rusak").unwrap();
        drop(pending); // the verification said no
        assert_eq!(fs::read(&target).unwrap(), b"asli");
        assert!(temps(&dir).is_empty());
    }

    #[test]
    fn a_write_that_dies_halfway_never_touches_the_original() {
        // What a full disk or a power cut in the middle of writing looks like
        // from here: the first half goes out, then the write fails.
        let dir = scratch("halfway");
        let target = dir.join("dok.pdf");
        fs::write(&target, b"asli yang utuh").unwrap();
        let result = PendingWrite::write_with(&target, |f| {
            f.write_all(b"separuh")?;
            Err(io::Error::other("disk penuh"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"asli yang utuh");
        assert!(temps(&dir).is_empty(), "the half-written temp is removed");
    }

    #[test]
    fn a_new_file_can_be_created_the_same_way() {
        let dir = scratch("fresh");
        let target = dir.join("baru.pdf");
        PendingWrite::write(&target, b"isi")
            .unwrap()
            .commit()
            .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"isi");
    }

    #[test]
    fn stale_temporaries_are_swept_and_fresh_ones_are_not() {
        let dir = scratch("sweep");
        let stale = dir.join(format!("{TEMP_PREFIX}1-1{TEMP_SUFFIX}"));
        fs::write(&stale, b"x").unwrap();
        let unrelated = dir.join("catatan.tmp");
        fs::write(&unrelated, b"x").unwrap();
        assert_eq!(sweep_stale(&dir, Duration::from_secs(3600)), 0, "too young");
        assert_eq!(sweep_stale(&dir, Duration::ZERO), 1);
        assert!(!stale.exists());
        assert!(unrelated.exists(), "only our own temporaries are touched");
    }
}
