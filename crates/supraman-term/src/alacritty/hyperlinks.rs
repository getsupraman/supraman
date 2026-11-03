// Vendored from Zed crates/terminal/src/alacritty/hyperlinks.rs @ 03efd49220ba06703d04f9c4732fdf814f9c523c
// (GPL-3.0-or-later).

use alacritty_terminal::{
    Term,
    event::EventListener,
    grid::Dimensions,
    index::{Boundary, Column, Direction as AlacDirection, Point as AlacPoint},
    term::{
        cell::Flags,
        search::{Match, RegexIter, RegexSearch},
    },
};
use log::{info, warn};
use regex::Regex;
use std::{
    ops::{Index, Range as StdRange},
    time::{Duration, Instant},
};
use url::Url;
use util::paths::{PathStyle, UrlExt};

use crate::Range;

const URL_REGEX: &str = r#"(ipfs:|ipns:|magnet:|mailto:|gemini://|gopher://|https://|http://|news:|file://|git://|ssh:|ftp://|zed://)[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>"\s{-}\^⟨⟩`']+"#;
const WIDE_CHAR_SPACERS: Flags =
    Flags::from_bits(Flags::LEADING_WIDE_CHAR_SPACER.bits() | Flags::WIDE_CHAR_SPACER.bits())
        .unwrap();

pub(crate) struct RegexSearches {
    url_regex: Option<RegexSearch>,
    path_hyperlink_regexes: Vec<Regex>,
    path_hyperlink_timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HyperlinkMatch {
    pub(crate) text: String,
    pub(crate) is_url: bool,
    pub(crate) range: Range,
}

impl From<(String, bool, Match)> for HyperlinkMatch {
    fn from((text, is_url, range): (String, bool, Match)) -> Self {
        Self {
            text,
            is_url,
            range: Range::from_alacritty(range),
        }
    }
}

impl Default for RegexSearches {
    fn default() -> Self {
        Self::new(Vec::<String>::new(), Duration::ZERO)
    }
}

impl RegexSearches {
    pub fn new(
        path_hyperlink_regexes: impl IntoIterator<Item: AsRef<str>>,
        path_hyperlink_timeout: Duration,
    ) -> Self {
        Self {
            url_regex: RegexSearch::new(URL_REGEX).ok(),
            path_hyperlink_regexes: Self::path_hyperlink_regexes(path_hyperlink_regexes),
            path_hyperlink_timeout,
        }
    }

