//! Characters to paragraphs.
//!
//! The order is PDFium's: its text page already puts characters in reading
//! order and marks where it thinks a line ends (a generated line break), and
//! second-guessing that with our own sort does worse on the documents people
//! actually have — two-column journals included. What is decided here is what
//! PDFium does not say: where a paragraph ends, what it looks like, and where
//! a picture goes between them.
//!
//! Every threshold is relative to the text's own size, so a 9 pt footnote and
//! a 24 pt title are judged alike.

use crate::page::{Glyph, PageLayout, Picture, Rect};

/// How a paragraph's lines sit between the margins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

/// A stretch of text in one style.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// `\t` stands for a tab: a wide gap inside a line, which is how a table
    /// row or a "Name ...... page" line survives as one line.
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    /// Points.
    pub size: f32,
    pub color: [u8; 3],
    pub font: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    pub align: Align,
    /// 1..=3 for a heading, by size rank across the document.
    pub heading: Option<u8>,
    /// From the left margin, points.
    pub indent_left: f32,
    /// First line relative to the others, points; negative hangs.
    pub first_line: f32,
    /// Points above, from the gap on the page.
    pub space_before: f32,
    /// Multiple of single spacing.
    pub line_spacing: f32,
    pub page_break_before: bool,
    /// Tab stops, points from the left margin: where each cell of a table row
    /// started on the page, so the columns line up in Word as they did.
    pub tabs: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PictureBlock {
    /// Index into [`Document::pictures`].
    pub picture: usize,
    /// Size in the document, points.
    pub width: f32,
    pub height: f32,
    pub align: Align,
    pub indent_left: f32,
    pub space_before: f32,
    pub page_break_before: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph(Paragraph),
    Picture(PictureBlock),
}

/// What the conversion could and could not carry, for the message afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    pub pages: u32,
    pub paragraphs: u32,
    pub pictures: u32,
    /// Sideways characters left out.
    pub skipped_turned: u32,
    /// Pictures that could not be read.
    pub skipped_pictures: u32,
    /// Pages with no text at all, which came across as their pictures only —
    /// a scan that has not been through OCR.
    pub pages_without_text: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub pictures: Vec<Picture>,
    /// The first page's size, points: one section, one page size.
    pub page_width: f32,
    pub page_height: f32,
    /// Left, top, right, bottom, points.
    pub margins: [f32; 4],
    /// The size most of the text is set in, points.
    pub body_size: f32,
    /// The font most of the text is set in.
    pub body_font: String,
    pub stats: Stats,
}

/// One line of text as read off a page.
#[derive(Debug, Clone)]
struct Line {
    /// The characters as read, kept to rebuild the line when two are joined.
    glyphs: Vec<Glyph>,
    pieces: Vec<Piece>,
    /// Over the drawn characters, not the spaces PDFium inferred.
    bbox: Rect,
    baseline: f32,
    size: f32,
    bold: bool,
}

#[derive(Debug, Clone)]
enum Piece {
    Glyph(Glyph),
    /// The x where the text after the gap starts, points.
    Tab(f32),
}

impl Line {
    fn text(&self) -> String {
        self.pieces
            .iter()
            .map(|p| match p {
                Piece::Glyph(g) => g.ch,
                Piece::Tab(_) => '\t',
            })
            .collect()
    }

    fn center(&self) -> f32 {
        (self.bbox.left + self.bbox.right) / 2.0
    }
}

/// A gap inside a line at least this many times the text size is a column
/// gap, written as a tab. An ordinary or even a justified space is well under.
const TAB_GAP: f32 = 2.2;
/// A picture covering this much of a page that also has text is the scan the
/// text was recognised from: the text is what the user wants to edit, and the
/// scan under it would only repeat it.
const SCAN_COVER: f32 = 0.7;

fn is_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{a0}'
}

