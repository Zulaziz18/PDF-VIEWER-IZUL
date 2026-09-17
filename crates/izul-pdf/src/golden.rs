//! Golden-image parity for every annotation kind (SPEC 3.3, SPEC 16, SPEC 17).
//!
//! Lives inside the crate rather than in `tests/` for one reason: PDFium may be
//! initialised once per process and `test_support` is what enforces that. An
//! integration test binary would be a second process with a second engine and a
//! second copy of the one-thread rule, which is exactly the harness defect that
//! cost Phase 2 a silent-skip bug.

//! Phase 3's pass criterion, measured (SPEC 3.3, SPEC 16, SPEC 17).
//!
//! > *Lulus bila: golden image membuktikan paritas saat objek diam untuk setiap
//! > jenis anotasi, ambang < 0,5%.*
//!
//! Baselines live in `crates/izul-pdf/golden/`.
//!
//! Every annotation kind is built as a real object, turned into a display list
//! by `izul-model`, serialised by the appearance-stream backend, wrapped in a
//! real PDF, and rendered by the real PDFium — then compared, pixel by pixel,
//! against a baseline committed next to this file.
//!
//! ## What this proves, and what it does not
//!
//! It proves the **export half** of the parity contract: that what the
//! appearance-stream backend writes renders to a known, stable picture, for
//! every kind, including rotation and opacity. A regression anywhere between
//! the object model and the PDF operators changes a baseline and fails here.
//!
//! It does not, on its own, prove the **screen half** — that the canvas backend
//! draws the same picture. That comparison needs a browser, and it lives in
//! `tests/canvas_parity.spec.ts`, which drives a real Chromium against these
//! same display lists. The two together are the contract; this file is the one
//! that can run in CI without a display.
//!
//! ## Why the text in the baselines is unevenly spaced
//!
//! The layout uses [`FixedFont`], where every glyph is 0.55 em wide, while
//! PDFium draws the glyphs of real Helvetica — so an `i` sits in a slot as wide
//! as an `m`. That is deliberate: a baseline whose spacing depended on the
//! fonts installed on the machine would fail on the next machine for reasons
//! that have nothing to do with the code. It does not weaken what is under
//! test, because *both* backends take their positions from the same metrics —
//! which is the property being proved. Phase 4 embeds the user's font and
//! supplies its real metrics, and the baselines are regenerated then.
//!
//! ## Updating a baseline
//!
//! ```text
//! IZUL_UPDATE_GOLDEN=1 cargo test -p izul-pdf --test annotation_golden
//! ```
//!
//! Do it only with the change in front of you, and look at the new image. A
//! baseline updated without being looked at is a test that has been switched
//! off quietly.

use std::path::PathBuf;

use crate::render::Quality;
use izul_model::annot::{
    AnnotId, AnnotKind, AnnotObject, AnnotPayload, FontSpec, NoteIcon, ShapeStyle, TextAlign,
};
use izul_model::ap::{appearance, Appearance};
use izul_model::build::display_list;
use izul_model::display::{ImageRef, Rgba};
use izul_model::font::FixedFont;
use izul_model::geom::{PdfPointF, PdfRectF};

/// Page the annotations are drawn on, in points.
const PAGE_W: f32 = 240.0;
const PAGE_H: f32 = 120.0;
/// Rendered at 2x so antialiasing differences have somewhere to show.
const SCALE: f32 = 2.0;

/// How different a channel may be before a pixel counts as changed.
///
/// Eight of 255 is about 3 %: below what anyone sees, and above the
/// one-or-two-level wobble that a different PDFium build or CPU can produce on
/// an antialiased edge. Without it the test would fail for reasons that have
/// nothing to do with the code.
const CHANNEL_TOLERANCE: u8 = 8;

/// SPEC 3.3's threshold: fewer than 0.5 % of pixels may differ.
const MAX_DIFFERING_FRACTION: f64 = 0.005;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden")
}

fn rect() -> PdfRectF {
    PdfRectF::new(20.0, 20.0, 220.0, 100.0)
}

