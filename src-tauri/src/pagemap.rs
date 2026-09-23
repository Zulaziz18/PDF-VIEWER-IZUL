//! Page operations (SPEC 11.3, Phase 5): the commands, and where pages from
//! other files come from.
//!
//! The page map itself lives with the annotations (`izul_model::pages`, held
//! by `AnnotState`) because it undoes with them. What lives here is the part
//! that touches files and workers:
//!
//! * **Pages from another file are read from a copy** in the data folder, made
//!   when they are brought in. Reading the user's file directly would mean
//!   the result depends on what that file holds at *save* time — and the file
//!   may have been saved over, moved, or be another tab that is itself being
//!   rearranged. A copy is the file as it was when the user dragged from it.
//! * **Each such copy is opened as a hidden document** — a worker document
//!   that has no tab — so its pages render in the viewer exactly like the
//!   document's own: same tiles, same cache, same protocol.
//! * **Every page of the document is imported before the first page
//!   operation.** Annotations are imported lazily, page by page as they come
//!   into view; once pages move, "page n of the file" and "page n on screen"
//!   stop being the same page, and the lazy path would read the wrong one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::render::DocInfo;
use izul_ipc::message::{DocId, Request, Response};
use izul_model::annot::{AnnotObject, AnnotPayload};
use izul_model::display::ImageRef;
use izul_model::pages::{PageCommand, PageEntry};
use parking_lot::Mutex;
use serde::Serialize;

use crate::annots::{EditResult, SourceFile};
use crate::commands::AppState;
use crate::saving;
use crate::supervisor::Pool;

type CmdResult<T> = Result<T, String>;

/// Hidden documents, by (document, source id) → the worker document that
/// renders that source.
#[derive(Debug, Default)]
pub struct HiddenSources {
    docs: Mutex<HashMap<u64, HashMap<u32, u64>>>,
}

impl HiddenSources {
    fn get(&self, doc: u64, source: u32) -> Option<u64> {
        self.docs
            .lock()
            .get(&doc)
            .and_then(|m| m.get(&source))
            .copied()
    }

    fn put(&self, doc: u64, source: u32, hidden: u64) {
        self.docs
            .lock()
            .entry(doc)
            .or_default()
            .insert(source, hidden);
    }

    /// Every hidden document of `doc`, forgotten here; the caller closes them.
    pub fn take(&self, doc: u64) -> Vec<u64> {
        self.docs
            .lock()
            .remove(&doc)
            .map(|m| m.into_values().collect())
            .unwrap_or_default()
    }

    /// All hidden documents of `doc`, still registered — for passing a
    /// generation or a trim along.
    pub fn of(&self, doc: u64) -> Vec<u64> {
        self.docs
            .lock()
            .get(&doc)
            .map(|m| m.values().copied().collect())
            .unwrap_or_default()
    }
}

/// One page as the viewer lays it out.
#[derive(Debug, Clone, Serialize)]
pub struct PageView {
    /// The worker document to ask for its tiles, text and thumbnail; `None`
    /// for a blank page, which the viewer paints white itself.
    pub render_doc: Option<u64>,
    /// Page number in `render_doc`.
    pub page: u32,
    /// Quarter turns on top of the page's own `/Rotate`.
    pub rotation: u8,
    /// Display size with the page's own `/Rotate`, before `rotation`.
    pub width: f32,
    pub height: f32,
    /// 0 for the document's own file.
    pub source: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PagesView {
    pub pages: Vec<PageView>,
    /// The pages are not the file's own in order: saving will rearrange.
    pub mapped: bool,
    pub map_revision: u64,
    /// The original names of the files pages were brought in from.
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PagesResult {
    pub view: PagesView,
    pub edit: EditResult,
}

/// Copies `path` into the data folder's `sources/`, named by content, and
/// returns the copy's path. The same bytes are copied once.
pub fn copy_source(data_dir: &Path, path: &str) -> CmdResult<String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let hash = izul_store::content_hash(&bytes);
    let name: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
    let dir = sources_dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let out = dir.join(format!("{name}.pdf"));
    if out.is_file() {
        // Used again: it is not stale (see `sweep_sources`).
        if let Ok(f) = std::fs::File::options().write(true).open(&out) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
    } else {
        izul_write::atomic::PendingWrite::write(&out, &bytes)
            .and_then(izul_write::atomic::PendingWrite::commit)
            .map_err(|e| format!("{}: {e}", out.display()))?;
    }
    Ok(out.to_string_lossy().to_string())
}

/// Opens `path` in a worker as a document with no tab; returns its id and
/// page sizes.
async fn open_hidden(state: &AppState, path: &str) -> CmdResult<(u64, Vec<(f32, f32)>)> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let key = izul_store::content_hash(&bytes);
    drop(bytes);
    let doc = DocId(
        state
            .next_doc_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    );
    let reply = {
        let mut pool = state.pool.write().await;
        pool.open(doc, path, key, None).await
    };
    let sizes = match reply {
        Ok(Response::Opened { page_sizes, .. }) => page_sizes,
        Ok(Response::Error {
            message_id, detail, ..
        }) => return Err(format!("{message_id}: {detail}")),
        Ok(other) => return Err(format!("balasan tak terduga: {other:?}")),
        Err(e) => return Err(e.to_string()),
    };
    state.render.register(
        doc.0,
        DocInfo {
            path: path.to_string(),
            page_sizes: sizes.clone(),
            generation: 0,
        },
    );
    Ok((doc.0, sizes))
}

