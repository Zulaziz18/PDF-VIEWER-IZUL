use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use pdfium_render::prelude::{Pdfium, PdfiumLibraryBindings, FPDF_DOCUMENT, FPDF_PAGE, FS_SIZEF};

use crate::error::{PdfError, Result};
use crate::ffi_guard::guard;
use crate::geom::PageSize;

/// Process-wide PDFium instance.
///
/// We take `pdfium-render` for its dynamic bindings and its ABI-versioned
/// declarations, but we own the `FPDF_DOCUMENT` and `FPDF_PAGE` handles
/// ourselves rather than going through its `PdfDocument`/`PdfPage` wrappers.
/// The reason is concrete: the viewport needs `FPDF_RenderPageBitmapWithMatrix`
/// against a bitmap created with `FPDFBitmap_CreateEx` over a slice of shared
/// memory. The safe wrapper exposes neither — it allocates a fresh `Vec` per
/// render and offers no clip rectangle — so tiles and the zero-copy pixel path
/// (SPEC 6, SPEC 9) are both unreachable through it.
///
/// Threading: PDFium's `FPDF_*` calls are not safe to make concurrently, so
/// `Engine` and [`Document`] are deliberately neither `Send` nor `Sync`. Each
/// worker process owns one `Engine` on one dedicated PDFium thread, and
/// parallelism comes from the worker pool (SPEC 3.4), not from threads inside a
/// worker.
pub struct Engine {
    bindings: Box<dyn PdfiumLibraryBindings>,
    library_path: PathBuf,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("library_path", &self.library_path)
            .finish_non_exhaustive()
    }
}

/// `FPDF_InitLibrary` must be called exactly once per process, and PDFium has no
/// way to report that it was called twice — it simply corrupts its global state.
static LIBRARY_INITIALISED: AtomicBool = AtomicBool::new(false);

impl Engine {
    /// Loads PDFium from an explicit path and initialises the library.
    ///
    /// Returns a `&'static Engine`: a worker process creates exactly one and
    /// keeps it until exit, and every [`Document`] borrows it for the process
    /// lifetime. Leaking one allocation per process is the price of not making
    /// every document handle carry a lifetime parameter through the IPC layer.
    pub fn load_from(library_path: impl AsRef<Path>) -> Result<&'static Engine> {
        let library_path = library_path.as_ref().to_path_buf();

        if LIBRARY_INITIALISED.swap(true, Ordering::SeqCst) {
            return Err(PdfError::LibraryAlreadyLoaded);
        }

        let bindings =
            Pdfium::bind_to_library(&library_path).map_err(|source| PdfError::LibraryLoad {
                path: library_path.clone(),
                source,
            })?;

        // SAFETY: guarded by the atomic above, so this runs at most once per
        // process, before any other FPDF_* call. Nothing else in this crate
        // touches PDFium before an `Engine` exists.
        unsafe {
            bindings.FPDF_InitLibrary();
        }

