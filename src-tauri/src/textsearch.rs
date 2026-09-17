//! The regex search path, and the geometry every hit needs (SPEC 11.1).
//!
//! ## Why regex is a separate path, and stays one
//!
//! SPEC 11.1 allows a regular-expression search but refuses to let it be sold
//! as the same feature as the other two tiers, and the reason is structural
//! rather than a matter of effort. FTS5 answers "which pages contain these
//! words" from an index of *tokens*: it never sees the page's characters in
//! order, so there is nothing there for a pattern like `\d{3}-\d{4}` to run
//! against. A regex can only be applied to text somebody has extracted and is
//! holding — which means a document that is open, one page at a time.
//!
//! Pretending otherwise would produce a search box that quietly returns results
//! from one document when the user believes they are searching their library.
//! So the command is named for what it does, refuses to run on a document that
//! is not open, and the panel says so where the user can read it.
//!
//! ## Turning a character range into boxes
//!
//! PDFium's own finder returns highlight rectangles; a regex match is just a
//! range of character indices, and the boxes have to be built from the
//! per-character boxes the text layer already carries. Consecutive characters
//! are merged into one rectangle for as long as they stay on the same line, and
//! a match that wraps produces one rectangle per line — the same shape
//! [`izul_ipc::message::SearchHitWire`] uses, and for the same reason: a single
//! box spanning two lines paints over everything between them.

use izul_ipc::message::CharBoxWire;
use izul_model::geom::PdfRectF;

/// Most matches returned from one page, whatever the caller asks for.
///
/// A pattern like `.` matches once per character; a dense page is thousands of
/// hits, and neither the wire nor a results list is better for having the tail
/// of that.
pub const MAX_HITS_PER_PAGE: usize = 500;