/// Closes a hidden document.
pub async fn close_hidden(state: &AppState, hidden: u64) {
    let worker = { state.pool.read().await.worker_for(DocId(hidden)) };
    if let Some(worker) = worker {
        let _ = Pool::ask(&worker, Request::Close { doc: DocId(hidden) }).await;
    }
    state.render.forget(hidden);
}

/// The worker document rendering source `source` of `doc`, opening it on
/// first use (after a restored draft, for instance).
async fn render_doc_of(state: &AppState, doc: u64, source: u32) -> CmdResult<u64> {
    if source == 0 {
        return Ok(doc);
    }
    if let Some(h) = state.hidden.get(doc, source) {
        return Ok(h);
    }
    let file = state
        .annots
        .sources(doc)
        .get(source as usize - 1)
        .cloned()
        .ok_or_else(|| format!("sumber halaman {source} tidak dikenal"))?;
    let (hidden, sizes) = open_hidden(state, &file.path).await?;
    // Fills the sizes in when the source came back from a draft without them.
    state.annots.add_source(doc, SourceFile { sizes, ..file });
    state.hidden.put(doc, source, hidden);
    Ok(hidden)
}

/// The pages as the viewer should lay them out.
pub async fn view(state: &AppState, doc: u64) -> CmdResult<PagesView> {
    let entries = state.annots.resolved_pages(doc);
    let own = state.annots.own_sizes(doc);
    let mapped = state.annots.page_map(doc).is_some();
    let mut pages = Vec::with_capacity(entries.len());
    for entry in entries {
        pages.push(match entry {
            PageEntry::Page {
                source,
                page,
                rotation,
            } => {
                let render = render_doc_of(state, doc, source).await?;
                let (width, height) = if source == 0 {
                    own.get(page as usize).copied()
                } else {
                    state
                        .annots
                        .sources(doc)
                        .get(source as usize - 1)
                        .and_then(|s| s.sizes.get(page as usize).copied())
                }
                .unwrap_or((612.0, 792.0));
                PageView {
                    render_doc: Some(render),
                    page,
                    rotation,
                    width,
                    height,
                    source,
                }
            }
            PageEntry::Blank {
                width,
                height,
                rotation,
            } => PageView {
                render_doc: None,
                page: 0,
                rotation,
                width,
                height,
                source: 0,
            },
        });
    }
    Ok(PagesView {
        pages,
        mapped,
        map_revision: state.annots.edit_state(doc).map_revision,
        sources: state
            .annots
            .sources(doc)
            .into_iter()
            .map(|s| s.origin)
            .collect(),
    })
}

/// Reads every page's saved annotations into the editor, once.
async fn ensure_all_imported(state: &AppState, doc: u64) -> CmdResult<()> {
    if state.annots.all_imported(doc) {
        return Ok(());
    }
    let n = state.annots.file_pages(doc);
    for page in 0..n {
        saving::import_page(&state.pool, &state.annots, doc, page).await?;
    }
    state.annots.mark_all_imported(doc);
    Ok(())
}

async fn apply(state: &AppState, doc: u64, cmd: PageCommand) -> CmdResult<PagesResult> {
    let edit = state.annots.apply_pages(doc, cmd)?;
    Ok(PagesResult {
        view: view(state, doc).await?,
        edit,
    })
}

fn open_path(state: &AppState, doc: u64) -> CmdResult<String> {
    let ws = state.workspace.lock();
    ws.get(doc)
        .map(|d| d.path.clone())
        .ok_or_else(|| "dokumen tidak terbuka".to_string())
}

// ---- commands -----------------------------------------------------------

#[tauri::command]
pub async fn pages_state(state: tauri::State<'_, AppState>, doc: u64) -> CmdResult<PagesView> {
    view(&state, doc).await
}

/// Delete, move, rotate, insert blank pages, duplicate. Pages from elsewhere
/// come through the two commands below, which know where they come from.
#[tauri::command]
pub async fn pages_apply(
    state: tauri::State<'_, AppState>,
    doc: u64,
    cmd: PageCommand,
) -> CmdResult<PagesResult> {
    if matches!(cmd, PageCommand::Insert { .. }) {
        return Err(
            "sisipan dari berkas lain memakai pages_insert_file atau pages_copy_from".into(),
        );
    }
    ensure_all_imported(&state, doc).await?;
    apply(&state, doc, cmd).await
}

