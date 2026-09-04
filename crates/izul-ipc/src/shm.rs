//! Platform shared memory for the pixel channel (SPEC 6).
//!
//! One primitive, two implementations: a named, zero-initialised region that
//! two processes can map by name. On Windows that is a pagefile-backed file
//! mapping, which is what the SPEC calls for and what avoids leaving a real file
//! behind. On Unix — which is where CI and day-to-day development run — it is a
//! POSIX shared-memory object, which has the same semantics.
//!
//! Nothing above this module knows which one it got.

use std::io;

/// A mapped shared-memory region.
///
/// The creator keeps the region alive; openers get a view of the same pages.
/// Dropping the last handle releases it on both platforms.
#[derive(Debug)]
pub struct SharedRegion {
    inner: platform::Region,
    name: String,
    len: usize,
}

impl SharedRegion {
    /// Creates a new region of `len` bytes, zero-initialised.
    ///
    /// `name` must be unique per region; the supervisor derives it from the
    /// worker's id and a process-unique counter so a restarted worker never
    /// collides with the corpse of its predecessor.
    pub fn create(name: &str, len: usize) -> io::Result<Self> {
        if len == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "shm len 0"));
        }
        Ok(Self {
            inner: platform::Region::create(name, len)?,
            name: name.to_string(),
            len,
        })
    }

    /// Opens an existing region created by another process.
    pub fn open(name: &str, len: usize) -> io::Result<Self> {
        Ok(Self {
            inner: platform::Region::open(name, len)?,
            name: name.to_string(),
            len,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Raw pointer to the first byte of the mapping.
    ///
    /// The mapping is shared and writable by both processes, so this is
    /// deliberately not exposed as a `&[u8]`: Rust's aliasing rules do not hold
    /// for memory another process can write. Everything above this module goes
    /// through [`crate::ring::TileRing`], which mediates access with atomic slot
    /// states so that only one side touches a given slot's bytes at a time.
    pub fn as_ptr(&self) -> *mut u8 {
        self.inner.as_ptr()
    }
}

// SAFETY: the region is a plain mapping with no thread affinity; ownership can
// move between threads. Concurrent *access* is not made safe by this — that is
// the ring's job — but moving the handle is.
unsafe impl Send for SharedRegion {}
// SAFETY: `as_ptr` hands out a raw pointer and makes no aliasing promise, so
// sharing the handle across threads adds no capability beyond what a raw
// pointer already allows.
unsafe impl Sync for SharedRegion {}

#[cfg(windows)]
mod platform {
    use std::io;

    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows::Win32::System::Memory::{
        CreateFileMappingW, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_ALL_ACCESS,
        MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
    };

    #[derive(Debug)]
    pub struct Region {
        handle: HANDLE,
        view: MEMORY_MAPPED_VIEW_ADDRESS,
    }

    /// Windows session-local namespace. `Local\` keeps the object invisible to
    /// other logon sessions, which is what we want for a sandboxed worker: the
    /// name is not a capability anyone else on the machine can reach.
    fn qualify(name: &str) -> HSTRING {
        HSTRING::from(format!("Local\\izul-{name}"))
    }

    impl Region {
        pub fn create(name: &str, len: usize) -> io::Result<Self> {
            let hi = (len as u64 >> 32) as u32;
            let lo = (len as u64 & 0xFFFF_FFFF) as u32;
            // SAFETY: INVALID_HANDLE_VALUE selects a pagefile-backed mapping, so
            // no file is created. The name is null-terminated by HSTRING.
            let handle = unsafe {
                CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    None,
                    PAGE_READWRITE,
                    hi,
                    lo,
                    &qualify(name),
                )
            }
            .map_err(io::Error::other)?;
            // SAFETY: `handle` is a live mapping of at least `len` bytes.
            let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, len) };
            if view.Value.is_null() {
                // SAFETY: closing a handle we own and are about to discard.
                let _ = unsafe { CloseHandle(handle) };
                return Err(io::Error::last_os_error());
            }
            Ok(Region { handle, view })
        }

        pub fn open(name: &str, len: usize) -> io::Result<Self> {
            // SAFETY: opening an existing named mapping; `false` means the
            // handle is not inherited by child processes.
            let handle = unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS.0, false, &qualify(name)) }
                .map_err(io::Error::other)?;
            // SAFETY: `handle` is a live mapping; the creator sized it to `len`.
            let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, len) };
            if view.Value.is_null() {
                // SAFETY: as above.
                let _ = unsafe { CloseHandle(handle) };
                return Err(io::Error::last_os_error());
            }
            Ok(Region { handle, view })
        }

        pub fn as_ptr(&self) -> *mut u8 {
            self.view.Value.cast()
        }
    }

    impl Drop for Region {
        fn drop(&mut self) {
            // SAFETY: both were produced by this type and are released once.
            unsafe {
                let _ = UnmapViewOfFile(self.view);
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::ffi::CString;
    use std::io;

    #[derive(Debug)]
    pub struct Region {
        ptr: *mut u8,
        len: usize,
        /// Only the creator unlinks the object, so a worker that opens a region
        /// cannot delete the supervisor's.
        unlink_name: Option<CString>,
    }

    fn qualify(name: &str) -> io::Result<CString> {
        // POSIX requires a leading slash and no others.
        CString::new(format!("/izul-{}", name.replace('/', "_")))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
    }

    fn map(fd: libc::c_int, len: usize) -> io::Result<*mut u8> {
        // SAFETY: `fd` is a live shared-memory descriptor sized to at least
        // `len` by the caller.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(ptr.cast())
    }

    impl Region {
        pub fn create(name: &str, len: usize) -> io::Result<Self> {
            let cname = qualify(name)?;
            // SAFETY: `cname` is a valid null-terminated C string that outlives
            // the call.
            let fd = unsafe {
                libc::shm_open(
                    cname.as_ptr(),
                    libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `fd` is a live descriptor we just created.
            if unsafe { libc::ftruncate(fd, len as libc::off_t) } < 0 {
                let err = io::Error::last_os_error();
                // SAFETY: cleaning up the descriptor and object we just made.
                unsafe {
                    libc::close(fd);
                    libc::shm_unlink(cname.as_ptr());
                }
                return Err(err);
            }
            let ptr = map(fd, len);
            // SAFETY: the mapping holds its own reference to the object, so the
            // descriptor is not needed once mmap has returned.
            unsafe { libc::close(fd) };
            match ptr {
                Ok(ptr) => Ok(Region {
                    ptr,
                    len,
                    unlink_name: Some(cname),
                }),
                Err(e) => {
                    // SAFETY: removing the object we created and cannot map.
                    unsafe { libc::shm_unlink(cname.as_ptr()) };
                    Err(e)
                }
            }
        }

        pub fn open(name: &str, len: usize) -> io::Result<Self> {
            let cname = qualify(name)?;
            // SAFETY: as in `create`; opening an existing object read-write.
            let fd = unsafe { libc::shm_open(cname.as_ptr(), libc::O_RDWR, 0o600) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let ptr = map(fd, len);
            // SAFETY: as in `create`.
            unsafe { libc::close(fd) };
            Ok(Region {
                ptr: ptr?,
                len,
                unlink_name: None,
            })
        }

        pub fn as_ptr(&self) -> *mut u8 {
            self.ptr
        }
    }

    impl Drop for Region {
        fn drop(&mut self) {
            // SAFETY: `ptr`/`len` are exactly what mmap returned, unmapped once.
            unsafe { libc::munmap(self.ptr.cast(), self.len) };
            if let Some(name) = &self.unlink_name {
                // SAFETY: `name` is a valid C string naming an object we created.
                unsafe { libc::shm_unlink(name.as_ptr()) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(tag: &str) -> String {
        format!(
            "test-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        )
    }

    #[test]
    fn create_open_and_share_bytes() {
        let name = unique("share");
        let a = SharedRegion::create(&name, 4096).expect("create");
        let b = SharedRegion::open(&name, 4096).expect("open");

        // SAFETY: single-threaded test; only this test touches these bytes, and
        // `a` and `b` map the same pages.
        unsafe {
            a.as_ptr().write(0xAB);
            a.as_ptr().add(4095).write(0xCD);
            assert_eq!(b.as_ptr().read(), 0xAB);
            assert_eq!(b.as_ptr().add(4095).read(), 0xCD);
        }
    }

    #[test]
    fn new_region_is_zeroed() {
        let name = unique("zero");
        let r = SharedRegion::create(&name, 8192).expect("create");
        // SAFETY: reading bytes of a region we just created and solely own.
        let all_zero = (0..8192).all(|i| unsafe { r.as_ptr().add(i).read() } == 0);
        assert!(all_zero, "a fresh mapping must be zero-filled");
    }

    #[test]
    fn zero_length_is_rejected() {
        assert!(SharedRegion::create(&unique("empty"), 0).is_err());
    }

    #[test]
    fn opening_a_missing_region_fails() {
        assert!(SharedRegion::open(&unique("absent"), 4096).is_err());
    }
}