        tracing::info!(path = ?library_path, "PDFium initialised");
        Ok(Box::leak(Box::new(Engine {
            bindings,
            library_path,
        })))
    }

    /// Resolves the library shipped next to the current executable, then loads it.
    ///
    /// We never fall back to searching the system for `pdfium.dll`: picking up a
    /// stranger's copy from `PATH` is both a correctness risk (ABI drift against
    /// the pinned bindings) and a security one.
    pub fn load_bundled() -> Result<&'static Engine> {
        let exe = std::env::current_exe().map_err(|source| PdfError::FileNotReadable {
            path: PathBuf::from("<current_exe>"),
            source,
        })?;
        let dir = exe.parent().unwrap_or_else(|| Path::new("."));
        Engine::load_from(dir.join(Pdfium::pdfium_platform_library_name()))
    }

    pub fn library_path(&self) -> &Path {
        &self.library_path
    }

    pub(crate) fn bindings(&self) -> &dyn PdfiumLibraryBindings {
        self.bindings.as_ref()
    }

    /// Opens a document from disk.
    ///
    /// The file is memory-mapped rather than read into a `Vec`. The Phase 0
    /// spike made the reason concrete: ten heavy documents held as owned buffers
    /// cost 854 MB of anonymous RSS before a single bitmap was cached, against a
    /// budget of 1.5 GB for ten documents *including* a 2 GB bitmap cache
    /// (SPEC 13). A mapping is file-backed, so the same pages are evictable
    /// under pressure and are not charged as private dirty memory.
    ///
    /// PDFium only ever reads the buffer, so a read-only shared mapping is
    /// sound. The mapping lives inside [`Document`] and is unmapped after
    /// `FPDF_CloseDocument`.
    ///
    /// The trade is that a file truncated by another process while we hold the
    /// mapping turns a read into SIGBUS rather than a short read. That is the
    /// same exposure any mapped-file reader has; the supervisor's poison-document
    /// path (SPEC 3.4) already contains a worker that dies for any reason, and
    /// [`Engine::open_copied`] exists for callers that would rather pay the RSS.
    pub fn open(&'static self, path: impl AsRef<Path>, password: Option<&str>) -> Result<Document> {
        let path = path.as_ref().to_path_buf();
        let file = std::fs::File::open(&path).map_err(|source| PdfError::FileNotReadable {
            path: path.clone(),
            source,
        })?;
        // SAFETY: `Mmap::map` is unsafe purely because the mapping's contents
        // can change if another process writes the file. We only ever read it,
        // and a torn read yields a corrupt-PDF error from PDFium rather than
        // unsoundness in Rust, because the bytes are handled as a plain `&[u8]`
        // with no validity invariant of their own.
        let map =
            unsafe { memmap2::Mmap::map(&file) }.map_err(|source| PdfError::FileNotReadable {
                path: path.clone(),
                source,
            })?;
        self.open_backing(Backing::Mapped(map), password, Some(path))
    }

    /// Opens a document by reading the whole file into an owned buffer.
    ///
    /// Costs RSS proportional to the file, but is immune to the file changing
    /// underneath us. Used for documents on removable or network volumes, and
    /// as the fallback when mapping fails.
    pub fn open_copied(
        &'static self,
        path: impl AsRef<Path>,
        password: Option<&str>,
    ) -> Result<Document> {
        let path = path.as_ref().to_path_buf();
        let bytes = std::fs::read(&path).map_err(|source| PdfError::FileNotReadable {
            path: path.clone(),
            source,
        })?;
        self.open_backing(Backing::Owned(bytes), password, Some(path))
    }

    pub fn open_bytes(
        &'static self,
        bytes: Vec<u8>,
        password: Option<&str>,
        origin: Option<PathBuf>,
    ) -> Result<Document> {
        self.open_backing(Backing::Owned(bytes), password, origin)
    }

    fn open_backing(
        &'static self,
        bytes: Backing,
        password: Option<&str>,
        origin: Option<PathBuf>,
    ) -> Result<Document> {
        let bindings = self.bindings();
        guard("open", move || {
            // SAFETY: `bytes` outlives the returned handle — PDFium keeps a
            // pointer into this buffer for the life of the document, so
            // `Document` owns the `Vec` and only drops it after
            // `FPDF_CloseDocument`. See the field order note on `Document`.
            let handle = unsafe { bindings.FPDF_LoadMemDocument64(bytes.as_slice(), password) };
            if handle.is_null() {
                // SAFETY: reading the thread-local error code PDFium just set.
                let code = unsafe { bindings.FPDF_GetLastError() };
                return Err(open_error(code, password.is_some()));
            }
            // SAFETY: `handle` is a live document handle checked non-null above.
            let page_count = unsafe { bindings.FPDF_GetPageCount(handle) };
            let page_count = u32::try_from(page_count).unwrap_or(0);
            Ok(Document {
                handle,
                engine: self,
                page_count,
                origin,
                pages: RefCell::new(HashMap::new()),
                _bytes: bytes,
            })
        })
    }
}

/// PDFium error codes from `public/fpdfview.h`.
fn open_error(code: std::os::raw::c_ulong, password_supplied: bool) -> PdfError {
    match code {
        1 => PdfError::Corrupt {
            detail: "FPDF_ERR_UNKNOWN".into(),
        },
        2 => PdfError::FileNotReadable {
            path: PathBuf::from("<memory>"),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, "FPDF_ERR_FILE"),
        },
        3 => PdfError::Corrupt {
            detail: "FPDF_ERR_FORMAT".into(),
        },
        4 => {
            if password_supplied {
                PdfError::PasswordWrong
            } else {
                PdfError::PasswordRequired
            }
        }
        5 => PdfError::Corrupt {
            detail: "FPDF_ERR_SECURITY: skema enkripsi tidak didukung".into(),
        },
        6 => PdfError::Corrupt {
            detail: "FPDF_ERR_PAGE".into(),
        },
        other => PdfError::Corrupt {
            detail: format!("FPDF error {other}"),
        },
    }
}

