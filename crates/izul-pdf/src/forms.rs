//! AcroForm fields: reading them, and filling them the way a viewer does
//! (Phase 7).
//!
//! Filling goes through PDFium's form-fill environment — focus the widget,
//! select its text, replace it; pick an option; click a box — rather than
//! writing `/V` into the field dictionary. Only the first makes PDFium build
//! the widget's appearance again, and the appearance is what every other
//! reader draws: a `/V` changed on its own is a value that shows up in a form
//! viewer that regenerates appearances and nowhere else. For the same reason
//! a checkbox is ticked by a click, never by writing `/AS /Yes`: its "on"
//! state has whatever name its producer gave it (`/Ya`, `/1`, `/On`), and a
//! click makes PDFium use the right one.
//!
//! Widget rectangles are reported in display space, like everything else the
//! viewer draws over a page.
//!
//! The environment lives as long as the document (`Document::form`): PDFium
//! draws widgets only through it (`FPDF_FFLDraw`, see `render.rs`), so a
//! document whose environment came and went per call rendered its fields as
//! nothing — measured on `tools/ui-harness`'s sample form: 0 dark pixels in
//! a bordered text field PDFium drew without it, 680 in poppler's render.

use std::os::raw::{c_int, c_ulong};
use std::pin::Pin;

use pdfium_render::prelude::*;

use crate::engine::{Document, Engine};
use crate::error::{PdfError, Result};
use crate::ffi_guard::guard;
use crate::geom::{PdfRectF, RotationQuarter};

// `fpdf_formfill.h`.
const FIELD_PUSHBUTTON: c_int = 1;
const FIELD_CHECKBOX: c_int = 2;
const FIELD_RADIOBUTTON: c_int = 3;
const FIELD_COMBOBOX: c_int = 4;
const FIELD_LISTBOX: c_int = 5;
const FIELD_TEXTFIELD: c_int = 6;
const FIELD_SIGNATURE: c_int = 7;
/// `FORMTYPE_NONE` from `fpdf_formfill.h`: the document has no form.
const FORMTYPE_NONE: c_int = 0;
// `fpdf_annot.h`.
const FLAG_READONLY: c_int = 1 << 0;
const FLAG_TEXT_MULTILINE: c_int = 1 << 12;
const FLAG_CHOICE_EDIT: c_int = 1 << 18;
const FLAG_CHOICE_MULTI_SELECT: c_int = 1 << 21;
const ANNOT_WIDGET: FPDF_ANNOTATION_SUBTYPE = 20;

/// What kind of field a widget belongs to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FieldKind {
    Text {
        multiline: bool,
    },
    CheckBox,
    Radio,
    ComboBox {
        editable: bool,
    },
    ListBox {
        multiple: bool,
    },
    /// Push buttons and signatures: shown, not filled here.
    Other,
}

/// One widget of a field.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FormWidget {
    pub page: u32,
    /// Display space.
    pub rect: PdfRectF,
    /// The field's full name (`parent.child`), which is what is filled.
    pub name: String,
    pub kind: FieldKind,
    pub read_only: bool,
    /// Text value, or the field value of a choice.
    pub value: String,
    /// A checkbox or radio widget: whether it is on, and its "on" name.
    pub checked: bool,
    pub export: String,
    /// A choice: its options and which are selected.
    pub options: Vec<String>,
    pub selected: Vec<u32>,
}

/// A value to put into a field: the model's own type, so the value the undo
/// stack holds is the value written.
pub use izul_model::FormValue as FieldValue;

/// PDFium's form-fill environment for one document, for as long as this
/// lives. The info struct is pinned: PDFium keeps a pointer to it.
pub(crate) struct FormEnv {
    engine: &'static Engine,
    handle: FPDF_FORMHANDLE,
    _info: Pin<Box<FPDF_FORMFILLINFO>>,
}

impl std::fmt::Debug for FormEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FormEnv").finish_non_exhaustive()
    }
}

impl FormEnv {
    pub(crate) fn handle(&self) -> FPDF_FORMHANDLE {
        self.handle
    }

