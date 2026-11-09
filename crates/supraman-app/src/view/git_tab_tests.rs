use gpui::TestAppContext;
use supraman_git_view::git_view::test_support::sizes;
use supraman_git_view::git_view::{GitSizes, GitView};
use supraman_gitman::fixture::Fixture;
use supraman_ui::hit::HitTargets;

use super::open_or_focus;
use crate::saved_tabs::{self, Reopen, SavedTab, SavedTabs, World};
use crate::view::{Tab, TabKind};

#[gpui::test]
fn git_tab_opens_once_per_folder_and_comes_back_after_a_restart(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (mut tabs, mut next) = (Vec::<Tab>::new(), 1u64);
    let made = std::cell::Cell::new(0);
    for _ in 0..2 {
        let (id, _) = cx.update(|cx| {
            open_or_focus(&mut tabs, &mut next, &f.dir, "Git · repo".into(), cx, |cx| {
                made.set(made.get() + 1);
                GitView::open(f.dir.clone(), None, HitTargets::default(), &sizes(), cx)
            })
        });
        assert_eq!(id, 1, "a second open focuses the first tab");
    }
    assert_eq!((tabs.len(), made.get()), (1, 1), "one tab and one view for the folder");
    let other = f.temp.path().join("wt");
    let (id, new) = cx.update(|cx| open_or_focus(&mut tabs, &mut next, &other, "Git · wt".into(), cx, |cx| GitView::open(other.clone(), None, HitTargets::default(), &GitSizes::default(), cx)));
    assert_eq!((id, new, tabs.len()), (2, true, 2), "another folder, another tab");

    let TabKind::Git(view) = &tabs[0].kind else { panic!("a Git tab") };
    let saved = cx.update(|cx| SavedTab::git(tabs[0].title.clone(), view.read(cx).folder.clone()));
    let home = tempfile::tempdir().unwrap();
    saved_tabs::save(home.path(), &SavedTabs { tabs: vec![saved.clone()], active: Some(0) }).unwrap();
    let (back, notice) = saved_tabs::load(home.path());
    assert_eq!((back.tabs.as_slice(), notice), (std::slice::from_ref(&saved), None));
    let world = World { sessions: &[], now: 0, workspaces: &[], resume_on_click: false, is_dir: &|p| p.is_dir(), is_file: &|_| true, has_transcript: &|_, _| false };
    let Ok(Reopen::Git { title, folder, sizes: back }) = saved_tabs::plan(&back.tabs[0], &world) else { panic!("a Git tab reopens") };
    assert_eq!((title.as_str(), folder.as_path(), back), ("Git · repo", f.dir.as_path(), GitSizes::default()), "the sizes are the layout file's, not the tab's");
    let gone = SavedTab::git("Git · x".into(), f.temp.path().join("gone"));
    assert!(saved_tabs::plan(&gone, &world).unwrap_err().contains("is gone"));

    let old = serde_json::json!({ "kind": "git", "title": "Git · repo", "folder": f.dir, "widths": [200, 90, 300, 130, 70], "panel": 360, "message": 150, "refs": 210, "refs_shut": true, "hidden": ["refs/heads/feature", "stash/0123"] });
    let old: SavedTab = serde_json::from_value(old).unwrap();
    let Ok(Reopen::Git { sizes: back, .. }) = saved_tabs::plan(&old, &world) else { panic!("a Git tab reopens") };
    assert_eq!(back, sizes(), "a tab saved before the layout file still hands over its sizes");
}
