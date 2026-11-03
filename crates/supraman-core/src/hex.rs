const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// `bytes` as lowercase hex, two digits each.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 0x0f)] as char);
    }
    out
}

/// Hex (either case) back to bytes; `None` for an odd length or a non-hex digit.
pub fn decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect()
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn encode_is_two_lowercase_digits_per_byte() {
        assert_eq!(encode(&[]), "");
        assert_eq!(encode(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
        let all: Vec<u8> = (0..=255).collect();
        let formatted: String = all.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(encode(&all), formatted);
        let digest = Sha256::digest(b"supraman");
        assert_eq!(encode(&digest), format!("{digest:x}"));
    }

    #[test]
    fn decode_inverts_encode_and_refuses_bad_hex() {
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&all)), Some(all));
        assert_eq!(decode("A0ff"), Some(vec![0xa0, 0xff]));
        assert_eq!(decode("abc"), None);
        assert_eq!(decode("zz"), None);
    }
}
