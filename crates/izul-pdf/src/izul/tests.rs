//! Phase 4's pass criterion, the half that can run without a window:
//!
//! > *Lulus bila: anotasi yang disimpan lalu dibuka ulang tetap editable …*
//!
//! Every one of the thirteen kinds is saved through the real path — PDFium
//! places placeholders and writes the file, `izul-write` fills them in — and
//! the file is opened again. The objects must come back equal to what was
//! saved, the picture must come back as the picture, and for every kind that
//! draws no text the saved annotation must render within SPEC 3.3's 0.5 % of
//! the Phase 3 golden rendering of the same appearance stream. (Text kinds
//! are drawn with the golden test's own Type 3 font there, and with real
//! standard-14 faces here, so for them the check is that they draw at all.)

use izul_model::annot::{AnnotKind, AnnotObject, AnnotPayload};
use izul_model::ap::appearance;
use izul_model::build::display_list;
use izul_model::display::{FontRef, ImageRef};
use izul_model::font::FixedFont;
use izul_write::{patch, AnnotWrite};

use crate::engine_and_lock;
use crate::fonts::{metrics_document, StandardFonts};
use crate::golden::{
    differing_fraction, page_with, render, sample, CHECKER, GLYPH_ADVANCE_MILLI,
    MAX_DIFFERING_FRACTION, PAGE_H, PAGE_W,
};

const KINDS: [AnnotKind; 13] = [
    AnnotKind::Highlight,
    AnnotKind::Underline,
    AnnotKind::StrikeOut,
    AnnotKind::FreeText,
    AnnotKind::Image,
    AnnotKind::Ink,
    AnnotKind::Line,
    AnnotKind::Arrow,
    AnnotKind::Rect,
    AnnotKind::Ellipse,
    AnnotKind::Polygon,
    AnnotKind::Note,
    AnnotKind::Stamp,
];

fn draws_text(kind: AnnotKind) -> bool {
    matches!(kind, AnnotKind::FreeText | AnnotKind::Stamp)
}

/// A blank page the size of the golden page.
fn blank_pdf() -> Vec<u8> {
    let objs = [
        "<</Type/Catalog/Pages 2 0 R>>".to_string(),
        "<</Type/Pages/Kids[3 0 R]/Count 1>>".to_string(),
        format!("<</Type/Page/Parent 2 0 R/MediaBox[0 0 {PAGE_W} {PAGE_H}]/Contents 4 0 R>>"),
        "<</Length 0>>\nstream\n\nendstream".to_string(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offs.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let x = pdf.len();
    pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for o in offs {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<</Size 5/Root 1 0 R>>\nstartxref\n{x}\n%%EOF\n").as_bytes(),
    );
    pdf
}

/// The golden checker as a PNG file, which is what a user would insert.
fn checker_png() -> Vec<u8> {
    let mut rgba = Vec::new();
    for px in CHECKER.chunks_exact(3) {
        rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    let img = image::RgbaImage::from_raw(2, 2, rgba).expect("checker");
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .expect("png");
    out.into_inner()
}

struct Assets<'a> {
    fonts: &'a StandardFonts,
    fixed: bool,
}

impl izul_write::save::Assets for Assets<'_> {
    fn base_font(&self, font: FontRef) -> Option<&'static str> {
        if self.fixed {
            Some("Helvetica")
        } else {
            self.fonts.base_font(font)
        }
    }
    fn image(&self, image: ImageRef) -> Option<Vec<u8>> {
        (image.0 == 0).then(checker_png)
    }
}

/// Saves `objects` onto a blank page through the real path.
fn save(
    engine: &'static crate::engine::Engine,
    objects: &[(AnnotObject, izul_model::DisplayList)],
    assets: &Assets<'_>,
) -> Vec<u8> {
    let doc = engine
        .open_bytes(blank_pdf(), None::<&str>, None)
        .expect("sumber");
    doc.set_strip_izul(false);
    let ids: Vec<u64> = objects.iter().map(|(o, _)| o.id.0).collect();
    doc.put_placeholders(0, &ids).expect("placeholder");
    let pdfium = doc.save_to_vec().expect("simpan");
    let writes: Vec<AnnotWrite<'_>> = objects
        .iter()
        .map(|(obj, list)| AnnotWrite { obj, list })
        .collect();
    patch(pdfium, &writes, assets, &[]).expect("patch")
}