    fn path_hyperlink_regexes(
        path_hyperlink_regexes: impl IntoIterator<Item: AsRef<str>>,
    ) -> Vec<Regex> {
        path_hyperlink_regexes
            .into_iter()
            .filter_map(|regex| {
                Regex::new(regex.as_ref())
                    .inspect_err(|error| {
                        warn!(
                            concat!(
                                "Ignoring path hyperlink regex specified in ",
                                "`terminal.path_hyperlink_regexes`:\n\n\t{}\n\nError: {}",
                            ),
                            regex.as_ref(),
                            error
                        );
                    })
                    .ok()
            })
            .collect()
    }
}

pub(crate) fn find_from_grid_point<T: EventListener>(
    term: &Term<T>,
    point: AlacPoint,
    regex_searches: &mut RegexSearches,
    path_style: PathStyle,
) -> Option<HyperlinkMatch> {
    let grid = term.grid();
    let link = grid.index(point).hyperlink();
    let found_word = if let Some(ref url) = link {
        let mut min_index = point;
        loop {
            let new_min_index = min_index.sub(term, Boundary::Cursor, 1);
            if new_min_index == min_index || grid.index(new_min_index).hyperlink() != link {
                break;
            } else {
                min_index = new_min_index
            }
        }

        let mut max_index = point;
        loop {
            let new_max_index = max_index.add(term, Boundary::Cursor, 1);
            if new_max_index == max_index || grid.index(new_max_index).hyperlink() != link {
                break;
            } else {
                max_index = new_max_index
            }
        }

        let url = url.uri().to_owned();
        let url_match = min_index..=max_index;

        Some((url, true, url_match))
    } else {
        let (line_start, line_end) = (term.line_search_left(point), term.line_search_right(point));
        let url_match = regex_searches.url_regex.as_mut().and_then(|url_regex| {
            RegexIter::new(line_start, line_end, AlacDirection::Right, term, url_regex)
                .find(|rm| rm.contains(&point))
                .map(|url_match| {
                    let url = term.bounds_to_string(*url_match.start(), *url_match.end());
                    sanitize_url_punctuation(url, url_match, term)
                })
        });

        if let Some((url, url_match)) = url_match {
            Some((url, true, url_match))
        } else {
            path_match(
                &term,
                line_start,
                line_end,
                point,
                &mut regex_searches.path_hyperlink_regexes,
                regex_searches.path_hyperlink_timeout,
            )
            .map(|(path, path_match)| (path, false, path_match))
        }
    };

    found_word.map(|found_word| normalize_found_word(found_word, path_style))
}

fn normalize_found_word(
    found_word: (String, bool, Match),
    path_style: PathStyle,
) -> HyperlinkMatch {
    let (maybe_url_or_path, is_url, word_match) = found_word;
    normalize_hyperlink_match(
        maybe_url_or_path,
        is_url,
        Range::from_alacritty(word_match),
        path_style,
    )
}

fn normalize_hyperlink_match(
    maybe_url_or_path: String,
    is_url: bool,
    range: Range,
    path_style: PathStyle,
) -> HyperlinkMatch {
    if is_url {
        // file:// IRIs are paths, so a trailing line number parses; `to_file_path` handles drive letters.
        if maybe_url_or_path.starts_with("file://") {
            if let Ok(url) = Url::parse(&maybe_url_or_path)
                && let Ok(path) = url.to_file_path_ext(path_style)
            {
                return HyperlinkMatch {
                    text: path.to_string_lossy().into_owned(),
                    is_url: false,
                    range,
                };
            }
            let path = maybe_url_or_path
                .strip_prefix("file://")
                .unwrap_or(&maybe_url_or_path);
            HyperlinkMatch {
                text: path.to_string(),
                is_url: false,
                range,
            }
        } else {
            HyperlinkMatch {
                text: maybe_url_or_path,
                is_url: true,
                range,
            }
        }
    } else {
        HyperlinkMatch {
            text: maybe_url_or_path,
            is_url: false,
            range,
        }
    }
}

fn sanitize_url_punctuation<T: EventListener>(
    url: String,
    url_match: Match,
    term: &Term<T>,
) -> (String, Match) {
    let mut sanitized_url = url;
    let mut chars_trimmed = 0;

    let (open_parens, mut close_parens) =
        sanitized_url
            .chars()
            .fold((0, 0), |(opens, closes), c| match c {
                '(' => (opens + 1, closes),
                ')' => (opens, closes + 1),
                _ => (opens, closes),
            });

    while let Some(last_char) = sanitized_url.chars().last() {
        let should_remove = match last_char {
            // Valid in a URL, but in plain text usually a delimiter after it.
            '.' | ',' | ':' | ';' => true,
            '(' => true,
            ')' if close_parens > open_parens => {
                close_parens -= 1;

                true
            }
            _ => false,
        };

        if should_remove {
            sanitized_url.pop();
            chars_trimmed += 1;
        } else {
            break;
        }
    }

    if chars_trimmed > 0 {
        let new_end = url_match.end().sub(term, Boundary::Grid, chars_trimmed);
        let sanitized_match = Match::new(*url_match.start(), new_end);
        (sanitized_url, sanitized_match)
    } else {
        (sanitized_url, url_match)
    }
}

fn first_unbalanced_open_paren(s: &str) -> Option<usize> {
    let mut balance: i32 = 0;
    let mut first_unmatched = None;
    for (i, c) in s.char_indices() {
        match c {
            '(' => {
                if balance == 0 {
                    first_unmatched = Some(i + c.len_utf8());
                }
                balance += 1;
            }
            ')' => {
                balance -= 1;
                if balance <= 0 {
                    balance = 0;
                    first_unmatched = None;
                }
            }
            _ => {}
        }
    }
    first_unmatched.filter(|_| balance > 0)
}

fn path_match<T>(
    term: &Term<T>,
    line_start: AlacPoint,
    line_end: AlacPoint,
    hovered: AlacPoint,
    path_hyperlink_regexes: &mut Vec<Regex>,
    path_hyperlink_timeout: Duration,
) -> Option<(String, Match)> {
    if path_hyperlink_regexes.is_empty() || path_hyperlink_timeout.as_millis() == 0 {
        return None;
    }
    debug_assert!(line_start <= hovered);
    debug_assert!(line_end >= hovered);
    let search_start_time = Instant::now();

    let timed_out = || {
        let elapsed_time = Instant::now().saturating_duration_since(search_start_time);
        (elapsed_time > path_hyperlink_timeout)
            .then_some((elapsed_time.as_millis(), path_hyperlink_timeout.as_millis()))
    };

    // Cell-accurate, unlike `bounds_to_string`, which compresses a tab into one space.
    let mut line = String::with_capacity(
        (line_end.line.0 - line_start.line.0 + 1) as usize * term.grid().columns(),
    );
    let first_cell = &term.grid()[line_start];
    let mut prev_len = 0;
    line.push(first_cell.c);
    let mut hovered_point_byte_offset = None;

    if line_start == hovered {
        hovered_point_byte_offset = Some(0);
    }

    for cell in term.grid().iter_from(line_start) {
        if cell.point > line_end {
            break;
        }

        if !cell.flags.intersects(WIDE_CHAR_SPACERS) {
            prev_len = line.len();
            match cell.c {
                ' ' | '\t' => line.push(' '),
                c => line.push(c),
            }
        }

        if cell.point == hovered {
            debug_assert!(hovered_point_byte_offset.is_none());
            hovered_point_byte_offset = Some(prev_len);
        }
    }
    let line = line.trim_ascii_end();
    let hovered_point_byte_offset = hovered_point_byte_offset?;
    if line.len() <= hovered_point_byte_offset {
        return None;
    }
    let found_from_range = |path_range: StdRange<usize>,
                            link_range: StdRange<usize>,
                            position: Option<(u32, Option<u32>)>| {
        let advance_point_by_str = |mut point: AlacPoint, s: &str| {
            for _ in s.chars() {
                point = term
                    .expand_wide(point, AlacDirection::Right)
                    .add(term, Boundary::Grid, 1);
            }

            // alacritty has no "move to the start of the current wide char".
            let flags = term.grid().index(point).flags;
            if flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
                AlacPoint::new(point.line + 1, Column(0))
            } else if flags.contains(Flags::WIDE_CHAR_SPACER) {
                AlacPoint::new(point.line, point.column - 1)
            } else {
                point
            }
        };

        let link_start = advance_point_by_str(line_start, &line[..link_range.start]);
        let link_end = advance_point_by_str(link_start, &line[link_range]);
        let link_match = link_start
            ..=term
                .expand_wide(link_end, AlacDirection::Left)
                .sub(term, Boundary::Grid, 1);

        (
            {
                let mut path = line[path_range].to_string();
                position.inspect(|(line, column)| {
                    path += &format!(":{line}");
                    column.inspect(|column| path += &format!(":{column}"));
                });
                path
            },
            link_match,
        )
    };