/// Splits a page's characters into lines, in PDFium's order.
fn lines_of(page: &PageLayout) -> Vec<Line> {
    let mut lines: Vec<Vec<Glyph>> = Vec::new();
    let mut current: Vec<Glyph> = Vec::new();
    let flush = |current: &mut Vec<Glyph>, lines: &mut Vec<Vec<Glyph>>| {
        if current.iter().any(|g| !is_space(g.ch)) {
            lines.push(std::mem::take(current));
        } else {
            current.clear();
        }
    };
    for g in &page.glyphs {
        if g.ch == '\r' || g.ch == '\n' {
            flush(&mut current, &mut lines);
            continue;
        }
        if !g.generated && !is_space(g.ch) {
            if let Some(last) = current
                .iter()
                .rev()
                .find(|x| !x.generated && !is_space(x.ch))
            {
                let size = last.size.max(g.size).max(1.0);
                let jumped = (last.baseline - g.baseline).abs() > size * 0.6;
                let small = g.size < last.size * 0.8 || last.size < g.size * 0.8;
                let back = g.rect.left < last.rect.left - size;
                // A superscript or subscript sits off the baseline but is
                // still on the line; a jump with a step back is a new line,
                // and so is a step back to somewhere wholly below.
                let new_line =
                    (jumped && (!small || back)) || (back && g.rect.top < last.rect.bottom);
                if new_line {
                    flush(&mut current, &mut lines);
                }
            }
        }
        current.push(g.clone());
    }
    flush(&mut current, &mut lines);

    // PDFium ends a line at a baseline shift, so a superscript ("naik¹ pada")
    // splits one printed line in two. A piece that carries on along the same
    // baseline, to the right of where the last one stopped, is joined back.
    let mut out: Vec<Line> = Vec::new();
    for line in lines.into_iter().filter_map(build_line) {
        let joined = match out.last() {
            Some(prev) => {
                let size = prev.size.max(line.size);
                (prev.baseline - line.baseline).abs() < size * 0.3
                    && line.bbox.left >= prev.bbox.right - size * 0.5
                    && (prev.size - line.size).abs() <= 0.75
            }
            None => false,
        };
        match (joined, out.pop()) {
            (true, Some(prev)) => {
                let mut glyphs = prev.glyphs;
                glyphs.extend(line.glyphs);
                if let Some(rebuilt) = build_line(glyphs) {
                    out.push(rebuilt);
                }
            }
            (_, prev) => {
                out.extend(prev);
                out.push(line);
            }
        }
    }
    out
}

fn build_line(glyphs: Vec<Glyph>) -> Option<Line> {
    // Spaces at either end are PDFium's guesses at the line's edge.
    let first = glyphs.iter().position(|g| !is_space(g.ch))?;
    let last = glyphs.iter().rposition(|g| !is_space(g.ch))?;
    let glyphs = glyphs.get(first..=last)?.to_vec();
    let drawn: Vec<&Glyph> = glyphs.iter().filter(|g| !is_space(g.ch)).collect();
    let bbox = drawn.iter().map(|g| g.rect).reduce(|a, b| a.union(&b))?;
    let size = dominant(drawn.iter().map(|g| g.size));
    let mut baselines: Vec<f32> = drawn
        .iter()
        .filter(|g| (g.size - size).abs() < 0.6)
        .map(|g| g.baseline)
        .collect();
    baselines.sort_by(f32::total_cmp);
    let baseline = baselines
        .get(baselines.len() / 2)
        .copied()
        .unwrap_or(bbox.bottom);
    let bold = drawn.iter().filter(|g| g.bold).count() * 2 > drawn.len();

    // Wide gaps become tabs, and the spaces inside them go.
    let mut pieces = Vec::with_capacity(glyphs.len());
    let mut prev: Option<&Glyph> = None;
    let mut pending_spaces: Vec<Glyph> = Vec::new();
    for g in &glyphs {
        if is_space(g.ch) {
            pending_spaces.push(g.clone());
            continue;
        }
        if let Some(p) = prev {
            let gap = g.rect.left - p.rect.right;
            let s = p.size.max(g.size);
            if gap > s * TAB_GAP && gap > 12.0 {
                pieces.push(Piece::Tab(g.rect.left));
                pending_spaces.clear();
            } else if pending_spaces.is_empty() && gap > s * 0.25 {
                // A space the PDF drew as a gap and PDFium did not infer.
                let mut space = p.clone();
                space.ch = ' ';
                space.generated = true;
                pieces.push(Piece::Glyph(space));
            }
        }
        // Runs of inferred spaces collapse to one.
        if let Some(space) = pending_spaces.first() {
            pieces.push(Piece::Glyph(space.clone()));
        }
        pending_spaces.clear();
        pieces.push(Piece::Glyph(g.clone()));
        prev = Some(g);
    }
    Some(Line {
        glyphs,
        pieces,
        bbox,
        baseline,
        size,
        bold,
    })
}

