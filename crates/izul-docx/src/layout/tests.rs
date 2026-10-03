use super::*;
use crate::page::{Glyph, PageLayout, Picture, Rect};

/// Glyphs for one line, half an em wide each, followed by PDFium's
/// generated line break.
fn line(page: &mut PageLayout, text: &str, x: f32, baseline: f32, size: f32, bold: bool) {
    let mut at = x;
    for ch in text.chars() {
        let w = size * 0.5;
        page.glyphs.push(Glyph {
            ch,
            rect: Rect::new(at, baseline - size * 0.22, at + w, baseline + size * 0.9),
            baseline,
            size,
            bold,
            italic: false,
            color: [0, 0, 0],
            font: 0,
            generated: false,
        });
        at += w;
    }
    for ch in ['\r', '\n'] {
        page.glyphs.push(Glyph {
            ch,
            rect: Rect::default(),
            baseline,
            size,
            bold,
            italic: false,
            color: [0, 0, 0],
            font: 0,
            generated: true,
        });
    }
}

fn page() -> PageLayout {
    PageLayout {
        width: 595.0,
        height: 842.0,
        fonts: vec!["Helvetica".into()],
        ..PageLayout::default()
    }
}

fn texts(doc: &Document) -> Vec<String> {
    doc.blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p.runs.iter().map(|r| r.text.as_str()).collect()),
            Block::Picture(_) => None,
        })
        .collect()
}

fn paragraphs(doc: &Document) -> Vec<&Paragraph> {
    doc.blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            Block::Picture(_) => None,
        })
        .collect()
}

/// 90 characters at 11 pt: a full line between 72 and 567.
const FULL: &str =
    "Dokumen ini disusun sebagai bahan uji tampilan dan setiap paragraf berisi kalimat biasa yan";

#[test]
fn lines_of_one_paragraph_become_one_paragraph() {
    let mut p = page();
    line(&mut p, FULL, 72.0, 700.0, 11.0, false);
    line(&mut p, FULL, 72.0, 686.0, 11.0, false);
    line(&mut p, "dan baris terakhir.", 72.0, 672.0, 11.0, false);
    let doc = analyse(vec![p]);
    assert_eq!(texts(&doc).len(), 1, "{:?}", texts(&doc));
    assert!(texts(&doc)[0].ends_with("dan baris terakhir."));
    assert_eq!(paragraphs(&doc)[0].align, Align::Justify);
}

#[test]
fn a_blank_line_between_them_makes_two() {
    let mut p = page();
    line(&mut p, FULL, 72.0, 700.0, 11.0, false);
    line(&mut p, "akhir paragraf pertama.", 72.0, 686.0, 11.0, false);
    line(&mut p, FULL, 72.0, 650.0, 11.0, false);
    let doc = analyse(vec![p]);
    assert_eq!(texts(&doc).len(), 2, "{:?}", texts(&doc));
    assert!(paragraphs(&doc)[1].space_before > 10.0);
}

#[test]
fn a_larger_line_is_a_heading_and_its_own_paragraph() {
    let mut p = page();
    line(&mut p, "Bab Satu", 72.0, 760.0, 22.0, true);
    line(&mut p, FULL, 72.0, 730.0, 11.0, false);
    line(&mut p, FULL, 72.0, 716.0, 11.0, false);
    let doc = analyse(vec![p]);
    let ps = paragraphs(&doc);
    assert_eq!(ps.len(), 2);
    assert_eq!(ps[0].heading, Some(1));
    assert_eq!(ps[1].heading, None);
    assert!(ps[0].runs[0].bold);
    assert_eq!(ps[0].runs[0].size, 22.0);
}

#[test]
fn a_centred_title_is_centred() {
    let mut p = page();
    // 10 chars at 16 pt = 80 pt wide, centred on 297.5.
    line(&mut p, "JUDUL BUKU", 257.5, 760.0, 16.0, true);
    line(&mut p, FULL, 72.0, 730.0, 11.0, false);
    line(&mut p, FULL, 72.0, 716.0, 11.0, false);
    let doc = analyse(vec![p]);
    assert_eq!(paragraphs(&doc)[0].align, Align::Center);
}

#[test]
fn list_items_each_start_a_paragraph() {
    let mut p = page();
    line(&mut p, "1. Pertama", 72.0, 700.0, 11.0, false);
    line(&mut p, "2. Kedua", 72.0, 686.0, 11.0, false);
    line(&mut p, "• Ketiga", 72.0, 672.0, 11.0, false);
    let doc = analyse(vec![p]);
    assert_eq!(texts(&doc), vec!["1. Pertama", "2. Kedua", "• Ketiga"]);
}

