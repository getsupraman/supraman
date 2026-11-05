//! The changed words inside a hunk: a character diff of its two sides, each changed range widened to the
//! words it cuts into, ranges a blank or one character apart joined, then cut at line ends.

use std::ops::Range;

use crate::Hunk;

/// Hunks whose two sides together are longer than this get line backgrounds only.
pub const MAX_HUNK_BYTES: usize = 16 * 1024;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn before(text: &str, at: usize) -> Option<char> {
    text[..at].chars().next_back()
}

fn after(text: &str, at: usize) -> Option<char> {
    text[at..].chars().next()
}

fn inside_word(text: &str, at: usize) -> bool {
    before(text, at).is_some_and(is_word) && after(text, at).is_some_and(is_word)
}

/// `range` grown to the whole words its ends fall inside.
fn widen(text: &str, range: Range<usize>) -> Range<usize> {
    let (mut start, mut end) = (range.start, range.end);
    if inside_word(text, start) {
        while let Some(c) = before(text, start).filter(|c| is_word(*c)) {
            start -= c.len_utf8();
        }
    }
    if inside_word(text, end) {
        while let Some(c) = after(text, end).filter(|c| is_word(*c)) {
            end += c.len_utf8();
        }
    }
    start..end
}

/// Whether the text between two ranges is small enough to mark with them.
fn joins(gap: &str) -> bool {
    !gap.contains('\n') && (gap.chars().all(char::is_whitespace) || gap.chars().nth(1).is_none())
}

/// `ranges` (sorted) widened, joined and cut at line ends; empty ranges dropped.
fn shape(text: &str, ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut joined: Vec<Range<usize>> = Vec::new();
    for range in ranges.into_iter().map(|r| widen(text, r)).filter(|r| !r.is_empty()) {
        match joined.last_mut() {
            Some(last) if range.start <= last.end || joins(&text[last.end..range.start]) => last.end = last.end.max(range.end),
            _ => joined.push(range),
        }
    }
    joined.into_iter().flat_map(|r| lines_of(text, r)).collect()
}

fn lines_of(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = range.start;
    for (i, _) in text[range.clone()].match_indices('\n') {
        out.push(start..range.start + i);
        start = range.start + i + 1;
    }
    out.push(start..range.end);
    out.into_iter()
        .map(|r| if text[r.clone()].ends_with('\r') { r.start..r.end - 1 } else { r })
        .filter(|r| !r.is_empty())
        .collect()
}

/// The changed words of `old` and of `new`, as byte ranges of each.
pub fn word_ranges(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    if old.is_empty() || new.is_empty() || old.len() + new.len() > MAX_HUNK_BYTES {
        return (Vec::new(), Vec::new());
    }
    let (mut olds, mut news) = (Vec::new(), Vec::new());
    let mut shift = 0isize;
    for (range, replacement) in language::char_diff(old, new) {
        let start = range.start.saturating_add_signed(shift);
        shift += replacement.len() as isize - range.len() as isize;
        news.push(start..start + replacement.len());
        olds.push(range);
    }
    (shape(old, olds), shape(new, news))
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect()
}

fn bytes(starts: &[usize], len: usize, rows: &Range<u32>) -> Range<usize> {
    let at = |row: u32| starts.get(row as usize).copied().unwrap_or(len);
    at(rows.start)..at(rows.end)
}

/// `hunks` with their changed words, by [`word_ranges`] of each hunk's rows in `old` and `new`.
pub fn with_words(old: &str, new: &str, mut hunks: Vec<Hunk>) -> Vec<Hunk> {
    let (old_starts, new_starts) = (line_starts(old), line_starts(new));
    for h in &mut hunks {
        let o = bytes(&old_starts, old.len(), &h.old_rows);
        let n = bytes(&new_starts, new.len(), &h.new_rows);
        let (ow, nw) = word_ranges(&old[o.clone()], &new[n.clone()]);
        h.old_words = ow.into_iter().map(|r| o.start + r.start..o.start + r.end).collect();
        h.new_words = nw.into_iter().map(|r| n.start + r.start..n.start + r.end).collect();
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked<'a>(text: &'a str, ranges: &[Range<usize>]) -> Vec<&'a str> {
        ranges.iter().map(|r| &text[r.clone()]).collect()
    }

    #[test]
    fn a_changed_word_is_marked_on_both_sides() {
        let (old, new) = ("let x = 1;\n", "let x = 2;\n");
        let (o, n) = word_ranges(old, new);
        assert_eq!((o, n), (vec![8..9], vec![8..9]));
    }

    #[test]
    fn fragments_inside_a_word_join_into_the_word() {
        let (old, new) = ("    await slot.ReacquireAsync(Aborted);\n", "    await slot.EndWaitAsync(Aborted);\n");
        let (o, n) = word_ranges(old, new);
        assert_eq!((marked(old, &o), marked(new, &n)), (vec!["ReacquireAsync"], vec!["EndWaitAsync"]));
        let (old, new) = ("slot.Release();\n", "slot.BeginWait();\n");
        let (o, n) = word_ranges(old, new);
        assert_eq!((marked(old, &o), marked(new, &n)), (vec!["Release"], vec!["BeginWait"]));
    }

    #[test]
    fn words_a_blank_apart_join_and_lines_stay_apart() {
        let (old, new) = ("a = red fox;\nkeep\nb = 1;\n", "a = big cat;\nkeep\nb = 22;\n");
        let (o, n) = word_ranges(old, new);
        assert_eq!(marked(old, &o), ["red fox", "1"]);
        assert_eq!(marked(new, &n), ["big cat", "22"]);
    }

    #[test]
    fn an_insertion_at_the_end_marks_only_the_new_side() {
        let (old, new) = ("f(a)\n", "f(a, b)\n");
        let (o, n) = word_ranges(old, new);
        assert_eq!((marked(old, &o), marked(new, &n)), (vec![], vec![", b"]));
    }

    #[test]
    fn whole_added_or_removed_rows_have_no_words() {
        assert_eq!(word_ranges("", "x\n"), (vec![], vec![]));
        assert_eq!(word_ranges("x\n", ""), (vec![], vec![]));
    }

    #[test]
    fn hunk_rows_map_to_their_bytes() {
        let hunk = Hunk { old_rows: 1..2, new_rows: 1..2, old_words: Vec::new(), new_words: Vec::new() };
        let (old, new) = ("a\nlet x = 1;\nc\n", "a\nlet x = 2;\nc\n");
        let h = &with_words(old, new, vec![hunk])[0];
        assert_eq!((marked(old, &h.old_words), marked(new, &h.new_words)), (vec!["1"], vec!["2"]));
    }
}
