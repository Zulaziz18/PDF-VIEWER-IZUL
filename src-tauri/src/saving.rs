//! Saving and exporting, orchestrated from the UI process (SPEC 8, SPEC 11.3).
//!
//! The division of labour, and why:
//!
//! * The **worker** does everything that needs PDFium: it opens a working copy
//!   of the file, puts placeholders where the editor's annotations go, writes
//!   the document, flattens, extracts pages, renders pages to images. It hands
//!   back bytes, never touching the user's files — so saving keeps working
//!   when workers get a low-integrity token (SPEC 3.4).
//! * **`izul-write`** fills the placeholders in: standard dictionaries and
//!   appearance streams built from the same display lists the canvas draws.
//! * **This module** owns the user's files: it writes the result to a
//!   temporary file beside the target, has a worker reopen it to prove it is a
//!   sound PDF carrying every annotation, and only then replaces the target
//!   with one atomic rename.
//!
//! A failure at any step leaves the original exactly as it was.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use std::collections::BTreeSet;

use izul_ipc::message::{
    ArrangePage, DocId, RedactPageWire, RedactedPageWire, Request, Response, BLOB_CHUNK,
};
use izul_model::PageEntry;
use izul_write::atomic::{sweep_stale, PendingWrite};
use izul_write::AnnotWrite;
use tokio::sync::RwLock;

use crate::annots::AnnotState;
use crate::supervisor::worker::Worker;
use crate::supervisor::Pool;

/// How old a temporary file must be before a later save may remove it.
const STALE_TEMP: Duration = Duration::from_secs(60 * 60);

type Out<T> = Result<T, String>;

pub(crate) async fn worker_of(pool: &Arc<RwLock<Pool>>, doc: u64) -> Out<Arc<Worker>> {
    pool.read()
        .await
        .worker_for(DocId(doc))
        .ok_or_else(|| "dokumen tidak terbuka".to_string())
}

/// One request, with the worker's own refusals turned into a message.
pub(crate) async fn ask(worker: &Worker, req: Request) -> Out<Response> {
    match Pool::ask(worker, req).await {
        Ok(Response::Error {
            message_id, detail, ..
        }) => Err(format!("{message_id}: {detail}")),
        Ok(other) => Ok(other),
        Err(e) => Err(e.to_string()),
    }
}

/// Fetches a blob piece by piece and frees it in the worker.
pub async fn fetch_blob(worker: &Worker, blob: u64, len: u64) -> Out<Vec<u8>> {
    let mut out = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    while (out.len() as u64) < len {
        let req = Request::BlobRead {
            blob,
            offset: out.len() as u64,
            len: BLOB_CHUNK,
        };
        match ask(worker, req).await? {
            Response::BlobBytes { data, .. } if !data.is_empty() => out.extend_from_slice(&data),
            Response::BlobBytes { .. } => return Err("blob berhenti sebelum lengkap".into()),
            other => return Err(format!("balasan tak terduga: {other:?}")),
        }
    }
    let _ = ask(worker, Request::BlobDrop { blob }).await;
    Ok(out)
}

async fn expect_blob(worker: &Worker, req: Request) -> Out<Vec<u8>> {
    match ask(worker, req).await? {
        Response::BlobReady { blob, len } => fetch_blob(worker, blob, len).await,
        other => Err(format!("balasan tak terduga: {other:?}")),
    }
}

async fn expect_work(worker: &Worker, req: Request) -> Out<u32> {
    match ask(worker, req).await? {
        Response::WorkReady { page_count, .. } => Ok(page_count),
        other => Err(format!("balasan tak terduga: {other:?}")),
    }
}

/// Applying redaction marks (Phase 6): the areas per page, in display space,
/// and the editor objects that go with them.
#[derive(Debug, Clone)]
pub struct Redaction {
    pub pages: Vec<RedactPageWire>,
    pub doomed: BTreeSet<u64>,
}

