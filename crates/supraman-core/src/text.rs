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
}
