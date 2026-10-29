use std::time::{SystemTime, UNIX_EPOCH};

/// Unix epoch milliseconds now.
pub fn now_ms() -> i64 {
    epoch_ms(SystemTime::now())
}

/// `t` in Unix epoch milliseconds; 0 before the epoch.
pub fn epoch_ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// A file's modification time in epoch milliseconds; 0 when the platform has none.
pub fn modified_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified().map_or(0, epoch_ms)
}

/// Six hex digits from the clock and this process's id: a scratch name unlikely to repeat across runs.
pub fn hex6() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    format!("{:06x}", (nanos ^ (u128::from(std::process::id()) << 20)) & 0xff_ffff)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn epoch_ms_counts_from_the_epoch_and_clamps_before_it() {
        assert_eq!(epoch_ms(UNIX_EPOCH + Duration::from_millis(1_234)), 1_234);
        assert_eq!(epoch_ms(UNIX_EPOCH - Duration::from_secs(1)), 0);
        assert!(now_ms() > 1_700_000_000_000);
    }

    #[test]
    fn hex6_is_six_lowercase_hex_digits() {
        let h = hex6();
        assert_eq!(h.len(), 6);
        assert!(h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{h}");
    }
}