/// One object per kind, sized to the same box so the baselines are comparable
/// by eye when they are opened side by side.
fn sample(kind: AnnotKind) -> AnnotObject {
    let payload = match kind {
        AnnotKind::Highlight | AnnotKind::Underline | AnnotKind::StrikeOut => {
            AnnotPayload::Markup {
                quads: vec![
                    PdfRectF::new(30.0, 70.0, 200.0, 90.0),
                    PdfRectF::new(30.0, 40.0, 140.0, 60.0),
                ],
                color: Rgba::from_rgb8(255, 210, 0, 0.6),
            }
        }
        AnnotKind::FreeText => AnnotPayload::FreeText {
            text: "Paritas diuji per jenis anotasi".into(),
            font: FontSpec {
                family: "Helvetica".into(),
                size: 14.0,
                bold: false,
                italic: false,
            },
            color: Rgba::from_rgb8(20, 20, 20, 1.0),
            align: TextAlign::Left,
            line_spacing: 1.3,
            background: Some(Rgba::from_rgb8(255, 250, 205, 1.0)),
            border: Some(Rgba::from_rgb8(120, 120, 120, 1.0)),
        },
        AnnotKind::Image => AnnotPayload::Image {
            image: ImageRef(0),
            crop: PdfRectF::new(0.0, 0.0, 1.0, 1.0),
            opacity: 0.85,
        },
        AnnotKind::Ink => AnnotPayload::Ink {
            strokes: vec![vec![
                PdfPointF::new(30.0, 40.0),
                PdfPointF::new(70.0, 90.0),
                PdfPointF::new(110.0, 35.0),
                PdfPointF::new(150.0, 88.0),
                PdfPointF::new(200.0, 45.0),
            ]],
            color: Rgba::from_rgb8(200, 30, 30, 1.0),
            width: 3.0,
            smooth: true,
        },
        AnnotKind::Line | AnnotKind::Arrow => AnnotPayload::Line {
            from: PdfPointF::new(30.0, 35.0),
            to: PdfPointF::new(205.0, 90.0),
            color: Rgba::from_rgb8(0, 80, 200, 1.0),
            width: 3.0,
            dashed: kind == AnnotKind::Line,
            arrow_head: if kind == AnnotKind::Arrow { 14.0 } else { 0.0 },
        },
        AnnotKind::Rect | AnnotKind::Ellipse => AnnotPayload::Shape {
            style: ShapeStyle {
                fill: Some(Rgba::from_rgb8(150, 200, 255, 0.7)),
                stroke: Some(Rgba::from_rgb8(0, 60, 140, 1.0)),
                stroke_width: 2.5,
                dash: [6.0, 4.0],
                dashed: kind == AnnotKind::Rect,
            },
        },
        AnnotKind::Polygon => AnnotPayload::Polygon {
            points: vec![
                PdfPointF::new(30.0, 30.0),
                PdfPointF::new(120.0, 95.0),
                PdfPointF::new(210.0, 35.0),
                PdfPointF::new(120.0, 55.0),
            ],
            closed: true,
            style: ShapeStyle {
                fill: Some(Rgba::from_rgb8(255, 220, 180, 0.8)),
                stroke: Some(Rgba::from_rgb8(180, 90, 0, 1.0)),
                stroke_width: 2.0,
                dash: [4.0, 3.0],
                dashed: false,
            },
        },
        AnnotKind::Note => AnnotPayload::Note {
            icon: NoteIcon::Comment,
            color: Rgba::from_rgb8(255, 190, 0, 1.0),
            text: "periksa bagian ini".into(),
        },
        AnnotKind::Stamp => AnnotPayload::Stamp {
            label: "DISETUJUI".into(),
            color: Rgba::from_rgb8(0, 140, 60, 1.0),
            font: FontSpec {
                family: "Helvetica".into(),
                size: 18.0,
                bold: true,
                italic: false,
            },
        },
    };
    let mut obj = AnnotObject::new(AnnotId(1), 0, kind, rect(), payload);
    // A note is a badge, not a banner: drawn at the size it is actually used.
    if kind == AnnotKind::Note {
        obj.rect = PdfRectF::new(30.0, 40.0, 70.0, 90.0);
    }
    obj
}

