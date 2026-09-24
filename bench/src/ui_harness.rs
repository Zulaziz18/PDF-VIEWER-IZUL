//! Backend for the UI screenshot harness (`npm run ui:shots`).
//!
//! The harness runs the real frontend in Chromium with Tauri's IPC mocked, so
//! it needs something to answer the requests a webview would send to the
//! application. Most of those are canned data. Pixels are not: a screenshot of
//! a viewer whose pages are grey boxes cannot be compared with a reference that
//! shows real pages, so this process renders tiles with the same PDFium, the
//! same tile grid (`izul_render::tile_source`) and the same display-list
//! builder the application uses.
//!
//! Protocol: one JSON request per line on stdin; one JSON header line on
//! stdout, followed by `len` raw bytes when the header carries them. Nothing
//! here touches the network, and nothing here is part of the shipped
//! application.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use izul_model::annot::{
    AnnotId, AnnotKind, AnnotObject, AnnotPayload, FontSpec, NoteIcon, ShapeStyle, TextAlign,
};
use izul_model::display::Rgba;
use izul_model::geom::{PdfPointF, PdfRectF, RotationQuarter};
use izul_pdf::engine::{Document, Engine};
use izul_pdf::fonts::{metrics_document, StandardFonts};
use izul_pdf::render::{Quality, TileRequest};
use serde::Deserialize;
use serde_json::{json, Value};

const TILE_EDGE: u32 = 512;

#[derive(Deserialize)]
struct Req {
    op: String,
    path: String,
    #[serde(default)]
    page: u32,
    #[serde(default)]
    rotation: u8,
    #[serde(default)]
    scale: u32,
    #[serde(default)]
    col: u16,
    #[serde(default)]
    row: u16,
    #[serde(default)]
    tier: String,
    /// Where `redact` writes its result.
    #[serde(default)]
    out: String,
}

/// Phase 6's sample: on "Data Pegawai", the runs of digits and dashes long
/// enough to be an identity or phone number, in display space — what a user
/// would mark for redaction on that page.
fn private_runs(doc: &Document, page: u32) -> Result<Vec<PdfRectF>, String> {
    let text = doc
        .page_text_boxed(page, RotationQuarter::None)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut run: Vec<PdfRectF> = Vec::new();
    let flush = |run: &mut Vec<PdfRectF>, out: &mut Vec<PdfRectF>| {
        if run.len() >= 12 {
            if let Some(r) = run.iter().copied().reduce(|a, b| a.union(&b)) {
                out.push(r);
            }
        }
        run.clear();
    };
    for c in &text.chars {
        if c.unicode.is_ascii_digit() || c.unicode == '-' {
            run.push(c.rect);
        } else {
            flush(&mut run, &mut out);
        }
    }
    flush(&mut run, &mut out);
    Ok(out)
}

