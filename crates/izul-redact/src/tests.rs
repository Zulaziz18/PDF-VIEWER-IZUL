//! Page-level tests: a whole file in, a whole file out, and the output read
//! back with this crate's own parser. The independent proof — PDFium and five
//! other extractors reading the result — lives with the worker and in
//! `tools/redaction-proof`.

use crate::file::tests::build;
use crate::file::{Pdf, Stored};
use crate::filters::decode_plain;
use crate::object::{Obj, Ref};
use crate::{redact, PageAreas, Rect};

/// Courier (every glyph 600/1000 em) without /Widths: the AFM metrics apply.
const COURIER: &[u8] = b"<</Type/Font/Subtype/Type1/BaseFont/Courier/Encoding/WinAnsiEncoding>>";

fn file_with(content: &str, extra_page: &str, extra: &[&[u8]]) -> Vec<u8> {
    let page = format!(
        "<</Type/Page/Parent 2 0 R/MediaBox[0 0 600 800]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>{extra_page}>>>>"
    );
    let stream = format!(
        "<</Length {}>>\nstream\n{content}\nendstream",
        content.len() + 1
    );
    let mut objects: Vec<&[u8]> = vec![
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        page.as_bytes(),
        stream.as_bytes(),
        COURIER,
    ];
    objects.extend_from_slice(extra);
    build(&objects)
}

fn page_content(out: &[u8]) -> String {
    let pdf = Pdf::parse(out).unwrap();
    let page = pdf.pages().unwrap()[0];
    let d = pdf.resolve_dict(&Obj::Ref(page)).unwrap().unwrap();
    let s = pdf.stream_of(d.get(b"Contents").unwrap()).unwrap().unwrap();
    String::from_utf8_lossy(&decode_plain(Some(&pdf), &s.dict, &s.data).unwrap()).into_owned()
}

fn one(page: u32, area: Rect) -> Vec<PageAreas> {
    vec![PageAreas {
        page,
        areas: vec![area],
        fill: Some([0.0, 0.0, 0.0]),
    }]
}

fn hex(s: &str) -> String {
    let mut out = String::from("<");
    for b in s.bytes() {
        out.push_str(&format!("{b:02X}"));
    }
    out.push('>');
    out
}

/// "SECRET public" at 10 pt Courier from x=100: each glyph is 6 pt wide, so
/// SECRET spans 100..136. Its six glyphs become one gap of 3600/1000 em; the
/// rest of the line is untouched and starts exactly where it did.
#[test]
fn removed_glyphs_leave_a_gap_of_exactly_their_width() {
    let src = file_with("BT /F1 10 Tf 100 700 Td (SECRET public) Tj ET", "", &[]);
    let (out, report) = redact(&src, &one(0, Rect::new(99.0, 695.0, 137.0, 712.0))).unwrap();
    let c = page_content(&out);
    assert!(c.contains(&format!("[-3600 {}] TJ", hex(" public"))), "{c}");
    assert!(!c.contains("SECRET"));
    assert_eq!(report[0].counts.glyphs, 6);
    // Filled afterwards, inside the page's own content.
    assert!(c.contains("q 0 0 0 rg\n99 695 38 17 re f\nQ"), "{c}");
    // And the original stream is not in the file at all.
    assert!(!String::from_utf8_lossy(&out).contains("SECRET"));
}

/// Character spacing, word spacing and horizontal scaling all move the pen;
/// the gap must include them, in the units TJ numbers use.
#[test]
fn the_gap_includes_spacing_and_scaling() {
    // Tz 50 halves advances; Tc 1 adds one unit per glyph and Tw 2 two more
    // for the space. Taking out "A " : A advances (6+1)*0.5 = 3.5, the space
    // (6+1+2)*0.5 = 4.5. In TJ units (divided by size·scale): -(7 + 9)/10*1000.
    let src = file_with(
        "BT /F1 10 Tf 50 Tz 1 Tc 2 Tw 100 700 Td (A B) Tj ET",
        "",
        &[],
    );
    let (out, _) = redact(&src, &one(0, Rect::new(99.0, 695.0, 107.9, 712.0))).unwrap();
    let c = page_content(&out);
    assert!(c.contains(&format!("[-1600 {}] TJ", hex("B"))), "{c}");
}

