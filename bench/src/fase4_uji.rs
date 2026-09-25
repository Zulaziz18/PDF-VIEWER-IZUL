//! Builds the Phase 4 test files for checking by eye in other readers:
//!
//! ```text
//! cargo run -p izul-bench --bin fase4-uji
//! ```
//!
//! writes, into `test-fixtures/`:
//!
//! * `fase4-uji-simpan.pdf` — one of each of the thirteen annotation kinds,
//!   each labelled, saved through the application's own save path (PDFium
//!   writes the document, `izul-write` adds the annotations). Open it in Adobe
//!   Acrobat, Chrome and Edge: every annotation should be drawn, where its label
//!   says, and Acrobat's comment list should name each by its kind.
//! * `fase4-uji-rata.pdf` — the same, flattened: no annotations left, the same
//!   picture.
//!
//! Nothing here goes through a worker process; the functions are the ones the
//! worker and the UI process call.
//!
//! With `--bench`, it instead times that save path on the 500-page fixtures
//! (`bench/make_fixtures.py`): the fourteen annotations on each of twenty
//! pages, then every stage the application runs — PDFium's full save, the
//! incremental section, reopening to verify, flattening — and writes the
//! numbers to `bench/results/phase4-<os>.txt`.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use izul_model::annot::{
    AnnotId, AnnotKind, AnnotObject, AnnotPayload, FontSpec, NoteIcon, ShapeStyle, TextAlign,
};
use izul_model::build::display_list;
use izul_model::display::{FontRef, ImageRef, Rgba};
use izul_model::geom::{PdfPointF, PdfRectF};
use izul_pdf::{metrics_document, Engine, StandardFonts};
use izul_write::{patch, AnnotWrite};

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

/// Cell `i` of a 3 x 5 grid on an A4 page, below the heading.
fn cell(i: usize) -> PdfRectF {
    let (col, row) = ((i % 3) as f32, (i / 3) as f32);
    let (w, h) = (165.0, 128.0);
    let left = 40.0 + col * (w + 12.0);
    let top = 742.0 - row * (h + 12.0);
    PdfRectF::new(left, top - h, left + w, top)
}

fn inner(r: PdfRectF) -> PdfRectF {
    PdfRectF::new(r.left + 12.0, r.bottom + 12.0, r.right - 12.0, r.top - 30.0)
}

fn rgb(hex: u32) -> Rgba {
    Rgba::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 1.0)
}