/// How a document's bytes are held for the life of the document.
///
/// PDFium keeps a pointer into whichever of these we hand it, so the backing
/// must outlive the `FPDF_DOCUMENT`.
enum Backing {
    /// A read-only file mapping. File-backed and evictable; the default.
    Mapped(memmap2::Mmap),
    /// An owned copy. Costs private RSS but is stable if the file changes.
    Owned(Vec<u8>),
}

impl Backing {
    fn as_slice(&self) -> &[u8] {
        match self {
            Backing::Mapped(m) => m,
            Backing::Owned(v) => v,
        }
    }

    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn is_mapped(&self) -> bool {
        matches!(self, Backing::Mapped(_))
    }
}

/// An open PDF document and its cache of loaded page handles.
///
/// Field order matters and is load-bearing: Rust drops fields in declaration
/// order, so `pages` (page handles) closes before `handle` (the document), and
/// `_bytes` — the buffer PDFium still points into — is dropped last of all.
pub struct Document {
    pages: RefCell<HashMap<u32, PageHandle>>,
    handle: FPDF_DOCUMENT,
    engine: &'static Engine,
    page_count: u32,
    origin: Option<PathBuf>,
    _bytes: Backing,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("page_count", &self.page_count)
            .field("origin", &self.origin)
            .field("pages_loaded", &self.pages.borrow().len())
            .field("bytes", &self._bytes.len())
            .field("mapped", &self._bytes.is_mapped())
            .finish_non_exhaustive()
    }
}

/// Owns one `FPDF_PAGE`, closing it on drop.
pub(crate) struct PageHandle {
    raw: FPDF_PAGE,
    engine: &'static Engine,
}

impl PageHandle {
    pub(crate) fn raw(&self) -> FPDF_PAGE {
        self.raw
    }
}

impl Drop for PageHandle {
    fn drop(&mut self) {
        // SAFETY: `raw` came from a non-null `FPDF_LoadPage` and is closed
        // exactly once, here. `PageHandle` is not `Clone` and not `Copy`.
        unsafe { self.engine.bindings().FPDF_ClosePage(self.raw) }
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        self.pages.borrow_mut().clear();
        // SAFETY: `handle` came from a non-null `FPDF_LoadMemDocument64`, every
        // page taken from it has just been closed above, and the document is
        // closed exactly once because `Document` is neither `Clone` nor `Copy`.
        unsafe { self.engine.bindings().FPDF_CloseDocument(self.handle) }
    }
}

impl Document {
    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    pub fn origin(&self) -> Option<&Path> {
        self.origin.as_deref()
    }

    /// Size in bytes of the document's backing store.
    pub fn byte_len(&self) -> usize {
        self._bytes.len()
    }

    /// True when the bytes are a file mapping rather than an owned copy.
    pub fn is_mapped(&self) -> bool {
        self._bytes.is_mapped()
    }

    pub fn handle(&self) -> FPDF_DOCUMENT {
        self.handle
    }