/// Local OCR on save (Phase 7): which pages to read, and how to report.
///
/// The pages are read one request at a time, after the page map and any
/// redaction are applied and before the annotations go in. Between pages the
/// job reports progress and looks at `cancel`; a cancelled job fails the save
/// and nothing is written — "stop" leaves the file as it was.
#[derive(Clone)]
pub struct OcrJob {
    /// Display pages of the result, 0-based.
    pub pages: Vec<u32>,
    /// Read pages that already have text as well.
    pub force: bool,
    /// Folder holding `text-detection.rten` and `text-recognition.rten`.
    pub models: PathBuf,
    pub progress: Option<Arc<dyn Fn(u32, u32) + Send + Sync>>,
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for OcrJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OcrJob")
            .field("pages", &self.pages.len())
            .field("force", &self.force)
            .field("models", &self.models)
            .finish()
    }
}

/// What OCR did, for the user.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct OcrSummary {
    pub pages_read: u32,
    /// Pages left alone because they already had text.
    pub pages_had_text: u32,
    pub words: u32,
}

/// Changes to the document's content made while saving, beyond the
/// annotations: redaction (Phase 6) and OCR (Phase 7).
#[derive(Debug, Clone, Default)]
pub struct Rewrite {
    pub redaction: Option<Redaction>,
    pub ocr: Option<OcrJob>,
    pub text: Option<TextJob>,
}

/// One text replacement (Phase 7): on a page of the file as it is on disk,
/// the text in `rect` (that page's display space) becomes `text`.
#[derive(Debug, Clone)]
pub struct TextJob {
    pub page: u32,
    pub rect: izul_model::geom::PdfRectF,
    pub text: String,
}

/// What a text replacement did, for the user.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TextSummary {
    pub before: String,
    pub glyphs: u32,
}

/// Why a replacement was refused, in the words the worker gave.
fn edit_refused(error: &str) -> String {
    let detail = error.strip_prefix("textedit.failed: ").unwrap_or(error);
    format!("Teks tidak diganti, berkas tidak diubah: {detail}")
}

/// The message a cancelled OCR job fails the save with.
pub const OCR_CANCELLED: &str = "OCR dibatalkan; berkas tidak diubah.";

/// What a redaction took out, summed over its pages, for the user.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RedactionSummary {
    pub pages: u32,
    pub glyphs: u32,
    pub images_removed: u32,
    pub images_cleared: u32,
    pub images_unsupported: u32,
    pub paths: u32,
    pub annotations: u32,
}

impl RedactionSummary {
    fn of(pages: &[RedactedPageWire]) -> Self {
        let mut s = RedactionSummary {
            pages: pages.len() as u32,
            ..Default::default()
        };
        for p in pages {
            s.glyphs += p.glyphs;
            s.images_removed += p.images_removed;
            s.images_cleared += p.images_cleared;
            s.images_unsupported += p.images_unsupported;
            s.paths += p.paths;
            s.annotations += p.annotations;
        }
        s
    }
}