#[test]
fn a_word_broken_by_a_hyphen_is_joined() {
    let mut p = page();
    let first = format!("{}penyusu-", &FULL[..82]);
    line(&mut p, &first, 72.0, 700.0, 11.0, false);
    line(&mut p, "nan selesai.", 72.0, 686.0, 11.0, false);
    let doc = analyse(vec![p]);
    assert!(
        texts(&doc)[0].contains("penyusunan selesai."),
        "{:?}",
        texts(&doc)
    );
}

#[test]
fn a_wide_gap_in_a_line_is_a_tab() {
    let mut p = page();
    line(&mut p, "Kegiatan", 72.0, 700.0, 11.0, false);
    // Same baseline, far to the right: the next cell of the row. PDFium gives
    // it as part of the same line, so drop the break between them.
    p.glyphs.truncate(p.glyphs.len() - 2);
    line(&mut p, "Mulai", 300.0, 700.0, 11.0, false);
    let doc = analyse(vec![p]);
    assert_eq!(texts(&doc), vec!["Kegiatan\tMulai"]);
    // A tab stop where the cell started, so the column lines up in Word.
    let stop = paragraphs(&doc)[0].tabs[0];
    assert!((stop - (300.0 - doc.margins[0])).abs() < 0.01, "{stop}");
}

#[test]
fn every_page_after_the_first_starts_on_a_new_page() {
    let mut a = page();
    line(&mut a, "Halaman satu.", 72.0, 700.0, 11.0, false);
    let mut b = page();
    line(&mut b, "Halaman dua.", 72.0, 700.0, 11.0, false);
    let doc = analyse(vec![a, b]);
    let ps = paragraphs(&doc);
    assert!(!ps[0].page_break_before);
    assert!(ps[1].page_break_before);
    assert_eq!(doc.stats.pages, 2);
}

#[test]
fn a_picture_goes_where_it_sits_between_paragraphs() {
    let mut p = page();
    line(&mut p, "Di atas gambar.", 72.0, 760.0, 11.0, false);
    line(&mut p, "Di bawah gambar.", 72.0, 500.0, 11.0, false);
    p.pictures.push(Picture {
        rect: Rect::new(200.0, 550.0, 395.0, 740.0),
        mime: "image/png".into(),
        bytes: vec![1, 2, 3],
        width_px: 10,
        height_px: 10,
    });
    let doc = analyse(vec![p]);
    let kinds: Vec<&str> = doc
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(_) => "p",
            Block::Picture(_) => "pic",
        })
        .collect();
    assert_eq!(kinds, vec!["p", "pic", "p"]);
    let Some(Block::Picture(pic)) = doc.blocks.get(1) else {
        panic!("gambar hilang");
    };
    assert_eq!(pic.align, Align::Center);
    assert!((pic.width - 195.0).abs() < 0.01);
}

#[test]
fn a_scan_under_recognised_text_is_left_out() {
    let mut p = page();
    line(&mut p, "Teks hasil OCR.", 72.0, 700.0, 11.0, false);
    p.pictures.push(Picture {
        rect: Rect::new(0.0, 0.0, 595.0, 842.0),
        mime: "image/jpeg".into(),
        bytes: vec![0xFF, 0xD8],
        width_px: 100,
        height_px: 100,
    });
    let doc = analyse(vec![p]);
    assert_eq!(doc.stats.pictures, 0);
    assert_eq!(texts(&doc), vec!["Teks hasil OCR."]);
}

#[test]
fn a_scan_without_text_comes_across_as_its_picture() {
    let mut p = page();
    p.pictures.push(Picture {
        rect: Rect::new(0.0, 0.0, 595.0, 842.0),
        mime: "image/jpeg".into(),
        bytes: vec![0xFF, 0xD8],
        width_px: 100,
        height_px: 100,
    });
    let doc = analyse(vec![p]);
    assert_eq!(doc.stats.pictures, 1);
    assert_eq!(doc.stats.pages_without_text, 1);
}

#[test]
fn styles_change_within_a_paragraph_as_runs() {
    let mut p = page();
    line(&mut p, "Biasa ", 72.0, 700.0, 11.0, false);
    p.glyphs.truncate(p.glyphs.len() - 2);
    line(&mut p, "tebal", 105.0, 700.0, 11.0, true);
    let doc = analyse(vec![p]);
    let runs = &paragraphs(&doc)[0].runs;
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert!(!runs[0].bold && runs[1].bold);
}