/// A glyph whose box only grazes the area stays; the neighbouring line is not
/// collateral.
#[test]
fn a_glyph_barely_touched_stays() {
    let src = file_with(
        "BT /F1 10 Tf 100 700 Td (upper) Tj 0 -12 Td (lower) Tj ET",
        "",
        &[],
    );
    // Courier descends 1.57 pt below the upper baseline (700); the area's top
    // edge sits 1 pt below it, so it clips the upper line's descenders only.
    let (out, report) = redact(&src, &one(0, Rect::new(90.0, 686.0, 200.0, 699.0))).unwrap();
    let c = page_content(&out);
    assert!(c.contains("(upper) Tj"), "{c}");
    assert!(!c.contains("lower"), "{c}");
    assert_eq!(report[0].counts.glyphs, 5);
}

/// Invisible text (render mode 3, as an OCR layer is) is text all the same.
#[test]
fn invisible_text_is_removed_too() {
    let src = file_with("BT 3 Tr /F1 10 Tf 100 700 Td (hidden) Tj ET", "", &[]);
    let (out, report) = redact(&src, &one(0, Rect::new(90.0, 690.0, 200.0, 720.0))).unwrap();
    assert_eq!(report[0].counts.glyphs, 6);
    assert!(!page_content(&out).contains(&hex("hidden")));
}

/// Text inside a form XObject: the page gets a new copy of the form under a
/// new name; the old form is no longer reachable and is not written.
#[test]
fn forms_are_redacted_as_new_copies() {
    let form = b"<</Type/XObject/Subtype/Form/BBox[0 0 600 800]/Matrix[1 0 0 1 50 0]/Resources<</Font<</F1 5 0 R>>>>/Length 43>>\nstream\nBT /F1 10 Tf 50 700 Td (INFORM) Tj ET\n\nendstream";
    let src = file_with("q /Fm0 Do Q", "/XObject<</Fm0 6 0 R>>", &[form]);
    // Drawn at x = 50 + 50 = 100.
    let (out, report) = redact(&src, &one(0, Rect::new(90.0, 690.0, 200.0, 720.0))).unwrap();
    assert_eq!(report[0].counts.forms, 1);
    assert_eq!(report[0].counts.glyphs, 6);
    let c = page_content(&out);
    assert!(c.contains("/IzRd1 Do"), "{c}");
    assert!(!String::from_utf8_lossy(&out).contains("INFORM"));
    let pdf = Pdf::parse(&out).unwrap();
    assert!(pdf.get(6).is_err(), "form lama tidak lagi ditulis");
}

/// Replacement text over removed glyphs goes; over kept glyphs it stays.
#[test]
fn actual_text_over_removed_glyphs_is_dropped() {
    let src = file_with(
        "/Span <</ActualText (SECRET)>> BDC BT /F1 10 Tf 100 700 Td (SECRET) Tj ET EMC \
         /Span <</ActualText (kept)>> BDC BT /F1 10 Tf 100 500 Td (kept) Tj ET EMC",
        "",
        &[],
    );
    let (out, report) = redact(&src, &one(0, Rect::new(90.0, 690.0, 200.0, 720.0))).unwrap();
    assert_eq!(report[0].counts.marked_content, 1);
    let c = page_content(&out);
    assert!(!c.contains(&hex("SECRET")), "{c}");
    assert!(c.contains("<</ActualText (kept)>> BDC"), "{c}");
}

/// Annotations over the area go, with their popup; a widget goes from the
/// form's field list too.
#[test]
fn annotations_and_fields_over_the_area_go() {
    let page = "<</Type/Page/Parent 2 0 R/MediaBox[0 0 600 800]/Contents 4 0 R/Annots[6 0 R 7 0 R 8 0 R 9 0 R]>>";
    let src = build(&[
        b"<</Type/Catalog/Pages 2 0 R/AcroForm<</Fields[8 0 R 9 0 R]>>>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        page.as_bytes(),
        b"<</Length 2>>\nstream\nq\n\nendstream",
        COURIER,
        b"<</Type/Annot/Subtype/Text/Rect[100 700 120 720]/Contents(SECRET NOTE)/Popup 7 0 R>>",
        b"<</Type/Annot/Subtype/Popup/Rect[300 300 400 400]/Parent 6 0 R>>",
        b"<</Type/Annot/Subtype/Widget/FT/Tx/T(ssn)/V(123-45-6789)/Rect[100 650 200 670]>>",
        b"<</Type/Annot/Subtype/Widget/FT/Tx/T(name)/V(public)/Rect[100 100 200 120]>>",
    ]);
    let (out, report) = redact(&src, &one(0, Rect::new(90.0, 640.0, 250.0, 730.0))).unwrap();
    assert_eq!(report[0].annotations, 3);
    let text = String::from_utf8_lossy(&out);
    assert!(!text.contains("SECRET NOTE"));
    assert!(!text.contains("123-45-6789"));
    assert!(text.contains("(public)") || text.contains(&hex("public")));
    let pdf = Pdf::parse(&out).unwrap();
    let (_, cat) = pdf.catalog().unwrap();
    let af = pdf
        .resolve_dict(cat.get(b"AcroForm").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        af.get(b"Fields"),
        Some(&Obj::Array(vec![Obj::Ref(Ref { num: 9, gen: 0 })]))
    );
}

