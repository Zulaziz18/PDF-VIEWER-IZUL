//! "Ekspor daftar" of the annotation panel (SPEC 11.2), found missing in the
//! Phase 8 audit: every annotation of the document as a CSV file.
//!
//! Every page, not only the ones that have been on screen: the panel lists
//! what has been imported so far, and an export that silently left out page
//! 340 would be worse than none. So the pages are imported first.
//!
//! Semicolons, not commas, and a byte-order mark: Excel with Indonesian regional
//! settings splits on `;` (the comma is the decimal separator there), and
//! without the mark it reads UTF-8 as ANSI and mangles every "é" and "—".

use std::collections::HashMap;

use izul_model::annot::{AnnotObject, AnnotPayload};
use izul_model::display::Rgba;

use crate::commands::AppState;

type CmdResult<T> = Result<T, String>;

/// One field, quoted when it has to be.
fn field(s: &str) -> String {
    if s.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn hex(c: &Rgba) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

fn content_of(o: &AnnotObject) -> String {
    let text = match &o.payload {
        AnnotPayload::FreeText { text, .. } => text.clone(),
        AnnotPayload::Stamp { label, .. } => label.clone(),
        AnnotPayload::Note { text, .. } => text.clone(),
        _ => String::new(),
    };
    if text.trim().is_empty() {
        o.author_note.clone()
    } else {
        text
    }
}

fn color_of(o: &AnnotObject) -> Option<Rgba> {
    match &o.payload {
        AnnotPayload::Markup { color, .. }
        | AnnotPayload::FreeText { color, .. }
        | AnnotPayload::Ink { color, .. }
        | AnnotPayload::Line { color, .. }
        | AnnotPayload::Note { color, .. }
        | AnnotPayload::Stamp { color, .. } => Some(*color),
        AnnotPayload::Shape { style } | AnnotPayload::Polygon { style, .. } => style.stroke,
        _ => None,
    }
}

/// The CSV text, objects in page order and top to bottom within a page, the
/// kind named by `labels` (the interface's own words for it).
pub fn csv(objects: &[AnnotObject], labels: &HashMap<String, String>) -> String {
    let mut sorted: Vec<&AnnotObject> = objects.iter().collect();
    sorted.sort_by(|a, b| {
        a.page
            .cmp(&b.page)
            .then(b.rect.top.total_cmp(&a.rect.top))
            .then(a.rect.left.total_cmp(&b.rect.left))
    });
    let mut out = String::from("\u{feff}Halaman;Jenis;Isi;Warna;Terkunci\r\n");
    for o in sorted {
        let kind = format!("{:?}", o.kind);
        let label = labels.get(&kind).cloned().unwrap_or(kind);
        let row = [
            (o.page + 1).to_string(),
            field(&label),
            field(&content_of(o)),
            color_of(o).map(|c| hex(&c)).unwrap_or_default(),
            if o.locked { "ya" } else { "tidak" }.to_string(),
        ];
        out.push_str(&row.join(";"));
        out.push_str("\r\n");
    }
    out
}

/// Writes every annotation of `doc` to `path` (a `.csv` the save dialog gave).
#[tauri::command]
pub async fn annots_export_csv(
    state: tauri::State<'_, AppState>,
    doc: u64,
    path: String,
    labels: HashMap<String, String>,
) -> CmdResult<usize> {
    if !path.to_ascii_lowercase().ends_with(".csv") {
        return Err("daftar anotasi hanya diekspor sebagai berkas .csv".into());
    }
    crate::pagemap::ensure_all_imported(&state, doc).await?;
    let objects = state.annots.objects(doc, None);
    std::fs::write(&path, csv(&objects, &labels)).map_err(|e| format!("{path}: {e}"))?;
    Ok(objects.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use izul_model::annot::{AnnotId, AnnotKind, FontSpec, TextAlign};
    use izul_model::geom::PdfRectF;

    fn text(id: u64, page: u32, top: f32, s: &str) -> AnnotObject {
        AnnotObject::new(
            AnnotId(id),
            page,
            AnnotKind::FreeText,
            PdfRectF::new(10.0, top - 20.0, 100.0, top),
            AnnotPayload::FreeText {
                text: s.into(),
                font: FontSpec::default(),
                color: Rgba::new(1.0, 0.0, 0.0, 1.0),
                align: TextAlign::Left,
                line_spacing: 1.2,
                background: None,
                border: None,
            },
        )
    }

    #[test]
    fn rows_come_in_reading_order_with_the_interfaces_names() {
        let labels = HashMap::from([("FreeText".to_string(), "Kotak teks".to_string())]);
        let out = csv(
            &[
                text(1, 1, 500.0, "kedua"),
                text(2, 0, 300.0, "bawah"),
                text(3, 0, 700.0, "atas"),
            ],
            &labels,
        );
        let rows: Vec<&str> = out.trim_end().split("\r\n").collect();
        assert!(rows[0].starts_with('\u{feff}'), "BOM untuk Excel");
        assert_eq!(rows[1], "1;Kotak teks;atas;#ff0000;tidak");
        assert_eq!(rows[2], "1;Kotak teks;bawah;#ff0000;tidak");
        assert_eq!(rows[3], "2;Kotak teks;kedua;#ff0000;tidak");
    }

    #[test]
    fn a_field_with_separators_or_quotes_is_quoted() {
        let out = csv(&[text(1, 0, 100.0, "a;b \"c\"\nd")], &HashMap::new());
        assert!(out.contains("1;FreeText;\"a;b \"\"c\"\"\nd\";"), "{out}");
    }
}