#[test]
fn the_key_matches_the_writer() {
    assert_eq!(super::KEY, izul_write::metadata::KEY);
    assert_eq!(super::NM_PREFIX, izul_write::metadata::NM_PREFIX);
}

#[test]
fn every_kind_survives_save_and_reopen_as_a_live_object() {
    let (engine, _pdfium) = engine_and_lock!();
    let metrics_doc = metrics_document(engine).expect("metrik");
    let fonts = StandardFonts::new(engine, metrics_doc.handle());
    let assets = Assets {
        fonts: &fonts,
        fixed: false,
    };

    let mut objects = Vec::new();
    for (i, kind) in KINDS.iter().enumerate() {
        let mut obj = sample(*kind);
        obj.id = izul_model::annot::AnnotId(100 + i as u64);
        obj.created_at = 1_790_132_645_000;
        let list = display_list(&obj, &fonts).expect("display list");
        objects.push((obj, list));
    }
    let saved = save(engine, &objects, &assets);

    // Opened as the editor opens it: our annotations are taken out of the page
    // and handed over, and they come back equal to what was saved.
    let viewing = engine
        .open_bytes(saved.clone(), None::<&str>, None)
        .expect("buka ulang");
    let found = viewing.izul_annots(0).expect("anotasi");
    assert_eq!(found.len(), KINDS.len());
    let mut back: Vec<AnnotObject> = found
        .iter()
        .map(|a| izul_write::metadata::decode(&a.metadata).expect("metadata"))
        .collect();
    back.sort_by_key(|o| o.id);
    let originals: Vec<AnnotObject> = objects.iter().map(|(o, _)| o.clone()).collect();
    assert_eq!(back, originals, "objek kembali persis seperti disimpan");

    // The picture comes back as the picture.
    let image = found
        .iter()
        .find(|a| a.metadata.contains("\"Image\""))
        .and_then(|a| a.image.clone())
        .expect("gambar kembali");
    assert_eq!(image.mime, "image/png");
    let decoded = image::load_from_memory(&image.bytes)
        .expect("png")
        .to_rgb8();
    assert_eq!(decoded.dimensions(), (2, 2));
    assert_eq!(decoded.into_raw(), CHECKER.to_vec());

    // Nothing is drawn twice: the viewing copy renders a blank page, because
    // the editor draws these from their objects.
    let (_, _, pixels) = render(engine, saved.clone());
    drop(viewing);
    assert!(
        pixels.iter().all(|b| *b == 255),
        "halaman tampilan harus kosong"
    );

    // And a copy that keeps them — the one another reader sees — draws them.
    let kept = engine
        .open_bytes(saved, None::<&str>, None)
        .expect("salinan");
    kept.set_strip_izul(false);
    assert_eq!(kept.count_izul(0).expect("hitung"), KINDS.len() as u32);
}

