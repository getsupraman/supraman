//! Paths with spaces, which the path regexes cut at the first space: a quoted path whole, and an absolute
//! Windows path through its last word with a separator. The click trims the second to what exists.

use std::ops::Range;

/// Characters no Windows path holds: an unquoted path ends before one.
const NOT_IN_PATH: [char; 6] = ['<', '>', '"', '|', '*', '\t'];

/// The byte range in `line` of a path with a space that holds the byte `hovered`, its `:line:column` kept.
pub(crate) fn at(line: &str, hovered: usize) -> Option<Range<usize>> {
    quoted(line, hovered).or_else(|| unquoted(line, hovered)).filter(|r| r.contains(&hovered))
}

fn has_separator(s: &str) -> bool {
    s.contains(['\\', '/'])
}

fn edge(c: Option<char>) -> bool {
    c.is_none_or(|c| !c.is_alphanumeric())
}

fn quoted(line: &str, hovered: usize) -> Option<Range<usize>> {
    ['"', '\''].into_iter().find_map(|q| {
        let open = line[..hovered].rfind(q)?;
        let close = hovered + line[hovered..].find(q)?;
        let content = &line[open + 1..close];
        let after = &line[close + 1..];
        let bounded = edge(line[..open].chars().next_back()) && edge(after.chars().next());
        // `File "…", line N` is the first regex's
        let python = q == '"' && after.starts_with(", line ");
        let trimmed = !content.starts_with(|c: char| c.is_whitespace() || "([{<".contains(c)) && !content.ends_with(char::is_whitespace);
        (bounded && !python && trimmed && has_separator(content)).then_some(open + 1..close)
    })
}

fn is_start(line: &str, i: usize) -> bool {
    let b = &line.as_bytes()[i..];
    let before = line[..i].chars().next_back();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/');
    (drive || b.starts_with(br"\\")) && edge(before) && !matches!(before, Some('\\' | '"' | '\''))
}

fn unquoted(line: &str, hovered: usize) -> Option<Range<usize>> {
    let mut from = 0;
    while let Some(start) = line[from..].char_indices().map(|(i, _)| from + i).find(|&i| is_start(line, i)) {
        if start > hovered {
            return None;
        }
        let end = extent(line, start);
        if hovered < end {
            return line[start..end].contains(' ').then_some(start..end);
        }
        from = end.max(start + 1);
        while !line.is_char_boundary(from) {
            from += 1;
        }
    }
    None
}

/// Where the path from `start` ends: the end of its body's last word with a separator, trailing punctuation off.
fn extent(line: &str, start: usize) -> usize {
    let body_end = line[start..]
        .char_indices()
        .map(|(i, c)| (start + i, c))
        .find(|&(i, c)| NOT_IN_PATH.contains(&c) || (i > start && line[..i].ends_with(' ') && (is_start(line, i) || matches!(c, '\\' | '/'))))
        .map_or(line.len(), |(i, _)| i);
    let mut end = start;
    let mut offset = start;
    for word in line[start..body_end].split(' ') {
        if has_separator(word) {
            end = offset + word.len();
        }
        offset += word.len() + 1;
    }
    loop {
        let path = &line[start..end];
        let unbalanced = path.matches(')').count() > path.matches('(').count();
        match path.chars().next_back() {
            Some('.' | ',' | ';' | ':' | '!' | '?' | '\'') => end -= 1,
            Some(')') if unbalanced => end -= 1,
            _ => return end,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::at;

    fn link<'a>(line: &'a str, hovered: &str) -> Option<&'a str> {
        let at_byte = line.find(hovered).unwrap();
        at(line, at_byte).map(|r| &line[r])
    }

    #[test]
    fn a_quoted_path_is_one_link_and_prose_or_a_bracket_is_not() {
        assert_eq!(link(r#"open "C:\Program Files\x\y.rs:12:3" now"#, "Files"), Some(r"C:\Program Files\x\y.rs:12:3"));
        assert_eq!(link(r"cd 'C:\Program Files (x86)'", "x86"), Some(r"C:\Program Files (x86)"));
        assert_eq!(link(r#""\\server\my share\x""#, "share"), Some(r"\\server\my share\x"));
        assert_eq!(link(r#""a b" and "c d""#, "and"), None, "between two quotes");
        assert_eq!(link(r"it's in C:\x\y, isn't it", "x"), None, "apostrophes in words are no quotes");
        assert_eq!(link(r#"File "C:\a b\x.py", line 4"#, "b"), None, "Python's form is the regex's");
        assert_eq!(link(r#""just words""#, "words"), None);
    }

    #[test]
    fn an_unquoted_absolute_path_runs_to_its_last_word_with_a_separator() {
        assert_eq!(link(r"C:\Tools\Screaming Frog SEO Spider\", "Frog"), Some(r"C:\Tools\Screaming Frog SEO Spider\"));
        assert_eq!(link(r"see C:\a b\c.txt for details", "c.txt"), Some(r"C:\a b\c.txt"));
        assert_eq!(link(r"see C:\a b\c.txt.", "a"), Some(r"C:\a b\c.txt"));
        assert_eq!(link(r"PS C:\Users\some one\x> dir", "one"), Some(r"C:\Users\some one\x"));
        assert_eq!(link(r"C:\a b\x.rs C:\c d\y.rs", "d"), Some(r"C:\c d\y.rs"), "the next path starts its own");
        assert_eq!(link(r"C:\a b\x.rs \c\y.rs", "b"), Some(r"C:\a b\x.rs"), "so does a rooted one");
        assert_eq!(link(r"see C:\a b\c.txt for details", "details"), None);
        assert_eq!(link(r"C:\x\y.rs built in 3 s", "y.rs"), None, "no space: the regex's");
        assert_eq!(link(r"error: C:\x\y.rs:12:3: expected `;`", "y.rs"), None);
    }
}