    /// The environment for `doc`, or `None` when it has no form — no reason
    /// to pay for one on every book, statement and scan.
    pub(crate) fn open(engine: &'static Engine, doc: FPDF_DOCUMENT) -> Option<Self> {
        // SAFETY: `doc` is a live document.
        if unsafe { engine.bindings().FPDF_GetFormType(doc) } == FORMTYPE_NONE {
            return None;
        }
        let mut info = Box::pin(FPDF_FORMFILLINFO {
            version: 2,
            Release: None,
            FFI_Invalidate: None,
            FFI_OutputSelectedRect: None,
            FFI_SetCursor: None,
            FFI_SetTimer: None,
            FFI_KillTimer: None,
            FFI_GetLocalTime: None,
            FFI_OnChange: None,
            FFI_GetPage: None,
            FFI_GetCurrentPage: None,
            FFI_GetRotation: None,
            FFI_ExecuteNamedAction: None,
            FFI_SetTextFieldFocus: None,
            FFI_DoURIAction: None,
            FFI_DoGoToAction: None,
            m_pJsPlatform: std::ptr::null_mut(),
            xfa_disabled: 1,
            FFI_DisplayCaret: None,
            FFI_GetCurrentPageIndex: None,
            FFI_SetCurrentPage: None,
            FFI_GotoURL: None,
            FFI_GetPageViewRect: None,
            FFI_PageEvent: None,
            FFI_PopupMenu: None,
            FFI_OpenFile: None,
            FFI_EmailTo: None,
            FFI_UploadTo: None,
            FFI_GetPlatform: None,
            FFI_GetLanguage: None,
            FFI_DownloadFromURL: None,
            FFI_PostRequestURL: None,
            FFI_PutRequestURL: None,
            FFI_OnFocusChange: None,
            FFI_DoURIActionWithKeyboardModifier: None,
        });
        // SAFETY: `doc` is live; `info` is pinned and outlives the handle,
        // which `Drop` releases first.
        let handle = unsafe {
            engine
                .bindings()
                .FPDFDOC_InitFormFillEnvironment(doc, &mut *info)
        };
        (!handle.is_null()).then_some(FormEnv {
            engine,
            handle,
            _info: info,
        })
    }
}

impl Drop for FormEnv {
    fn drop(&mut self) {
        // SAFETY: `handle` came from `FPDFDOC_InitFormFillEnvironment` and is
        // released once, after every page introduced to it has left it
        // (`Document::drop`).
        unsafe {
            self.engine
                .bindings()
                .FPDFDOC_ExitFormFillEnvironment(self.handle)
        };
    }
}

