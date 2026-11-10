//! The UI's font families; the terminal's are the user's (`supraman_core::Settings`).

/// The window text.
pub const UI: &str = "Segoe UI";
/// Monospaced text outside a terminal: commands, tables.
pub const MONO: &str = "Cascadia Mono";

/// Segoe Fluent Icons ships with Windows 11 (build 22000+); Windows 10 has the same glyphs in MDL2.
#[cfg(windows)]
pub fn icons() -> &'static str {
    if supraman_core::platform::desktop::windows_build().is_ok_and(|build| build >= 22000) { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" }
}

#[cfg(not(windows))]
pub fn icons() -> &'static str {
    "Segoe Fluent Icons"
}

/// The UI font with tabular figures, so digits keep one width: dates and shas.
pub fn figures<E: gpui::Styled>(element: E) -> E {
    element.font_family(UI).font_features(gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])))
}