/// "Gabung": every page of another PDF, with its annotations, before `at`.
#[tauri::command]
pub async fn pages_insert_file(
    state: tauri::State<'_, AppState>,
    doc: u64,
    path: String,
    at: u32,
) -> CmdResult<PagesResult> {
    if let Ok(own) = open_path(&state, doc) {
        if std::fs::canonicalize(&own).ok() == std::fs::canonicalize(&path).ok() {
            return Err("untuk menggandakan halaman dokumen ini, pakai Duplikat".into());
        }
    }
    ensure_all_imported(&state, doc).await?;
    let copy = copy_source(&state.data_dir, &path)?;
    let (hidden, sizes) = open_hidden(&state, &copy).await?;
    let n = sizes.len() as u32;
    let source = state.annots.add_source(
        doc,
        SourceFile {
            path: copy,
            origin: path,
            sizes,
        },
    );
    if state.hidden.get(doc, source).is_none() {
        state.hidden.put(doc, source, hidden);
    } else {
        close_hidden(&state, hidden).await;
    }
    let reader = state.hidden.get(doc, source).unwrap_or(hidden);
    let mut objects = Vec::new();
    for page in 0..n {
        for mut obj in saving::read_saved(&state.pool, &state.annots, reader, page, doc).await? {
            obj.page = page;
            objects.push(obj);
        }
    }
    let entries = (0..n)
        .map(|page| PageEntry::Page {
            source,
            page,
            rotation: 0,
        })
        .collect();
    apply(
        &state,
        doc,
        PageCommand::Insert {
            at,
            entries,
            objects,
        },
    )
    .await
}

/// Pages dragged from another open document, with their annotations —
/// unsaved ones included, which is the point of dragging rather than saving
/// and merging. With `remove`, they are then deleted from `from` (a move).
#[tauri::command]
pub async fn pages_copy_from(
    state: tauri::State<'_, AppState>,
    doc: u64,
    from: u64,
    pages: Vec<u32>,
    at: u32,
    remove: bool,
) -> CmdResult<PagesResult> {
    if doc == from {
        return Err("di dalam satu dokumen, halaman dipindah atau diduplikat".into());
    }
    ensure_all_imported(&state, doc).await?;
    ensure_all_imported(&state, from).await?;
    let from_path = open_path(&state, from)?;
    let resolved = state.annots.resolved_pages(from);
    let their_sources = state.annots.sources(from);
    // Source ids of `from`, translated into `doc`'s table on first use.
    let mut translated: HashMap<u32, u32> = HashMap::new();
    let mut entries = Vec::with_capacity(pages.len());
    let mut objects: Vec<AnnotObject> = Vec::new();
    for (rel, &p) in pages.iter().enumerate() {
        let entry = *resolved
            .get(p as usize)
            .ok_or_else(|| format!("halaman {} tidak ada", p + 1))?;
        let entry = match entry {
            PageEntry::Page {
                source,
                page,
                rotation,
            } => {
                let mine = match translated.get(&source) {
                    Some(s) => *s,
                    None => {
                        let file = if source == 0 {
                            SourceFile {
                                path: copy_source(&state.data_dir, &from_path)?,
                                origin: from_path.clone(),
                                sizes: state.annots.own_sizes(from),
                            }
                        } else {
                            their_sources
                                .get(source as usize - 1)
                                .cloned()
                                .ok_or_else(|| format!("sumber halaman {source} tidak dikenal"))?
                        };
                        let s = state.annots.add_source(doc, file);
                        translated.insert(source, s);
                        s
                    }
                };
                PageEntry::Page {
                    source: mine,
                    page,
                    rotation,
                }
            }
            blank @ PageEntry::Blank { .. } => blank,
        };
        entries.push(entry);
        for mut obj in state.annots.objects(from, Some(p)) {
            obj.page = rel as u32;
            // A picture's bytes belong to the document that drew it; the
            // copy gets its own.
            if let AnnotPayload::Image { image, .. } = &mut obj.payload {
                let stored = state
                    .annots
                    .image(from, image.0)
                    .ok_or("gambar anotasi tidak ditemukan")?;
                *image = ImageRef(state.annots.add_image(doc, stored.bytes)?);
            }
            objects.push(obj);
        }
    }
    let result = apply(
        &state,
        doc,
        PageCommand::Insert {
            at,
            entries,
            objects,
        },
    )
    .await?;
    if remove {
        state
            .annots
            .apply_pages(from, PageCommand::Delete { pages })?;
    }
    Ok(result)
}

/// How long a copy of a source file is kept. A draft that still refers to a
/// copy older than this can no longer restore those pages, and says so.
const SOURCE_KEEP: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);

/// Deletes copies of source files not touched for [`SOURCE_KEEP`]; called
/// once at startup. A copy is re-touched every time it is brought in again,
/// and one still open fails to delete on Windows and is simply kept.
pub fn sweep_sources(data_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(sources_dir(data_dir)) else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > SOURCE_KEEP);
        if old && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Where copies of source files live.
pub fn sources_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("sources")
}
