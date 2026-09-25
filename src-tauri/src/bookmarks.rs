//! User bookmarks (SPEC 11.1), per file, in the application's database —
//! never written into the PDF, so marking a place in a document someone sent
//! does not change the document (Phase 8).

use izul_store::bookmarks::{self, Bookmark};
use izul_store::files::FileId;

use crate::commands::AppState;

type CmdResult<T> = Result<T, String>;

/// Longer than any heading a reader would type; short enough for one row.
const MAX_LABEL: usize = 200;

fn file_of(state: &AppState, doc: u64) -> CmdResult<FileId> {
    state
        .workspace
        .lock()
        .get(doc)
        .map(|d| d.file_id)
        .filter(|id| *id > 0)
        .map(FileId)
        .ok_or_else(|| format!("dokumen {doc} tidak terbuka"))
}

fn label_of(label: &str) -> CmdResult<String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("Nama bookmark tidak boleh kosong.".into());
    }
    Ok(label.chars().take(MAX_LABEL).collect())
}

#[tauri::command]
pub fn bookmarks_list(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<Vec<Bookmark>> {
    let file = file_of(&state, doc)?;
    bookmarks::list(&state.db()?, file).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn bookmark_add(
    state: tauri::State<'_, AppState>,
    doc: u64,
    page: u32,
    label: String,
) -> CmdResult<Vec<Bookmark>> {
    let file = file_of(&state, doc)?;
    let conn = state.db()?;
    bookmarks::add(&conn, file, page, &label_of(&label)?).map_err(|e| e.to_string())?;
    bookmarks::list(&conn, file).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn bookmark_rename(
    state: tauri::State<'_, AppState>,
    doc: u64,
    id: i64,
    label: String,
) -> CmdResult<Vec<Bookmark>> {
    let file = file_of(&state, doc)?;
    let conn = state.db()?;
    bookmarks::rename(&conn, file, id, &label_of(&label)?).map_err(|e| e.to_string())?;
    bookmarks::list(&conn, file).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn bookmark_remove(
    state: tauri::State<'_, AppState>,
    doc: u64,
    id: i64,
) -> CmdResult<Vec<Bookmark>> {
    let file = file_of(&state, doc)?;
    let conn = state.db()?;
    bookmarks::remove(&conn, file, id).map_err(|e| e.to_string())?;
    bookmarks::list(&conn, file).map_err(|e| e.to_string())
}
