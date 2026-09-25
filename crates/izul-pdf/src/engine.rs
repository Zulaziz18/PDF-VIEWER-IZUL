use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use pdfium_render::prelude::{
    Pdfium, PdfiumLibraryBindings, FPDF_DOCUMENT, FPDF_FORMHANDLE, FPDF_PAGE, FS_RECTF, FS_SIZEF,
};

use crate::error::{PdfError, Result};
use crate::ffi_guard::guard;
use crate::geom::{PageSize, PdfRectF, RotationQuarter};

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
                page_count: Cell::new(page_count),
                origin,
                pages: RefCell::new(HashMap::new()),
                strip_izul: Cell::new(true),
                izul: RefCell::new(HashMap::new()),
                form: OnceCell::new(),
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

#[cfg(test)]
mod geometry_tests {
    use super::*;

    const A: PdfRectF = PdfRectF {
        left: 0.0,
        bottom: 0.0,
        right: 612.0,
        top: 792.0,
    };

    fn geom(intrinsic: RotationQuarter, bbox: PdfRectF) -> PageGeometry {
        PageGeometry { bbox, intrinsic }
    }

    #[test]
    fn without_rotation_display_space_is_user_space_shifted_to_the_origin() {
        let g = geom(RotationQuarter::None, A);
        let r = PdfRectF::new(10.0, 20.0, 30.0, 40.0);
        assert_eq!(g.to_display(r, RotationQuarter::None), r);
    }

    #[test]
    fn a_quarter_turn_moves_a_box_to_where_the_glyph_is_drawn() {
        // Page turned 90 degrees clockwise: what was near the page's bottom-left
        // is now near the display's top-left, so its display y is close to the
        // display height (which is the page's width, 612).
        let g = geom(RotationQuarter::None, A);
        let r = PdfRectF::new(0.0, 0.0, 10.0, 20.0);
        let d = g.to_display(r, RotationQuarter::Cw90);
        assert!((d.left - 0.0).abs() < 1e-3, "left = {}", d.left);
        assert!((d.right - 20.0).abs() < 1e-3, "right = {}", d.right);
        assert!((d.top - 612.0).abs() < 1e-3, "top = {}", d.top);
        assert!((d.bottom - 602.0).abs() < 1e-3, "bottom = {}", d.bottom);
    }

    /// `PageFrame::display_to_user` (izul-model, used when saving) must be
    /// the exact inverse of what the viewer does here, on every rotation and
    /// with a box away from the origin.
    #[test]
    fn the_frame_undoes_to_display() {
        let bbox = PdfRectF::new(100.0, 200.0, 695.0, 1042.0);
        for rot in [
            RotationQuarter::None,
            RotationQuarter::Cw90,
            RotationQuarter::Cw180,
            RotationQuarter::Cw270,
        ] {
            let g = geom(rot, bbox);
            let user = PdfRectF::new(150.0, 300.0, 260.0, 420.0);
            let shown = g.to_display(user, RotationQuarter::None);
            let back = g.frame().rect_to_user(shown);
            for (a, b) in [
                (back.left, user.left),
                (back.bottom, user.bottom),
                (back.right, user.right),
                (back.top, user.top),
            ] {
                assert!((a - b).abs() < 1e-3, "{rot:?}: {back:?} vs {user:?}");
            }
        }
    }

    #[test]
    fn the_pages_own_rotation_counts_the_same_as_the_users() {
        let a = geom(RotationQuarter::Cw90, A)
            .to_display(PdfRectF::new(0.0, 0.0, 10.0, 20.0), RotationQuarter::None);
        let b = geom(RotationQuarter::None, A)
            .to_display(PdfRectF::new(0.0, 0.0, 10.0, 20.0), RotationQuarter::Cw90);
        assert_eq!(a, b);
    }

    #[test]
    fn a_mapped_box_always_lands_inside_the_displayed_page() {
        let shifted = PdfRectF::new(-50.0, 20.0, 562.0, 812.0);
        for intrinsic in [
            RotationQuarter::None,
            RotationQuarter::Cw90,
            RotationQuarter::Cw180,
            RotationQuarter::Cw270,
        ] {
            for extra in [
                RotationQuarter::None,
                RotationQuarter::Cw90,
                RotationQuarter::Cw180,
                RotationQuarter::Cw270,
            ] {
                let g = geom(intrinsic, shifted);
                let size = g.display_size(extra);
                let d = g.to_display(PdfRectF::new(-40.0, 30.0, 100.0, 200.0), extra);
                assert!(d.is_valid(), "{intrinsic:?}+{extra:?}: {d:?}");
                assert!(
                    d.left >= -1e-3
                        && d.bottom >= -1e-3
                        && d.right <= size.width + 1e-3
                        && d.top <= size.height + 1e-3,
                    "{intrinsic:?}+{extra:?}: {d:?} outside {size:?}"
                );
            }
        }
    }

