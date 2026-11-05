use std::borrow::Cow;

/// Whole capture names a grammar's own query uses where Zed's themes have another name.
const WHOLE: &[(&str, &str)] = &[
    ("text.title", "title"),
    ("markup.heading", "title"),
    ("text.uri", "link_uri"),
    ("markup.link.url", "link_uri"),
    ("text.reference", "link_text"),
    ("markup.link.label", "link_text"),
    ("markup.raw", "text.literal"),
    ("markup.italic", "emphasis"),
    ("text.emphasis", "emphasis"),
    ("markup.bold", "emphasis.strong"),
    ("markup.strong", "emphasis.strong"),
    ("text.strong", "emphasis.strong"),
    ("symbol", "string.special.symbol"),
    ("constant.numeric", "number"),
    ("constant.character.escape", "string.escape"),
];

/// A capture name's first part a grammar's own query uses where Zed's themes have another.
const FIRST: &[(&str, &str)] = &[
    ("include", "keyword"),
    ("conditional", "keyword"),
    ("repeat", "keyword"),
    ("exception", "keyword"),
    ("storageclass", "keyword"),
    ("define", "preproc"),
    ("macro", "preproc"),
    ("float", "number"),
    ("character", "string"),
    ("escape", "string.escape"),
    ("field", "property"),
    ("parameter", "variable.parameter"),
    ("method", "function.method"),
    ("delimiter", "punctuation.delimiter"),
    ("annotation", "attribute"),
    ("decorator", "attribute"),
    ("markup", "text"),
];

/// The capture name Zed's themes colour for `name`, a grammar's own.
pub(crate) fn themed_name(name: &str) -> Cow<'_, str> {
    if let Some((_, to)) = WHOLE.iter().find(|(from, _)| *from == name) {
        return Cow::Borrowed(to);
    }
    let (first, rest) = name.split_once('.').map_or((name, None), |(f, r)| (f, Some(r)));
    match FIRST.iter().find(|(from, _)| *from == first) {
        Some((_, to)) => Cow::Owned(rest.map_or_else(|| to.to_string(), |rest| format!("{to}.{rest}"))),
        None => Cow::Borrowed(name),
    }
}

/// `query` with every capture renamed by `themed_name`; strings and comments stay as they are.
pub(crate) fn themed(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    let mut chars = query.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        out.push(c);
        match c {
            '"' => copy_string(&mut chars, &mut out),
            ';' => copy_line(&mut chars, &mut out),
            '@' => {
                let start = at + 1;
                let mut end = start;
                while let Some(&(i, c)) = chars.peek() {
                    if !(c.is_alphanumeric() || matches!(c, '_' | '.' | '-')) {
                        break;
                    }
                    end = i + c.len_utf8();
                    chars.next();
                }
                out.push_str(&themed_name(&query[start..end]));
            }
            _ => {}
        }
    }
    out
}

fn copy_string(chars: &mut std::iter::Peekable<std::str::CharIndices>, out: &mut String) {
    while let Some((_, c)) = chars.next() {
        out.push(c);
        match c {
            '\\' => out.extend(chars.next().map(|(_, c)| c)),
            '"' => return,
            _ => {}
        }
    }
}

fn copy_line(chars: &mut std::iter::Peekable<std::str::CharIndices>, out: &mut String) {
    for (_, c) in chars.by_ref() {
        out.push(c);
        if c == '\n' {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_take_the_names_zeds_themes_colour() {
        assert_eq!(themed_name("keyword.function"), "keyword.function");
        assert_eq!(themed_name("include"), "keyword");
        assert_eq!(themed_name("parameter.reference"), "variable.parameter.reference");
        assert_eq!(themed_name("markup.heading"), "title");
        assert_eq!(themed_name("_private"), "_private");
        let query = "(a) @include\n; a comment @float\n(\"@field\" @field)\n";
        assert_eq!(themed(query), "(a) @keyword\n; a comment @float\n(\"@field\" @property)\n");
    }
}