/// The value most of `sizes` round to, to the half point.
fn dominant(sizes: impl Iterator<Item = f32>) -> f32 {
    let mut counts: Vec<(i32, usize)> = Vec::new();
    for s in sizes {
        let key = (s * 2.0).round() as i32;
        match counts.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => counts.push((key, 1)),
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(k, _)| k as f32 / 2.0)
        .unwrap_or(11.0)
}

/// Whether a line opens a list item: a bullet or "1." / "a)" before a space.
fn starts_item(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if matches!(
        first,
        '•' | '◦' | '▪' | '■' | '●' | '○' | '–' | '—' | '-' | '*' | '·'
    ) {
        return chars.next().is_some_and(is_space);
    }
    let head: String = text.chars().take(5).collect();
    let digits = head.chars().take_while(char::is_ascii_digit).count();
    let letter = head.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    let marker_at = if digits > 0 && digits <= 3 {
        digits
    } else if letter {
        1
    } else {
        return false;
    };
    let mut rest = head.chars().skip(marker_at);
    matches!(rest.next(), Some('.' | ')')) && rest.next().is_some_and(is_space)
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .chars()
        .last()
        .is_some_and(|c| matches!(c, '.' | ':' | '?' | '!' | '"' | '”' | ')'))
}

/// The page's text column: the leftmost edge that several lines start at, and
/// the rightmost that several end at. One bullet hanging into the margin or
/// one wide caption is not the column.
fn column_of(lines: &[Line]) -> (f32, f32) {
    let enough = (lines.len() / 10).max(2);
    let edge = |values: Vec<f32>, leftmost: bool| -> f32 {
        let mut sorted = values;
        sorted.sort_by(f32::total_cmp);
        if !leftmost {
            sorted.reverse();
        }
        let shared = sorted
            .iter()
            .find(|v| sorted.iter().filter(|w| (**w - **v).abs() <= 1.5).count() >= enough);
        shared.or(sorted.first()).copied().unwrap_or(0.0)
    };
    (
        edge(lines.iter().map(|l| l.bbox.left).collect(), true),
        edge(lines.iter().map(|l| l.bbox.right).collect(), false),
    )
}

/// Groups a page's lines into paragraphs.
fn paragraphs_of(lines: &[Line]) -> Vec<Vec<usize>> {
    let (_, col_right) = column_of(lines);
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let joins = match out.last() {
            None => false,
            Some(para) => {
                let prev = para.last().and_then(|&k| lines.get(k));
                prev.is_some_and(|prev| continues(prev, line, para, lines, col_right))
            }
        };
        if joins {
            if let Some(para) = out.last_mut() {
                para.push(i);
            }
        } else {
            out.push(vec![i]);
        }
    }
    out
}