/// Wraps an appearance stream in a one-page PDF that PDFium can open.
///
/// Written by hand rather than through a PDF library on purpose: the point of
/// the test is what *our* backend emitted, and a library that normalised the
/// operators on the way in would hide exactly the bugs this is looking for.
fn page_with(ap: &Appearance, needs_image: bool) -> Vec<u8> {
    let mut resources = String::from("<</ProcSet[/PDF/Text/ImageC]");
    if !ap.resources.fonts.is_empty() {
        resources.push_str("/Font<<");
        for name in ap.resources.fonts.keys() {
            // Standard-14 Helvetica: Phase 3 proves geometry, and embedding the
            // user's own font is Phase 4's job. The glyph *positions* come from
            // the display list either way, which is the part under test.
            resources.push_str(&format!(
                "/{name}<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>"
            ));
        }
        resources.push_str(">>");
    }
    if !ap.resources.states.is_empty() {
        resources.push_str("/ExtGState<<");
        for (name, state) in &ap.resources.states {
            let blend = match state.blend {
                izul_model::display::BlendMode::Normal => "Normal",
                izul_model::display::BlendMode::Multiply => "Multiply",
                izul_model::display::BlendMode::Screen => "Screen",
                izul_model::display::BlendMode::Darken => "Darken",
                izul_model::display::BlendMode::Lighten => "Lighten",
            };
            resources.push_str(&format!(
                "/{name}<</Type/ExtGState/ca {:.3}/CA {:.3}/BM/{blend}>>",
                state.fill_alpha, state.stroke_alpha
            ));
        }
        resources.push_str(">>");
    }
    if needs_image {
        resources.push_str("/XObject<<");
        for name in ap.resources.images.keys() {
            resources.push_str(&format!("/{name} 5 0 R"));
        }
        resources.push_str(">>");
    }
    resources.push_str(">>");

    // A 2x2 checker, which makes a wrong image matrix obvious: a flipped or
    // transposed placement moves the coloured squares.
    let pixels: [u8; 12] = [
        220, 40, 40, // red
        40, 80, 220, // blue
        250, 210, 40, // yellow
        30, 150, 90, // green
    ];

    let mut objects: Vec<Vec<u8>> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!(
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 {PAGE_W} {PAGE_H}]/Resources {resources}/Contents 4 0 R>>"
        )
        .into_bytes(),
        format!(
            "<</Length {}>>\nstream\n{}endstream",
            ap.content.len(),
            ap.content
        )
        .into_bytes(),
    ];
    if needs_image {
        let mut stream = format!(
            "<</Type/XObject/Subtype/Image/Width 2/Height 2/ColorSpace/DeviceRGB/BitsPerComponent 8/Length {}>>\nstream\n",
            pixels.len()
        )
        .into_bytes();
        stream.extend_from_slice(&pixels);
        stream.extend_from_slice(b"\nendstream");
        objects.push(stream);
    }

    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for off in &offsets {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

/// Renders the page and returns tightly packed RGBA.
fn render(pdf: Vec<u8>) -> (u32, u32, Vec<u8>) {
    let (engine, _pdfium) = match crate::test_support::engine() {
        Some(e) => (e, crate::test_support::pdfium_lock()),
        None => panic!("PDFium belum diambil; jalankan vendor/pdfium/fetch.sh"),
    };
    let doc = engine
        .open_bytes(pdf, None::<&str>, None)
        .expect("dokumen terbuka");
    let (geom, buf) = doc.render_page(0, SCALE, Quality::Sharp).expect("render");
    let (w, h) = (geom.width, geom.height);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let src = y * geom.stride + x * 4;
            let dst = (y * w as usize + x) * 4;
            // PDFium writes BGRA; the baseline is a PNG, which is RGBA.
            rgba[dst] = buf[src + 2];
            rgba[dst + 1] = buf[src + 1];
            rgba[dst + 2] = buf[src];
            rgba[dst + 3] = buf[src + 3];
        }
    }
    (w, h, rgba)
}

/// Fraction of pixels that differ by more than the tolerance.
fn differing_fraction(a: &[u8], b: &[u8]) -> f64 {
    let total = a.len() / 4;
    if total == 0 || a.len() != b.len() {
        return 1.0;
    }
    let differing = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(p, q)| {
            p.iter()
                .zip(q.iter())
                .any(|(x, y)| x.abs_diff(*y) > CHANNEL_TOLERANCE)
        })
        .count();
    differing as f64 / total as f64
}

