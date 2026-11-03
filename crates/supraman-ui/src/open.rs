use std::ffi::{OsStr, OsString};

use gpui::{App, Task};

/// The schemes a link may open; a program's output must not reach any other protocol handler (sched2/11).
pub const LINK_SCHEMES: [&str; 4] = ["http", "https", "mailto", "file"];
/// The schemes of a web page (release notes, the sign-in page).
pub const WEB_SCHEMES: [&str; 2] = ["http", "https"];

/// `url`'s scheme, lowercased; empty without one.
pub fn scheme(url: &str) -> String {
    url.split_once(':').map_or("", |(scheme, _)| scheme).to_ascii_lowercase()
}

/// `Err` with `url`'s scheme unless it is one of `schemes`.
pub fn allowed(url: &str, schemes: &[&str]) -> Result<(), String> {
    let scheme = scheme(url);
    if schemes.contains(&scheme.as_str()) { Ok(()) } else { Err(scheme) }
}

/// Why a link of `scheme` was not opened.
pub fn refused(scheme: &str) -> String {
    format!("Not opened: a {scheme}: link. Only http, https, mailto and file links open.")
}

/// Opens `target`, an [`allowed`] URL or a path, with its handler; blocking, so never on the UI thread: ShellExecuteW
/// pumps its thread's messages, and there gpui re-enters inside the click's borrow (m5/68).
pub fn open(target: &OsStr) -> Result<(), String> {
    supraman_core::process::shell_execute("open", target, None, None).map_err(|code| format!("ShellExecuteW returned {code}"))
}

/// [`open`] on the background executor.
pub fn open_off_thread(target: impl Into<OsString>, cx: &App) -> Task<Result<(), String>> {
    let target = target.into();
    cx.background_executor().spawn(async move { open(&target) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_allowed_schemes_pass_and_a_refusal_names_the_scheme() {
        for uri in ["https://example.com/", "HTTP://example.com/a?b=c", "mailto:a@b.c", "file:///C:/tmp/a.txt"] {
            assert_eq!(allowed(uri, &LINK_SCHEMES), Ok(()), "{uri}");
        }
        for (uri, want) in [("ms-settings:privacy", "ms-settings"), ("C:/tmp/a.exe", "c"), ("no-scheme", ""), ("mailto:a@b.c", "mailto")] {
            assert_eq!(allowed(uri, &WEB_SCHEMES), Err(want.to_string()), "{uri}");
        }
        assert_eq!(allowed("https://work.supraman.ai/authorize", &WEB_SCHEMES), Ok(()));
        assert_eq!(refused("ms-settings"), "Not opened: a ms-settings: link. Only http, https, mailto and file links open.");
    }
}