/// SPEC 3.3 against a saved file: each non-text kind, saved as an annotation,
/// renders within 0.5 % of the same appearance painted as page content — the
/// exact picture the Phase 3 golden baseline checks.
#[test]
fn a_saved_annotation_renders_like_its_golden_appearance() {
    let (engine, _pdfium) = engine_and_lock!();
    let metrics_doc = metrics_document(engine).expect("metrik");
    let fonts = StandardFonts::new(engine, metrics_doc.handle());
    let fixed = FixedFont {
        advance_milli: GLYPH_ADVANCE_MILLI,
        ..Default::default()
    };
    for kind in KINDS.iter().copied().filter(|k| !draws_text(*k)) {
        for rotated in [false, true] {
            let mut obj = sample(kind);
            if rotated {
                obj.rotation = 12.0;
                obj.opacity = 0.8;
            }
            let list = display_list(&obj, &fixed).expect("display list");
            let ap = appearance(&list, obj.rect);
            let needs_image = !ap.resources.images.is_empty();
            let (w, h, golden) = render(engine, page_with(&ap, needs_image, ""));

            let saved = save(
                engine,
                &[(obj.clone(), list)],
                &Assets {
                    fonts: &fonts,
                    fixed: true,
                },
            );
            let kept = engine
                .open_bytes(saved, None::<&str>, None)
                .expect("simpanan");
            kept.set_strip_izul(false);
            let (geom, buf) = kept
                .render_page(0, 2.0, crate::render::Quality::Sharp)
                .expect("render");
            assert_eq!((geom.width, geom.height), (w, h));
            let mut actual = vec![0u8; (w * h * 4) as usize];
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let s = y * geom.stride + x * 4;
                    let d = (y * w as usize + x) * 4;
                    actual[d] = buf[s + 2];
                    actual[d + 1] = buf[s + 1];
                    actual[d + 2] = buf[s];
                    actual[d + 3] = buf[s + 3];
                }
            }
            let diff = differing_fraction(&golden, &actual);
            // Printed for the phase report: `--nocapture` shows the margin,
            // not just that it passed.
            eprintln!("paritas {kind:?} diputar={rotated}: {:.3} %", diff * 100.0);
            assert!(
                diff < MAX_DIFFERING_FRACTION,
                "{kind:?} (diputar: {rotated}): {:.3} % piksel berbeda dari golden",
                diff * 100.0
            );
        }
    }
}

#[test]
fn a_page_without_our_annotations_is_left_alone() {
    let (engine, _pdfium) = engine_and_lock!();
    let doc = engine
        .open_bytes(blank_pdf(), None::<&str>, None)
        .expect("dokumen");
    assert!(doc.izul_annots(0).expect("anotasi").is_empty());
    let payload_is_image = |o: &AnnotObject| matches!(o.payload, AnnotPayload::Image { .. });
    assert!(!payload_is_image(&sample(AnnotKind::Rect)));
}

#[test]
fn flattening_burns_annotations_into_the_page() {
    let (engine, _pdfium) = engine_and_lock!();
    let metrics_doc = metrics_document(engine).expect("metrik");
    let fonts = StandardFonts::new(engine, metrics_doc.handle());
    let mut obj = sample(AnnotKind::Rect);
    obj.id = izul_model::annot::AnnotId(5);
    let list = display_list(&obj, &fonts).expect("list");
    let saved = save(
        engine,
        &[(obj, list)],
        &Assets {
            fonts: &fonts,
            fixed: false,
        },
    );

    let work = engine.open_bytes(saved, None::<&str>, None).expect("kerja");
    work.set_strip_izul(false);
    work.flatten_page(0).expect("ratakan");
    let flat = work.save_to_vec().expect("simpan rata");
    drop(work);

    // No annotation left — and yet the rectangle is still drawn, now as page
    // content that no reader can move or hide.
    let check = engine
        .open_bytes(flat.clone(), None::<&str>, None)
        .expect("cek");
    check.set_strip_izul(false);
    assert_eq!(check.count_izul(0).expect("hitung"), 0);
    drop(check);
    let (_, _, pixels) = render(engine, flat);
    assert!(
        pixels.iter().any(|b| *b < 200),
        "isi anotasi harus tetap tergambar"
    );
}

#[test]
fn extracting_pages_makes_a_document_of_just_those_pages() {
    let (engine, _pdfium) = engine_and_lock!();
    let doc = engine
        .open_bytes(blank_pdf(), None::<&str>, None)
        .expect("dokumen");
    let one = doc.extract_pages(&[0, 0]).expect("ekstrak");
    let back = engine.open_bytes(one, None::<&str>, None).expect("hasil");
    assert_eq!(back.page_count(), 2);
    assert!(
        doc.extract_pages(&[3]).is_err(),
        "halaman di luar dokumen ditolak"
    );
}