#[test]
fn starts_item_knows_markers_from_words() {
    assert!(starts_item("1. Satu"));
    assert!(starts_item("12) Dua"));
    assert!(starts_item("a. tiga"));
    assert!(starts_item("• empat"));
    assert!(!starts_item("2025 adalah tahun"));
    assert!(!starts_item("Dokumen ini"));
    assert!(!starts_item("-5 derajat"));
}

#[test]
fn a_cover_s_display_type_does_not_take_the_chapter_headings_level() {
    let mut cover = page();
    line(&mut cover, "SAMPUL BESAR", 72.0, 700.0, 40.0, true);
    let mut chapters = Vec::new();
    for n in 0..2 {
        let mut p = page();
        line(&mut p, &format!("Bab {n}"), 72.0, 760.0, 16.0, true);
        line(&mut p, FULL, 72.0, 730.0, 11.0, false);
        line(&mut p, FULL, 72.0, 716.0, 11.0, false);
        chapters.push(p);
    }
    let mut all = vec![cover];
    all.extend(chapters);
    let doc = analyse(all);
    let ps = paragraphs(&doc);
    assert_eq!(ps[0].heading, None, "sampul: ukurannya tetap, bukan Judul");
    assert_eq!(ps[0].runs[0].size, 40.0);
    assert_eq!(ps[1].heading, Some(1), "judul bab = Judul 1");
}

#[test]
fn white_text_on_a_band_that_does_not_come_across_is_set_in_black() {
    let mut p = page();
    line(&mut p, "Judul di atas pita", 72.0, 760.0, 16.0, true);
    for g in &mut p.glyphs {
        g.color = [255, 255, 255];
    }
    line(
        &mut p,
        "Teks abu-abu tetap abu-abu.",
        72.0,
        700.0,
        11.0,
        false,
    );
    for g in p.glyphs.iter_mut().skip(20) {
        g.color = [120, 120, 120];
    }
    let doc = analyse(vec![p]);
    let ps = paragraphs(&doc);
    assert_eq!(ps[0].runs[0].color, [0, 0, 0]);
    assert_eq!(ps[1].runs[0].color, [120, 120, 120]);
}

#[test]
fn a_full_page_keeps_to_one_page_by_closing_up_its_gaps() {
    let mut p = page();
    // Lines from top to bottom with big gaps: as written, the gaps alone
    // would be more than the page.
    let mut y = 800.0;
    while y > 40.0 {
        line(&mut p, "Baris.", 72.0, y, 11.0, false);
        y -= 60.0;
    }
    let doc = analyse(vec![p]);
    let available = doc.page_height - doc.margins[1] - doc.margins[3];
    let used: f32 = paragraphs(&doc)
        .iter()
        .map(|p| p.space_before + 11.0 * 1.12)
        .sum();
    assert!(used <= available, "{used} > {available}");
}

#[test]
fn margins_follow_most_pages_not_the_widest_one() {
    let mut cover = page();
    line(&mut cover, "Sampul di tepi", 20.0, 800.0, 11.0, false);
    let mut pages = vec![cover];
    for _ in 0..3 {
        let mut p = page();
        line(&mut p, FULL, 72.0, 700.0, 11.0, false);
        pages.push(p);
    }
    let doc = analyse(pages);
    assert!((doc.margins[0] - 72.0).abs() < 0.01, "{:?}", doc.margins);
    assert!(
        paragraphs(&doc)[0].indent_left < 0.0,
        "sampul menjorok keluar"
    );
}

/// Two columns of 40 characters at 10 pt (200 pt wide): left at 57, right
/// at 312.
fn two_columns(p: &mut PageLayout, rows: usize, top: f32) {
    let text = "Teks kolom yang cukup panjang untuk baris";
    for col in [57.0, 312.0] {
        for r in 0..rows {
            line(p, text, col, top - r as f32 * 13.0, 10.0, false);
        }
    }
}

#[test]
fn the_second_column_is_not_indented_by_half_a_page() {
    let mut p = page();
    two_columns(&mut p, 6, 760.0);
    let doc = analyse(vec![p]);
    for para in paragraphs(&doc) {
        assert!(para.indent_left.abs() < 1.0, "{}", para.indent_left);
    }
}