/// The document as it is in the editor — the file plus every annotation —
/// as finished PDF bytes. `source` is the file on disk.
///
/// Returns the bytes, the pages the editor owns, how many annotations those
/// pages carry, for verification, and — when `redaction` is given — what the
/// redaction took out of each page.
#[allow(clippy::type_complexity)]
pub async fn build_current(
    pool: &Arc<RwLock<Pool>>,
    annots: &AnnotState,
    doc: u64,
    source: &str,
    rewrite: Option<&Rewrite>,
) -> Out<(
    Vec<u8>,
    Vec<u32>,
    u32,
    Option<Vec<RedactedPageWire>>,
    Option<OcrSummary>,
    Option<TextSummary>,
)> {
    let redaction = rewrite.and_then(|r| r.redaction.as_ref());
    let text = rewrite.and_then(|r| r.text.as_ref());
    let ocr = rewrite.and_then(|r| r.ocr.as_ref());
    let worker = worker_of(pool, doc).await?;
    annots.ensure_all_metrics(pool, doc).await?;
    let empty = BTreeSet::new();
    let (pages, objects) =
        annots.write_set_without(doc, redaction.map_or(&empty, |r| &r.doomed))?;
    let placeholders: Vec<(u32, u64)> = objects.iter().map(|(o, _)| (o.page, o.id.0)).collect();

    expect_work(
        &worker,
        Request::WorkOpen {
            doc: DocId(doc),
            path: source.to_string(),
        },
    )
    .await?;
    let arrange = annots.page_map(doc).map(|map| {
        let pages: Vec<ArrangePage> = map
            .iter()
            .map(|e| match *e {
                PageEntry::Page {
                    source,
                    page,
                    rotation,
                } => ArrangePage::Page {
                    source,
                    page,
                    rotation,
                },
                PageEntry::Blank {
                    width,
                    height,
                    rotation,
                } => ArrangePage::Blank {
                    width,
                    height,
                    rotation,
                },
            })
            .collect();
        let mut sources = vec![source.to_string()];
        sources.extend(annots.sources(doc).into_iter().map(|s| s.path));
        (pages, sources)
    });
    let built = async {
        // First, on the file's own pages, before anything moves them: the
        // selection was made on the page as the file has it.
        let replaced = match text {
            Some(job) => match ask(
                &worker,
                Request::WorkReplaceText {
                    doc: DocId(doc),
                    page: job.page,
                    rect: job.rect,
                    text: job.text.clone(),
                },
            )
            .await
            {
                Ok(Response::TextReplaced { before, glyphs, .. }) => {
                    Some(TextSummary { before, glyphs })
                }
                Ok(other) => return Err(format!("balasan tak terduga: {other:?}")),
                Err(e) => return Err(edit_refused(&e)),
            },
            None => None,
        };
        if let Some((pages, sources)) = arrange {
            expect_work(
                &worker,
                Request::WorkArrange {
                    doc: DocId(doc),
                    pages,
                    sources,
                },
            )
            .await?;
        }
        // After the pages are in their final order (the marks are on display
        // pages), and before our annotations go in: what is written after
        // this point is written over redacted content, never under it.
        let redacted = match redaction {
            Some(r) => match ask(
                &worker,
                Request::WorkRedact {
                    doc: DocId(doc),
                    pages: r.pages.clone(),
                },
            )
            .await
            {
                Ok(Response::WorkRedacted { pages, .. }) => Some(pages),
                Ok(other) => return Err(format!("balasan tak terduga: {other:?}")),
                Err(e) => return Err(redaction_refused(&e)),
            },
            None => None,
        };
        // OCR writes into the page content: after the redaction (never under
        // what is taken out, never bringing it back), before our annotations.
        let read = match ocr {
            Some(job) => Some(run_ocr(&worker, doc, job).await?),
            None => None,
        };
        // Form values (Phase 7) go into the fields through PDFium's form
        // environment, which draws each widget's appearance again.
        let form = annots.form_values(doc);
        if !form.is_empty() {
            match ask(
                &worker,
                Request::FillForm {
                    doc: DocId(doc),
                    working: true,
                    values: form,
                },
            )
            .await?
            {
                Response::FormFilled { .. } => {}
                other => return Err(format!("balasan tak terduga: {other:?}")),
            }
        }
        expect_work(
            &worker,
            Request::WorkPlaceholders {
                doc: DocId(doc),
                pages: pages.clone(),
                placeholders,
            },
        )
        .await?;
        // Where display space sits on each final page: the annotations are
        // kept in display space and must be written in user space.
        let frames = match ask(&worker, Request::WorkFrames { doc: DocId(doc) }).await? {
            Response::WorkFramesReady { frames, .. } => frames,
            other => return Err(format!("balasan tak terduga: {other:?}")),
        };
        let bytes = expect_blob(&worker, Request::WorkSave { doc: DocId(doc) }).await?;
        Ok((bytes, redacted, frames, read, replaced))
    }
    .await;
    let _ = ask(&worker, Request::WorkClose { doc: DocId(doc) }).await;
    let (pdfium, redacted, frames, read, replaced) = built?;
    if let Some((obj, _)) = objects
        .iter()
        .find(|(o, _)| o.page as usize >= frames.len())
    {
        return Err(format!(
            "anotasi di halaman {} tetapi dokumen hanya {} halaman",
            obj.page + 1,
            frames.len()
        ));
    }

    let writes: Vec<AnnotWrite<'_>> = objects
        .iter()
        .map(|(obj, list)| AnnotWrite { obj, list })
        .collect();
    let assets = annots.assets(doc);
    let bytes = izul_write::patch(pdfium, &writes, &assets, &frames).map_err(|e| match e {
        izul_write::SaveError::Syntax(izul_write::incremental::SyntaxError::Encrypted) => {
            "Dokumen ini terenkripsi. Menyimpan anotasi ke dokumen terenkripsi belum didukung; \
             anotasinya tetap tersimpan sebagai draf."
                .to_string()
        }
        other => other.to_string(),
    })?;
    Ok((bytes, pages, objects.len() as u32, redacted, read, replaced))
}

