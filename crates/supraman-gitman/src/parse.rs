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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_names() {
        assert_eq!(nul_records(b"a\0b\0").collect::<Vec<_>>(), [&b"a"[..], b"b"]);
        assert_eq!(nul_records(b"").count(), 0);
        assert_eq!(name_email("Bea Two <bea@example.test>"), ("Bea Two".into(), "bea@example.test".into()));
    }
}