/// A shape wholly inside the area goes; a clipping path, and a shape that
/// crosses the area's edge, stay.
#[test]
fn paths_inside_go_and_clips_stay() {
    let src = file_with(
        "0 0 1 rg 110 700 20 10 re f 0 0 600 800 re W n 50 700 200 10 re f",
        "",
        &[],
    );
    let (out, report) = redact(&src, &one(0, Rect::new(100.0, 690.0, 200.0, 720.0))).unwrap();
    assert_eq!(report[0].counts.paths, 1);
    let c = page_content(&out);
    assert!(!c.contains("110 700 20 10 re"), "{c}");
    assert!(c.contains("0 0 600 800 re\nW\nn"), "{c}");
    assert!(c.contains("50 700 200 10 re"), "{c}");
}

/// The file written has no free-floating copies: every object in it is
/// reachable, and the page count is intact.
#[test]
fn nothing_unreachable_is_written() {
    let src = file_with(
        "BT /F1 10 Tf 100 700 Td (SECRET) Tj ET",
        "",
        &[b"(unused object)"],
    );
    let (out, _) = redact(&src, &one(0, Rect::new(90.0, 690.0, 200.0, 720.0))).unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(!text.contains("unused object"));
    let pdf = Pdf::parse(&out).unwrap();
    assert_eq!(pdf.pages().unwrap().len(), 1);
    let reach = pdf.reachable().unwrap();
    for n in 1..40u32 {
        assert_eq!(pdf.get(n).is_ok(), reach.contains(&n), "objek {n}");
    }
}

/// A page nobody marked is left exactly as it was, byte for byte.
#[test]
fn unmarked_pages_are_untouched() {
    let src = file_with("BT /F1 10 Tf 100 700 Td (SECRET) Tj ET", "", &[]);
    let (out, report) = redact(&src, &[]).unwrap();
    assert!(report.is_empty());
    let pdf = Pdf::parse(&out).unwrap();
    let Stored::Stream(s) = pdf.get(4).unwrap() else {
        panic!()
    };
    assert!(String::from_utf8_lossy(&s.data).contains("(SECRET) Tj"));
}

/// Rotated text is judged by where it lands, not by its unrotated box.
#[test]
fn rotated_text_is_judged_where_it_lands() {
    // Rotated 90°: the line runs up the page from (300, 100).
    let src = file_with("BT /F1 10 Tf 0 1 -1 0 300 100 Tm (UPWARD) Tj ET", "", &[]);
    // Beside where it would be unrotated: nothing.
    let (_, r) = redact(&src, &one(0, Rect::new(300.0, 90.0, 360.0, 110.0))).unwrap();
    assert_eq!(r[0].counts.glyphs, 0);
    // Over the column it really occupies (x 290..302, y 100..136).
    let (_, r) = redact(&src, &one(0, Rect::new(285.0, 95.0, 305.0, 140.0))).unwrap();
    assert_eq!(r[0].counts.glyphs, 6);
}

/// A removed annotation and a removed field stay referenced from elsewhere —
/// the structure tree's object reference, the form's calculation order — and
/// must still not be in the file.
#[test]
fn removed_objects_do_not_survive_through_other_references() {
    let page =
        "<</Type/Page/Parent 2 0 R/MediaBox[0 0 600 800]/Contents 4 0 R/Annots[6 0 R 7 0 R]>>";
    let src = build(&[
        b"<</Type/Catalog/Pages 2 0 R/AcroForm<</Fields[8 0 R]/CO[8 0 R]>>/StructTreeRoot 9 0 R>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        page.as_bytes(),
        b"<</Length 2>>\nstream\nq\n\nendstream",
        COURIER,
        b"<</Type/Annot/Subtype/FreeText/Rect[100 700 200 720]/Contents(SECRET MEMO)>>",
        b"<</Type/Annot/Subtype/Widget/Parent 8 0 R/Rect[100 650 200 670]>>",
        b"<</FT/Tx/T(salary)/V(98765)/Kids[7 0 R]>>",
        b"<</Type/StructTreeRoot/K[<</S/Annot/K<</Type/OBJR/Obj 6 0 R>>>>]>>",
    ]);
    let (out, _) = redact(&src, &one(0, Rect::new(90.0, 640.0, 250.0, 730.0))).unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(!text.contains("SECRET MEMO"), "{text}");
    assert!(!text.contains("98765"), "{text}");
}

