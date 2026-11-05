pub(crate) fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The NUL-terminated records of `-z` output; a trailing empty record is dropped.
pub(crate) fn nul_records(out: &[u8]) -> impl Iterator<Item = &[u8]> {
    out.strip_suffix(b"\0").unwrap_or(out).split(|b| *b == 0).filter(|_| !out.is_empty())
}

/// `Name <email>` as its two parts; a value without `<` is all name.
pub(crate) fn name_email(value: &str) -> (String, String) {
    match value.rsplit_once('<') {
        Some((name, email)) => (name.trim().to_string(), email.trim().trim_end_matches('>').to_string()),
        None => (value.trim().to_string(), String::new()),
    }
}

/// A path as git prints it in a diff header: as is, or C-quoted (`"a\tb"`, octal bytes `\303\251`).
pub(crate) fn unquote(path: &str) -> String {
    let Some(inner) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return path.to_string();
    };
    let mut out = Vec::with_capacity(inner.len());
    let mut bytes = inner.bytes();
    while let Some(b) = bytes.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        match bytes.next() {
            Some(d @ b'0'..=b'7') => {
                let rest: Vec<u8> = bytes.by_ref().take(2).collect();
                let digits = std::iter::once(d).chain(rest).fold(0u32, |n, o| n * 8 + u32::from(o - b'0'));
                out.push(digits as u8);
            }
            Some(e) => out.push(match e {
                b'a' => 7,
                b'b' => 8,
                b't' => b'\t',
                b'n' => b'\n',
                b'v' => 11,
                b'f' => 12,
                b'r' => b'\r',
                other => other,
            }),
            None => out.push(b'\\'),
        }
    }
    text(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_quoted_paths() {
        assert_eq!(nul_records(b"a\0b\0").collect::<Vec<_>>(), [&b"a"[..], b"b"]);
        assert_eq!(nul_records(b"").count(), 0);
        assert_eq!(unquote(r#""t\303\251st\t\"x\".txt""#), "tést\t\"x\".txt");
        assert_eq!(unquote("plain name.txt"), "plain name.txt");
        assert_eq!(name_email("Bea Two <bea@example.test>"), ("Bea Two".into(), "bea@example.test".into()));
    }
}