fn continues(prev: &Line, line: &Line, para: &[usize], lines: &[Line], col_right: f32) -> bool {
    let size = prev.size.max(line.size);
    if (prev.size - line.size).abs() > 0.75 || prev.bold != line.bold {
        return false;
    }
    let gap = prev.baseline - line.baseline;
    if gap <= 0.0 {
        // Up and to the right: the next column. The paragraph carries on
        // there when it had not finished and the column starts mid-sentence.
        let next_column = line.bbox.left > prev.bbox.right - size;
        let mid_sentence = !ends_sentence(&prev.text())
            && line.text().chars().next().is_some_and(char::is_lowercase);
        return next_column && mid_sentence;
    }
    // The pitch this paragraph has kept so far, or the usual one for its size.
    let pitch = match para {
        [.., a, b] => match (lines.get(*a), lines.get(*b)) {
            (Some(a), Some(b)) => (a.baseline - b.baseline).max(size),
            _ => size * 1.45,
        },
        _ => size * 1.45,
    };
    if gap > pitch * 1.3 + 0.5 {
        return false;
    }
    let text = line.text();
    if starts_item(&text) {
        return false;
    }
    // A table row is a paragraph of its own; joined, the rows would run
    // together into one line of cells.
    if text.contains('\t') || prev.text().contains('\t') {
        return false;
    }
    // A line that ends a sentence well short of the column ends its paragraph.
    if ends_sentence(&prev.text()) && prev.bbox.right < col_right - size * 3.0 {
        return false;
    }
    let first = para.first().and_then(|&k| lines.get(k));
    let left_moved = (line.bbox.left - prev.bbox.left).abs() > size * 1.5;
    if left_moved {
        // A first-line indent: the paragraph's first line starts further in
        // than the second.
        let indented_first = para.len() == 1
            && first.is_some_and(|f| {
                f.bbox.left > line.bbox.left && f.bbox.left - line.bbox.left < size * 5.0
            });
        // The lines after a list item's marker hang in from it.
        let hanging = para.len() == 1
            && first.is_some_and(|f| {
                starts_item(&f.text())
                    && line.bbox.left > f.bbox.left
                    && line.bbox.left - f.bbox.left < size * 4.0
            });
        // Centred lines move about by nature.
        let centred = (line.center() - prev.center()).abs() < size;
        if !(indented_first || hanging || centred) {
            return false;
        }
    }
    true
}

fn align_of(lines: &[&Line], col: (f32, f32), page_width: f32) -> Align {
    let (left, right) = col;
    let width = right - left;
    let Some(first) = lines.first() else {
        return Align::Left;
    };
    let size = first.size;
    // A table row is laid out by its tab stops, from the left.
    if lines
        .iter()
        .any(|l| l.pieces.iter().any(|p| matches!(p, Piece::Tab(_))))
    {
        return Align::Left;
    }
    // Centred on the text column, or on the page when the column is lopsided
    // (a wide left margin for binding, a sidebar).
    let centred = [(left + right) / 2.0, page_width / 2.0]
        .iter()
        .any(|mid| lines.iter().all(|l| (l.center() - mid).abs() < size * 1.2));
    let at_left = lines
        .iter()
        .all(|l| (l.bbox.left - left).abs() < size * 1.5);
    let at_right = lines
        .iter()
        .all(|l| (right - l.bbox.right).abs() < size * 1.5);
    if centred && !(at_left && at_right) {
        return Align::Center;
    }
    if at_right && !at_left && lines.iter().all(|l| l.bbox.left > left + size * 2.0) {
        return Align::Right;
    }
    if let Some((_last, body)) = lines.split_last() {
        if !body.is_empty()
            && body
                .iter()
                .all(|l| right - l.bbox.right < size * 0.8 && l.bbox.width() > width * 0.6)
        {
            return Align::Justify;
        }
    }
    Align::Left
}

/// Text runs for the lines of one paragraph, joined the way the words were.
fn runs_of(lines: &[&Line], fonts: &[String]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut style: Option<Glyph> = None;
    let push = |runs: &mut Vec<Run>, g: &Glyph, text: &str| {
        let font = fonts.get(usize::from(g.font)).cloned().unwrap_or_default();
        let size = (g.size * 2.0).round() / 2.0;
        match runs.last_mut() {
            Some(r)
                if r.bold == g.bold
                    && r.italic == g.italic
                    && r.color == legible(g.color)
                    && r.font == font
                    && (r.size - size).abs() < 0.01 =>
            {
                r.text.push_str(text);
            }
            _ => runs.push(Run {
                text: text.to_string(),
                bold: g.bold,
                italic: g.italic,
                size,
                color: legible(g.color),
                font,
            }),
        }
    };
    for (n, line) in lines.iter().enumerate() {
        if n > 0 {
            // A hyphen that broke a word over the line goes; otherwise one
            // space between the lines.
            let ends_hyphen = runs.last().is_some_and(|r| r.text.ends_with('-'));
            let next_lower = line.text().chars().next().is_some_and(char::is_lowercase);
            if ends_hyphen && next_lower {
                if let Some(r) = runs.last_mut() {
                    r.text.pop();
                }
            } else if let Some(g) = style.clone() {
                if !runs.last().is_some_and(|r| r.text.ends_with(' ')) {
                    push(&mut runs, &g, " ");
                }
            }
        }
        for piece in &line.pieces {
            match piece {
                Piece::Glyph(g) => {
                    // An inferred space takes the style of the text before it,
                    // so it never splits a run.
                    let g = if g.generated {
                        match &style {
                            Some(s) => s,
                            None => g,
                        }
                        .clone()
                    } else {
                        g.clone()
                    };
                    let mut buf = [0u8; 4];
                    let ch = if is_space(piece_char(piece)) {
                        ' '
                    } else {
                        piece_char(piece)
                    };
                    push(&mut runs, &g, ch.encode_utf8(&mut buf));
                    style = Some(g);
                }
                Piece::Tab(_) => {
                    if let Some(g) = &style {
                        push(&mut runs, g, "\t");
                    }
                }
            }
        }
    }
    runs
}