#[test]
fn a_figure_stays_in_its_column() {
    let mut p = page();
    // Left column: text, a gap, text. Right column: text only.
    for r in 0..4 {
        line(
            &mut p,
            "Teks kolom kiri di atas gambar ini",
            57.0,
            760.0 - r as f32 * 13.0,
            10.0,
            false,
        );
    }
    for r in 0..4 {
        line(
            &mut p,
            "Teks kolom kiri di bawah gambar ini",
            57.0,
            500.0 - r as f32 * 13.0,
            10.0,
            false,
        );
    }
    for r in 0..30 {
        line(
            &mut p,
            "Teks kolom kanan yang terus berlanjut",
            312.0,
            760.0 - r as f32 * 13.0,
            10.0,
            false,
        );
    }
    p.pictures.push(Picture {
        rect: Rect::new(57.0, 540.0, 257.0, 700.0),
        mime: "image/png".into(),
        bytes: vec![1],
        width_px: 1,
        height_px: 1,
    });
    let doc = analyse(vec![p]);
    let order: Vec<String> = doc
        .blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => {
                p.runs.iter().map(|r| r.text.as_str()).collect::<String>()[..20].to_string()
            }
            Block::Picture(_) => "GAMBAR".to_string(),
        })
        .collect();
    let pic = order.iter().position(|t| t == "GAMBAR").unwrap();
    assert!(
        order[pic - 1].starts_with("Teks kolom kiri di a"),
        "{order:?}"
    );
    assert!(
        order[pic + 1].starts_with("Teks kolom kiri di b"),
        "{order:?}"
    );
}

#[test]
fn a_superscript_does_not_split_its_line() {
    let mut p = page();
    line(&mut p, "Angka ini naik", 72.0, 700.0, 10.0, false);
    p.glyphs.truncate(p.glyphs.len() - 2);
    // The footnote mark, smaller and raised; PDFium ends the line after it.
    line(&mut p, "1", 142.0, 703.5, 7.0, false);
    line(&mut p, " pada semester kedua.", 146.0, 700.0, 10.0, false);
    let doc = analyse(vec![p]);
    assert_eq!(texts(&doc), vec!["Angka ini naik1 pada semester kedua."]);
}

#[test]
fn running_header_first_and_page_number_last() {
    let mut p = page();
    line(&mut p, "Halaman 3", 280.0, 30.0, 8.0, false);
    line(&mut p, "Jurnal Contoh", 72.0, 815.0, 8.0, false);
    line(&mut p, "Isi halaman.", 72.0, 700.0, 10.0, false);
    let doc = analyse(vec![p]);
    assert_eq!(
        texts(&doc),
        vec!["Jurnal Contoh", "Isi halaman.", "Halaman 3"]
    );
}

#[test]
fn table_rows_stay_separate_paragraphs() {
    let mut p = page();
    for (r, (a, b)) in [("Konsultasi", "12 hari"), ("Legalisir", "7 hari")]
        .iter()
        .enumerate()
    {
        let y = 700.0 - r as f32 * 14.0;
        line(&mut p, a, 72.0, y, 9.0, false);
        p.glyphs.truncate(p.glyphs.len() - 2);
        line(&mut p, b, 200.0, y, 9.0, false);
    }
    let doc = analyse(vec![p]);
    assert_eq!(
        texts(&doc),
        vec!["Konsultasi\t12 hari", "Legalisir\t7 hari"]
    );
}

#[test]
fn a_paragraph_carries_on_into_the_next_column() {
    let mut p = page();
    let text = "Teks kolom yang cukup panjang untuk baris";
    for r in 0..3 {
        line(&mut p, text, 57.0, 200.0 - r as f32 * 13.0, 10.0, false);
    }
    line(
        &mut p,
        "lanjutan di kolom kanan.",
        312.0,
        760.0,
        10.0,
        false,
    );
    let doc = analyse(vec![p]);
    assert_eq!(texts(&doc).len(), 1, "{:?}", texts(&doc));
}

#[test]
fn a_new_column_that_starts_a_sentence_starts_a_paragraph() {
    let mut p = page();
    line(
        &mut p,
        "Akhir kalimat di kolom kiri.",
        57.0,
        200.0,
        10.0,
        false,
    );
    line(
        &mut p,
        "Kalimat baru di kolom kanan.",
        312.0,
        760.0,
        10.0,
        false,
    );
    let doc = analyse(vec![p]);
    assert_eq!(texts(&doc).len(), 2);
}

#[test]
fn a_hanging_bullet_does_not_set_the_margin() {
    let mut p = page();
    for r in 0..6 {
        line(&mut p, FULL, 72.0, 760.0 - r as f32 * 14.0, 11.0, false);
    }
    line(&mut p, "• butir", 60.0, 660.0, 11.0, false);
    let doc = analyse(vec![p]);
    assert!((doc.margins[0] - 72.0).abs() < 0.01, "{:?}", doc.margins);
}
