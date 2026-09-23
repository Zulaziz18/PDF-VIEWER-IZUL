//! Tauri commands for Phase 4: save, save as, export, the export history,
//! autosaved drafts, and noticing a file changed by another program.
//!
//! The work is in `saving.rs`; these translate between the frontend's calls
//! and it, and keep the bookkeeping — the recent-files row, the session, the
//! draft table, the file stamps — in step with what was written.

use std::path::PathBuf;

use izul_store::{drafts, exports, files, FileStamp};
use serde::Serialize;

use crate::annots::Snapshot;
use crate::commands::{prepare_fonts, AppState};
use crate::saving::{self, Export, SaveReport};

type CmdResult<T> = Result<T, String>;

/// The identity a draft is recorded against: the file's size and time.
/// Cheap to compute on every autosave, and it changes whenever another
/// program writes the file — which is exactly when a draft must stop applying.
fn base_of(stamp: &FileStamp) -> Vec<u8> {
    format!("{}:{}", stamp.size, stamp.mtime).into_bytes()
}

fn doc_path(state: &AppState, doc: u64) -> CmdResult<(String, i64, u32)> {
    let ws = state.workspace.lock();
    let open = ws
        .get(doc)
        .ok_or_else(|| "dokumen tidak terbuka".to_string())?;
    Ok((open.path.clone(), open.file_id, open.page_count))
}

/// Whether two paths name the same existing file. A target that does not
/// exist yet cannot be a file some tab has open, so it is never "the same".
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Refuses a write over a file another tab has open (`except` is the tab
/// doing the writing, whose own file a plain save may replace).
///
/// On Windows the rename would fail halfway, because the other tab's worker
/// holds the file mapped; elsewhere it would succeed and silently change the
/// pages of a document someone is reading. Either is worse than asking for
/// another name.
fn refuse_open_target(state: &AppState, target: &str, except: Option<u64>) -> CmdResult<()> {
    let target = PathBuf::from(target);
    let ws = state.workspace.lock();
    for tab in ws.tabs() {
        if Some(tab.doc) == except {
            continue;
        }
        if same_file(&target, &PathBuf::from(&tab.path)) {
            let name = PathBuf::from(&tab.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            return Err(format!(
                "{name} sedang terbuka di tab lain. Tutup tab itu dulu, atau pilih nama lain."
            ));
        }
    }
    Ok(())
}

/// Makes sure every face the objects draw text in has metrics cached, so the
/// display lists the writer serialises are the ones the canvas drew.
async fn prepare_all_fonts(state: &AppState, doc: u64) -> CmdResult<()> {
    let objects = state.annots.objects(doc, None);
    prepare_fonts(state, doc, &objects).await
}

/// Saves to the tab's own file, or to `target` ("save as").
#[tauri::command]
pub async fn save_document(
    state: tauri::State<'_, AppState>,
    doc: u64,
    target: Option<String>,
) -> CmdResult<SaveReport> {
    let (path, file_id, page_count) = doc_path(&state, doc)?;
    if let Some(target) = &target {
        refuse_open_target(&state, target, Some(doc))?;
    }
    prepare_all_fonts(&state, doc).await?;
    let report = saving::save(
        &state.pool,
        &state.annots,
        doc,
        &path,
        page_count,
        target.as_deref(),
    )
    .await?;

    let saved = PathBuf::from(&report.path);
    if let Ok(stamp) = FileStamp::of(&saved) {
        state.stamps.lock().insert(doc, stamp);
    }
    if let Some(sizes) = &report.restructured {
        // The file has new pages. Everything that knew the old ones — the
        // tile cache, the render registry, the tab's page count, the hidden
        // documents the map drew from, the text index — starts again.
        state.render.forget(doc);
        state.render.register(
            doc,
            crate::render::DocInfo {
                path: report.path.clone(),
                page_sizes: sizes.clone(),
                generation: 0,
            },
        );
        state
            .workspace
            .lock()
            .set_page_count(doc, sizes.len() as u32);
        for hidden in state.hidden.take(doc) {
            crate::pagemap::close_hidden(&state, hidden).await;
        }
        state.indexer.forget(doc);
    }
    if let Ok(conn) = state.db() {
        if file_id > 0 {
            let _ = drafts::delete(&conn, files::FileId(file_id));
        }
        if let Ok(stamp) = FileStamp::of(&saved) {
            if let Ok(new_id) = files::touch(&conn, &saved, stamp) {
                if target.is_some() {
                    state
                        .workspace
                        .lock()
                        .set_path(doc, report.path.clone(), new_id.0);
                    state.save_session();
                }
            }
        }
    }
    if let Some(sizes) = &report.restructured {
        let file_id = state.workspace.lock().get(doc).map_or(0, |d| d.file_id);
        if file_id > 0 {
            state.indexer.start(crate::indexing::Job {
                doc,
                file_id,
                page_count: sizes.len() as u32,
                data_dir: state.data_dir.clone(),
                pool: std::sync::Arc::clone(&state.pool),
            });
        }
    }
    tracing::info!(doc, path = %report.path, bytes = report.bytes, annots = report.annotations, "dokumen disimpan");
    Ok(report)
}

/// Writes a derived file; the tab keeps its own.
#[tauri::command]
pub async fn export_document(
    state: tauri::State<'_, AppState>,
    doc: u64,
    spec: Export,
) -> CmdResult<Vec<String>> {
    let (path, file_id, _) = doc_path(&state, doc)?;
    // An export never replaces the file it was made from, either: that is
    // what "Simpan" is for, and an export that did it would leave the tab
    // showing pages its file no longer has.
    match &spec {
        Export::Flat { target } | Export::Pages { target, .. } => {
            refuse_open_target(&state, target, None)?;
        }
        Export::Split {
            folder,
            stem,
            ranges,
        } => {
            for k in 0..ranges.len() {
                let out = PathBuf::from(folder).join(format!("{stem}-{}.pdf", k + 1));
                refuse_open_target(&state, &out.to_string_lossy(), None)?;
            }
        }
        Export::Images { .. } => {}
    }
    prepare_all_fonts(&state, doc).await?;
    let kind = match &spec {
        Export::Flat { .. } => "flat",
        Export::Pages { .. } => "pages",
        Export::Split { .. } => "split",
        Export::Images {
            jpeg_quality: None, ..
        } => "png",
        Export::Images { .. } => "jpg",
    };
    let scratch = state.data_dir.join("tmp");
    let written = saving::export(&state.pool, &state.annots, doc, &path, &scratch, spec).await?;
    if let (Ok(conn), true) = (state.db(), file_id > 0) {
        for out in &written {
            let _ = exports::record(&conn, files::FileId(file_id), out, kind);
        }
    }
    Ok(written)
}

#[derive(Debug, Serialize)]
pub struct ExportOut {
    pub source: String,
    pub out_path: String,
    pub kind: String,
    pub created_at: i64,
    /// False when the file has been moved or deleted since.
    pub exists: bool,
    /// Bytes on disk now; `None` when it is gone.
    pub size: Option<u64>,
}

#[tauri::command]
pub fn export_history(state: tauri::State<'_, AppState>) -> CmdResult<Vec<ExportOut>> {
    let conn = state.db()?;
    let rows = exports::recent(&conn, 100).map_err(|e| format!("basis data: {e}"))?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let size = std::fs::metadata(&r.out_path)
                .ok()
                .filter(|m| m.is_file())
                .map(|m| m.len());
            ExportOut {
                exists: size.is_some(),
                size,
                source: r.source,
                out_path: r.out_path,
                kind: r.kind,
                created_at: r.created_at,
            }
        })
        .collect())
}