fn piece_char(p: &Piece) -> char {
    match p {
        Piece::Glyph(g) => g.ch,
        Piece::Tab(_) => '\t',
    }
}

/// Builds the document from its pages.
pub fn analyse(pages: Vec<PageLayout>) -> Document {
    let mut stats = Stats {
        pages: pages.len() as u32,
        ..Stats::default()
    };
    let body_size = dominant(pages.iter().flat_map(|p| {
        p.glyphs
            .iter()
            .filter(|g| !g.generated && !is_space(g.ch))
            .map(|g| g.size)
    }));
    let body_font = {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for p in &pages {
            for g in p.glyphs.iter().filter(|g| !g.generated) {
                let name = p
                    .fonts
                    .get(usize::from(g.font))
                    .cloned()
                    .unwrap_or_default();
                match counts.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, c)) => *c += 1,
                    None => counts.push((name, 1)),
                }
            }
        }
        counts
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .map(|(n, _)| n)
            .unwrap_or_default()
    };

    // Heading levels: the sizes clearly above the body, largest first — but in
    // a document of several pages only sizes used on two pages or more. A
    // cover's one-off display type would otherwise take "Heading 1" from the
    // chapter titles; its runs keep their size either way.
    let page_lines: Vec<Vec<Line>> = pages.iter().map(lines_of).collect();
    let mut heading_sizes: Vec<(f32, usize)> = Vec::new();
    for lines in &page_lines {
        let mut seen: Vec<f32> = Vec::new();
        for l in lines {
            let s = (l.size * 2.0).round() / 2.0;
            if s >= body_size * 1.3 && !seen.iter().any(|h| (h - s).abs() < 0.01) {
                seen.push(s);
            }
        }
        for s in seen {
            match heading_sizes.iter_mut().find(|(h, _)| (h - s).abs() < 0.01) {
                Some((_, n)) => *n += 1,
                None => heading_sizes.push((s, 1)),
            }
        }
    }
    let needed = if page_lines.len() > 1 { 2 } else { 1 };
    let mut heading_sizes: Vec<f32> = heading_sizes
        .into_iter()
        .filter(|(_, pages)| *pages >= needed)
        .map(|(s, _)| s)
        .collect();
    heading_sizes.sort_by(|a, b| b.total_cmp(a));

    let (page_width, page_height) = pages
        .first()
        .map(|p| (p.width, p.height))
        .unwrap_or((595.0, 842.0));
    let margins = margins_of(&pages, &page_lines, page_width, page_height);
    let content_width = (page_width - margins[0] - margins[2]).max(72.0);

    let mut blocks = Vec::new();
    let mut pictures = Vec::new();
    for (index, (page, lines)) in pages.into_iter().zip(page_lines).enumerate() {
        stats.skipped_turned += page.skipped_turned;
        stats.skipped_pictures += page.skipped_pictures;
        let has_text = !lines.is_empty();
        if !has_text {
            stats.pages_without_text += 1;
        }
        let page_area = (page.width * page.height).max(1.0);
        let (col_left, col_right) = if has_text {
            column_of(&lines)
        } else {
            (margins[0], page.width - margins[2])
        };
        let paras = reading_order(paragraphs_of(&lines), &lines, page.height);

        // The page's items in reading order: paragraphs in PDFium's order,
        // each picture placed in its own column (see `picture_slot`).
        enum Item {
            Para(Vec<usize>),
            Pic(Picture),
        }
        let boxes: Vec<Rect> = paras.iter().map(|p| bbox_of(p, &lines)).collect();
        let mut slots: Vec<(usize, Picture)> = Vec::new();
        for pic in page.pictures {
            if has_text && pic.rect.width() * pic.rect.height() >= page_area * SCAN_COVER {
                continue;
            }
            slots.push((picture_slot(&pic.rect, &boxes), pic));
        }
        let mut items: Vec<Item> = Vec::new();
        for (k, para) in paras.into_iter().enumerate() {
            let mut k_slots: Vec<usize> = (0..slots.len())
                .filter(|&s| slots.get(s).is_some_and(|x| x.0 == k))
                .collect();
            k_slots.sort_unstable();
            for s in k_slots {
                if let Some((_, pic)) = slots.get(s) {
                    items.push(Item::Pic(pic.clone()));
                }
            }
            items.push(Item::Para(para));
        }
        let count = boxes.len();
        for (slot, pic) in slots {
            if slot >= count {
                items.push(Item::Pic(pic));
            }
        }

        // The first block keeps its distance from the top margin.
        let mut above: Option<f32> = Some(page.height - margins[1]);
        let mut first_on_page = true;
        let page_start = blocks.len();
        // How tall each block is on the PDF page, to check the page still
        // fits in Word once the gaps are added back.
        let mut heights: Vec<f32> = Vec::new();
        for item in items {
            let page_break_before = index > 0 && first_on_page;
            first_on_page = false;
            match item {
                Item::Para(ids) => {
                    let ls: Vec<&Line> = ids.iter().filter_map(|&k| lines.get(k)).collect();
                    let Some(first) = ls.first() else { continue };
                    let size = first.size;
                    let align = align_of(&ls, (col_left, col_right), page.width);
                    let rest_left = ls
                        .iter()
                        .skip(1)
                        .map(|l| l.bbox.left)
                        .fold(f32::INFINITY, f32::min);
                    let para_left = if ls.len() > 1 {
                        rest_left
                    } else {
                        first.bbox.left
                    };
                    // Where this paragraph's column starts. Word has one
                    // column, so a second column's text is measured from its
                    // own edge — else every paragraph of it would be indented
                    // by half a page. The first (or only) column is measured
                    // from the margin, which keeps a real indent.
                    let own = ls
                        .iter()
                        .map(|l| l.bbox)
                        .reduce(|a, b| a.union(&b))
                        .unwrap_or_default();
                    // Lines mostly inside this paragraph's span: a title
                    // across both columns overlaps the second column too, but
                    // most of it is elsewhere.
                    let column = lines
                        .iter()
                        .filter(|l| {
                            let shared = l.bbox.right.min(own.right) - l.bbox.left.max(own.left);
                            shared > l.bbox.width().max(1.0) * 0.6
                        })
                        .map(|l| l.bbox.left)
                        .fold(own.left, f32::min);
                    let origin = if column - margins[0] > size * 4.0 {
                        column
                    } else {
                        margins[0]
                    };
                    let (indent_left, first_line) = match align {
                        Align::Center | Align::Right => (0.0, 0.0),
                        _ => {
                            // Negative where the text sits outside the usual
                            // margin, but never past the paper's edge.
                            let indent = (para_left - origin).max(6.0 - margins[0]);
                            let first_line = if ls.len() > 1 {
                                first.bbox.left - para_left
                            } else {
                                0.0
                            };
                            (
                                if indent.abs() < size * 0.5 {
                                    0.0
                                } else {
                                    indent
                                },
                                if first_line.abs() < size * 0.5 {
                                    0.0
                                } else {
                                    first_line
                                },
                            )
                        }
                    };
                    let line_spacing = match ls.as_slice() {
                        [a, b, ..] => ((a.baseline - b.baseline) / (size * 1.17)).clamp(1.0, 3.0),
                        _ => 1.0,
                    };
                    let space_before = match above {
                        Some(bottom) => (bottom - first.bbox.top).clamp(0.0, 144.0),
                        None => 0.0,
                    };
                    above = ls.last().map(|l| l.bbox.bottom);
                    heights.push(ls.last().map_or(0.0, |l| first.bbox.top - l.bbox.bottom));
                    let rounded = (size * 2.0).round() / 2.0;
                    let heading = if ls.len() <= 4 {
                        heading_sizes
                            .iter()
                            .position(|h| (h - rounded).abs() < 0.01)
                            .map(|k| (k.min(2) + 1) as u8)
                    } else {
                        None
                    };
                    let mut tabs: Vec<f32> = Vec::new();
                    for l in &ls {
                        for piece in &l.pieces {
                            if let Piece::Tab(x) = piece {
                                let at = x - origin;
                                if at > 0.0 && !tabs.iter().any(|t| (t - at).abs() < 2.0) {
                                    tabs.push(at);
                                }
                            }
                        }
                    }
                    tabs.sort_by(f32::total_cmp);
                    stats.paragraphs += 1;
                    blocks.push(Block::Paragraph(Paragraph {
                        runs: runs_of(&ls, &page.fonts),
                        align,
                        heading,
                        indent_left,
                        first_line,
                        space_before,
                        line_spacing,
                        page_break_before,
                        tabs,
                    }));
                }
                Item::Pic(pic) => {
                    let scale = (content_width / pic.rect.width()).min(1.0);
                    let width = pic.rect.width() * scale;
                    let height = pic.rect.height() * scale;
                    let centre = (pic.rect.left + pic.rect.right) / 2.0;
                    let centred = (centre - page.width / 2.0).abs() < page.width * 0.08;
                    let space_before = match above {
                        Some(bottom) => (bottom - pic.rect.top).clamp(0.0, 144.0),
                        None => 0.0,
                    };
                    above = Some(pic.rect.bottom);
                    heights.push(height);
                    let indent_left = if centred {
                        0.0
                    } else {
                        (pic.rect.left - margins[0]).clamp(0.0, (content_width - width).max(0.0))
                    };
                    stats.pictures += 1;
                    blocks.push(Block::Picture(PictureBlock {
                        picture: pictures.len(),
                        width,
                        height,
                        align: if centred { Align::Center } else { Align::Left },
                        indent_left,
                        space_before,
                        page_break_before,
                    }));
                    pictures.push(pic);
                }
            }
        }
        fit_page(
            blocks.get_mut(page_start..).unwrap_or_default(),
            &heights,
            page_height - margins[1] - margins[3],
        );
    }

    Document {
        blocks,
        pictures,
        page_width,
        page_height,
        margins,
        body_size,
        body_font,
        stats,
    }
}