async fn run_ocr(worker: &Worker, doc: u64, job: &OcrJob) -> Out<OcrSummary> {
    use std::sync::atomic::Ordering;
    let total = job.pages.len() as u32;
    let mut summary = OcrSummary::default();
    for (done, &page) in job.pages.iter().enumerate() {
        if job.cancel.load(Ordering::Relaxed) {
            return Err(OCR_CANCELLED.to_string());
        }
        if let Some(report) = &job.progress {
            report(done as u32, total);
        }
        let reply = ask(
            worker,
            Request::WorkOcr {
                doc: DocId(doc),
                page,
                models: job.models.to_string_lossy().to_string(),
                force: job.force,
            },
        )
        .await?;
        match reply {
            Response::WorkOcrDone { words: Some(n), .. } => {
                summary.pages_read += 1;
                summary.words += n;
            }
            Response::WorkOcrDone { words: None, .. } => summary.pages_had_text += 1,
            other => return Err(format!("balasan tak terduga: {other:?}")),
        }
    }
    if job.cancel.load(Ordering::Relaxed) {
        return Err(OCR_CANCELLED.to_string());
    }
    if let Some(report) = &job.progress {
        report(total, total);
    }
    Ok(summary)
}

/// A worker's refusal to redact, as the user reads it. The worker's detail is
/// already a sentence; the message id in front of it is not.
fn redaction_refused(error: &str) -> String {
    let detail = error.strip_prefix("redact.failed: ").unwrap_or(error);
    format!("Redaksi dibatalkan, berkas tidak diubah: {detail}")
}

/// Writes `bytes` beside `target` and has a worker reopen the result.
///
/// `expect` is `(page count, pages to count on, annotations expected there)`;
/// `None` skips the annotation count (a flattened file has none by design).
pub async fn write_verified(
    worker: &Worker,
    target: &Path,
    bytes: &[u8],
    expect: (u32, Vec<u32>, Option<u32>),
) -> Out<PendingWrite> {
    if let Some(dir) = target.parent() {
        sweep_stale(dir, STALE_TEMP);
    }
    let pending = PendingWrite::write(target, bytes).map_err(|e| {
        format!(
            "berkas sementara tidak dapat ditulis di samping {}: {e}",
            target.display()
        )
    })?;
    let (pages, check_pages, annots) = expect;
    let req = Request::VerifyFile {
        path: pending.temp_path().to_string_lossy().to_string(),
        pages: check_pages,
    };
    match ask(worker, req).await? {
        Response::Verified {
            page_count,
            izul_annots,
        } => {
            if page_count != pages {
                return Err(format!(
                    "verifikasi gagal: {page_count} halaman, seharusnya {pages}"
                ));
            }
            if let Some(expected) = annots {
                if izul_annots != expected {
                    return Err(format!(
                        "verifikasi gagal: {izul_annots} anotasi terbaca, seharusnya {expected}"
                    ));
                }
            }
            Ok(pending)
        }
        other => Err(format!("balasan tak terduga: {other:?}")),
    }
}