/// Autosave (SPEC 8): writes a draft for every open document changed since
/// its last draft or save. The frontend calls this every 20 seconds and when
/// the window loses focus. Returns how many drafts were written.
#[tauri::command]
pub fn autosave_drafts(state: tauri::State<'_, AppState>) -> CmdResult<u32> {
    let docs: Vec<(u64, i64)> = {
        let ws = state.workspace.lock();
        ws.tabs().iter().map(|t| (t.doc, t.file_id)).collect()
    };
    let mut written = 0;
    let conn = state.db()?;
    for (doc, file_id) in docs {
        if file_id <= 0 {
            continue;
        }
        let Some(snap) = state.annots.snapshot_if_changed(doc) else {
            continue;
        };
        let Some(stamp) = state.stamps.lock().get(&doc).copied() else {
            continue;
        };
        let blob = serde_json::to_vec(&snap).map_err(|e| e.to_string())?;
        drafts::put(&conn, files::FileId(file_id), &base_of(&stamp), &blob)
            .map_err(|e| format!("draf tidak dapat ditulis: {e}"))?;
        written += 1;
    }
    Ok(written)
}

/// Writes the draft of one document now, whatever autosave last did — the
/// step before a reload that must not lose edits.
#[tauri::command]
pub fn draft_save_now(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<()> {
    let (_, file_id, _) = doc_path(&state, doc)?;
    if file_id <= 0 {
        return Err("berkas ini belum tercatat".into());
    }
    let stamp = state
        .stamps
        .lock()
        .get(&doc)
        .copied()
        .ok_or("berkas tidak dikenal")?;
    let blob = serde_json::to_vec(&state.annots.snapshot(doc)).map_err(|e| e.to_string())?;
    let conn = state.db()?;
    drafts::put(&conn, files::FileId(file_id), &base_of(&stamp), &blob).map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct DraftInfo {
    /// Seconds since the Unix epoch.
    pub updated_at: i64,
    pub objects: usize,
    /// False when the file changed after the draft was written. SPEC 8: such a
    /// draft is never applied without the user asking for exactly that.
    pub matches_file: bool,
}

/// Whether a draft is waiting for this document.
#[tauri::command]
pub fn draft_status(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<Option<DraftInfo>> {
    let (path, file_id, _) = doc_path(&state, doc)?;
    if file_id <= 0 {
        return Ok(None);
    }
    let conn = state.db()?;
    let Some(draft) = drafts::get(&conn, files::FileId(file_id)).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let snap: Snapshot = match serde_json::from_slice(&draft.blob) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(doc, error = %e, "draf rusak, dibuang");
            let _ = drafts::delete(&conn, files::FileId(file_id));
            return Ok(None);
        }
    };
    let now = FileStamp::of(&PathBuf::from(path)).ok();
    Ok(Some(DraftInfo {
        updated_at: draft.updated_at,
        objects: snap.objects.len(),
        matches_file: now.is_some_and(|s| base_of(&s) == draft.base),
    }))
}

/// Puts a draft back into the editor. Refuses a draft made against a file
/// that has since changed, unless `force` — which only a reload the user asked
/// for sets.
#[tauri::command]
pub fn draft_restore(
    state: tauri::State<'_, AppState>,
    doc: u64,
    force: bool,
) -> CmdResult<crate::annots::EditResult> {
    let (path, file_id, _) = doc_path(&state, doc)?;
    let conn = state.db()?;
    let draft = drafts::get(&conn, files::FileId(file_id))
        .map_err(|e| e.to_string())?
        .ok_or("tidak ada draf")?;
    let now = FileStamp::of(&PathBuf::from(path)).map_err(|e| e.to_string())?;
    if !force && base_of(&now) != draft.base {
        return Err("berkas sudah berubah sejak draf ini dibuat".into());
    }
    let snap: Snapshot = serde_json::from_slice(&draft.blob).map_err(|e| e.to_string())?;
    state.annots.restore(doc, snap);
    Ok(state.annots.edit_state(doc))
}

/// Throws the draft away — the "don't save" answer when a tab closes.
///
/// Also marks the current edits as drafted: autosave runs on a timer, and one
/// that fired between this and the tab closing would write the edits the user
/// just declined straight back, to be offered for recovery next time.
#[tauri::command]
pub fn draft_discard(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<()> {
    let (_, file_id, _) = doc_path(&state, doc)?;
    state.annots.mark_drafted(doc);
    let conn = state.db()?;
    drafts::delete(&conn, files::FileId(file_id)).map_err(|e| e.to_string())
}

/// "Ignore": the user has seen that another program changed the file and
/// keeps working on what they have. The file as it is now becomes the one
/// the tab is measured against, so the warning does not come straight back.
#[tauri::command]
pub fn file_acknowledge(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<()> {
    let (path, _, _) = doc_path(&state, doc)?;
    let stamp = FileStamp::of(&PathBuf::from(path)).map_err(|e| e.to_string())?;
    state.stamps.lock().insert(doc, stamp);
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct FileStatus {
    /// Another program wrote the file since this application opened or saved it.
    pub changed_on_disk: bool,
    /// The file is gone from where it was.
    pub missing: bool,
    pub dirty: bool,
    pub size: u64,
}

#[tauri::command]
pub fn file_status(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<FileStatus> {
    let (path, _, _) = doc_path(&state, doc)?;
    let known = state.stamps.lock().get(&doc).copied();
    let now = FileStamp::of(&PathBuf::from(&path)).ok();
    Ok(FileStatus {
        changed_on_disk: matches!((known, now), (Some(a), Some(b)) if a != b),
        missing: now.is_none(),
        dirty: state.annots.is_dirty(doc),
        size: now.map_or(0, |s| s.size),
    })
}

#[cfg(test)]
mod tests {
    use super::same_file;

    #[test]
    fn same_file_sees_through_spelling_and_ignores_missing_targets() {
        let dir = std::env::temp_dir().join(format!("izul-same-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let a = dir.join("a.pdf");
        std::fs::write(&a, b"%PDF").unwrap();
        // The same file reached another way round.
        assert!(same_file(&a, &dir.join("sub").join("..").join("a.pdf")));
        assert!(
            !same_file(&a, &dir.join("b.pdf")),
            "a new name is never open"
        );
        std::fs::write(dir.join("b.pdf"), b"%PDF").unwrap();
        assert!(!same_file(&a, &dir.join("b.pdf")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