/// Margins from where the text sits on most pages: the median of each side.
/// The narrowest would let one cover or one wide table set the margins of
/// every page, and the body would then run wider in Word than it did in the
/// PDF; text that sits further out gets a negative indent instead.
fn margins_of(pages: &[PageLayout], page_lines: &[Vec<Line>], width: f32, height: f32) -> [f32; 4] {
    let mut sides: [Vec<f32>; 4] = Default::default();
    for (page, lines) in pages.iter().zip(page_lines) {
        if lines.is_empty() {
            continue;
        }
        let (l, r) = column_of(lines);
        let top = lines
            .iter()
            .map(|x| x.bbox.top)
            .fold(f32::NEG_INFINITY, f32::max);
        let bottom = lines
            .iter()
            .map(|x| x.bbox.bottom)
            .fold(f32::INFINITY, f32::min);
        sides[0].push(l);
        sides[1].push(page.height - top);
        sides[2].push(page.width - r);
        sides[3].push(bottom);
    }
    let fallback = [width * 0.12, height * 0.1, width * 0.12, height * 0.1];
    let mut out = [0.0; 4];
    for ((slot, values), fallback) in out.iter_mut().zip(sides.iter_mut()).zip(fallback) {
        values.sort_by(f32::total_cmp);
        let v = values.get(values.len() / 2).copied().unwrap_or(fallback);
        *slot = v.clamp(18.0, 108.0);
    }
    out
}