    for regex in path_hyperlink_regexes {
        let mut path_found = false;

        for (line_start_offset, captures) in regex
            .captures_iter(&line)
            .map(|captures| (0usize, captures))
        {
            path_found = true;
            let match_range = captures.get(0).unwrap().range();
            let (mut path_range, line_column) = if let Some(path) = captures.name("path") {
                let parse = |name: &str| {
                    captures
                        .name(name)
                        .and_then(|capture| capture.as_str().parse().ok())
                };

                (
                    path.range(),
                    parse("line").map(|line| (line, parse("column"))),
                )
            } else {
                (match_range.clone(), None)
            };
            let mut link_range = captures
                .name("link")
                .map_or_else(|| match_range.clone(), |link| link.range());

            path_range.start += line_start_offset;
            path_range.end += line_start_offset;
            link_range.start += line_start_offset;
            link_range.end += line_start_offset;

            // Strips a delimiter `(` as in `Update(.claude/SKILL.md)`; balanced ones as in `file(copy).txt` stay.
            if let Some(trim) = first_unbalanced_open_paren(&line[path_range.clone()]) {
                path_range.start += trim;
                link_range.start = link_range.start.max(path_range.start);
            }

            if !link_range.contains(&hovered_point_byte_offset) {
                continue;
            }
            let found = found_from_range(path_range, link_range, line_column);

            if found.1.contains(&hovered) {
                return Some(found);
            }
        }

        if path_found {
            return None;
        }

        if let Some((timed_out_ms, timeout_ms)) = timed_out() {
            warn!("Timed out processing path hyperlink regexes after {timed_out_ms}ms");
            info!("{timeout_ms}ms time out specified in `terminal.path_hyperlink_timeout_ms`");
            return None;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::{
        event::VoidListener,
        grid::Dimensions,
        index::{Boundary, Column, Line, Point as AlacPoint},
        term::{Config, cell::Flags, test::TermSize},
        vte::ansi::Handler,
    };
    use regex::Regex;
    use std::{cell::RefCell, ops::RangeInclusive, path::PathBuf};
    use url::Url;
    use util::paths::PathWithPosition;

    fn path_hyperlink_regexes() -> Vec<String> {
        vec![
            r#"File "(?<path>[^"]+)", line (?<line>[0-9]+)"#.to_string(),
            [
                r#"(?x)"#,
                r#"(?<path>"#,
                r#"    ("#,
                r#"        # multi-char path: first char (not opening delimiter, space, or box drawing char)"#,
                r#"        [^({\[<"'`\ \u2500-\u257F]"#,
                r#"        # middle chars: non-space, and colon/paren only if not followed by digit/paren/space"#,
                r#"        ([^\ :(]|[:(][^0-9()\ ])*"#,
                r#"        # last char: not closing delimiter or colon"#,
                r#"        [^()}\]>"'`.,;:\ ]"#,
                r#"    |"#,
                r#"        # single-char path: not delimiter, punctuation, space, or box drawing char"#,
                r#"        [^(){}\[\]<>"'`.,;:\ \u2500-\u257F]"#,
                r#"    )"#,
                r#"    # optional line/column suffix (included in path for PathWithPosition::parse_str)"#,
                r#"    (:+[0-9]+(:[0-9]+)?|:?\([0-9]+([,:]?[0-9]+)?\))?"#,
                r#")"#,
            ]
            .join("\n"),
        ]
    }

    #[test]
    fn test_url_regex() {
        let found = |hay: &str| -> Vec<String> {
            Regex::new(URL_REGEX)
                .unwrap()
                .find_iter(hay)
                .map(|m| m.as_str().to_string())
                .collect()
        };
        assert_eq!(
            found(
                "test http://example.com test 'https://website1.com' test mailto:bob@example.com train"
            ),
            [
                "http://example.com",
                "https://website1.com",
                "mailto:bob@example.com"
            ]
        );
        assert_eq!(
            found("open zed://channel/the-channel and zed://settings/theme now"),
            ["zed://channel/the-channel", "zed://settings/theme"]
        );
    }

    #[test]
    fn test_url_trailing_punctuation_and_parens() {
        let cases = [
            ("https://example.com.", "https://example.com"),
            ("https://example.com/path,", "https://example.com/path"),
            ("https://example.com:", "https://example.com"),
            ("https://example.com;", "https://example.com"),
            ("https://example.com.:;", "https://example.com"),
            (
                "https://en.wikipedia.org/wiki/C.E.O.",
                "https://en.wikipedia.org/wiki/C.E.O",
            ),
            ("https://example.com!.", "https://example.com!"),
            ("https://example.com:8080.", "https://example.com:8080"),
            (
                "https://example.com/path:8080",
                "https://example.com/path:8080",
            ),
            (
                "https://example.com/v1.0/api",
                "https://example.com/v1.0/api",
            ),
            ("https://example.com?a=1&b=2", "https://example.com?a=1&b=2"),
            ("https://example.com/path)", "https://example.com/path"),
            ("https://test.com/))", "https://test.com/"),
            ("https://test.com/(((", "https://test.com/"),
            ("https://test.com/(test)(", "https://test.com/(test)"),
            ("https://example.com/).", "https://example.com/"),
            ("https://example.com/;)", "https://example.com/"),
            (
                "https://en.wikipedia.org/wiki/Example_(disambiguation)",
                "https://en.wikipedia.org/wiki/Example_(disambiguation)",
            ),
            (
                "https://example.com/path(1)(2)",
                "https://example.com/path(1)(2)",
            ),
        ];

        let term = Term::new(Config::default(), &TermSize::new(80, 24), VoidListener);
        for (input, expected) in cases {
            let url_match = Match::new(
                AlacPoint::new(Line(0), Column(0)),
                AlacPoint::new(Line(0), Column(input.len())),
            );
            let (result, _) = sanitize_url_punctuation(input.to_string(), url_match, &term);
            assert_eq!(result, expected, "Failed for input: {}", input);
        }
    }

    macro_rules! test_hyperlink {
        ($($lines:expr),+; $hyperlink_kind:ident) => { {
            use crate::alacritty::hyperlinks::tests::line_cells_count;
            use std::cmp;

            let test_lines = vec![$($lines),+];
            let (total_cells, longest_line_cells) =
                test_lines.iter().copied()
                    .map(line_cells_count)
                    .fold((0, 0), |state, cells| (state.0 + cells, cmp::max(state.1, cells)));
            let contains_tab_char = test_lines.iter().copied()
                .map(str::chars).flatten().find(|&c| c == '\t');
            let columns = if contains_tab_char.is_some() {
                // A tab at a line's end eats the wrap.
                vec![longest_line_cells + 1]
            } else {
                // Alacritty has issues with 2 columns.
                vec![3, longest_line_cells / 2, longest_line_cells + 1]
            };
            test_hyperlink!(
                columns;
                total_cells;
                test_lines.iter().copied();
                $hyperlink_kind
            )
        } };

        ($columns:expr; $total_cells:expr; $lines:expr; $hyperlink_kind:ident) => { {
            use crate::alacritty::hyperlinks::tests::{ test_hyperlink, HyperlinkKind };

            let source_location = format!("{}:{}", std::file!(), std::line!());
            for columns in $columns {
                test_hyperlink(columns, $total_cells, $lines, HyperlinkKind::$hyperlink_kind,
                    &source_location);
            }
        } };
    }

    mod path {
        /// 👉 hovers the next char, 👈 the spacer of the previous wide char; ‹› is the link, «» its path, line and column.
        macro_rules! test_path {
            ($($lines:literal),+) => { test_hyperlink!($($lines),+; Path) };
        }

        #[test]
        fn simple() {
            test_path!("‹«/👉test/cool.rs»›");
            test_path!("‹«/test/cool👉.rs»›");

            test_path!("‹«/👉test/cool.rs»:«4»›");
            test_path!("‹«/test/cool.rs»👉:«4»›");
            test_path!("‹«/test/cool.rs»:«👉4»›");
            test_path!("‹«/test/cool.rs»(«4»👉)›");
            test_path!("‹«/test/cool.rs»:«4»:«👉2»›");
            test_path!("‹«/test/cool.rs»(«4»👉,«2»)›");
            test_path!("‹«/test/cool.rs»:(«4»:«2»👉)›:");
            test_path!("‹«/test/co👉ol.rs»::«42»›");

            test_path!("/test/cool.rs:4:2👉:", "What is this?");
            test_path!("‹«/test/co👉ol.rs»:«4»:«2»›:Error!");

            test_path!("    Compiling Cool 👉(/test/Cool)");
            test_path!("    Compiling Cool (‹«/👉test/Cool»›)");
            test_path!("    Compiling Cool (/test/Cool👉)");

            test_path!("Update👉(src/cool.rs)");
            test_path!("Update(‹«src/👉cool.rs»›)");
            test_path!("Update(src/cool.rs👉)");
            test_path!("Write(‹«/👉test/Cool»›)");

            test_path!("‹«awe👉some.py»›");
            test_path!("‹«👉a»› ");

            test_path!("    ‹F👉ile \"«/awesome.py»\", line «42»›: Wat?");
            test_path!("    ‹File \"«/awesome.py»👉\", line «42»›: Wat?");
            test_path!("    ‹File \"«/awesome.py»\", line «4👉2»›");
        }

        #[test]
        fn multiple_same_line() {
            test_path!("‹«/👉test/cool.rs»› /test/cool.rs");
            test_path!("/test/cool.rs ‹«/👉test/cool.rs»›");
            test_path!(
                "Cargo.toml\t\texperiments\t\t‹«note👉books»›\t\trust-toolchain.toml\ttooling"
            );
        }

        #[test]
        fn quotes_and_brackets() {
            test_path!("\"‹«/test/co👉ol.rs»:«4»›\"");
            test_path!("'‹«/test/co👉ol.rs»:«4»›'");
            test_path!("`‹«/test/co👉ol.rs»:«4»›`");
            test_path!("[‹«/test/co👉ol.rs»:«4»›]");
            test_path!("(‹«/test/co👉ol.rs»:«4»:«2»›)");
            test_path!("{‹«/test/co👉ol.rs»(«4»)›}");
            test_path!("<‹«/test/co👉ol.rs»(«4»,«2»)›>");

            test_path!("[\"‹«/test/co👉ol.rs»:«4»›\"]");
            test_path!("'(‹«/test/co👉ol.rs»:«4»›)'");
            test_path!("([‹«/test/co👉ol.rs»:«4»›] was here...)");
            test_path!("[Here's <‹«/test/co👉ol.rs»:«4»›>]");
        }

        #[test]
        fn trailing_punctuation() {
            test_path!("‹«/test/co👉ol.rs»›:,..");
            test_path!("/test/cool.rs:,👉..");
            test_path!("‹«/test/co👉ol.rs»:«4»›:,");
            test_path!("/test/cool.rs:4:👉,");
            test_path!("[\"‹«/test/co👉ol.rs»:«4»›\"]:,");
            test_path!("'(‹«/test/co👉ol.rs»:«4»›),,'...");
            test_path!("('‹«/test/co👉ol.rs»:«4»›'::: was here...)");
            test_path!("[Here's <‹«/test/co👉ol.rs»:«4»›>]::: ");
            test_path!(
                "‹«test/c👉ontrollers/template_items_controller_test.rb»:«20»›:in 'block (2 levels) in <class:TemplateItemsControllerTest>'"
            );
        }

        #[test]
        fn parens_in_filename() {
            test_path!("‹«docker-compose.prod(👉copy).yml»›");
        }

        #[test]
        fn wide_chars() {
            test_path!("‹«/👉例/cool.rs»›");
            test_path!("‹«/例👈/cool.rs»›");
            test_path!("‹«/例/cool.rs»:«4»:«👉2»›");
            test_path!("    Compiling Cool (‹«/例👈/Cool»›)");
            test_path!("‹«例👈wesome.py»›");
            test_path!("    ‹File \"«/👉例wesome.py»\", line «42»›: Wat?");

            test_path!("‹«test👉、2.txt»›");
            test_path!("‹«test。👈3.txt»›");
            test_path!("‹«/👉🏃/🦀.rs»›");
            test_path!("‹«/🏃/🦀👈.rs»:«4»:«2»›");
            test_path!("‹«/awesome.👉🔥»› is some good Mojo!");
            test_path!("‹«/awesome.🔥👈»› is some good Mojo!");
        }

        #[test]
        fn wrapped_bracketed_path() {
            test_path!("[‹«lib/blitz_apex_👉server/stats/aggregate_rank_stats.ex»:«35»›: BlitzApexServer.Stats.AggregateRankStats.update/2]
                1 #=> 1");
        }

        #[test]
        fn after_a_line_number_prefix() {
            test_path!("0: ‹«foo/👉bar.txt»›");
            test_path!("42: ‹«👉foo/bar.txt»›");
            test_path!("1: ‹«/👉test/cool.rs»:«4»:«2»›");
        }

        #[test]
        fn after_box_drawing() {
            test_path!("─‹«/👉test/cool.rs»:«4»:«2»›");
            test_path!("└──‹«/👉test/cool.rs»:«4»:«2»›");
            test_path!("├─[‹«/👉test/cool.rs»:«4»:«2»›]");
            test_path!("┬‹«/👉test/cool.rs»:«4»:«2»›┬");
        }

        #[test]
        fn windows_prompts_unc_and_verbatim() {
            test_path!(r#"‹«C:\Users\someone\👉test»›>"#);
            test_path!(r#"C:\Users\someone\test👉>"#);
            test_path!(r#"PS ‹«C:\Users\someone\👉test\cool.rs»›>"#);
            test_path!(r#"PS C:\Users\someone\test\cool.rs👉>"#);

            test_path!(r#"‹«\\server\share\👉test\cool.rs»›"#);
            test_path!(r#"‹«\\?\C:\test\cool👉.rs»›"#);
            test_path!(r#"‹«\\?\UNC\server\share\👉test\cool.rs»›"#);
        }
    }

    mod file_iri {
        macro_rules! test_file_iri {
            ($file_iri:literal) => { { test_hyperlink!(concat!("‹«👉", $file_iri, "»›"); FileIri) } };
        }

        #[test]
        fn drive_letter() {
            test_file_iri!("file:///C:/test/cool/index.rs");
            test_file_iri!("file:///C:/test/cool/");
        }

        #[test]
        fn percent_encoded() {
            test_file_iri!("file:///C:/test/%E1%BF%AC%CF%8C%CE%B4%CE%BF%CF%82/");
            test_file_iri!("file:///C:/te%20st/co%20ol/index.rs");
        }
    }

    mod iri {
        macro_rules! test_iri {
            ($iri:literal) => { { test_hyperlink!(concat!("‹«👉", $iri, "»›"); Iri) } };
        }

        #[test]
        fn simple() {
            test_iri!("mailto:someone@somewhere.here");
            test_iri!("http://10.10.10.10:1111/cool.html");
            test_iri!("https://test/cool/index.html?amazing=1#right%20here");
            test_iri!("git://test/cool.git");
            test_iri!("ssh://user@somewhere.over.here:12345/test/cool.git");
            test_iri!("ftp://test/cool.ftp");
        }

        #[test]
        fn wide_chars() {
            test_iri!("https://例🏃🦀/cool/index.html?amazing=1#right%20here");
            test_iri!("ssh://user@somewhere.over.here:12345/例🏃🦀/cool.git");
        }

        #[test]
        fn iris() {
            test_iri!("https://en.wiktionary.org/wiki/Ῥόδος");
            test_iri!("https://en.wiktionary.org/wiki/%E1%BF%AC%CF%8C%CE%B4%CE%BF%CF%82");
        }

        #[test]
        #[should_panic(expected = "Expected a path, but was a iri")]
        fn file_is_a_path() {
            test_iri!("file://test/cool/index.rs");
        }
    }

    #[derive(Debug, PartialEq)]
    enum HyperlinkKind {
        FileIri,
        Iri,
        Path,
    }

    struct ExpectedHyperlink {
        hovered_grid_point: AlacPoint,
        hovered_char: char,
        hyperlink_kind: HyperlinkKind,
        iri_or_path: String,
        row: Option<u32>,
        column: Option<u32>,
        hyperlink_match: RangeInclusive<AlacPoint>,
    }

    /// Converts to Windows style paths on Windows.
    fn build_term_from_test_lines<'a>(
        hyperlink_kind: HyperlinkKind,
        term_size: TermSize,
        test_lines: impl Iterator<Item = &'a str>,
    ) -> (Term<VoidListener>, ExpectedHyperlink) {
        #[derive(Default, Eq, PartialEq)]
        enum HoveredState {
            #[default]
            HoveredScan,
            HoveredNextChar,
            Done,
        }

        #[derive(Default, Eq, PartialEq)]
        enum MatchState {
            #[default]
            MatchScan,
            MatchNextChar,
            Match(AlacPoint),
            Done,
        }

        #[derive(Default, Eq, PartialEq)]
        enum CapturesState {
            #[default]
            PathScan,
            PathNextChar,
            Path(AlacPoint),
            RowScan,
            Row(String),
            ColumnScan,
            Column(String),
            Done,
        }

        fn prev_input_point_from_term(term: &Term<VoidListener>) -> AlacPoint {
            let grid = term.grid();
            let cursor = &grid.cursor;
            let mut point = cursor.point;

            if !cursor.input_needs_wrap {
                point = point.sub(term, Boundary::Grid, 1);
            }

            if grid.index(point).flags.contains(Flags::WIDE_CHAR_SPACER) {
                point.column -= 1;
            }

            point
        }

        fn end_point_from_prev_input_point(
            term: &Term<VoidListener>,
            prev_input_point: AlacPoint,
        ) -> AlacPoint {
            if term
                .grid()
                .index(prev_input_point)
                .flags
                .contains(Flags::WIDE_CHAR)
            {
                prev_input_point.add(term, Boundary::Grid, 1)
            } else {
                prev_input_point
            }
        }

        fn process_input(term: &mut Term<VoidListener>, c: char) {
            match c {
                '\t' => term.put_tab(1),
                c @ _ => term.input(c),
            }
        }

        let mut hovered_grid_point: Option<AlacPoint> = None;
        let mut hyperlink_match = AlacPoint::default()..=AlacPoint::default();
        let mut iri_or_path = String::default();
        let mut row = None;
        let mut column = None;
        let mut prev_input_point = AlacPoint::default();
        let mut hovered_state = HoveredState::default();
        let mut match_state = MatchState::default();
        let mut captures_state = CapturesState::default();
        let mut term = Term::new(Config::default(), &term_size, VoidListener);

        for text in test_lines {
            let chars: Box<dyn Iterator<Item = char>> =
                if cfg!(windows) && hyperlink_kind == HyperlinkKind::Path {
                    Box::new(text.chars().map(|c| if c == '/' { '\\' } else { c })) as _
                } else {
                    Box::new(text.chars()) as _
                };
            let mut chars = chars.peekable();
            while let Some(c) = chars.next() {
                match c {
                    '👉' => {
                        hovered_state = HoveredState::HoveredNextChar;
                    }
                    '👈' => {
                        hovered_grid_point = Some(prev_input_point.add(&term, Boundary::Grid, 1));
                    }
                    '«' | '»' => {
                        captures_state = match captures_state {
                            CapturesState::PathScan => CapturesState::PathNextChar,
                            CapturesState::PathNextChar => {
                                panic!("Should have been handled by char input")
                            }
                            CapturesState::Path(start_point) => {
                                iri_or_path = term.bounds_to_string(
                                    start_point,
                                    end_point_from_prev_input_point(&term, prev_input_point),
                                );
                                CapturesState::RowScan
                            }
                            CapturesState::RowScan => CapturesState::Row(String::new()),
                            CapturesState::Row(number) => {
                                row = Some(number.parse::<u32>().unwrap());
                                CapturesState::ColumnScan
                            }
                            CapturesState::ColumnScan => CapturesState::Column(String::new()),
                            CapturesState::Column(number) => {
                                column = Some(number.parse::<u32>().unwrap());
                                CapturesState::Done
                            }
                            CapturesState::Done => {
                                panic!("Extra '«', '»'")
                            }
                        }
                    }
                    '‹' | '›' => {
                        match_state = match match_state {
                            MatchState::MatchScan => MatchState::MatchNextChar,
                            MatchState::MatchNextChar => {
                                panic!("Should have been handled by char input")
                            }
                            MatchState::Match(start_point) => {
                                hyperlink_match = start_point
                                    ..=end_point_from_prev_input_point(&term, prev_input_point);
                                MatchState::Done
                            }
                            MatchState::Done => {
                                panic!("Extra '‹', '›'")
                            }
                        }
                    }
                    _ => {
                        if let CapturesState::Row(number) | CapturesState::Column(number) =
                            &mut captures_state
                        {
                            number.push(c)
                        }

                        let is_windows_abs_path_start = captures_state
                            == CapturesState::PathNextChar
                            && cfg!(windows)
                            && hyperlink_kind == HyperlinkKind::Path
                            && c == '\\'
                            && chars.peek().is_some_and(|c| *c != '\\');

                        if is_windows_abs_path_start {
                            term.input('C');
                            prev_input_point = prev_input_point_from_term(&term);
                            term.input(':');
                            process_input(&mut term, c);
                        } else {
                            process_input(&mut term, c);
                            prev_input_point = prev_input_point_from_term(&term);
                        }

                        if hovered_state == HoveredState::HoveredNextChar {
                            hovered_grid_point = Some(prev_input_point);
                            hovered_state = HoveredState::Done;
                        }
                        if captures_state == CapturesState::PathNextChar {
                            captures_state = CapturesState::Path(prev_input_point);
                        }
                        if match_state == MatchState::MatchNextChar {
                            match_state = MatchState::Match(prev_input_point);
                        }
                    }
                }
            }
            term.move_down_and_cr(1);
        }

        if hyperlink_kind == HyperlinkKind::FileIri {
            let Ok(url) = Url::parse(&iri_or_path) else {
                panic!("Failed to parse file IRI `{iri_or_path}`");
            };
            let Ok(path) = url.to_file_path() else {
                panic!("Failed to interpret file IRI `{iri_or_path}` as a path");
            };
            iri_or_path = path.to_string_lossy().into_owned();
        }

        let hovered_grid_point = hovered_grid_point.expect("Missing hovered point (👉 or 👈)");
        let hovered_char = term.grid().index(hovered_grid_point).c;
        (
            term,
            ExpectedHyperlink {
                hovered_grid_point,
                hovered_char,
                hyperlink_kind,
                iri_or_path,
                row,
                column,
                hyperlink_match,
            },
        )
    }

    fn line_cells_count(line: &str) -> usize {
        fn width(c: char) -> usize {
            match c {
                '例' | '🏃' | '🦀' | '🔥' => 2,
                '\t' => 8, // it's really 0-8, use the max always
                _ => 1,
            }
        }
        const CONTROL_CHARS: &str = "‹«👉👈»›";
        line.chars()
            .filter(|c| !CONTROL_CHARS.contains(*c))
            .map(width)
            .sum::<usize>()
    }

    struct CheckHyperlinkMatch<'a> {
        term: &'a Term<VoidListener>,
        expected_hyperlink: &'a ExpectedHyperlink,
        source_location: &'a str,
    }

    impl<'a> CheckHyperlinkMatch<'a> {
        fn new(
            term: &'a Term<VoidListener>,
            expected_hyperlink: &'a ExpectedHyperlink,
            source_location: &'a str,
        ) -> Self {
            Self {
                term,
                expected_hyperlink,
                source_location,
            }
        }

        fn check_path_with_position_and_match(
            &self,
            path_with_position: PathWithPosition,
            hyperlink_match: &Match,
        ) {
            let format_path_with_position_and_match =
                |path_with_position: &PathWithPosition, hyperlink_match: &Match| {
                    let mut result =
                        format!("Path = «{}»", path_with_position.path.to_string_lossy());
                    if let Some(row) = path_with_position.row {
                        result += &format!(", line = {row}");
                        if let Some(column) = path_with_position.column {
                            result += &format!(", column = {column}");
                        }
                    }

                    result += &format!(
                        ", at grid cells {}",
                        Self::format_hyperlink_match(hyperlink_match)
                    );
                    result
                };

            assert_ne!(
                self.expected_hyperlink.hyperlink_kind,
                HyperlinkKind::Iri,
                "\n    at {}\nExpected a path, but was a iri:\n{}",
                self.source_location,
                self.format_renderable_content()
            );

            assert_eq!(
                format_path_with_position_and_match(
                    &PathWithPosition {
                        path: PathBuf::from(self.expected_hyperlink.iri_or_path.clone()),
                        row: self.expected_hyperlink.row,
                        column: self.expected_hyperlink.column
                    },
                    &self.expected_hyperlink.hyperlink_match
                ),
                format_path_with_position_and_match(&path_with_position, hyperlink_match),
                "\n    at {}:\n{}",
                self.source_location,
                self.format_renderable_content()
            );
        }

        fn check_iri_and_match(&self, iri: String, hyperlink_match: &Match) {
            let format_iri_and_match = |iri: &String, hyperlink_match: &Match| {
                format!(
                    "Url = «{iri}», at grid cells {}",
                    Self::format_hyperlink_match(hyperlink_match)
                )
            };

            assert_eq!(
                self.expected_hyperlink.hyperlink_kind,
                HyperlinkKind::Iri,
                "\n    at {}\nExpected a iri, but was a path:\n{}",
                self.source_location,
                self.format_renderable_content()
            );

            assert_eq!(
                format_iri_and_match(
                    &self.expected_hyperlink.iri_or_path,
                    &self.expected_hyperlink.hyperlink_match
                ),
                format_iri_and_match(&iri, hyperlink_match),
                "\n    at {}:\n{}",
                self.source_location,
                self.format_renderable_content()
            );
        }

        fn format_hyperlink_match(hyperlink_match: &Match) -> String {
            format!(
                "({}, {})..=({}, {})",
                hyperlink_match.start().line.0,
                hyperlink_match.start().column.0,
                hyperlink_match.end().line.0,
                hyperlink_match.end().column.0
            )
        }

        fn format_renderable_content(&self) -> String {
            let mut result = format!("\nHovered on '{}'\n", self.expected_hyperlink.hovered_char);

            let mut first_header_row = String::new();
            let mut second_header_row = String::new();
            let mut marker_header_row = String::new();
            for index in 0..self.term.columns() {
                let remainder = index % 10;
                if index > 0 && remainder == 0 {
                    first_header_row.push_str(&format!("{:>10}", (index / 10)));
                }
                second_header_row += &remainder.to_string();
                if index == self.expected_hyperlink.hovered_grid_point.column.0 {
                    marker_header_row.push('↓');
                } else {
                    marker_header_row.push(' ');
                }
            }

            let remainder = (self.term.columns() - 1) % 10;
            if remainder != 0 {
                first_header_row.push_str(&" ".repeat(remainder));
            }

            result += &format!("\n      [ {}]\n", first_header_row);
            result += &format!("      [{}]\n", second_header_row);
            result += &format!("       {}", marker_header_row);

            for cell in self
                .term
                .renderable_content()
                .display_iter
                .filter(|cell| !cell.flags.intersects(WIDE_CHAR_SPACERS))
            {
                if cell.point.column.0 == 0 {
                    let prefix =
                        if cell.point.line == self.expected_hyperlink.hovered_grid_point.line {
                            '→'
                        } else {
                            ' '
                        };
                    result += &format!("\n{prefix}[{:>3}] ", cell.point.line.to_string());
                }

                match cell.c {
                    '\t' => result.push(' '),
                    c @ _ => result.push(c),
                }
            }

            result
        }
    }

    fn test_hyperlink<'a>(
        columns: usize,
        total_cells: usize,
        test_lines: impl Iterator<Item = &'a str>,
        hyperlink_kind: HyperlinkKind,
        source_location: &str,
    ) {
        thread_local! {
            static TEST_REGEX_SEARCHES: RefCell<RegexSearches> = RefCell::new(
                RegexSearches::new(path_hyperlink_regexes(), Duration::from_millis(1000)),
            );
        }

        let term_size = TermSize::new(columns, total_cells / columns + 2);
        let (term, expected_hyperlink) =
            build_term_from_test_lines(hyperlink_kind, term_size, test_lines);
        let hyperlink_found = TEST_REGEX_SEARCHES.with(|regex_searches| {
            find_from_grid_point(
                &term,
                expected_hyperlink.hovered_grid_point,
                &mut regex_searches.borrow_mut(),
                PathStyle::local(),
            )
        });
        let check_hyperlink_match =
            CheckHyperlinkMatch::new(&term, &expected_hyperlink, source_location);
        match hyperlink_found {
            Some(hyperlink) if !hyperlink.is_url => {
                let hyperlink_match = hyperlink.range.to_alacritty();
                check_hyperlink_match.check_path_with_position_and_match(
                    PathWithPosition::parse_str(&hyperlink.text),
                    &hyperlink_match,
                );
            }
            Some(hyperlink) => {
                let hyperlink_match = hyperlink.range.to_alacritty();
                check_hyperlink_match.check_iri_and_match(hyperlink.text, &hyperlink_match);
            }
            None => {
                if expected_hyperlink.hyperlink_match.start()
                    != expected_hyperlink.hyperlink_match.end()
                {
                    assert!(
                        false,
                        "No hyperlink found\n     at {source_location}:\n{}",
                        check_hyperlink_match.format_renderable_content()
                    )
                }
            }
        }
    }
}