/// One match, ready for the wire.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HitOut {
    pub char_index: u32,
    pub char_count: u32,
    pub rects: Vec<RectOut>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct RectOut {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

impl From<PdfRectF> for RectOut {
    fn from(r: PdfRectF) -> Self {
        RectOut {
            left: r.left,
            bottom: r.bottom,
            right: r.right,
            top: r.top,
        }
    }
}

/// Whether two character boxes sit on the same line of text.
///
/// Vertical overlap rather than equal baselines: the boxes of a line are not
/// all the same height — a comma and a capital letter differ — and superscripts
/// share a line with the text they hang off. Half the shorter box's height is
/// enough overlap to mean "same line" and little enough that the next line
/// down, which typically clears it entirely, never qualifies.
fn same_line(a: &PdfRectF, b: &PdfRectF) -> bool {
    let overlap = a.top.min(b.top) - a.bottom.max(b.bottom);
    let shorter = (a.top - a.bottom).min(b.top - b.bottom);
    overlap > 0.0 && (shorter <= 0.0 || overlap >= shorter * 0.5)
}

fn union(a: PdfRectF, b: &PdfRectF) -> PdfRectF {
    PdfRectF {
        left: a.left.min(b.left),
        bottom: a.bottom.min(b.bottom),
        right: a.right.max(b.right),
        top: a.top.max(b.top),
    }
}

/// Boxes covering characters `start..start + len`, one per line.
///
/// Characters with no box of their own — PDFium gives a zero rectangle for a
/// line break, among others — are skipped rather than merged, so a match that
/// wraps does not acquire a rectangle at the origin.
pub fn rects_for_range(chars: &[CharBoxWire], start: usize, len: usize) -> Vec<RectOut> {
    let mut out: Vec<PdfRectF> = Vec::new();
    for ch in chars.iter().skip(start).take(len) {
        let r = ch.rect;
        if r.right <= r.left || r.top <= r.bottom {
            continue;
        }
        match out.last_mut() {
            Some(last) if same_line(last, &r) => *last = union(*last, &r),
            _ => out.push(r),
        }
    }
    out.into_iter().map(RectOut::from).collect()
}

/// Character offset and length of every match of `re` in `text`.
///
/// Offsets are in `char`s, not bytes, because that is what the character boxes
/// and [`izul_ipc::message::SearchHitWire`] are indexed by. Getting this wrong
/// shows up only on pages with non-ASCII text, which is most of them in
/// Indonesian typography once a single "×" or a curly quote appears.
///
/// An empty match — which `a*` produces at every position — is skipped: it
/// highlights nothing and there is one per character.
pub fn regex_hits(text: &str, re: &regex::Regex, max: usize) -> Vec<(u32, u32)> {
    // Byte offset -> char offset, walked once, rather than re-counting the
    // prefix for every match: a page with 3000 characters and 300 matches would
    // otherwise be quadratic for no reason.
    let mut out = Vec::new();
    let mut byte_to_char: Vec<(usize, u32)> = Vec::new();
    for (chars, (byte, _)) in text.char_indices().enumerate() {
        byte_to_char.push((byte, chars as u32));
    }
    let char_at = |byte: usize| -> u32 {
        match byte_to_char.binary_search_by_key(&byte, |(b, _)| *b) {
            Ok(i) => byte_to_char.get(i).map(|(_, c)| *c).unwrap_or(0),
            Err(_) => byte_to_char.len() as u32,
        }
    };
    for m in re.find_iter(text) {
        if m.start() == m.end() {
            continue;
        }
        let start = char_at(m.start());
        let end = char_at(m.end());
        out.push((start, end.saturating_sub(start)));
        if out.len() >= max {
            break;
        }
    }
    out
}

/// Compiles a user-typed pattern, with a size limit.
///
/// `regex` is backtracking-free, so a pattern cannot hang the process the way
/// one can in most languages — but it *can* compile to an enormous automaton,
/// and this runs in the UI process. The limit turns that into an error message
/// instead of a memory spike.
pub fn compile(pattern: &str, case_sensitive: bool) -> Result<regex::Regex, String> {
    regex::RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| format!("pola tidak sah: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes(spec: &[(f32, f32, f32, f32)]) -> Vec<CharBoxWire> {
        spec.iter()
            .map(|(left, bottom, right, top)| CharBoxWire {
                unicode: 'x',
                rect: PdfRectF {
                    left: *left,
                    bottom: *bottom,
                    right: *right,
                    top: *top,
                },
            })
            .collect()
    }

    #[test]
    fn characters_on_one_line_become_one_box() {
        let chars = boxes(&[
            (10.0, 100.0, 16.0, 112.0),
            (16.0, 100.0, 22.0, 112.0),
            (22.0, 102.0, 28.0, 112.0),
        ]);
        let rects = rects_for_range(&chars, 0, 3);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].left, 10.0);
        assert_eq!(rects[0].right, 28.0);
        assert_eq!(rects[0].bottom, 100.0);
    }

    /// The whole reason `rects` is a list: one box around a wrapped match would
    /// paint over every line between its two halves.
    #[test]
    fn a_match_across_a_line_break_gets_one_box_per_line() {
        let chars = boxes(&[
            (400.0, 100.0, 406.0, 112.0),
            (406.0, 100.0, 412.0, 112.0),
            (10.0, 80.0, 16.0, 92.0),
            (16.0, 80.0, 22.0, 92.0),
        ]);
        let rects = rects_for_range(&chars, 0, 4);
        assert_eq!(rects.len(), 2, "dua baris, dua kotak");
        assert_eq!(rects[0].bottom, 100.0);
        assert_eq!(rects[1].bottom, 80.0);
    }

    #[test]
    fn empty_boxes_are_skipped_rather_than_merged() {
        let chars = boxes(&[
            (10.0, 100.0, 16.0, 112.0),
            (0.0, 0.0, 0.0, 0.0),
            (16.0, 100.0, 22.0, 112.0),
        ]);
        let rects = rects_for_range(&chars, 0, 3);
        assert_eq!(
            rects.len(),
            1,
            "kotak kosong tidak menarik apa pun ke origin"
        );
        assert_eq!(rects[0].left, 10.0);
    }

    #[test]
    fn a_range_outside_the_page_yields_nothing() {
        let chars = boxes(&[(10.0, 100.0, 16.0, 112.0)]);
        assert!(rects_for_range(&chars, 5, 3).is_empty());
    }

    /// Character offsets, not byte offsets: the boxes are indexed by character,
    /// and a page with one "×" on it would misalign every later highlight.
    #[test]
    fn hits_are_counted_in_characters_not_bytes() {
        let re = compile("dunia", false).expect("pola sah");
        let hits = regex_hits("halo × dunia", &re, 10);
        assert_eq!(hits, vec![(7, 5)]);
        let text = "halo × dunia";
        let chars: Vec<char> = text.chars().collect();
        let word: String = chars.iter().skip(7).take(5).collect();
        assert_eq!(word, "dunia", "offset menunjuk ke kata yang benar");
    }

    #[test]
    fn a_pattern_that_matches_nothing_is_not_an_error() {
        let re = compile(r"\d{4}", false).expect("pola sah");
        assert!(regex_hits("tanpa angka di sini", &re, 10).is_empty());
    }

    #[test]
    fn empty_matches_are_dropped() {
        let re = compile("a*", false).expect("pola sah");
        let hits = regex_hits("bbb", &re, 100);
        assert!(hits.is_empty(), "kecocokan kosong tidak menyoroti apa pun");
    }

    #[test]
    fn the_hit_count_is_capped() {
        let re = compile(".", false).expect("pola sah");
        let text = "x".repeat(1000);
        assert_eq!(regex_hits(&text, &re, 20).len(), 20);
    }

    #[test]
    fn a_broken_pattern_is_reported_not_panicked() {
        let err = compile("(unclosed", false).expect_err("harus gagal");
        assert!(err.contains("pola tidak sah"), "{err}");
    }

    #[test]
    fn case_folding_follows_the_toggle() {
        let sensitive = compile("Bab", true).expect("pola sah");
        assert!(regex_hits("bab dan Bab", &sensitive, 10).len() == 1);
        let insensitive = compile("Bab", false).expect("pola sah");
        assert!(regex_hits("bab dan Bab", &insensitive, 10).len() == 2);
    }
}