fn bbox_of(para: &[usize], lines: &[Line]) -> Rect {
    para.iter()
        .filter_map(|&k| lines.get(k).map(|l| l.bbox))
        .reduce(|a, b| a.union(&b))
        .unwrap_or_default()
}

fn horizontal_overlap(a: &Rect, b: &Rect) -> f32 {
    let shared = a.right.min(b.right) - a.left.max(b.left);
    let narrower = a.width().min(b.width()).max(1.0);
    shared / narrower
}

/// Running headers first and page numbers last. A PDF often draws them
/// before the body (they belong to the page template), which put "Halaman 3"
/// above the chapter it numbers.
fn reading_order(paras: Vec<Vec<usize>>, lines: &[Line], height: f32) -> Vec<Vec<usize>> {
    let band = height * 0.07;
    let mut head = Vec::new();
    let mut body = Vec::new();
    let mut foot = Vec::new();
    for p in paras {
        let b = bbox_of(&p, lines);
        if b.bottom >= height - band {
            head.push(p);
        } else if b.top <= band {
            foot.push(p);
        } else {
            body.push(p);
        }
    }
    head.extend(body);
    head.extend(foot);
    head
}

/// Before which paragraph a picture goes: right after the nearest paragraph
/// above it *in its own column* (one it shares most of its width with), or
/// else right before the nearest one below it. In a two-column page that is
/// what keeps a figure with the text around it rather than with whatever
/// line of the other column happens to sit at the same height. `boxes.len()`
/// means the end of the page.
fn picture_slot(pic: &Rect, boxes: &[Rect]) -> usize {
    let same_column: Vec<usize> = (0..boxes.len())
        .filter(|&k| {
            boxes
                .get(k)
                .is_some_and(|b| horizontal_overlap(b, pic) > 0.3)
        })
        .collect();
    let above = same_column
        .iter()
        .filter(|&&k| boxes.get(k).is_some_and(|b| b.bottom >= pic.top - 2.0))
        .min_by(|&&a, &&b| {
            let (a, b) = (
                boxes.get(a).map_or(0.0, |r| r.bottom),
                boxes.get(b).map_or(0.0, |r| r.bottom),
            );
            a.total_cmp(&b)
        });
    if let Some(&k) = above {
        return k + 1;
    }
    let below = same_column
        .iter()
        .filter(|&&k| boxes.get(k).is_some_and(|b| b.top <= pic.bottom + 2.0))
        .max_by(|&&a, &&b| {
            let (a, b) = (
                boxes.get(a).map_or(0.0, |r| r.top),
                boxes.get(b).map_or(0.0, |r| r.top),
            );
            a.total_cmp(&b)
        });
    if let Some(&k) = below {
        return k;
    }
    // Nothing shares its column: by height alone.
    boxes
        .iter()
        .position(|b| b.top <= pic.top)
        .unwrap_or(boxes.len())
}