/// What a save did, for the status line and the session.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SaveReport {
    pub path: String,
    pub bytes: u64,
    pub annotations: u32,
    /// The save applied a page map: the file has new pages, and these are
    /// their display sizes as the reopened document reports them. The tab
    /// must lay itself out again.
    pub restructured: Option<Vec<(f32, f32)>>,
    /// The save applied redaction marks, and this is what they took out.
    pub redaction: Option<RedactionSummary>,
    /// The save ran OCR, and this is what it read.
    pub ocr: Option<OcrSummary>,
    /// The save replaced text, and this is what was there.
    pub text: Option<TextSummary>,
}

/// Saves `doc` to `target` (its own file when `None`).
///
/// When the target is the file the tab has open, the worker's view of it is
/// released for the instant of the rename and reopened straight after, on the
/// same worker.
pub async fn save(
    pool: &Arc<RwLock<Pool>>,
    annots: &AnnotState,
    doc: u64,
    source: &str,
    page_count: u32,
    target: Option<&str>,
    rewrite: Option<&Rewrite>,
) -> Out<SaveReport> {
    let redaction = rewrite.and_then(|r| r.redaction.as_ref());
    let target_path = PathBuf::from(target.unwrap_or(source));
    let mapped = annots.page_map(doc).map(|m| m.len() as u32);
    // A map changes how many pages the result has; verify against that.
    let page_count = mapped.unwrap_or(page_count);
    let (bytes, pages, count, redacted, read, replaced) =
        build_current(pool, annots, doc, source, rewrite).await?;
    let worker = worker_of(pool, doc).await?;
    let pending = write_verified(
        &worker,
        &target_path,
        &bytes,
        (page_count, pages, Some(count)),
    )
    .await?;
    if let Some(redacted) = &redacted {
        // The same check once more, on the very bytes about to replace the
        // file: nothing between the redaction and here may have put anything
        // back.
        let req = Request::VerifyRedacted {
            path: pending.temp_path().to_string_lossy().to_string(),
            pages: redacted.iter().map(|p| (p.page, p.areas.clone())).collect(),
        };
        match ask(&worker, req).await {
            Ok(Response::RedactionVerified { .. }) => {}
            Ok(other) => return Err(format!("balasan tak terduga: {other:?}")),
            Err(e) => return Err(redaction_refused(&e)),
        }
    }

    let same_file = std::fs::canonicalize(&target_path).ok() == std::fs::canonicalize(source).ok()
        && target_path.exists();
    if same_file {
        pool.write()
            .await
            .release(DocId(doc))
            .await
            .map_err(|e| e.to_string())?;
    }
    let committed = pending
        .commit()
        .map_err(|e| format!("berkas tidak dapat diganti: {e}"));
    let mut reopened_sizes = None;
    if same_file || committed.is_ok() && target.is_some() {
        // Reopen whatever is on disk now: the new file after a successful
        // save, the untouched original after a failed one. "Save as" moves the
        // tab to the new name, as every editor does.
        let reopen = if committed.is_ok() {
            target_path.to_string_lossy().to_string()
        } else {
            source.to_string()
        };
        if !same_file {
            let _ = pool.write().await.release(DocId(doc)).await;
        }
        let reply = pool
            .write()
            .await
            .reload(DocId(doc), &reopen)
            .await
            .map_err(|e| format!("dokumen tidak dapat dibuka ulang: {e}"))?;
        if let Response::Opened { page_sizes, .. } = reply {
            reopened_sizes = Some(page_sizes);
        }
    }
    committed?;
    if let Some(r) = redaction {
        annots.after_redaction(doc, &r.doomed);
    }
    annots.mark_saved(doc);
    let restructured = match (mapped, reopened_sizes) {
        (Some(_), Some(sizes)) => {
            annots.after_restructure(doc, sizes.clone());
            Some(sizes)
        }
        (Some(_), None) => {
            return Err("susunan halaman tersimpan tetapi dokumen tidak dibuka ulang".into());
        }
        (None, _) => None,
    };
    Ok(SaveReport {
        path: target_path.to_string_lossy().to_string(),
        bytes: bytes.len() as u64,
        annotations: count,
        restructured,
        redaction: redacted.as_deref().map(RedactionSummary::of),
        ocr: read,
        text: replaced,
    })
}

