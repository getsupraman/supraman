//! Bytes of a file as text, by their byte-order mark.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// Not valid UTF-8; the invalid bytes read as U+FFFD.
    Unknown,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 with BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
            Encoding::Unknown => "Unknown (shown as UTF-8)",
        }
    }

    /// The same encoding without its byte-order mark: UTF-8 with BOM as UTF-8; UTF-16 always carries one.
    pub fn without_bom(self) -> Encoding {
        match self {
            Encoding::Utf8Bom => Encoding::Utf8,
            other => other,
        }
    }
}

/// A byte count in the user's words: `512 B`, `1.5 KB`, `12 MB` (1 KB = 1,024 bytes, as Explorer counts).
pub fn byte_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1024.;
    let mut unit = 0;
    while size >= 1024. && unit + 1 < UNITS.len() {
        size /= 1024.;
        unit += 1;
    }
    if size < 10. { format!("{size:.1} {}", UNITS[unit]) } else { format!("{size:.0} {}", UNITS[unit]) }
}

/// UTF-16 with its BOM, UTF-8 with or without one, else UTF-8 with the invalid bytes replaced.
pub fn decode(bytes: &[u8]) -> (String, Encoding) {
    let utf16 = |b: &[u8], le: bool| {
        let units: Vec<u16> = b.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => (utf16(rest, true), Encoding::Utf16Le),
        [0xFE, 0xFF, rest @ ..] => (utf16(rest, false), Encoding::Utf16Be),
        [0xEF, 0xBB, 0xBF, rest @ ..] => (String::from_utf8_lossy(rest).into_owned(), Encoding::Utf8Bom),
        b => match std::str::from_utf8(b) {
            Ok(s) => (s.to_string(), Encoding::Utf8),
            Err(_) => (String::from_utf8_lossy(b).into_owned(), Encoding::Unknown),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_by_byte_order_mark() {
        assert_eq!(decode(b"plain"), ("plain".into(), Encoding::Utf8));
        assert_eq!(decode(b"\xEF\xBB\xBFbom"), ("bom".into(), Encoding::Utf8Bom));
        assert_eq!(decode(b"\xFF\xFEa\0b\0"), ("ab".into(), Encoding::Utf16Le));
        assert_eq!(decode(b"\xFE\xFF\0a\0b"), ("ab".into(), Encoding::Utf16Be));
        assert_eq!(decode(b"a\xFFb"), ("a\u{FFFD}b".into(), Encoding::Unknown));
    }

    #[test]
    fn only_utf8_sheds_its_bom() {
        assert_eq!(Encoding::Utf8Bom.without_bom(), Encoding::Utf8);
        assert_eq!(Encoding::Utf16Le.without_bom(), Encoding::Utf16Le);
    }

    #[test]
    fn byte_sizes_read_as_explorer_shows_them() {
        assert_eq!(byte_size(0), "0 B");
        assert_eq!(byte_size(1023), "1023 B");
        assert_eq!(byte_size(1536), "1.5 KB");
        assert_eq!(byte_size(20 * 1024), "20 KB");
        assert_eq!(byte_size(3 * 1024 * 1024), "3.0 MB");
    }
}