/// Closes up the gaps on one page when the page as converted would not fit
/// on one Word page. A page that spills shifts every page after it, so the
/// Word document's page 12 would no longer be the PDF's page 12 — which is
/// how readers find their place. Gaps go first, in proportion; text and
/// pictures keep their size.
fn fit_page(blocks: &mut [Block], heights: &[f32], available: f32) {
    let budget = available * 0.96;
    let content: f32 = heights.iter().sum();
    let gaps: f32 = blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => p.space_before,
            Block::Picture(p) => p.space_before,
        })
        .sum();
    if content + gaps <= budget || gaps <= 0.0 {
        return;
    }
    let keep = ((budget - content) / gaps).clamp(0.0, 1.0);
    for b in blocks {
        match b {
            Block::Paragraph(p) => p.space_before *= keep,
            Block::Picture(p) => p.space_before *= keep,
        }
    }
}

/// Text so pale it only showed against something the conversion does not
/// carry — white on a coloured band drawn as vector shapes — would be white
/// on white paper in Word. It is set in black instead.
fn legible(color: [u8; 3]) -> [u8; 3] {
    let [r, g, b] = color.map(|c| f32::from(c) / 255.0);
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if luminance > 0.88 {
        [0, 0, 0]
    } else {
        color
    }
}

#[cfg(test)]
mod tests;
