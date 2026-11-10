use gpui::TestAppContext;
use supraman_git_view::git_view::GitView;
use supraman_gitman::fixture::Fixture;
use supraman_ui::hit::HitTargets;

use super::open_or_focus;
use crate::saved_tabs::SavedTab;
use crate::view::{Tab, TabKind};

#[gpui::test]
fn git_tab_opens_once_per_folder_and_saves_its_title_and_folder(cx: &mut TestAppContext) {
    let f = Fixture::shared();
    let (mut tabs, mut next) = (Vec::<Tab>::new(), 1u64);
    let made = std::cell::Cell::new(0);
    for _ in 0..2 {
        let (id, _) = cx.update(|cx| {
            open_or_focus(&mut tabs, &mut next, &f.dir, "Git · repo".into(), cx, |cx| {
                made.set(made.get() + 1);
                GitView::open(f.dir.clone(), None, HitTargets::default(), cx)
            })
        });
        assert_eq!(id, 1, "a second open focuses the first tab");
    }
    assert_eq!((tabs.len(), made.get()), (1, 1), "one tab and one view for the folder");
    let other = f.temp.path().join("wt");
    let (id, new) = cx.update(|cx| open_or_focus(&mut tabs, &mut next, &other, "Git · wt".into(), cx, |cx| GitView::open(other.clone(), None, HitTargets::default(), cx)));
    assert_eq!((id, new, tabs.len()), (2, true, 2), "another folder, another tab");

    let TabKind::Git(view) = &tabs[0].kind else { panic!("a Git tab") };
    let saved = cx.update(|cx| SavedTab::Git { title: tabs[0].title.clone(), folder: view.read(cx).folder.clone() });
    assert_eq!(saved, SavedTab::Git { title: "Git · repo".into(), folder: f.dir.clone() }, "a Git tab saves its title and folder");
}