/// The source page: a heading, and each cell labelled with the kind it holds
/// and a line of text for the markup kinds to mark.
fn source(labels: &[&str]) -> Vec<u8> {
    let mut content = String::from(
        "BT /F2 16 Tf 40 800 Td (PDF Studio Izul - uji simpan Fase 4) Tj ET\n\
         BT /F1 9 Tf 40 784 Td (Tiap kotak berisi satu jenis anotasi. Buka di Acrobat, Chrome, dan Edge.) Tj ET\n\
         0.8 G 0.5 w\n",
    );
    for (i, label) in labels.iter().enumerate() {
        let c = cell(i);
        content.push_str(&format!(
            "{} {} {} {} re S\nBT /F2 10 Tf {} {} Td ({}) Tj ET\n",
            c.left,
            c.bottom,
            c.width(),
            c.height(),
            c.left + 8.0,
            c.top - 16.0,
            label
        ));
        if i < 3 {
            content.push_str(&format!(
                "BT /F1 11 Tf {} {} Td (Teks yang ditandai di sini.) Tj ET\n",
                c.left + 14.0,
                c.top - 60.0
            ));
        }
    }
    let objs = [
        "<</Type/Catalog/Pages 2 0 R>>".to_string(),
        "<</Type/Pages/Kids[3 0 R]/Count 1>>".to_string(),
        "<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Contents 4 0 R/Resources<</Font<</F1 5 0 R/F2 6 0 R>>>>>>".to_string(),
        format!("<</Length {}>>\nstream\n{content}\nendstream", content.len()),
        "<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>".to_string(),
        "<</Type/Font/Subtype/Type1/BaseFont/Helvetica-Bold/Encoding/WinAnsiEncoding>>".to_string(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offs.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let x = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{x}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

/// A small transparent picture: a gradient disc on a clear background.
fn picture() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(96, 64, |x, y| {
        let (dx, dy) = (x as f32 - 48.0, y as f32 - 32.0);
        let inside = dx * dx / 2304.0 + dy * dy / 1024.0 <= 1.0;
        image::Rgba([
            (x * 2) as u8 + 40,
            90,
            220 - (y * 2) as u8,
            if inside { 255 } else { 0 },
        ])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .expect("png");
    out.into_inner()
}

fn objects() -> Vec<AnnotObject> {
    let red = rgb(0xe03131);
    let shape = |fill: Option<Rgba>| ShapeStyle {
        fill,
        stroke: Some(rgb(0x1c4f9c)),
        stroke_width: 2.0,
        ..ShapeStyle::default()
    };
    let mut out = Vec::new();
    let mut add = |i: usize, kind: AnnotKind, payload: AnnotPayload| {
        let mut o = AnnotObject::new(AnnotId(i as u64 + 1), 0, kind, inner(cell(i)), payload);
        o.created_at = 1_790_132_645_000;
        o.modified_at = o.created_at;
        o.author_note = format!("Uji Fase 4: {kind:?}");
        o.recompute_rect();
        out.push(o);
    };
    let line = |i: usize| {
        let c = cell(i);
        PdfRectF::new(c.left + 12.0, c.top - 64.0, c.left + 150.0, c.top - 49.0)
    };
    add(
        0,
        AnnotKind::Highlight,
        AnnotPayload::Markup {
            quads: vec![line(0)],
            color: rgb(0xffd43b),
        },
    );
    add(
        1,
        AnnotKind::Underline,
        AnnotPayload::Markup {
            quads: vec![line(1)],
            color: rgb(0x1c7ed6),
        },
    );
    add(
        2,
        AnnotKind::StrikeOut,
        AnnotPayload::Markup {
            quads: vec![line(2)],
            color: red,
        },
    );
    add(
        3,
        AnnotKind::FreeText,
        AnnotPayload::FreeText {
            text: "Kotak teks\nTimes, dua baris.".into(),
            font: FontSpec {
                size: 13.0,
                ..FontSpec::default()
            },
            color: rgb(0x862e9c),
            align: TextAlign::Left,
            line_spacing: 1.2,
            background: Some(rgb(0xf8f0fc)),
            border: Some(rgb(0x862e9c)),
        },
    );
    add(
        4,
        AnnotKind::Image,
        AnnotPayload::Image {
            image: ImageRef(0),
            crop: PdfRectF::new(0.0, 0.0, 1.0, 1.0),
            opacity: 1.0,
        },
    );
    let c = inner(cell(5));
    add(
        5,
        AnnotKind::Ink,
        AnnotPayload::Ink {
            strokes: vec![(0..20)
                .map(|k| {
                    let t = k as f32 / 19.0;
                    PdfPointF::new(
                        c.left + t * c.width(),
                        c.bottom + c.height() * (0.5 + 0.4 * (t * 9.0).sin()),
                    )
                })
                .collect()],
            color: rgb(0x2b8a3e),
            width: 2.5,
            smooth: true,
        },
    );
    let c6 = inner(cell(6));
    add(
        6,
        AnnotKind::Line,
        AnnotPayload::Line {
            from: PdfPointF::new(c6.left, c6.bottom),
            to: PdfPointF::new(c6.right, c6.top),
            color: rgb(0x495057),
            width: 2.0,
            dashed: true,
            arrow_head: 0.0,
        },
    );
    let c7 = inner(cell(7));
    add(
        7,
        AnnotKind::Arrow,
        AnnotPayload::Line {
            from: PdfPointF::new(c7.left, c7.bottom),
            to: PdfPointF::new(c7.right, c7.top),
            color: red,
            width: 2.5,
            dashed: false,
            arrow_head: 12.0,
        },
    );
    add(
        8,
        AnnotKind::Rect,
        AnnotPayload::Shape {
            style: shape(Some(Rgba::from_rgb8(116, 192, 252, 0.5))),
        },
    );
    add(
        9,
        AnnotKind::Ellipse,
        AnnotPayload::Shape { style: shape(None) },
    );
    let c10 = inner(cell(10));
    add(
        10,
        AnnotKind::Polygon,
        AnnotPayload::Polygon {
            points: vec![
                PdfPointF::new(c10.left, c10.bottom),
                PdfPointF::new(c10.right, c10.bottom + 10.0),
                PdfPointF::new(c10.left + c10.width() * 0.6, c10.top),
            ],
            closed: true,
            style: shape(Some(Rgba::from_rgb8(255, 236, 153, 0.8))),
        },
    );
    add(
        11,
        AnnotKind::Note,
        AnnotPayload::Note {
            icon: NoteIcon::Comment,
            color: rgb(0xfab005),
            text: "Catatan tempel: buka untuk membaca.".into(),
        },
    );
    add(
        12,
        AnnotKind::Stamp,
        AnnotPayload::Stamp {
            label: "DISETUJUI".into(),
            color: rgb(0x2b8a3e),
            font: FontSpec {
                family: "Helvetica".into(),
                size: 18.0,
                bold: true,
                italic: false,
            },
        },
    );
    // One more in the last cell: rotated and half transparent, the case most
    // likely to be clipped or doubled by a reader.
    let mut rotated = AnnotObject::new(
        AnnotId(14),
        0,
        AnnotKind::Rect,
        inner(cell(13)),
        AnnotPayload::Shape {
            style: shape(Some(rgb(0xe03131))),
        },
    );
    rotated.rotation = 20.0;
    rotated.opacity = 0.5;
    rotated.author_note = "Diputar 20 derajat, opasitas 50 persen".into();
    out.push(rotated);
    out
}

struct Assets<'a> {
    fonts: &'a StandardFonts,
    picture: Vec<u8>,
}

impl izul_write::save::Assets for Assets<'_> {
    fn base_font(&self, font: FontRef) -> Option<&'static str> {
        self.fonts.base_font(font)
    }
    fn image(&self, image: ImageRef) -> Option<Vec<u8>> {
        (image.0 == 0).then(|| self.picture.clone())
    }
}

/// Pages the benchmark annotates: twenty, spread through the document.
const BENCH_PAGES: u32 = 20;

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

fn bench(engine: &'static Engine, fonts: &StandardFonts) {
    let fixtures = repo_root().join("test-fixtures");
    let objs = objects();
    let lists: Vec<_> = objs
        .iter()
        .map(|o| display_list(o, fonts).expect("display list"))
        .collect();
    let assets = Assets {
        fonts,
        picture: picture(),
    };
    let mut report = String::new();
    let _ = writeln!(
        report,
        "Fase 4 — jalur simpan, {} x 14 anotasi, {} ({})",
        BENCH_PAGES,
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(
        report,
        "{:<26} {:>9} {:>11} {:>11} {:>11} {:>11} {:>11} {:>11}",
        "berkas", "MB", "buka ms", "PDFium ms", "patch ms", "verif ms", "rata ms", "+KB"
    );
    for name in ["text-500p.pdf", "mixed-500p.pdf", "scan-50mb-500p.pdf"] {
        let path = fixtures.join(name);
        let Ok(meta) = std::fs::metadata(&path) else {
            let _ = writeln!(
                report,
                "{name:<26} (tidak ada — jalankan bench/make_fixtures.py)"
            );
            continue;
        };
        let t = Instant::now();
        let doc = engine.open(&path, None).expect("buka");
        doc.set_strip_izul(false);
        let open_ms = ms(t);
        let pages = doc.page_count();
        let step = (pages / BENCH_PAGES).max(1);
        let mut placed: Vec<AnnotObject> = Vec::new();
        let t = Instant::now();
        for k in 0..BENCH_PAGES.min(pages) {
            let page = k * step;
            let ids: Vec<u64> = objs.iter().map(|o| u64::from(k) * 100 + o.id.0).collect();
            doc.put_placeholders(page, &ids).expect("placeholder");
            for o in &objs {
                let mut copy = o.clone();
                copy.id = AnnotId(u64::from(k) * 100 + o.id.0);
                copy.page = page;
                placed.push(copy);
            }
        }
        let bytes = doc.save_to_vec().expect("simpan PDFium");
        let pdfium_ms = ms(t);
        drop(doc);
        let writes: Vec<AnnotWrite<'_>> = placed
            .iter()
            .zip(lists.iter().cycle())
            .map(|(obj, list)| AnnotWrite { obj, list })
            .collect();
        let t = Instant::now();
        let saved = patch(bytes, &writes, &assets, &[]).expect("patch");
        let patch_ms = ms(t);
        let grown_kb = (saved.len() as f64 - meta.len() as f64) / 1024.0;

        // What `VerifyFile` does before the rename: reopen, count ours back.
        let t = Instant::now();
        let check = engine
            .open_bytes(saved, None::<&str>, None)
            .expect("buka ulang");
        check.set_strip_izul(false);
        let mut found = 0;
        for k in 0..BENCH_PAGES.min(pages) {
            found += check.count_izul(k * step).expect("hitung");
        }
        let verify_ms = ms(t);
        assert_eq!(
            found as usize,
            placed.len(),
            "semua anotasi terbaca kembali"
        );

        let t = Instant::now();
        for k in 0..BENCH_PAGES.min(pages) {
            check.flatten_page(k * step).expect("ratakan");
        }
        let _flat = check.save_to_vec().expect("simpan rata");
        let flat_ms = ms(t);

        let _ = writeln!(
            report,
            "{name:<26} {:>9.1} {open_ms:>11.1} {pdfium_ms:>11.1} {patch_ms:>11.1} {verify_ms:>11.1} {flat_ms:>11.1} {grown_kb:>11.1}",
            meta.len() as f64 / 1_048_576.0
        );
    }
    print!("{report}");
    let out = repo_root()
        .join("bench/results")
        .join(format!("phase4-{}.txt", std::env::consts::OS));
    std::fs::write(&out, report).expect("tulis hasil");
    println!("ditulis ke {}", out.display());
}

fn main() {
    let engine = Engine::load_from(pdfium()).expect("PDFium (jalankan vendor/pdfium/fetch.sh)");
    let metrics = metrics_document(engine).expect("dokumen metrik");
    let fonts = StandardFonts::new(engine, metrics.handle());
    if std::env::args().any(|a| a == "--bench") {
        bench(engine, &fonts);
        return;
    }
    let labels = [
        "1. Stabilo (Highlight)",
        "2. Garis bawah (Underline)",
        "3. Coret (StrikeOut)",
        "4. Kotak teks (FreeText)",
        "5. Gambar (Stamp)",
        "6. Pena bebas (Ink)",
        "7. Garis putus (Line)",
        "8. Panah (Line + panah)",
        "9. Kotak (Square)",
        "10. Elips (Circle)",
        "11. Poligon (Polygon)",
        "12. Catatan (Text)",
        "13. Stempel (Stamp)",
        "14. Kotak diputar, 50%",
        "",
    ];
    let objs = objects();
    let lists: Vec<_> = objs
        .iter()
        .map(|o| display_list(o, &fonts).expect("display list"))
        .collect();

    let doc = engine
        .open_bytes(source(&labels), None::<&str>, None)
        .expect("sumber");
    doc.set_strip_izul(false);
    let ids: Vec<u64> = objs.iter().map(|o| o.id.0).collect();
    doc.put_placeholders(0, &ids).expect("placeholder");
    let pdfium_bytes = doc.save_to_vec().expect("simpan PDFium");
    drop(doc);
    let writes: Vec<AnnotWrite<'_>> = objs
        .iter()
        .zip(&lists)
        .map(|(obj, list)| AnnotWrite { obj, list })
        .collect();
    let saved = patch(
        pdfium_bytes,
        &writes,
        &Assets {
            fonts: &fonts,
            picture: picture(),
        },
        &[],
    )
    .expect("patch");

    let out_dir = repo_root().join("test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("folder test-fixtures");
    let out = out_dir.join("fase4-uji-simpan.pdf");
    std::fs::write(&out, &saved).expect("tulis");

    let work = engine
        .open_bytes(saved, None::<&str>, None)
        .expect("salinan");
    work.set_strip_izul(false);
    let annots = work.count_izul(0).expect("hitung");
    work.flatten_page(0).expect("ratakan");
    let flat = work.save_to_vec().expect("simpan rata");
    let flat_out = out_dir.join("fase4-uji-rata.pdf");
    std::fs::write(&flat_out, flat).expect("tulis rata");

    println!("{} anotasi ditulis ke {}", annots, out.display());
    println!("versi rata ditulis ke {}", flat_out.display());
}