    #[test]
    fn four_quarter_turns_are_the_identity() {
        let g = geom(RotationQuarter::None, A);
        let r = PdfRectF::new(11.0, 23.0, 47.0, 91.0);
        let mut cur = r;
        for _ in 0..4 {
            cur = geom(RotationQuarter::None, A).to_display(cur, RotationQuarter::Cw90);
            // Each turn maps a 612x792 page onto a 792x612 one and back, so the
            // box travels around the page and must return exactly.
        }
        let _ = g;
        assert!((cur.left - r.left).abs() < 1e-2, "{cur:?} vs {r:?}");
        assert!((cur.bottom - r.bottom).abs() < 1e-2, "{cur:?} vs {r:?}");
    }
}

/// An open PDF document and its cache of loaded page handles.
///
/// Field order matters and is load-bearing: Rust drops fields in declaration
/// order, so `pages` (page handles) closes before `handle` (the document), and
/// `_bytes` — the buffer PDFium still points into — is dropped last of all.
pub struct Document {
    pages: RefCell<HashMap<u32, PageHandle>>,
    /// Whether a page's own annotations written by this application are taken
    /// out when it first loads (see `izul.rs`).
    pub(crate) strip_izul: Cell<bool>,
    /// What was taken out, by page. Kept for the life of the document, so a
    /// page released and loaded again does not forget its annotations.
    pub(crate) izul: RefCell<HashMap<u32, Vec<crate::izul::IzulAnnot>>>,
    /// PDFium's form-fill environment, for a document with a form: made on
    /// the first page load and kept for the document's life, because widgets
    /// are drawn only through it (`FPDF_FFLDraw`) — measured: without it a
    /// form's fields render as nothing at all — and every page loaded while
    /// it lives has to be introduced to it. `None` inside: no form.
    pub(crate) form: OnceCell<Option<crate::forms::FormEnv>>,
    handle: FPDF_DOCUMENT,
    engine: &'static Engine,
    page_count: Cell<u32>,
    origin: Option<PathBuf>,
    _bytes: Backing,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("page_count", &self.page_count.get())
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
    /// The form environment the page was introduced to, or null.
    form: Cell<FPDF_FORMHANDLE>,
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
        // A page introduced to the form environment leaves it first, while
        // the environment still lives: `Document::drop` closes pages before
        // it releases the environment.
        unsafe {
            let form = self.form.get();
            if !form.is_null() {
                self.engine
                    .bindings()
                    .FORM_OnBeforeClosePage(self.raw, form);
            }
            self.engine.bindings().FPDF_ClosePage(self.raw)
        }
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        self.pages.borrow_mut().clear();
        // After the pages, before the document (`fpdf_formfill.h`).
        drop(self.form.take());
        // SAFETY: `handle` came from a non-null `FPDF_LoadMemDocument64`, every
        // page taken from it has just been closed above, and the document is
        // closed exactly once because `Document` is neither `Clone` nor `Copy`.
        unsafe { self.engine.bindings().FPDF_CloseDocument(self.handle) }
    }
}

/// A page's shape as the layout needs it.
///
/// Two facts a page's *size* alone cannot express:
///
/// * the bounding box (`/MediaBox` intersected with `/CropBox`) need not start
///   at the origin, so user-space coordinates are offset by its corner;
/// * the page carries its own `/Rotate`, so the page the reader sees may have
///   its axes swapped relative to the coordinates PDFium reports for text.
///
/// Both matter for mapping *text boxes* into display space, which is what
/// [`PageGeometry::to_display`] is for. Neither reaches the tile matrix:
/// PDFium composes the page's own display matrix before applying ours, and
/// that matrix has already subtracted the bounding box and applied `/Rotate`
/// (see `tile_matrix` in `render.rs`, and the pixel test that pins it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGeometry {
    /// Bounding box in unrotated user space.
    pub bbox: PdfRectF,
    /// The page's own `/Rotate`, in quarter turns clockwise.
    pub intrinsic: RotationQuarter,
}