/// A throwaway copy of the document as the editor has it, for exports to
/// start from. Deleted when the returned handle is dropped.
async fn current_as_temp(
    pool: &Arc<RwLock<Pool>>,
    annots: &AnnotState,
    doc: u64,
    source: &str,
    scratch: &Path,
) -> Out<PendingWrite> {
    let (bytes, _, _, _, _, _) = build_current(pool, annots, doc, source, None).await?;
    std::fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    PendingWrite::write(&scratch.join(format!("ekspor-{doc}.pdf")), &bytes)
        .map_err(|e| e.to_string())
}

/// Which export.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Export {
    /// Every annotation burnt into the page (SPEC 11.3 "ekspor rata").
    Flat { target: String },
    /// Some pages, in order, as a new PDF — annotations kept live.
    Pages { target: String, pages: Vec<u32> },
    /// One PDF per range — "Pecah" (SPEC 11.3), by page ranges or by the
    /// document's top-level bookmarks, which the frontend turns into ranges:
    /// `<stem>-<k>.pdf` in `folder`, `k` from 1. Annotations stay live.
    Split {
        folder: String,
        stem: String,
        ranges: Vec<Vec<u32>>,
    },
    /// Pages as image files: `<stem>-<n>.<ext>` in `folder`.
    Images {
        folder: String,
        stem: String,
        pages: Vec<u32>,
        dpi: u32,
        jpeg_quality: Option<u8>,
    },
}

/// Runs an export; returns the files written.
pub async fn export(
    pool: &Arc<RwLock<Pool>>,
    annots: &AnnotState,
    doc: u64,
    source: &str,
    scratch: &Path,
    what: Export,
) -> Out<Vec<String>> {
    let temp = current_as_temp(pool, annots, doc, source, scratch).await?;
    let worker = worker_of(pool, doc).await?;
    let id = DocId(doc);
    let page_count = expect_work(
        &worker,
        Request::WorkOpen {
            doc: id,
            path: temp.temp_path().to_string_lossy().to_string(),
        },
    )
    .await?;
    let result = async {
        match what {
            Export::Flat { target } => {
                // In batches, so the worker answers heartbeats in between.
                let mut first = 0;
                while first < page_count {
                    expect_work(
                        &worker,
                        Request::WorkFlatten {
                            doc: id,
                            first,
                            count: 25,
                        },
                    )
                    .await?;
                    first += 25;
                }
                let bytes = expect_blob(&worker, Request::WorkSave { doc: id }).await?;
                // Page count only: counting annotations would load every page
                // in one request, and a long document would outlast the
                // heartbeat. Flattening was already checked page by page.
                let pending = write_verified(
                    &worker,
                    Path::new(&target),
                    &bytes,
                    (page_count, Vec::new(), None),
                )
                .await?;
                pending.commit().map_err(|e| e.to_string())?;
                Ok(vec![target])
            }
            Export::Pages { target, pages } => {
                let n = expect_work(
                    &worker,
                    Request::WorkExtract {
                        doc: id,
                        pages: pages.clone(),
                    },
                )
                .await?;
                let bytes = expect_blob(&worker, Request::WorkSave { doc: id }).await?;
                let pending =
                    write_verified(&worker, Path::new(&target), &bytes, (n, Vec::new(), None))
                        .await?;
                pending.commit().map_err(|e| e.to_string())?;
                Ok(vec![target])
            }
            Export::Split {
                folder,
                stem,
                ranges,
            } => {
                let temp_path = temp.temp_path().to_string_lossy().to_string();
                let mut written = Vec::new();
                for (k, pages) in ranges.iter().enumerate() {
                    if pages.is_empty() {
                        continue;
                    }
                    // Extracting replaces the working copy, so each part
                    // starts again from the whole document.
                    if k > 0 {
                        expect_work(
                            &worker,
                            Request::WorkOpen {
                                doc: id,
                                path: temp_path.clone(),
                            },
                        )
                        .await?;
                    }
                    let n = expect_work(
                        &worker,
                        Request::WorkExtract {
                            doc: id,
                            pages: pages.clone(),
                        },
                    )
                    .await?;
                    let bytes = expect_blob(&worker, Request::WorkSave { doc: id }).await?;
                    let out = Path::new(&folder).join(format!("{stem}-{}.pdf", k + 1));
                    let pending =
                        write_verified(&worker, &out, &bytes, (n, Vec::new(), None)).await?;
                    pending
                        .commit()
                        .map_err(|e| format!("{}: {e}", out.display()))?;
                    written.push(out.to_string_lossy().to_string());
                }
                Ok(written)
            }
            Export::Images {
                folder,
                stem,
                pages,
                dpi,
                jpeg_quality,
            } => {
                let ext = if jpeg_quality.is_some() { "jpg" } else { "png" };
                let scale = dpi.clamp(36, 1200) as f32 / 72.0;
                let mut written = Vec::new();
                for page in pages {
                    let bytes = expect_blob(
                        &worker,
                        Request::WorkRender {
                            doc: id,
                            page,
                            scale,
                            jpeg_quality,
                        },
                    )
                    .await?;
                    let out = Path::new(&folder).join(format!("{stem}-{}.{ext}", page + 1));
                    PendingWrite::write(&out, &bytes)
                        .and_then(PendingWrite::commit)
                        .map_err(|e| format!("{}: {e}", out.display()))?;
                    written.push(out.to_string_lossy().to_string());
                }
                Ok(written)
            }
        }
    }
    .await;
    let _ = ask(&worker, Request::WorkClose { doc: id }).await;
    drop(temp);
    result
}

