//! What changed in a file whose two sides decode to the same text: the bytes the text does not show.

use supraman_core::text::{Encoding, byte_size, decode};

/// The header's encoding: one side's, or `old → new` when they differ.
pub fn encoding_label(old: Option<Encoding>, new: Option<Encoding>) -> String {
    match (old, new) {
        (Some(old), Some(new)) if old != new => format!("{} → {}", old.label(), new.label()),
        (old, new) => new.or(old).unwrap_or(Encoding::Utf8).label().to_string(),
    }
}

fn encoding_change(old: Encoding, new: Encoding) -> Option<String> {
    if old == new {
        return None;
    }
    let (from, to) = (old.label(), new.label());
    Some(match (old.without_bom() == new.without_bom(), new == new.without_bom()) {
        (true, true) => format!("Byte-order mark removed ({from} → {to})"),
        (true, false) => format!("Byte-order mark added ({from} → {to})"),
        (false, _) => format!("Encoding changed: {from} → {to}"),
    })
}

fn line_endings(old: &str, new: &str) -> Vec<String> {
    let (mut to_lf, mut to_crlf) = (0, 0);
    for (o, n) in old.split('\n').zip(new.split('\n')) {
        match (o.ends_with('\r'), n.ends_with('\r')) {
            (true, false) => to_lf += 1,
            (false, true) => to_crlf += 1,
            _ => {}
        }
    }
    let lines = |n: usize| if n == 1 { "1 line".to_string() } else { format!("{n} lines") };
    let mut said = Vec::new();
    if to_lf > 0 {
        said.push(format!("Line endings changed from CRLF to LF on {}", lines(to_lf)));
    }
    if to_crlf > 0 {
        said.push(format!("Line endings changed from LF to CRLF on {}", lines(to_crlf)));
    }
    said
}

/// Each change between `old` and `new`, bytes whose text is the same once decoded and read with LF endings;
/// empty when the bytes are the same too.
pub fn changes(old: &[u8], new: &[u8]) -> Vec<String> {
    if old == new {
        return Vec::new();
    }
    let ((old_text, old_enc), (new_text, new_enc)) = (decode(old), decode(new));
    let mut said: Vec<String> = encoding_change(old_enc, new_enc).into_iter().collect();
    said.extend(line_endings(&old_text, &new_text));
    if said.is_empty() {
        said.push(format!("Bytes the text does not show changed ({} → {})", byte_size(old.len() as u64), byte_size(new.len() as u64)));
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_bom_added_or_removed_with_the_encodings() {
        assert_eq!(changes(b"same\n", b"\xEF\xBB\xBFsame\n"), ["Byte-order mark added (UTF-8 → UTF-8 with BOM)"]);
        assert_eq!(changes(b"\xEF\xBB\xBFsame\n", b"same\n"), ["Byte-order mark removed (UTF-8 with BOM → UTF-8)"]);
        assert_eq!(changes(b"\xFF\xFEa\0", b"a"), ["Encoding changed: UTF-16 LE → UTF-8"]);
    }

    #[test]
    fn counts_the_lines_whose_endings_changed_each_way() {
        assert_eq!(changes(b"a\r\nb\r\nc\n", b"a\nb\nc\n"), ["Line endings changed from CRLF to LF on 2 lines"]);
        assert_eq!(changes(b"a\nb\n", b"a\r\nb\n"), ["Line endings changed from LF to CRLF on 1 line"]);
        assert_eq!(changes(b"a\r\nb\n", b"a\nb\r\n"), ["Line endings changed from CRLF to LF on 1 line", "Line endings changed from LF to CRLF on 1 line"]);
    }

    #[test]
    fn says_other_bytes_changed_when_neither_explains_it() {
        assert_eq!(changes(b"a\xFF\n", b"a\xFE\n"), ["Bytes the text does not show changed (3 B → 3 B)"]);
        assert!(changes(b"same\n", b"same\n").is_empty());
    }

    #[test]
    fn the_header_shows_both_encodings_only_when_they_differ() {
        assert_eq!(encoding_label(Some(Encoding::Utf8), Some(Encoding::Utf8Bom)), "UTF-8 → UTF-8 with BOM");
        assert_eq!(encoding_label(Some(Encoding::Utf8), Some(Encoding::Utf8)), "UTF-8");
        assert_eq!(encoding_label(Some(Encoding::Utf16Le), None), "UTF-16 LE");
    }
}