fn redaction_marks(runs: &[PdfRectF], page: u32) -> Vec<AnnotObject> {
    runs.iter()
        .enumerate()
        .map(|(i, r)| {
            let mut obj = AnnotObject::new(
                AnnotId(900 + i as u64),
                page,
                AnnotKind::Redact,
                *r,
                AnnotPayload::Markup {
                    quads: vec![*r],
                    color: Rgba::BLACK,
                },
            );
            obj.z = 900 + i as i32;
            obj.created_at = 1_758_000_000_000;
            obj.modified_at = 1_758_000_000_000;
            obj
        })
        .collect()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn pdfium() -> PathBuf {
    if cfg!(windows) {
        repo_root().join("vendor/pdfium/win-x64/bin/pdfium.dll")
    } else {
        repo_root().join("vendor/pdfium/linux-x64/lib/libpdfium.so")
    }
}

fn quarter(r: u8) -> RotationQuarter {
    RotationQuarter::from_degrees(i32::from(r) * 90)
}

fn rgba(hex: u32, a: f32) -> Rgba {
    Rgba::new(
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
        a,
    )
}

fn pt(x: f32, y: f32) -> PdfPointF {
    PdfPointF { x, y }
}

/// The annotated page the harness shows: one of nearly every kind, placed over
/// the sample document's own text so the screenshots show them in context.
fn sample_annotations(page: u32, height: f32) -> Vec<AnnotObject> {
    let top = height - 96.0;
    let mut out = Vec::new();
    let mut id = 1u64 + u64::from(page) * 100;
    let mut push = |kind: AnnotKind, rect: PdfRectF, payload: AnnotPayload| {
        let mut obj = AnnotObject::new(AnnotId(id), page, kind, rect, payload);
        obj.z = id as i32;
        obj.created_at = 1_758_000_000_000;
        obj.modified_at = 1_758_000_000_000;
        obj.recompute_rect();
        out.push(obj);
        id += 1;
    };
    let line = |i: f32| PdfRectF::new(56.0, top - 15.0 * i - 4.0, 520.0, top - 15.0 * i + 11.0);
    push(
        AnnotKind::Highlight,
        line(0.0),
        AnnotPayload::Markup {
            quads: vec![line(0.0), PdfRectF::new(56.0, top - 19.0, 300.0, top - 4.0)],
            color: rgba(0xffd43b, 1.0),
        },
    );
    push(
        AnnotKind::Underline,
        line(3.0),
        AnnotPayload::Markup {
            quads: vec![PdfRectF::new(56.0, top - 49.0, 420.0, top - 34.0)],
            color: rgba(0x1c7ed6, 1.0),
        },
    );
    push(
        AnnotKind::Rect,
        PdfRectF::new(48.0, top - 170.0, 548.0, top - 100.0),
        AnnotPayload::Shape {
            style: ShapeStyle {
                fill: None,
                stroke: Some(rgba(0xe03131, 1.0)),
                stroke_width: 2.0,
                ..ShapeStyle::default()
            },
        },
    );
    push(
        AnnotKind::Arrow,
        PdfRectF::new(380.0, top - 250.0, 470.0, top - 172.0),
        AnnotPayload::Line {
            from: pt(470.0, top - 250.0),
            to: pt(400.0, top - 174.0),
            color: rgba(0xe03131, 1.0),
            width: 2.0,
            dashed: false,
            arrow_head: 12.0,
        },
    );
    push(
        AnnotKind::FreeText,
        PdfRectF::new(360.0, top - 300.0, 560.0, top - 252.0),
        AnnotPayload::FreeText {
            text: "Periksa lagi tanggal ini\nsebelum dicetak.".into(),
            font: FontSpec {
                size: 12.0,
                ..FontSpec::default()
            },
            color: rgba(0xc92a2a, 1.0),
            align: TextAlign::Left,
            line_spacing: 1.2,
            background: Some(rgba(0xfff5f5, 1.0)),
            border: Some(rgba(0xe03131, 1.0)),
        },
    );
    push(
        AnnotKind::Ink,
        PdfRectF::new(60.0, top - 330.0, 260.0, top - 280.0),
        AnnotPayload::Ink {
            strokes: vec![(0..24)
                .map(|i| {
                    let t = i as f32 / 23.0;
                    pt(64.0 + 190.0 * t, top - 305.0 + 16.0 * (t * 9.0).sin())
                })
                .collect()],
            color: rgba(0x2f9e44, 1.0),
            width: 2.5,
            smooth: true,
        },
    );
    push(
        AnnotKind::Note,
        PdfRectF::new(530.0, top - 40.0, 552.0, top - 18.0),
        AnnotPayload::Note {
            icon: NoteIcon::Comment,
            color: rgba(0xfab005, 1.0),
            text: "Tambahkan contoh pengisian.".into(),
        },
    );
    push(
        AnnotKind::Stamp,
        PdfRectF::new(400.0, 60.0, 540.0, 104.0),
        AnnotPayload::Stamp {
            label: "DISETUJUI".into(),
            color: rgba(0x2f9e44, 1.0),
            font: FontSpec {
                family: "Helvetica".into(),
                size: 18.0,
                bold: true,
                italic: false,
            },
        },
    );
    out
}

struct Backend {
    engine: &'static Engine,
    docs: HashMap<String, Document>,
    fonts: StandardFonts,
    _fonts_doc: Document,
}

impl Backend {
    fn doc(&mut self, path: &str) -> Result<&Document, String> {
        if !self.docs.contains_key(path) {
            let doc = self.engine.open(path, None).map_err(|e| e.to_string())?;
            self.docs.insert(path.to_string(), doc);
        }
        self.docs.get(path).ok_or_else(|| "hilang".to_string())
    }

    fn handle(&mut self, req: &Req) -> Result<(Value, Vec<u8>), String> {
        match req.op.as_str() {
            "open" => {
                let doc = self.doc(&req.path)?;
                let sizes = doc.page_sizes().map_err(|e| e.to_string())?;
                let sizes: Vec<[f32; 2]> = sizes.iter().map(|s| [s.width, s.height]).collect();
                Ok((
                    json!({ "page_count": doc.page_count(), "sizes": sizes }),
                    Vec::new(),
                ))
            }
            "outline" => {
                let doc = self.doc(&req.path)?;
                let nodes = doc.outline().map_err(|e| e.to_string())?;
                let mut flat = Vec::new();
                fn walk(
                    out: &mut Vec<Value>,
                    nodes: &[izul_pdf::outline::OutlineNode],
                    depth: u32,
                ) {
                    for n in nodes {
                        out.push(
                            json!({ "title": n.title, "depth": depth, "page": n.page, "y": n.y }),
                        );
                        walk(out, &n.children, depth + 1);
                    }
                }
                walk(&mut flat, &nodes, 0);
                Ok((json!({ "outline": flat }), Vec::new()))
            }
            "text" => {
                let doc = self.doc(&req.path)?;
                let text = doc
                    .page_text_boxed(req.page, quarter(req.rotation))
                    .map_err(|e| e.to_string())?;
                let chars: Vec<Value> = text
                    .chars
                    .iter()
                    .map(|c| {
                        json!({ "c": c.unicode.to_string(), "left": c.rect.left, "bottom": c.rect.bottom,
                                "right": c.rect.right, "top": c.rect.top })
                    })
                    .collect();
                Ok((
                    json!({ "page": req.page, "text": text.text, "chars": chars }),
                    Vec::new(),
                ))
            }
            "annots" => {
                let objects = if req.path.ends_with("Data Pegawai.pdf") {
                    let runs = private_runs(self.doc(&req.path)?, req.page)?;
                    redaction_marks(&runs, req.page)
                } else {
                    let height = self
                        .doc(&req.path)?
                        .page_size(req.page)
                        .map_err(|e| e.to_string())?
                        .height;
                    sample_annotations(req.page, height)
                };
                let lists: Vec<Value> = objects
                    .iter()
                    .filter_map(|o| {
                        izul_model::build::display_list(o, &self.fonts)
                            .ok()
                            .map(|ops| json!({ "id": o.id.0, "ops": ops }))
                    })
                    .collect();
                Ok((json!({ "objects": objects, "lists": lists }), Vec::new()))
            }
            "tile" => {
                let rotation = quarter(req.rotation);
                let doc = self.doc(&req.path)?;
                let size = doc
                    .page_display_size(req.page, rotation)
                    .map_err(|e| e.to_string())?;
                let request = if req.tier == "preview" {
                    let cap = req.scale.clamp(16, TILE_EDGE) as f32;
                    let s = size.scale_to_fit(cap, cap);
                    TileRequest {
                        page: req.page,
                        source: PdfRectF::new(0.0, 0.0, size.width, size.height),
                        dest_w: ((size.width * s).round() as u32).clamp(1, TILE_EDGE),
                        dest_h: ((size.height * s).round() as u32).clamp(1, TILE_EDGE),
                        rotation,
                        draw_annotations: true,
                        quality: Quality::Fast,
                        limit_image_cache: false,
                    }
                } else {
                    let ppp = req.scale as f32 / 1000.0;
                    let (source, w, h) =
                        izul_render::tile_source(size.width, size.height, ppp, req.col, req.row)
                            .map_err(|e| e.to_string())?;
                    TileRequest {
                        page: req.page,
                        source,
                        dest_w: w,
                        dest_h: h,
                        rotation,
                        draw_annotations: true,
                        quality: Quality::Sharp,
                        limit_image_cache: false,
                    }
                };
                let mut buf = vec![0u8; (request.dest_w * request.dest_h * 4) as usize];
                let geo = doc
                    .render_tile_into(&request, &mut buf)
                    .map_err(|e| e.to_string())?;
                buf.truncate(geo.required_bytes());
                Ok((
                    json!({ "width": geo.width, "height": geo.height, "stride": geo.stride }),
                    buf,
                ))
            }
            "redact" => {
                // The "after" sample, made by the real pipeline: the same
                // planning and checks the worker runs, then izul-redact.
                let doc = self.doc(&req.path)?;
                let runs = private_runs(doc, req.page)?;
                let geometry = doc.page_geometry(req.page).map_err(|e| e.to_string())?;
                let chars = doc.char_layout(req.page).map_err(|e| e.to_string())?;
                let areas = izul_pdf::redaction::plan_areas(&geometry, &runs, &chars);
                let plan = izul_redact::PageAreas {
                    page: req.page,
                    areas: areas
                        .iter()
                        .map(|a| izul_redact::Area {
                            rect: izul_redact::Rect::new(
                                f64::from(a.left),
                                f64::from(a.bottom),
                                f64::from(a.right),
                                f64::from(a.top),
                            ),
                            fill: Some([0.0, 0.0, 0.0]),
                        })
                        .collect(),
                };
                let bytes = doc.save_to_vec().map_err(|e| e.to_string())?;
                let (out, _) = izul_redact::redact(&bytes, &[plan]).map_err(|e| e.to_string())?;
                let check = self
                    .engine
                    .open_bytes(out.clone(), None::<&str>, None)
                    .map_err(|e| e.to_string())?;
                let after = check.char_layout(req.page).map_err(|e| e.to_string())?;
                izul_pdf::redaction::check(&areas, &chars, &after).map_err(|e| e.to_string())?;
                std::fs::write(&req.out, &out).map_err(|e| e.to_string())?;
                Ok((json!({ "areas": areas.len() }), Vec::new()))
            }
            other => Err(format!("op tidak dikenal: {other}")),
        }
    }
}

fn main() {
    let engine = Engine::load_from(pdfium()).expect("PDFium (vendor/pdfium/fetch.sh)");
    let fonts_doc = metrics_document(engine).expect("dokumen metrik");
    let fonts = StandardFonts::new(engine, fonts_doc.handle());
    let mut backend = Backend {
        engine,
        docs: HashMap::new(),
        fonts,
        _fonts_doc: fonts_doc,
    };

    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let (header, bytes) = match serde_json::from_str::<Req>(&line) {
            Ok(req) => match backend.handle(&req) {
                Ok((mut v, bytes)) => {
                    v["ok"] = json!(true);
                    v["len"] = json!(bytes.len());
                    (v, bytes)
                }
                Err(e) => (json!({ "ok": false, "error": e, "len": 0 }), Vec::new()),
            },
            Err(e) => (
                json!({ "ok": false, "error": e.to_string(), "len": 0 }),
                Vec::new(),
            ),
        };
        writeln!(out, "{header}").expect("stdout");
        out.write_all(&bytes).expect("stdout");
        out.flush().expect("stdout");
    }
}
