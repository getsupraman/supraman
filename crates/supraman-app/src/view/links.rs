//! A Ctrl+click on a terminal link: what it opens, and a path checked on disk off the UI thread.

use std::path::{Path, PathBuf};

use gpui::Context;
use supraman_term::MaybeNavigationTarget;
use supraman_ui::open;
use util::paths::PathWithPosition;

use super::FleetView;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Url(String),
    Path(PathBuf),
}

/// What a Ctrl+click on `target` opens, or why it opens nothing; a `file:` URL is its path, a path loses its `:line:column`.
pub fn link_target(target: &MaybeNavigationTarget) -> Result<Link, String> {
    match target {
        MaybeNavigationTarget::Url(url) => {
            open::allowed(url, &open::LINK_SCHEMES).map_err(|scheme| open::refused(&scheme))?;
            if open::scheme(url) == "file" { open::file_url_path(url).map(Link::Path) } else { Ok(Link::Url(url.clone())) }
        }
        MaybeNavigationTarget::PathLike(link) => {
            let path = PathWithPosition::parse_str(&link.maybe_path).path;
            Ok(Link::Path(match &link.working_directory {
                Some(cwd) if path.is_relative() => cwd.join(path),
                _ => path,
            }))
        }
    }
}

/// Why a path link opens nothing: no file or folder there. Reads the disk, so never on the UI thread.
pub fn missing(path: &Path) -> Option<String> {
    (!path.is_absolute() || !path.exists()).then(|| format!("Not opened: there is no file or folder {}.", path.display()))
}

/// `path`, else its longest prefix ending before a space that exists (an unquoted path with spaces is offered
/// through its last word with a separator), else why nothing opens. Reads the disk, so never on the UI thread.
pub fn resolve(path: PathBuf) -> Result<PathBuf, String> {
    let Some(why) = missing(&path) else { return Ok(path) };
    let text = path.to_string_lossy();
    text.match_indices(' ')
        .rev()
        .map(|(i, _)| PathBuf::from(text[..i].trim_end_matches(['.', ',', ';', ':', '!', '?', ')'])))
        .find(|prefix| missing(prefix).is_none())
        .ok_or(why)
}

/// What a Ctrl+click on `target` hands to the opener, or why it opens nothing: a refused scheme, a missing path, or a
/// file that runs as a program. Reads the disk, so never on the UI thread.
pub fn followed(target: &MaybeNavigationTarget) -> Result<Link, String> {
    match link_target(target)? {
        Link::Path(path) => {
            let path = resolve(path)?;
            open::document(&path)?;
            Ok(Link::Path(path))
        }
        url => Ok(url),
    }
}

impl FleetView {
    pub(super) fn open_link(&mut self, target: &MaybeNavigationTarget, cx: &mut Context<Self>) {
        let target = target.clone();
        let found = cx.background_executor().spawn(async move { followed(&target) });
        cx.spawn(async move |this, cx| {
            let found = found.await;
            this.update(cx, |this, cx| match found {
                Ok(link) => (this.open_link_with)(&link, cx),
                Err(why) => this.pane_toast.show_why(why, |v: &mut Self| &mut v.pane_toast, cx),
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use supraman_term::PathLikeTarget;

    use super::*;

    fn opened(shown: &str) -> Result<PathBuf, String> {
        let target = MaybeNavigationTarget::PathLike(PathLikeTarget { maybe_path: shown.into(), working_directory: None });
        match link_target(&target) {
            Ok(Link::Path(path)) => resolve(path),
            other => panic!("{shown}: {other:?}"),
        }
    }

    #[test]
    fn a_spaced_path_opens_whole_when_it_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let spider = tmp.path().join(r"Tools\Screaming Frog SEO Spider");
        std::fs::create_dir_all(&spider).unwrap();
        let file = tmp.path().join(r"Program Files\x\y.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "").unwrap();
        assert_eq!(opened(&format!(r"{}\", spider.display())), Ok(PathBuf::from(format!(r"{}\", spider.display()))));
        assert_eq!(opened(&format!("{}:12:3", file.display())), Ok(file));
    }

    #[test]
    fn a_spaced_path_opens_its_longest_prefix_that_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let frog = tmp.path().join(r"Tools\Screaming Frog");
        std::fs::create_dir_all(&frog).unwrap();
        let file = tmp.path().join(r"a b\c.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "").unwrap();
        assert_eq!(opened(&format!(r"{}\Screaming Frog SEO Spider\", frog.parent().unwrap().display())), Ok(frog));
        assert_eq!(opened(&format!("{} for details.", file.display())), Ok(file.clone()));
        assert_eq!(opened(&format!("{}, see", file.display())), Ok(file));
        let gone = tmp.path().join("no such thing");
        assert_eq!(opened(&gone.display().to_string()), Err(format!("Not opened: there is no file or folder {}.", gone.display())));
    }

    #[test]
    fn a_ctrl_click_on_a_program_path_or_file_url_reaches_no_opener_and_says_why() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["setup.bat", "x.cmd", "notes.txt"] {
            std::fs::write(tmp.path().join(name), "").unwrap();
        }
        let path = |shown: String| MaybeNavigationTarget::PathLike(PathLikeTarget { maybe_path: shown, working_directory: None });
        let bat = path(format!(r"{}\setup.bat:3", tmp.path().display()));
        assert_eq!(followed(&bat), Err(open::refused_file("setup.bat")));
        let relative = MaybeNavigationTarget::PathLike(PathLikeTarget { maybe_path: "SETUP.BAT.".into(), working_directory: Some(tmp.path().into()) });
        assert_eq!(followed(&relative), Err(open::refused_file("SETUP.BAT.")));
        let file_url = |p: &Path| format!("file:///{}", p.display().to_string().replace('\\', "/").replace(' ', "%20"));
        let url = file_url(&tmp.path().join("x.cmd"));
        assert_eq!(followed(&MaybeNavigationTarget::Url(url)), Err(open::refused_file("x.cmd")));

        let txt = tmp.path().join("notes.txt");
        let txt_url = file_url(&txt);
        assert_eq!(followed(&MaybeNavigationTarget::Url(txt_url)), Ok(Link::Path(txt.clone())));
        assert_eq!(followed(&path(txt.display().to_string())), Ok(Link::Path(txt)));
        assert_eq!(followed(&path(tmp.path().display().to_string())), Ok(Link::Path(tmp.path().into())), "a folder opens");
        assert_eq!(followed(&MaybeNavigationTarget::Url("ms-settings:privacy".into())), Err(open::refused("ms-settings")));
    }
}
