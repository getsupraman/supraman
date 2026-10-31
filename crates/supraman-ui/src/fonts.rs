//! The UI's font families; the terminal's are the user's (`supraman_core::Settings`).

/// The window text.
pub const UI: &str = "Segoe UI";
/// Monospaced text outside a terminal: commands, tables.
pub const MONO: &str = "Cascadia Mono";

/// Segoe Fluent Icons ships with Windows 11 (build 22000+); Windows 10 has the same glyphs in MDL2.
#[cfg(windows)]
pub fn icons() -> &'static str {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    let mut version = unsafe { std::mem::zeroed() };
    let ok = unsafe { RtlGetVersion(&mut version) }.is_ok();
    if ok && version.dwBuildNumber >= 22000 { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" }
}

#[cfg(not(windows))]
pub fn icons() -> &'static str {
    "Segoe Fluent Icons"
}