    pub(crate) fn engine(&self) -> &'static Engine {
        self.engine
    }

    pub(crate) fn check_page(&self, page: u32) -> Result<()> {
        if page >= self.page_count {
            return Err(PdfError::PageOutOfRange {
                page,
                page_count: self.page_count,
            });
        }
        Ok(())
    }

    /// Runs `f` with a loaded handle for `page`, loading it on first use.
    ///
    /// Page handles are cached because `FPDF_LoadPage` re-parses the page's
    /// content stream every time, which dominates the cost of re-rendering the
    /// same page at a new zoom level. [`Document::release_page`] and
    /// [`Document::release_all_pages`] give the caller back the memory when a
    /// tab goes inactive (SPEC 10).
    pub(crate) fn with_page<T>(
        &self,
        page: u32,
        f: impl FnOnce(FPDF_PAGE) -> Result<T>,
    ) -> Result<T> {
        self.check_page(page)?;
        let mut cache = self.pages.borrow_mut();
        let entry = match cache.get(&page) {
            Some(h) => h.raw(),
            None => {
                // SAFETY: `handle` is a live document and `page` was bounds
                // checked against `FPDF_GetPageCount` above.
                let raw = unsafe {
                    self.engine
                        .bindings()
                        .FPDF_LoadPage(self.handle, page as c_int)
                };
                if raw.is_null() {
                    return Err(PdfError::Corrupt {
                        detail: format!("halaman {page} tidak dapat dimuat"),
                    });
                }
                cache.insert(
                    page,
                    PageHandle {
                        raw,
                        engine: self.engine,
                    },
                );
                raw
            }
        };
        drop(cache);
        f(entry)
    }

    pub fn release_page(&self, page: u32) {
        self.pages.borrow_mut().remove(&page);
    }

    pub fn release_all_pages(&self) {
        self.pages.borrow_mut().clear();
    }

    pub fn loaded_page_count(&self) -> usize {
        self.pages.borrow().len()
    }

    pub fn page_size(&self, page: u32) -> Result<PageSize> {
        let bindings = self.engine.bindings();
        guard("page_size", || {
            self.with_page(page, |p| {
                // SAFETY: `p` is a live page handle held by the page cache.
                Ok(unsafe {
                    PageSize {
                        width: bindings.FPDF_GetPageWidthF(p),
                        height: bindings.FPDF_GetPageHeightF(p),
                    }
                })
            })
        })
    }

    /// Size of one page without loading it.
    ///
    /// `FPDF_GetPageSizeByIndexF` reads `/MediaBox` straight out of the page
    /// tree, so it never parses the content stream. On a 500-page document with
    /// heavy vector content the Phase 0 spike measured this as roughly two
    /// orders of magnitude cheaper than loading each page to ask its width, and
    /// the scroll bar needs every page's size before it can size itself.
    pub fn page_size_fast(&self, page: u32) -> Result<PageSize> {
        self.check_page(page)?;
        let mut size = FS_SIZEF {
            width: 0.0,
            height: 0.0,
        };
        // SAFETY: `handle` is a live document, `page` was bounds checked above,
        // and `size` is a live, correctly typed out-parameter.
        let ok = unsafe {
            self.bindings_ref()
                .FPDF_GetPageSizeByIndexF(self.handle, page as c_int, &mut size)
        };
        if ok == 0 {
            // Fall back to the slow path: a damaged page tree can leave the
            // MediaBox unreachable while the page itself still parses.
            return self.page_size(page);
        }
        Ok(PageSize {
            width: size.width,
            height: size.height,
        })
    }

    /// Sizes of every page, for the viewport's scroll metrics.
    ///
    /// Uses the page-tree path and never leaves parsed pages resident, so
    /// measuring a 500-page document costs milliseconds rather than the half
    /// second the load-every-page approach took in the spike.
    pub fn page_sizes(&self) -> Result<Vec<PageSize>> {
        guard("page_sizes", || {
            let mut out = Vec::with_capacity(self.page_count as usize);
            for i in 0..self.page_count {
                out.push(self.page_size_fast(i)?);
            }
            Ok(out)
        })
    }

    /// Sizes of every page by loading each one. Kept because it is the only
    /// path that reflects a page's real geometry when the page tree disagrees
    /// with the page itself, and because the benchmark compares the two.
    pub fn page_sizes_by_loading(&self) -> Result<Vec<PageSize>> {
        let mut out = Vec::with_capacity(self.page_count as usize);
        for i in 0..self.page_count {
            let already_loaded = self.pages.borrow().contains_key(&i);
            out.push(self.page_size(i)?);
            if !already_loaded {
                self.release_page(i);
            }
        }
        Ok(out)
    }

    fn bindings_ref(&self) -> &dyn PdfiumLibraryBindings {
        self.engine.bindings()
    }

    /// Document permission bits (`FPDF_GetDocPermissions`). All ones when the
    /// document is unencrypted.
    ///
    /// PDFium's return type is `c_ulong`, which is 32 bits on Windows (LLP64)
    /// but 64 bits on Linux (LP64). Converting it to the fixed-width `u32` this
    /// API commits to is therefore a real narrowing conversion on one platform
    /// and a no-op on the other — clippy cannot know that at lint time, so it
    /// flags whichever form is written as wrong on the platform where it
    /// happens to be an identity conversion. The lint is silenced rather than
    /// worked around, because every alternative form has the same problem on
    /// the other platform. The permission bits PDFium defines fit in the low
    /// 32 bits, so the truncation never discards a bit in practice.
    #[allow(clippy::useless_conversion, clippy::unnecessary_cast)]
    pub fn permissions(&self) -> u32 {
        // SAFETY: `handle` is a live document handle owned by `self`.
        let raw = unsafe { self.engine.bindings().FPDF_GetDocPermissions(self.handle) };
        u32::try_from(raw).unwrap_or(u32::MAX)
    }

    /// True when the document carries a security handler, i.e. it was encrypted.
    pub fn is_encrypted(&self) -> bool {
        // SAFETY: as above. Returns -1 for an unencrypted document.
        unsafe {
            self.engine
                .bindings()
                .FPDF_GetSecurityHandlerRevision(self.handle)
                >= 0
        }
    }
}
