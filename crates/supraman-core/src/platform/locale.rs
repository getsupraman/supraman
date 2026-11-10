use crate::time::Locale;

/// The user's short date and time patterns and AM/PM designators; a part Windows does not give is [`Locale::default`]'s.
#[cfg(windows)]
pub fn user() -> Result<Locale, String> {
    use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_S1159, LOCALE_S2359, LOCALE_SSHORTDATE, LOCALE_SSHORTTIME};
    use windows::core::PCWSTR;
    let read = |kind: u32| {
        let mut buf = [0u16; 128];
        let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), kind, Some(&mut buf)) };
        let n = usize::try_from(n).unwrap_or(0).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..n])).filter(|s| !s.is_empty())
    };
    let fallback = Locale::default();
    Ok(Locale {
        date: read(LOCALE_SSHORTDATE).unwrap_or(fallback.date),
        time: read(LOCALE_SSHORTTIME).unwrap_or(fallback.time),
        am: read(LOCALE_S1159).unwrap_or(fallback.am),
        pm: read(LOCALE_S2359).unwrap_or(fallback.pm),
    })
}

#[cfg(not(windows))]
pub fn user() -> Result<Locale, String> {
    Err(super::refused("the user's locale"))
}
