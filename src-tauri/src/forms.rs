//! Filling AcroForm fields (Phase 7).
//!
//! The values live in the editor (`izul_model::Op::Form`, so undo and the
//! unsaved-changes prompt cover them) and go into the file when it is saved,
//! through PDFium's form-fill environment on the working copy — which builds
//! each widget's appearance again, so other readers show the value too
//! (`tools/form-proof`). Each change is also put into the open document at
//! once, and the page's tiles are dropped: the page shows what was typed
//! before anything is saved.

use std::collections::BTreeMap;

use izul_ipc::message::{DocId, FormKindWire, FormWidgetWire, Request, Response};
use izul_model::FormValue;
use serde::Serialize;

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::annots::{AnnotState, EditResult};
use crate::commands::AppState;
use crate::render::RenderService;
use crate::saving::{ask, worker_of};
use crate::supervisor::Pool;

type CmdResult<T> = Result<T, String>;

/// One field, all its widgets together.
#[derive(Debug, Clone, Serialize)]
pub struct FormFieldView {
    pub name: String,
    pub kind: FormKindWire,
    pub read_only: bool,
    /// Where its widgets are: (page, display-space rectangle).
    pub widgets: Vec<(u32, izul_model::geom::PdfRectF)>,
    /// The value now: this session's, or the file's.
    pub value: FormValue,
    /// A choice's options.
    pub options: Vec<String>,
    /// A radio group's buttons, by export value, in widget order.
    pub exports: Vec<String>,
}

/// The field's value as the file holds it.
fn file_value(kind: FormKindWire, widgets: &[&FormWidgetWire]) -> FormValue {
    let first = widgets.first();
    match kind {
        FormKindWire::Text { .. } | FormKindWire::Other => {
            FormValue::Text(first.map(|w| w.value.clone()).unwrap_or_default())
        }
        FormKindWire::CheckBox => FormValue::Checked(widgets.iter().any(|w| w.checked)),
        FormKindWire::Radio => FormValue::Radio(
            widgets
                .iter()
                .find(|w| w.checked)
                .map(|w| w.export.clone())
                .unwrap_or_default(),
        ),
        FormKindWire::ComboBox { .. } | FormKindWire::ListBox { .. } => {
            FormValue::Choice(first.map(|w| w.selected.clone()).unwrap_or_default())
        }
    }
}

async fn widgets(pool: &Arc<RwLock<Pool>>, doc: u64) -> CmdResult<Vec<FormWidgetWire>> {
    let worker = worker_of(pool, doc).await?;
    match ask(&worker, Request::FormFields { doc: DocId(doc) }).await? {
        Response::FormFieldsReady { widgets, .. } => Ok(widgets),
        other => Err(format!("balasan tak terduga: {other:?}")),
    }
}

/// Groups widgets into fields, in the order they first appear.
fn fields(widgets: &[FormWidgetWire], set: &BTreeMap<String, FormValue>) -> Vec<FormFieldView> {
    let mut order: Vec<&str> = Vec::new();
    for w in widgets {
        if !order.contains(&w.name.as_str()) {
            order.push(&w.name);
        }
    }
    order
        .into_iter()
        .filter_map(|name| {
            let mine: Vec<&FormWidgetWire> = widgets.iter().filter(|w| w.name == name).collect();
            let first = *mine.first()?;
            if matches!(first.kind, FormKindWire::Other) {
                return None;
            }
            Some(FormFieldView {
                name: name.to_string(),
                kind: first.kind,
                read_only: first.read_only,
                widgets: mine.iter().map(|w| (w.page, w.rect)).collect(),
                value: set
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| file_value(first.kind, &mine)),
                options: first.options.clone(),
                exports: if matches!(first.kind, FormKindWire::Radio) {
                    mine.iter().map(|w| w.export.clone()).collect()
                } else {
                    Vec::new()
                },
            })
        })
        .collect()
}

/// Puts `changes` into the open document and drops the tiles of the pages
/// they are on. Returns those pages.
pub(crate) async fn show(
    state: &AppState,
    doc: u64,
    changes: Vec<(String, FormValue)>,
) -> CmdResult<Vec<u32>> {
    show_in(&state.pool, Some(&state.render), doc, changes).await
}

/// [`show`], for callers without the application state (tests).
pub async fn show_in(
    pool: &Arc<RwLock<Pool>>,
    render: Option<&RenderService>,
    doc: u64,
    changes: Vec<(String, FormValue)>,
) -> CmdResult<Vec<u32>> {
    if changes.is_empty() {
        return Ok(Vec::new());
    }
    let worker = worker_of(pool, doc).await?;
    let pages = match ask(
        &worker,
        Request::FillForm {
            doc: DocId(doc),
            working: false,
            values: changes,
        },
    )
    .await?
    {
        Response::FormFilled { pages, .. } => pages,
        other => return Err(format!("balasan tak terduga: {other:?}")),
    };
    if let Some(render) = render {
        for &page in &pages {
            render.invalidate_page(doc, page);
        }
    }
    Ok(pages)
}

/// The document's fields, with this session's values.
pub async fn field_list(
    pool: &Arc<RwLock<Pool>>,
    annots: &AnnotState,
    doc: u64,
) -> CmdResult<Vec<FormFieldView>> {
    let widgets = widgets(pool, doc).await?;
    let set: BTreeMap<String, FormValue> = annots.form_values(doc).into_iter().collect();
    Ok(fields(&widgets, &set))
}

/// Sets one field's value (one undo step) and shows it on the page.
pub async fn set_field(
    pool: &Arc<RwLock<Pool>>,
    annots: &AnnotState,
    render: Option<&RenderService>,
    doc: u64,
    name: String,
    value: FormValue,
) -> CmdResult<EditResult> {
    let field = field_list(pool, annots, doc)
        .await?
        .into_iter()
        .find(|f| f.name == name)
        .ok_or_else(|| format!("isian '{name}' tidak ada di formulir"))?;
    if field.read_only {
        return Err(format!("isian '{name}' hanya-baca"));
    }
    if !field.value.same_kind(&value) {
        return Err(format!("isian '{name}' tidak menerima nilai itu"));
    }
    if field.value == value {
        return Ok(annots.edit_state(doc));
    }
    let mut result = annots
        .set_form(doc, name, field.value, value)
        .map_err(|e| e.to_string())?;
    result.repaint = show_in(pool, render, doc, std::mem::take(&mut result.form_changes)).await?;
    Ok(result)
}

#[tauri::command]
pub async fn form_fields(
    state: tauri::State<'_, AppState>,
    doc: u64,
) -> CmdResult<Vec<FormFieldView>> {
    field_list(&state.pool, &state.annots, doc).await
}

/// Sets one field's value (one undo step) and shows it on the page.
#[tauri::command]
pub async fn form_set(
    state: tauri::State<'_, AppState>,
    doc: u64,
    name: String,
    value: FormValue,
) -> CmdResult<EditResult> {
    set_field(
        &state.pool,
        &state.annots,
        Some(&state.render),
        doc,
        name,
        value,
    )
    .await
}