impl PageGeometry {
    /// Reads the geometry of a loaded page.
    ///
    /// Never fails: a page whose bounding box PDFium will not report falls back
    /// to its reported size, which is what every other reader would draw.
    pub(crate) fn of_page(engine: &'static Engine, page: FPDF_PAGE) -> Self {
        let bindings = engine.bindings();
        // SAFETY: `page` is a live page handle held by the caller's page cache.
        let intrinsic =
            RotationQuarter::from_degrees(unsafe { bindings.FPDFPage_GetRotation(page) } * 90);
        let mut rect = FS_RECTF {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        };
        // SAFETY: as above; `rect` is a live, correctly typed out-parameter.
        let ok = unsafe { bindings.FPDF_GetPageBoundingBox(page, &mut rect) };
        let bbox = if ok != 0 {
            PdfRectF::new(rect.left, rect.bottom, rect.right, rect.top)
        } else {
            // SAFETY: as above.
            let (w, h) = unsafe {
                (
                    bindings.FPDF_GetPageWidthF(page),
                    bindings.FPDF_GetPageHeightF(page),
                )
            };
            // Those two are rotation-aware, and a bounding box is not, so undo
            // the swap before using them as one.
            if intrinsic.swaps_axes() {
                PdfRectF::new(0.0, 0.0, h, w)
            } else {
                PdfRectF::new(0.0, 0.0, w, h)
            }
        };
        if !bbox.is_valid() {
            return PageGeometry {
                bbox: PdfRectF::new(0.0, 0.0, 612.0, 792.0),
                intrinsic,
            };
        }
        PageGeometry { bbox, intrinsic }
    }

    /// Maps a rectangle from the page's own user space into display space.
    ///
    /// Display space is what the viewport lays out in: origin bottom-left of
    /// the *displayed* page, extents [`PageGeometry::display_size`]. Text boxes
    /// come out of PDFium in user space, so without this a selection would sit
    /// where the glyph would have been if the page were not rotated.
    pub fn to_display(&self, r: PdfRectF, extra: RotationQuarter) -> PdfRectF {
        let a = self.point_to_display(r.left, r.bottom, extra);
        let b = self.point_to_display(r.right, r.top, extra);
        PdfRectF::new(a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
    }

    fn point_to_display(&self, x: f32, y: f32, extra: RotationQuarter) -> (f32, f32) {
        let (u, v) = (x - self.bbox.left, y - self.bbox.bottom);
        let (bw, bh) = (self.bbox.width(), self.bbox.height());
        match self.intrinsic.plus(extra) {
            RotationQuarter::None => (u, v),
            RotationQuarter::Cw90 => (v, bw - u),
            RotationQuarter::Cw180 => (bw - u, bh - v),
            RotationQuarter::Cw270 => (bh - v, u),
        }
    }

    /// The same geometry for the UI process, which does not link PDFium.
    pub fn frame(&self) -> izul_model::geom::PageFrame {
        izul_model::geom::PageFrame {
            bbox: self.bbox,
            rotation: self.intrinsic,
        }
    }

    /// Size of the page as it appears on screen after `extra` rotation is added
    /// to the page's own.
    pub fn display_size(&self, extra: RotationQuarter) -> PageSize {
        let size = PageSize {
            width: self.bbox.width(),
            height: self.bbox.height(),
        };
        size.rotated(self.intrinsic.plus(extra))
    }
}

impl Document {
    pub fn page_count(&self) -> u32 {
        self.page_count.get()
    }

    /// Re-reads the page count after the page tree changed underneath
    /// (`arrange`), and forgets every cached page handle, whose indices no
    /// longer mean what they did.
    pub(crate) fn reload_page_tree(&self) {
        self.release_all_pages();
        // SAFETY: `handle` is a live document.
        let n = unsafe { self.engine.bindings().FPDF_GetPageCount(self.handle) };
        self.page_count.set(u32::try_from(n).unwrap_or(0));
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

    /// Wraps a document PDFium created in memory (`FPDF_CreateNewDocument`).
    /// It has no bytes of its own; it is closed when this is dropped.
    pub(crate) fn adopt(engine: &'static Engine, handle: FPDF_DOCUMENT) -> Document {
        Document {
            handle,
            engine,
            page_count: Cell::new(0),
            origin: None,
            pages: RefCell::new(HashMap::new()),
            strip_izul: Cell::new(false),
            izul: RefCell::new(HashMap::new()),
            form: OnceCell::new(),
            _bytes: Backing::Owned(Vec::new()),
        }
    }

    pub(crate) fn engine(&self) -> &'static Engine {
        self.engine
    }