/// Takes one page's saved annotations from the worker into the editor.
/// Does nothing for a page already imported.
pub async fn import_page(
    pool: &Arc<RwLock<Pool>>,
    annots: &AnnotState,
    doc: u64,
    page: u32,
) -> Out<usize> {
    // Once every page is in the editor — which a page operation requires —
    // page numbers on screen are no longer page numbers in the file, and
    // reading "page n" again would read the wrong one.
    if annots.is_imported(doc, page) || annots.all_imported(doc) {
        return Ok(0);
    }
    let objects = read_saved(pool, annots, doc, page, doc).await?;
    Ok(annots.import_page(doc, page, objects))
}

/// Our annotations saved on `page` of the document open as `from`, decoded;
/// pictures are stored as images of `owner`, the document they will live in.
pub async fn read_saved(
    pool: &Arc<RwLock<Pool>>,
    annots: &AnnotState,
    from: u64,
    page: u32,
    owner: u64,
) -> Out<Vec<izul_model::annot::AnnotObject>> {
    let worker = worker_of(pool, from).await?;
    let found = match ask(
        &worker,
        Request::PageAnnots {
            doc: DocId(from),
            page,
        },
    )
    .await?
    {
        Response::PageAnnotsReady { annots, .. } => annots,
        other => return Err(format!("balasan tak terduga: {other:?}")),
    };
    let mut objects = Vec::with_capacity(found.len());
    for saved in found {
        let mut obj = match izul_write::metadata::decode(&saved.metadata) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(doc = from, page, error = %e, "anotasi tersimpan dilewati");
                continue;
            }
        };
        if let Some((blob, len, _mime)) = saved.image {
            let bytes = fetch_blob(&worker, blob, len).await?;
            let handle = annots.add_image(owner, bytes)?;
            if let izul_model::annot::AnnotPayload::Image { image, .. } = &mut obj.payload {
                *image = izul_model::display::ImageRef(handle);
            }
        }
        objects.push(obj);
    }
    Ok(objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontend recognises a cancelled run by this exact text
    /// (`OCR_CANCELLED` in `src/app/ocr.ts`); a reworded message there or
    /// here would turn "stopped" into "failed".
    #[test]
    fn the_cancel_message_matches_the_frontend() {
        let ts = include_str!("../../src/app/ocr.ts");
        assert!(
            ts.contains(&format!("\"{OCR_CANCELLED}\"")),
            "src/app/ocr.ts"
        );
    }
}