/// The original of a replaced image is erased when nothing draws it any more,
/// even though a shared resource dictionary still names it — and kept when
/// another page draws it.
#[test]
fn a_replaced_image_is_erased_unless_drawn_elsewhere() {
    // A 2×2 grey image drawn over 100..140; the area covers its left half.
    let image = b"<</Type/XObject/Subtype/Image/Width 2/Height 2/ColorSpace/DeviceGray/BitsPerComponent 8/Length 4>>\nstream\nABCD\nendstream";
    let res = b"<</XObject<</Im0 7 0 R>>>>";
    let make = |second_draws: bool| {
        let p1 = "<</Type/Page/Parent 2 0 R/MediaBox[0 0 600 800]/Contents 4 0 R/Resources 6 0 R>>";
        let p2 = "<</Type/Page/Parent 2 0 R/MediaBox[0 0 600 800]/Contents 8 0 R/Resources 6 0 R>>";
        let draw = b"<</Length 29>>\nstream\nq 40 0 0 40 100 100 cm /Im0 Do Q\nendstream";
        let none = b"<</Length 2>>\nstream\nq\n\nendstream";
        build(&[
            b"<</Type/Catalog/Pages 2 0 R>>",
            b"<</Type/Pages/Kids[3 0 R 5 0 R]/Count 2>>",
            p1.as_bytes(),
            draw,
            p2.as_bytes(),
            res,
            image,
            if second_draws { draw } else { none },
        ])
    };
    let area = one(0, Rect::new(90.0, 90.0, 120.0, 150.0));
    let (out, r) = redact(&make(false), &area).unwrap();
    assert_eq!(r[0].counts.images_cleared, 1);
    assert!(
        !String::from_utf8_lossy(&out).contains("ABCD"),
        "asli terhapus"
    );
    let (out, _) = redact(&make(true), &area).unwrap();
    assert!(
        String::from_utf8_lossy(&out).contains("ABCD"),
        "halaman 2 masih memakainya"
    );
}

/// `cm` concatenates onto the current matrix: a later translate is applied
/// inside the earlier scale, so text at x=0 lands at (0 + 50) · 2 = 100.
#[test]
fn nested_matrices_compose_in_order() {
    let src = file_with(
        "q 2 0 0 2 0 0 cm 1 0 0 1 50 0 cm BT /F1 5 Tf 0 350 Td (DOUBLE) Tj ET Q",
        "",
        &[],
    );
    // Six glyphs of 3 pt (at size 5), doubled: 100..136 across, from y 700.
    let (_, r) = redact(&src, &one(0, Rect::new(98.0, 695.0, 140.0, 715.0))).unwrap();
    assert_eq!(r[0].counts.glyphs, 6);
    let (_, r) = redact(&src, &one(0, Rect::new(45.0, 345.0, 90.0, 360.0))).unwrap();
    assert_eq!(r[0].counts.glyphs, 0);
}

/// A page thumbnail is a picture of the page as it was.
#[test]
fn the_thumbnail_goes_with_the_page_it_pictured() {
    let page = "<</Type/Page/Parent 2 0 R/MediaBox[0 0 600 800]/Contents 4 0 R/Thumb 6 0 R/PieceInfo<</App<</Private 7 0 R>>>>>>";
    let src = build(&[
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
        page.as_bytes(),
        b"<</Length 2>>\nstream\nq\n\nendstream",
        COURIER,
        b"<</Width 1/Height 1/ColorSpace/DeviceGray/BitsPerComponent 8/Length 9>>\nstream\nTHUMBDATA\nendstream",
        b"(EDITABLE SECRET COPY)",
    ]);
    let (out, _) = redact(&src, &one(0, Rect::new(0.0, 0.0, 10.0, 10.0))).unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(!text.contains("THUMBDATA"));
    assert!(!text.contains("EDITABLE SECRET COPY"));
}
