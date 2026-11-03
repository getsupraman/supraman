use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local, TimeZone};

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

/// `ms` in this computer's zone; `None` outside chrono's range.
pub fn local(ms: i64) -> Option<DateTime<Local>> {
    Local.timestamp_millis_opt(ms).single()
}

/// `5 s ago`, `3 min ago`, `2 h ago` for `elapsed_ms`; never negative.
pub fn ago(elapsed_ms: i64) -> String {
    let s = (elapsed_ms / 1000).max(0);
    match s {
        0..60 => format!("{s} s ago"),
        60..3600 => format!("{} min ago", s / 60),
        _ => format!("{} h ago", s / 3600),
    }
}

/// `09:00`; empty outside chrono's range.
pub fn clock(ms: i64) -> String {
    local(ms).map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}

/// `09:00` today, else `2026-11-03 09:00`.
pub fn short(ms: i64, now_ms: i64) -> Option<String> {
    let at = local(ms)?;
    let today = local(now_ms).is_some_and(|now| now.date_naive() == at.date_naive());
    Some(if today { at.format("%H:%M") } else { at.format("%Y-%m-%d %H:%M") }.to_string())
}

/// `09:00` today, `Mon 09:00` within six days, else `Mon 3 Nov 09:00`.
pub fn near(ms: i64, now_ms: i64) -> String {
    let (Some(at), Some(now)) = (local(ms), local(now_ms)) else {
        return ms.to_string();
    };
    if (ms - now_ms).abs() < 6 * 24 * 3_600_000 && at.date_naive() != now.date_naive() {
        at.format("%a %H:%M").to_string()
    } else if at.date_naive() == now.date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%a %-d %b %H:%M").to_string()
    }
}

/// `Mon 3 Nov 2026 at 08:00`.
pub fn date_words(ms: i64) -> String {
    local(ms).map_or_else(|| ms.to_string(), |t| t.format("%a %-d %b %Y at %H:%M").to_string())
}

/// An RFC 3339 time (a server's, in UTC) as `2026-10-26 19:46` here; one that does not parse keeps its first 16 characters.
pub fn rfc3339_local(at: &str) -> String {
    rfc3339_in(at, &Local)
}

/// [`rfc3339_local`] in `zone`.
pub fn rfc3339_in<Tz: TimeZone>(at: &str, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match DateTime::parse_from_rfc3339(at) {
        Ok(t) => t.with_timezone(zone).format("%Y-%m-%d %H:%M").to_string(),
        Err(_) => at.get(..16).unwrap_or(at).replace('T', " "),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn ago_counts_seconds_then_minutes_then_hours() {
        assert_eq!(ago(-5_000), "0 s ago");
        assert_eq!(ago(5_999), "5 s ago");
        assert_eq!(ago(59_999), "59 s ago");
        assert_eq!(ago(60_000), "1 min ago");
        assert_eq!(ago(3_599_999), "59 min ago");
        assert_eq!(ago(3_600_000), "1 h ago");
        assert_eq!(ago(50 * 3_600_000), "50 h ago");
    }

    #[test]
    fn local_formats_say_the_same_instant() {
        let at = local(NOW).unwrap();
        assert_eq!(clock(NOW), at.format("%H:%M").to_string());
        assert_eq!(short(NOW, NOW), Some(at.format("%H:%M").to_string()));
        assert_eq!(short(NOW, NOW + 3 * 24 * 3_600_000), Some(at.format("%Y-%m-%d %H:%M").to_string()));
        assert_eq!(near(NOW, NOW), at.format("%H:%M").to_string());
        assert_eq!(near(NOW, NOW + 3 * 24 * 3_600_000), at.format("%a %H:%M").to_string());
        assert_eq!(near(NOW, NOW + 30 * 24 * 3_600_000), at.format("%a %-d %b %H:%M").to_string());
        assert_eq!(date_words(NOW), at.format("%a %-d %b %Y at %H:%M").to_string());
        assert_eq!(clock(i64::MAX), "");
        assert_eq!(short(i64::MAX, NOW), None);
        assert_eq!(near(i64::MAX, NOW), i64::MAX.to_string());
        assert_eq!(date_words(i64::MAX), i64::MAX.to_string());
    }

    #[test]
    fn rfc3339_is_shown_in_the_zone_and_a_bad_one_as_written() {
        let cet = chrono::FixedOffset::east_opt(3600).unwrap();
        assert_eq!(rfc3339_in("2026-10-26T18:46:12+00:00", &cet), "2026-10-26 19:46");
        assert_eq!(rfc3339_in("2026-10-26T18:46:12.5Z", &cet), "2026-10-26 19:46");
        assert_eq!(rfc3339_in("2026-10-26T23:30:00-02:00", &cet), "2026-10-27 02:30");
        assert_eq!(rfc3339_in("2026-10-26T09:30", &cet), "2026-10-26 09:30");
        assert_eq!(rfc3339_local("x"), "x");
    }

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