fn check(kind: AnnotKind, rotated: bool) {
    let mut obj = sample(kind);
    if rotated {
        obj.rotation = 12.0;
        obj.opacity = 0.8;
    }
    let fonts = FixedFont {
        // Helvetica's real advance for a lowercase letter is around 0.55 em;
        // close enough that the baseline looks like text rather than a pile-up,
        // and fixed so the baseline does not depend on an installed font.
        advance_milli: 550,
        ..Default::default()
    };
    let list = display_list(&obj, &fonts).expect("display list");
    assert!(list.is_balanced(), "{kind:?}: daftar tidak seimbang");
    let ap = appearance(&list, obj.rect);
    assert_eq!(
        izul_model::ap::unbalanced_depth(&ap.content),
        0,
        "{kind:?}: q tanpa Q"
    );

    let needs_image = !ap.resources.images.is_empty();
    let (w, h, actual) = render(page_with(&ap, needs_image));

    let name = format!(
        "{}{}.png",
        format!("{kind:?}").to_lowercase(),
        if rotated { "-rotated" } else { "" }
    );
    let path = golden_dir().join(&name);

    if std::env::var_os("IZUL_UPDATE_GOLDEN").is_some() || !path.exists() {
        if !path.exists() && std::env::var_os("IZUL_UPDATE_GOLDEN").is_none() {
            // A missing baseline must not pass silently: it would mean the kind
            // is covered by a file nobody has ever looked at.
            std::fs::create_dir_all(golden_dir()).expect("folder golden");
            image::save_buffer(&path, &actual, w, h, image::ColorType::Rgba8)
                .expect("tulis baseline");
            panic!(
                "baseline {name} belum ada. Sudah dibuat di {} — periksa gambarnya, lalu jalankan lagi.",
                path.display()
            );
        }
        std::fs::create_dir_all(golden_dir()).expect("folder golden");
        image::save_buffer(&path, &actual, w, h, image::ColorType::Rgba8).expect("tulis baseline");
        return;
    }

    let baseline = image::open(&path).expect("baca baseline").to_rgba8();
    assert_eq!(
        (baseline.width(), baseline.height()),
        (w, h),
        "{name}: ukuran render berubah"
    );
    let fraction = differing_fraction(&actual, baseline.as_raw());
    if fraction > MAX_DIFFERING_FRACTION {
        let failed = golden_dir().join(format!("FAILED-{name}"));
        image::save_buffer(&failed, &actual, w, h, image::ColorType::Rgba8).expect("tulis hasil");
        panic!(
            "{name}: {:.3}% piksel berbeda, ambang {:.1}%. Hasil sekarang ditulis ke {}",
            fraction * 100.0,
            MAX_DIFFERING_FRACTION * 100.0,
            failed.display()
        );
    }
}

/// One test per kind rather than a loop, so a failure names the kind that broke
/// in the test list instead of in an assertion message.
macro_rules! golden_test {
    ($name:ident, $kind:expr) => {
        #[test]
        fn $name() {
            if crate::test_support::engine().is_none() {
                if std::env::var_os("IZUL_REQUIRE_FIXTURES").is_some() {
                    panic!("PDFium belum diambil dan IZUL_REQUIRE_FIXTURES diset");
                }
                eprintln!("LEWATI: PDFium belum diambil");
                return;
            }
            check($kind, false);
        }
    };
}

golden_test!(highlight_matches_its_baseline, AnnotKind::Highlight);
golden_test!(underline_matches_its_baseline, AnnotKind::Underline);
golden_test!(strikeout_matches_its_baseline, AnnotKind::StrikeOut);
golden_test!(freetext_matches_its_baseline, AnnotKind::FreeText);
golden_test!(image_matches_its_baseline, AnnotKind::Image);
golden_test!(ink_matches_its_baseline, AnnotKind::Ink);
golden_test!(line_matches_its_baseline, AnnotKind::Line);
golden_test!(arrow_matches_its_baseline, AnnotKind::Arrow);
golden_test!(rect_matches_its_baseline, AnnotKind::Rect);
golden_test!(ellipse_matches_its_baseline, AnnotKind::Ellipse);
golden_test!(polygon_matches_its_baseline, AnnotKind::Polygon);
golden_test!(note_matches_its_baseline, AnnotKind::Note);
golden_test!(stamp_matches_its_baseline, AnnotKind::Stamp);

/// Rotation and opacity go through a different path in both backends — a
/// transform and an `/ExtGState` — so they get their own baseline rather than
/// being assumed to follow from the upright one.
#[test]
fn a_rotated_translucent_object_matches_its_baseline() {
    if crate::test_support::engine().is_none() {
        if std::env::var_os("IZUL_REQUIRE_FIXTURES").is_some() {
            panic!("PDFium belum diambil dan IZUL_REQUIRE_FIXTURES diset");
        }
        eprintln!("LEWATI: PDFium belum diambil");
        return;
    }
    check(AnnotKind::Rect, true);
    check(AnnotKind::FreeText, true);
}

/// The comparison itself has to be able to fail, or every baseline passes.
#[test]
fn the_comparison_notices_a_difference() {
    let a = vec![255u8; 400];
    let mut b = a.clone();
    assert_eq!(differing_fraction(&a, &b), 0.0);
    // One pixel in a hundred is 1 %, which is over the 0.5 % threshold.
    b[0] = 0;
    let fraction = differing_fraction(&a, &b);
    assert!(fraction > MAX_DIFFERING_FRACTION, "{fraction}");
    // A change below the channel tolerance is not a difference.
    let mut c = a.clone();
    c[0] = 250;
    assert_eq!(differing_fraction(&a, &c), 0.0);
}
