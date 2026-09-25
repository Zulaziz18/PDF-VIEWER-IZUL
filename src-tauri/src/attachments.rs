//! The "Lampiran" sidebar (SPEC 11.1, Phase 8): the files embedded in the
//! document, and saving one of them where the user chooses.
//!
//! The worker reads the file out of the PDF; this process writes it, so the
//! worker — the process that parses untrusted documents — is never handed a
//! path of the user's choosing to write to.

use izul_ipc::{DocId, Request, Response};
use serde::Serialize;

use crate::commands::AppState;
use crate::saving::{ask, fetch_blob, worker_of};

type CmdResult<T> = Result<T, String>;

#[derive(Debug, Serialize)]
pub struct Attachment {
    pub index: u32,
    pub name: String,
    pub size: u64,
}

#[tauri::command]
pub async fn attachments_list(
    state: tauri::State<'_, AppState>,
    doc: u64,
) -> CmdResult<Vec<Attachment>> {
    let worker = worker_of(&state.pool, doc).await?;
    match ask(&worker, Request::Attachments { doc: DocId(doc) }).await? {
        Response::AttachmentsReady { items, .. } => Ok(items
            .into_iter()
            .enumerate()
            .map(|(i, (name, size))| Attachment {
                index: i as u32,
                name,
                size,
            })
            .collect()),
        other => Err(format!("balasan tak terduga: {other:?}")),
    }
}

/// Writes attachment `index` of `doc` to `path`, through a temporary file
/// beside it, so a failure halfway never leaves half a file under the name
/// the user chose.
#[tauri::command]
pub async fn attachment_save(
    state: tauri::State<'_, AppState>,
    doc: u64,
    index: u32,
    path: String,
) -> CmdResult<u64> {
    let worker = worker_of(&state.pool, doc).await?;
    let bytes = match ask(
        &worker,
        Request::AttachmentData {
            doc: DocId(doc),
            index,
        },
    )
    .await?
    {
        Response::BlobReady { blob, len } => fetch_blob(&worker, blob, len).await?,
        other => return Err(format!("balasan tak terduga: {other:?}")),
    };
    let target = std::path::PathBuf::from(&path);
    let temp = target.with_extension("izul-tmp");
    std::fs::write(&temp, &bytes).map_err(|e| format!("{path}: {e}"))?;
    std::fs::rename(&temp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("{path}: {e}")
    })?;
    Ok(bytes.len() as u64)
}
