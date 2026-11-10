use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use gpui::{App, Task};
use supraman_core::platform::open::runs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Url(String),
    Path(PathBuf),
}

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

/// `Ok` for a folder or a document; else why the file `path` is not opened, since its handler would run it.
/// Reads the disk and `PATHEXT`, so never on the UI thread.
pub fn document(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        return Ok(());
    }
    let name = path.file_name().map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy());
    match runs(&name, &supraman_core::platform::shell::pathext()) {
        Some(false) => Ok(()),
        Some(true) => Err(refused_file(&name)),
        None => Err(refused_bare(&name)),
    }
}

/// Why the file `name` was not opened: its extension runs it.
pub fn refused_file(name: &str) -> String {
    format!("Not opened: {name} runs as a program. Open its folder instead.")
}

/// Why the file `name` was not opened: without an extension Windows may still find a handler that runs it.
pub fn refused_bare(name: &str) -> String {
    format!("Not opened: {name} has no extension and may run as a program. Open its folder instead.")
}

/// The path a `file:` URL names, percent-decoded; else why it opens nothing.
pub fn file_url_path(url: &str) -> Result<PathBuf, String> {
    url::Url::parse(url)
        .ok()
        .filter(|u| u.scheme() == "file")
        .and_then(|u| u.to_file_path().ok())
        .ok_or_else(|| format!("Not opened: {url} names no file path."))
}

/// Opens `target`, an [`allowed`] URL or a path [`document`] passes, with its handler; blocking, so never on the UI thread: ShellExecuteW
/// pumps its thread's messages, and there gpui re-enters inside the click's borrow (m5/68).
pub fn open(target: &OsStr) -> Result<(), String> {
    supraman_core::platform::open::open(target)
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

    #[test]
    fn a_file_that_runs_scripts_installs_or_links_is_refused_by_its_extension_and_pathext() {
        let pathext = ".COM;.EXE;.BAT;.CMD;.FOO";
        for name in ["setup.bat", "x.js", "y.lnk", "z.msi", "tool.ps1", "page.hta", "fix.reg", "a.bat.", "a.bat. . ", "a.CMD", "A.Exe", "x.ps1:Zone", "plugin.foo", "app.appref-ms"] {
            assert_eq!(runs(name, pathext), Some(true), "{name}");
        }
        assert_eq!(runs("plugin.foo", ".COM;.EXE"), Some(false), "an extension only PATHEXT lists");
        for name in ["notes.txt", "README.md", "shot.png", "doc.pdf", "main.rs", ".bashrc", "a.bat.txt"] {
            assert_eq!(runs(name, pathext), Some(false), "{name}");
        }
        for name in ["Makefile", "setup.", "setup . "] {
            assert_eq!(runs(name, pathext), None, "{name}");
        }
    }

    #[test]
    fn a_folder_or_document_opens_and_a_program_or_bare_file_is_refused_with_its_name() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("tools.bat");
        std::fs::create_dir(&folder).unwrap();
        for name in ["notes.txt", "README.md", "Makefile", "setup.bat"] {
            std::fs::write(tmp.path().join(name), "").unwrap();
        }
        assert_eq!(document(&folder), Ok(()), "a folder opens whatever its name");
        assert_eq!(document(&tmp.path().join("notes.txt")), Ok(()));
        assert_eq!(document(&tmp.path().join("README.md")), Ok(()));
        assert_eq!(document(&tmp.path().join("setup.bat")), Err("Not opened: setup.bat runs as a program. Open its folder instead.".into()));
        assert_eq!(document(&tmp.path().join("SETUP.BAT.")), Err(refused_file("SETUP.BAT.")));
        assert_eq!(document(&tmp.path().join("Makefile")), Err(refused_bare("Makefile")));
    }

    #[test]
    fn a_file_url_is_its_decoded_path() {
        assert_eq!(file_url_path("file:///C:/My%20Tools/x.cmd"), Ok(PathBuf::from(r"C:\My Tools\x.cmd")));
        assert_eq!(file_url_path("https://example.com/x.cmd"), Err("Not opened: https://example.com/x.cmd names no file path.".into()));
    }
}
