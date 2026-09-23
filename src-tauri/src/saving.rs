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

use izul_ipc::message::{DocId, Request, Response, BLOB_CHUNK};
use izul_write::atomic::{sweep_stale, PendingWrite};
use izul_write::AnnotWrite;
use tokio::sync::RwLock;

use crate::annots::AnnotState;
use crate::supervisor::worker::Worker;
use crate::supervisor::Pool;

/// How old a temporary file must be before a later save may remove it.
const STALE_TEMP: Duration = Duration::from_secs(60 * 60);

type Out<T> = Result<T, String>;

async fn worker_of(pool: &Arc<RwLock<Pool>>, doc: u64) -> Out<Arc<Worker>> {
    pool.read()
        .await
        .worker_for(DocId(doc))
        .ok_or_else(|| "dokumen tidak terbuka".to_string())
}

/// One request, with the worker's own refusals turned into a message.
async fn ask(worker: &Worker, req: Request) -> Out<Response> {
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

/// The document as it is in the editor — the file plus every annotation —
/// as finished PDF bytes. `source` is the file on disk.
///
/// Returns the bytes, the pages the editor owns, and how many annotations
/// those pages carry, for verification.
pub async fn build_current(
    pool: &Arc<RwLock<Pool>>,
    annots: &AnnotState,
    doc: u64,
    source: &str,
) -> Out<(Vec<u8>, Vec<u32>, u32)> {
    let worker = worker_of(pool, doc).await?;
    annots.ensure_all_metrics(pool, doc).await?;
    let (pages, objects) = annots.write_set(doc)?;
    let placeholders: Vec<(u32, u64)> = objects.iter().map(|(o, _)| (o.page, o.id.0)).collect();

    expect_work(
        &worker,
        Request::WorkOpen {
            doc: DocId(doc),
            path: source.to_string(),
        },
    )
    .await?;
    let built = async {
        expect_work(
            &worker,
            Request::WorkPlaceholders {
                doc: DocId(doc),
                pages: pages.clone(),
                placeholders,
            },
        )
        .await?;
        expect_blob(&worker, Request::WorkSave { doc: DocId(doc) }).await
    }
    .await;
    let _ = ask(&worker, Request::WorkClose { doc: DocId(doc) }).await;
    let pdfium = built?;

    let writes: Vec<AnnotWrite<'_>> = objects
        .iter()
        .map(|(obj, list)| AnnotWrite { obj, list })
        .collect();
    let assets = annots.assets(doc);
    let bytes = izul_write::patch(pdfium, &writes, &assets).map_err(|e| match e {
        izul_write::SaveError::Syntax(izul_write::incremental::SyntaxError::Encrypted) => {
            "Dokumen ini terenkripsi. Menyimpan anotasi ke dokumen terenkripsi belum didukung; \
             anotasinya tetap tersimpan sebagai draf."
                .to_string()
        }
        other => other.to_string(),
    })?;
    Ok((bytes, pages, objects.len() as u32))
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
) -> Out<SaveReport> {
    let target_path = PathBuf::from(target.unwrap_or(source));
    let (bytes, pages, count) = build_current(pool, annots, doc, source).await?;
    let worker = worker_of(pool, doc).await?;
    let pending = write_verified(
        &worker,
        &target_path,
        &bytes,
        (page_count, pages, Some(count)),
    )
    .await?;

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
        pool.write()
            .await
            .reload(DocId(doc), &reopen)
            .await
            .map_err(|e| format!("dokumen tidak dapat dibuka ulang: {e}"))?;
    }
    committed?;
    annots.mark_saved(doc);
    Ok(SaveReport {
        path: target_path.to_string_lossy().to_string(),
        bytes: bytes.len() as u64,
        annotations: count,
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
    let (bytes, _, _) = build_current(pool, annots, doc, source).await?;
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
    if annots.is_imported(doc, page) {
        return Ok(0);
    }
    let worker = worker_of(pool, doc).await?;
    let found = match ask(
        &worker,
        Request::PageAnnots {
            doc: DocId(doc),
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
                tracing::warn!(doc, page, error = %e, "anotasi tersimpan dilewati");
                continue;
            }
        };
        if let Some((blob, len, _mime)) = saved.image {
            let bytes = fetch_blob(&worker, blob, len).await?;
            let handle = annots.add_image(doc, bytes)?;
            if let izul_model::annot::AnnotPayload::Image { image, .. } = &mut obj.payload {
                *image = izul_model::display::ImageRef(handle);
            }
        }
        objects.push(obj);
    }
    Ok(annots.import_page(doc, page, objects))
}