    pub(crate) fn check_page(&self, page: u32) -> Result<()> {
        if page >= self.page_count.get() {
            return Err(PdfError::PageOutOfRange {
                page,
                page_count: self.page_count.get(),
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
        // Before the page loads: a page loaded while the environment does not
        // exist yet would never be introduced to it.
        let form = self.form_handle();
        let mut cache = self.pages.borrow_mut();
        if let Some(h) = cache.get(&page) {
            let raw = h.raw();
            drop(cache);
            return f(raw);
        }
        // SAFETY: `handle` is a live document and `page` was bounds checked
        // against `FPDF_GetPageCount` above.
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
                form: Cell::new(std::ptr::null_mut()),
            },
        );
        drop(cache);
        // First load of this page in this document: take our own annotations
        // out before anything renders it. Only once — a page released and
        // loaded again no longer has them.
        if self.strip_izul.get() && !self.izul.borrow().contains_key(&page) {
            let found = crate::izul::take(self, raw);
            self.izul.borrow_mut().insert(page, found);
        }
        // Then introduced to the form environment, which builds its view of
        // the page's annotations now — after ours are gone, so it holds none
        // of them.
        if !form.is_null() {
            if let Some(h) = self.pages.borrow().get(&page) {
                // SAFETY: `raw` and `form` are live; the handle remembers
                // `form` so the page leaves it before closing.
                unsafe { self.engine.bindings().FORM_OnAfterLoadPage(raw, form) };
                h.form.set(form);
            }
        }
        f(raw)
    }

    /// The document's form environment, made on first use; null when the
    /// document has no form.
    pub(crate) fn form_handle(&self) -> FPDF_FORMHANDLE {
        self.form
            .get_or_init(|| crate::forms::FormEnv::open(self.engine, self.handle))
            .as_ref()
            .map_or(std::ptr::null_mut(), |env| env.handle())
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

    /// Geometry of one page, for the tile matrix.
    ///
    /// Loads the page, because neither the bounding box nor `/Rotate` is
    /// reachable from the page tree alone. Callers that only need a size use
    /// [`Document::page_size_fast`] instead.
    pub fn page_geometry(&self, page: u32) -> Result<PageGeometry> {
        self.check_page(page)?;
        let engine = self.engine;
        guard("page_geometry", || {
            self.with_page(page, |p| Ok(PageGeometry::of_page(engine, p)))
        })
    }

    /// Display size of a page at an extra rotation, from the page's real
    /// bounding box.
    pub fn page_display_size(&self, page: u32, extra: RotationQuarter) -> Result<PageSize> {
        Ok(self.page_geometry(page)?.display_size(extra))
    }

    /// Sizes of every page, for the viewport's scroll metrics.
    ///
    /// Uses the page-tree path and never leaves parsed pages resident, so
    /// measuring a 500-page document costs milliseconds rather than the half
    /// second the load-every-page approach took in the spike.
    pub fn page_sizes(&self) -> Result<Vec<PageSize>> {
        guard("page_sizes", || {
            let mut out = Vec::with_capacity(self.page_count.get() as usize);
            for i in 0..self.page_count.get() {
                out.push(self.page_size_fast(i)?);
            }
            Ok(out)
        })
    }

    /// Sizes of every page by loading each one. Kept because it is the only
    /// path that reflects a page's real geometry when the page tree disagrees
    /// with the page itself, and because the benchmark compares the two.
    pub fn page_sizes_by_loading(&self) -> Result<Vec<PageSize>> {
        let mut out = Vec::with_capacity(self.page_count.get() as usize);
        for i in 0..self.page_count.get() {
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

/// What an inactive tab gives back (SPEC 10).
#[cfg(test)]
mod trim_tests {
    use crate::{engine_and_lock, viewer_fixture};

    /// `Trim` is the whole of the inactive-tab memory rule, and it is one line
    /// of code with nothing above it to notice if that line stopped working.
    /// The page cache is the thing it clears, so the page cache is what this
    /// counts.
    ///
    /// Worth knowing alongside this: releasing the handles does not
    /// necessarily hand the pages back to the operating system. PDFium frees
    /// them into its own allocator, which keeps the arenas, so the resident
    /// set of a worker measured after a `Trim` barely moves — the Phase 2
    /// benchmark reports exactly that. What `Trim` does buy is that the memory
    /// is *reusable*: the next document opened in that worker takes it rather
    /// than growing the process.
    #[test]
    fn releasing_the_pages_empties_the_page_cache() {
        let (engine, _pdfium) = engine_and_lock!();
        let path = viewer_fixture!();
        let doc = match engine.open(&path, None) {
            Ok(d) => d,
            Err(e) => panic!("buka fixture: {e}"),
        };
        assert_eq!(doc.loaded_page_count(), 0, "belum ada halaman dimuat");

        for page in 0..doc.page_count().min(5) {
            doc.page_size(page).expect("ukuran halaman memuat halaman");
        }
        let loaded = doc.loaded_page_count();
        assert!(
            loaded >= 5,
            "lima halaman seharusnya tersimpan, bukan {loaded}"
        );

        doc.release_all_pages();
        assert_eq!(doc.loaded_page_count(), 0, "Trim harus melepas semuanya");

        // And the document is still usable afterwards: an inactive tab that had
        // to be reopened to be read again would not be a trim, it would be a
        // close.
        doc.page_size(0).expect("halaman dapat dimuat ulang");
        assert_eq!(doc.loaded_page_count(), 1);
    }
}
