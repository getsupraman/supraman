use super::GitSizes;

pub fn sizes() -> GitSizes {
    GitSizes { widths: vec![200, 90, 300, 130, 70], panel: Some(360), message: Some(150), refs: Some(210), refs_shut: true, hidden: vec!["refs/heads/feature".into(), "stash/0123".into()], collapsed: Vec::new(), solo: Vec::new(), sections: Vec::new() }
}

/// Panics with the tab's problem unless its first load is done, so a failed load is named, not a missing row.
#[cfg(test)]
pub(crate) fn assert_loaded(view: &gpui::Entity<super::GitView>, cx: &mut gpui::VisualTestContext) {
    let load = view.read_with(cx, |v, _| match &v.load {
        super::Load::Ready => None,
        super::Load::Loading => Some("still loading".to_string()),
        super::Load::Problem(problem) => Some(format!("{problem:?}")),
    });
    if let Some(why) = load {
        panic!("the Git tab of {} did not load: {why}", view.read_with(cx, |v, _| v.folder.display().to_string()));
    }
}
