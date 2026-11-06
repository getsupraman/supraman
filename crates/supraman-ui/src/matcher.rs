//! Case-insensitive matching (substring anywhere, prefix from the start) and the pieces of a results
//! list: a label with its matches highlighted and a count badge; [`subsequence`] for a fuzzy match.

use std::ops::Range;

use gpui::{App, Div, FontWeight, HighlightStyle, SharedString, Stateful, StyledText, div, prelude::*, px};
use theme::ActiveTheme as _;

fn same(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Every place `needle` occurs in `hay`, ignoring case, as byte ranges of `hay`, not overlapping.
pub fn find_all(hay: &str, needle: &str) -> Vec<Range<usize>> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let chars: Vec<(usize, char)> = hay.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        if chars[i..i + needle.len()].iter().zip(&needle).all(|((_, h), n)| same(*h, *n)) {
            let end = chars.get(i + needle.len()).map_or(hay.len(), |(at, _)| *at);
            out.push(chars[i].0..end);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// `needle`'s characters in order anywhere in `hay`, ignoring case, each taken at its first chance: the
/// matched byte ranges of `hay`, neighbours joined; `None` when they do not all occur, empty for an empty needle.
pub fn subsequence(hay: &str, needle: &str) -> Option<Vec<Range<usize>>> {
    let mut wanted = needle.chars().peekable();
    let mut out: Vec<Range<usize>> = Vec::new();
    for (at, c) in hay.char_indices() {
        let Some(&n) = wanted.peek() else { break };
        if !same(c, n) {
            continue;
        }
        wanted.next();
        let end = at + c.len_utf8();
        match out.last_mut() {
            Some(last) if last.end == at => last.end = end,
            _ => out.push(at..end),
        }
    }
    wanted.peek().is_none().then_some(out)
}

/// Whether `hay` starts with `needle`, ignoring ASCII case; an empty needle never matches.
pub fn prefix(hay: &str, needle: &str) -> bool {
    !needle.is_empty() && hay.len() >= needle.len() && hay.is_char_boundary(needle.len()) && hay[..needle.len()].eq_ignore_ascii_case(needle)
}

/// `text` on one line, cut with an ellipsis, `marks` (byte ranges) in the search match colours.
pub fn highlighted(key: impl Into<SharedString>, text: impl Into<SharedString>, marks: &[Range<usize>], cx: &App) -> Stateful<Div> {
    let style = HighlightStyle {
        background_color: Some(cx.theme().colors().search_match_background),
        font_weight: Some(FontWeight::BOLD),
        ..Default::default()
    };
    div()
        .id(key.into())
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(StyledText::new(text.into()).with_highlights(marks.iter().map(|r| (r.clone(), style))))
}

/// A result group's count badge.
pub fn count(n: usize, cx: &App) -> Div {
    let colors = cx.theme().colors();
    div()
        .flex_none()
        .min_w(px(18.))
        .px_1()
        .rounded_full()
        .bg(colors.element_background)
        .text_xs()
        .text_center()
        .text_color(colors.text)
        .child(SharedString::from(n.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_all_ignores_case_and_returns_byte_ranges() {
        assert_eq!(find_all("CI-CD-Pipeline", "pipe"), [6..10]);
        assert_eq!(find_all("aAa", "a"), [0..1, 1..2, 2..3]);
        assert_eq!(find_all("Grüße GRÜSSE", "grü"), [0..4, 8..12]);
        assert_eq!(find_all("abc", ""), Vec::<Range<usize>>::new());
        assert_eq!(find_all("ab", "abc"), Vec::<Range<usize>>::new());
    }

    #[test]
    fn a_subsequence_matches_in_order_and_joins_neighbours() {
        assert_eq!(subsequence("feature/Login-page", "flp"), Some(vec![0..1, 8..9, 14..15]));
        assert_eq!(subsequence("origin/main", "MAIN"), Some(vec![7..11]));
        assert_eq!(subsequence("Grüße", "gü"), Some(vec![0..1, 2..4]));
        assert_eq!(subsequence("main", "nm"), None, "out of order");
        assert_eq!(subsequence("abc", ""), Some(Vec::new()));
    }

    #[test]
    fn a_prefix_matches_from_the_start_only() {
        assert!(prefix("5e1f7e57", "5E1F"));
        assert!(!prefix("5e1f7e57", "7e57"));
        assert!(!prefix("5e1f", ""));
        assert!(!prefix("5e", "5e1f"));
        assert!(!prefix("éa", "e"), "not a char boundary");
    }
}