/// A UTF-16 string PDFium writes into a buffer after telling its length.
fn read_wide(get: impl Fn(*mut FPDF_WCHAR, c_ulong) -> c_ulong) -> String {
    let bytes = get(std::ptr::null_mut(), 0);
    if bytes < 2 {
        return String::new();
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
    get(buf.as_mut_ptr(), bytes);
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(buf.get(..end).unwrap_or_default())
}

fn utf16z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Document {
    /// Every widget on every page, or none when the document has no form.
    pub fn form_widgets(&self) -> Result<Vec<FormWidget>> {
        // No AcroForm, no widgets — and no reason to load every page of an
        // 800-page book to find that out.
        let form = self.form_handle();
        if form.is_null() {
            return Ok(Vec::new());
        }
        let geometries = (0..self.page_count())
            .map(|p| self.page_geometry(p))
            .collect::<Result<Vec<_>>>()?;
        let b = self.engine().bindings();
        let mut out = Vec::new();
        for (page, geometry) in (0..self.page_count()).zip(geometries) {
            let widgets = guard("form_widgets", || {
                self.with_page(page, |p| {
                    let mut found = Vec::new();
                    // SAFETY: `p` and `form` are live; every annotation handle
                    // is closed; buffers are sized from PDFium's own answer.
                    unsafe {
                        for i in 0..b.FPDFPage_GetAnnotCount(p) {
                            let a = b.FPDFPage_GetAnnot(p, i);
                            if a.is_null() {
                                continue;
                            }
                            if b.FPDFAnnot_GetSubtype(a) != ANNOT_WIDGET {
                                b.FPDFPage_CloseAnnot(a);
                                continue;
                            }
                            let flags = b.FPDFAnnot_GetFormFieldFlags(form, a);
                            let kind = match b.FPDFAnnot_GetFormFieldType(form, a) {
                                FIELD_TEXTFIELD => FieldKind::Text {
                                    multiline: flags & FLAG_TEXT_MULTILINE != 0,
                                },
                                FIELD_CHECKBOX => FieldKind::CheckBox,
                                FIELD_RADIOBUTTON => FieldKind::Radio,
                                FIELD_COMBOBOX => FieldKind::ComboBox {
                                    editable: flags & FLAG_CHOICE_EDIT != 0,
                                },
                                FIELD_LISTBOX => FieldKind::ListBox {
                                    multiple: flags & FLAG_CHOICE_MULTI_SELECT != 0,
                                },
                                FIELD_PUSHBUTTON | FIELD_SIGNATURE => FieldKind::Other,
                                _ => FieldKind::Other,
                            };
                            let name = read_wide(|buf, len| {
                                b.FPDFAnnot_GetFormFieldName(form, a, buf, len)
                            });
                            let value = read_wide(|buf, len| {
                                b.FPDFAnnot_GetFormFieldValue(form, a, buf, len)
                            });
                            let export = read_wide(|buf, len| {
                                b.FPDFAnnot_GetFormFieldExportValue(form, a, buf, len)
                            });
                            let checked = b.FPDFAnnot_IsChecked(form, a) != 0;
                            let count = b.FPDFAnnot_GetOptionCount(form, a).max(0);
                            let mut options = Vec::new();
                            let mut selected = Vec::new();
                            for o in 0..count {
                                options.push(read_wide(|buf, len| {
                                    b.FPDFAnnot_GetOptionLabel(form, a, o, buf, len)
                                }));
                                if b.FPDFAnnot_IsOptionSelected(form, a, o) != 0 {
                                    selected.push(o as u32);
                                }
                            }
                            let mut r = FS_RECTF {
                                left: 0.0,
                                top: 0.0,
                                right: 0.0,
                                bottom: 0.0,
                            };
                            let has_rect = b.FPDFAnnot_GetRect(a, &mut r) != 0;
                            b.FPDFPage_CloseAnnot(a);
                            if !has_rect || name.is_empty() {
                                continue;
                            }
                            let user = PdfRectF::new(
                                r.left.min(r.right),
                                r.bottom.min(r.top),
                                r.left.max(r.right),
                                r.bottom.max(r.top),
                            );
                            found.push(FormWidget {
                                page,
                                rect: geometry.to_display(user, RotationQuarter::None),
                                name,
                                kind,
                                read_only: flags & FLAG_READONLY != 0,
                                value,
                                checked,
                                export,
                                options,
                                selected,
                            });
                        }
                    }
                    Ok(found)
                })
            })?;
            out.extend(widgets);
        }
        Ok(out)
    }

    /// Fills the named fields. Returns how many widgets were changed and the
    /// pages they are on. A field
    /// that does not exist, is read-only, or does not take that kind of value
    /// is an error: a form half filled without saying so is worse than one
    /// not filled.
    pub fn fill_form(&self, values: &[(String, FieldValue)]) -> Result<(u32, Vec<u32>)> {
        if values.is_empty() {
            return Ok((0, Vec::new()));
        }
        let widgets = self.form_widgets()?;
        for (name, value) in values {
            let mine: Vec<&FormWidget> = widgets.iter().filter(|w| &w.name == name).collect();
            let Some(first) = mine.first() else {
                return Err(PdfError::Corrupt {
                    detail: format!("isian '{name}' tidak ada di formulir"),
                });
            };
            if first.read_only {
                return Err(PdfError::Corrupt {
                    detail: format!("isian '{name}' hanya-baca"),
                });
            }
            let fits = matches!(
                (&first.kind, value),
                (FieldKind::Text { .. }, FieldValue::Text(_))
                    | (FieldKind::CheckBox, FieldValue::Checked(_))
                    | (FieldKind::Radio, FieldValue::Radio(_))
                    | (FieldKind::ComboBox { .. }, FieldValue::Choice(_))
                    | (FieldKind::ComboBox { editable: true }, FieldValue::Text(_))
                    | (FieldKind::ListBox { .. }, FieldValue::Choice(_))
            );
            if !fits {
                return Err(PdfError::Corrupt {
                    detail: format!("isian '{name}' tidak menerima nilai itu"),
                });
            }
            if let FieldValue::Radio(export) = value {
                if !export.is_empty() && !mine.iter().any(|w| &w.export == export) {
                    return Err(PdfError::Corrupt {
                        detail: format!("isian '{name}' tidak punya pilihan '{export}'"),
                    });
                }
            }
        }
        let form = self.form_handle();
        if form.is_null() {
            return Err(PdfError::Corrupt {
                detail: "dokumen ini tidak punya formulir".into(),
            });
        }
        let b = self.engine().bindings();
        let mut changed = 0u32;
        for page in 0..self.page_count() {
            let wanted: Vec<&(String, FieldValue)> = values
                .iter()
                .filter(|(n, _)| widgets.iter().any(|w| w.page == page && &w.name == n))
                .collect();
            if wanted.is_empty() {
                continue;
            }
            changed += guard("fill_form", || {
                self.with_page(page, |p| {
                    let mut n = 0u32;
                    // SAFETY: `p` and `form` are live; every annotation handle
                    // is closed; strings are NUL-terminated UTF-16.
                    unsafe {
                        for i in 0..b.FPDFPage_GetAnnotCount(p) {
                            let a = b.FPDFPage_GetAnnot(p, i);
                            if a.is_null() {
                                continue;
                            }
                            let name = read_wide(|buf, len| {
                                b.FPDFAnnot_GetFormFieldName(form, a, buf, len)
                            });
                            let Some((_, value)) = wanted.iter().find(|(nm, _)| *nm == name) else {
                                b.FPDFPage_CloseAnnot(a);
                                continue;
                            };
                            let mut r = FS_RECTF {
                                left: 0.0,
                                top: 0.0,
                                right: 0.0,
                                bottom: 0.0,
                            };
                            b.FPDFAnnot_GetRect(a, &mut r);
                            let (cx, cy) = (
                                f64::from(r.left + r.right) / 2.0,
                                f64::from(r.top + r.bottom) / 2.0,
                            );
                            let click = || {
                                b.FORM_OnMouseMove(form, p, 0, cx, cy);
                                b.FORM_OnLButtonDown(form, p, 0, cx, cy);
                                b.FORM_OnLButtonUp(form, p, 0, cx, cy);
                                b.FORM_ForceToKillFocus(form);
                            };
                            match value {
                                FieldValue::Text(text) => {
                                    b.FORM_SetFocusedAnnot(form, a);
                                    b.FORM_SelectAllText(form, p);
                                    let wide = utf16z(text);
                                    b.FORM_ReplaceSelection(
                                        form,
                                        p,
                                        wide.as_ptr() as FPDF_WIDESTRING,
                                    );
                                    b.FORM_ForceToKillFocus(form);
                                    n += 1;
                                }
                                FieldValue::Checked(on) => {
                                    if (b.FPDFAnnot_IsChecked(form, a) != 0) != *on {
                                        click();
                                        n += 1;
                                    }
                                }
                                FieldValue::Radio(export) => {
                                    let mine = read_wide(|buf, len| {
                                        b.FPDFAnnot_GetFormFieldExportValue(form, a, buf, len)
                                    });
                                    if &mine == export && b.FPDFAnnot_IsChecked(form, a) == 0 {
                                        click();
                                        n += 1;
                                    }
                                }
                                FieldValue::Choice(indices) => {
                                    b.FORM_SetFocusedAnnot(form, a);
                                    let count = b.FPDFAnnot_GetOptionCount(form, a).max(0);
                                    for o in 0..count {
                                        let want = indices.contains(&(o as u32));
                                        if (b.FPDFAnnot_IsOptionSelected(form, a, o) != 0) != want {
                                            b.FORM_SetIndexSelected(form, p, o, i32::from(want));
                                        }
                                    }
                                    b.FORM_ForceToKillFocus(form);
                                    n += 1;
                                }
                            }
                            b.FPDFPage_CloseAnnot(a);
                        }
                    }
                    Ok(n)
                })
            })?;
        }
        let mut pages: Vec<u32> = widgets
            .iter()
            .filter(|w| values.iter().any(|(n, _)| n == &w.name))
            .map(|w| w.page)
            .collect();
        pages.sort_unstable();
        pages.dedup();
        Ok((changed, pages))
    }
}
