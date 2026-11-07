//! A commit's items in the graph's menus (docs/research/gitkraken-12.6.1.md §B B1): checkout this commit,
//! cherry-pick, rebase onto, reset ▸, edit message, revert, a patch, joined to GA7's [`RefMenu`] groups; and a
//! commit row's menu: B1 alone, the one branch group's menu, or B1 then a section per group.

use std::collections::HashSet;

use supraman_gitman::write::branch::{Target, current_branch};
use supraman_gitman::write::commit::ResetMode;
use supraman_gitman::{Head, Refs};
use supraman_ui::menu::{MenuItem, item};

use super::CommitAct;
use crate::git_view::branch::menu::{self, Facts, Group, RefAct, RefMenu, Subject};

/// What a commit's items read.
pub struct CommitFacts<'a> {
    pub sha: &'a str,
    pub parents: usize,
    pub refs: &'a Refs,
    pub under_way: bool,
}

impl CommitFacts<'_> {
    fn head_sha(&self) -> Option<&str> {
        match &self.refs.head {
            Head::Branch { sha, .. } => sha.as_deref(),
            Head::Detached { sha } => Some(sha),
        }
    }

    fn is_head(&self) -> bool {
        self.head_sha() == Some(self.sha)
    }
}

fn commit(key: &str, label: impl Into<String>, act: CommitAct) -> MenuItem<RefAct> {
    item(key.to_string(), label, RefAct::Commit(act))
}

/// The commit's items: with `checkout` its Checkout this commit, then the commit, copy and compare groups'.
pub fn add(menu: &mut RefMenu<RefAct>, f: &CommitFacts, checkout: bool) {
    let sha = f.sha.to_string();
    let detached_here = matches!(&f.refs.head, Head::Detached { sha: head } if head == f.sha);
    if checkout && !detached_here {
        menu.add(Group::Checkout, item("commit-checkout", "Checkout this commit", RefAct::Checkout(Target::Commit(sha.clone()))));
    }
    if f.under_way || f.head_sha().is_none() {
        menu.add(Group::Copy, commit("commit-patch", "Create patch from commit", CommitAct::Patch(sha)));
        return;
    }
    let current = current_branch(f.refs);
    if !f.is_head() {
        menu.add(Group::Commit, commit("commit-cherry-pick", "Cherry-pick commit", CommitAct::CherryPick(sha.clone())));
        if let Some(branch) = current {
            menu.add(Group::Commit, commit("commit-rebase", format!("Rebase {branch} onto this commit"), CommitAct::RebaseOnto { branch: branch.to_string(), sha: sha.clone() }));
        }
    }
    if f.parents == 1 || (f.is_head() && f.parents == 0) {
        menu.add(Group::Commit, commit("commit-edit-message", "Edit commit message", CommitAct::EditMessage(sha.clone())));
    }
    menu.add(Group::Commit, commit("commit-revert", "Revert commit", CommitAct::Revert(sha.clone())));
    let caption = format!("Reset {} to this commit", current.unwrap_or("HEAD"));
    for (i, (key, words, mode)) in [
        ("commit-reset-soft", "Soft: keep all changes", ResetMode::Soft),
        ("commit-reset-mixed", "Mixed: keep the working copy, reset the index", ResetMode::Mixed),
        ("commit-reset-hard", "Hard: discard all changes", ResetMode::Hard),
    ]
    .into_iter()
    .enumerate()
    {
        let reset = commit(key, words, CommitAct::Reset { sha: sha.clone(), mode });
        menu.add(Group::Commit, if i == 0 { reset.under(caption.clone()) } else { reset });
    }
    menu.add(Group::Copy, commit("commit-patch", "Create patch from commit", CommitAct::Patch(sha)));
}

/// The items a branch group's own menu repeats from the row's commit items.
const COMMIT_LEVEL: [&str; 4] = ["ref-create-branch", "ref-copy-sha", "ref-create-tag", "ref-create-annotated"];

/// A commit row's menu: `groups` are its branch groups' subjects (its tags kept out). None: B1; one: that group's
/// menu with the commit's items; more: B1, then a section per group with its own items.
pub fn row_menu(groups: &[Subject], facts: &Facts, f: &CommitFacts) -> Vec<MenuItem<RefAct>> {
    match groups {
        [] => {
            let mut menu = RefMenu::default();
            menu::commit_items(&mut menu, f.sha);
            add(&mut menu, f, true);
            menu.items()
        }
        [one] => {
            let mut menu = menu::build(one, facts);
            add(&mut menu, f, true);
            menu.items()
        }
        many => {
            let mut out = row_menu(&[], facts, f);
            let level: HashSet<&str> = COMMIT_LEVEL.into_iter().collect();
            let bare = Facts { graph: Vec::new(), ..*facts };
            for subject in many {
                let title = subject.title();
                let items: Vec<MenuItem<RefAct>> = menu::build(subject, &bare).items().into_iter().filter(|i| !level.contains(i.key.as_ref())).collect();
                for (j, mut it) in items.into_iter().enumerate() {
                    it.key = format!("{title}-{}", it.key).into();
                    out.push(if j == 0 { MenuItem { section: Some(title.clone()), ..it } } else { it });
                }
            }
            out
        }
    }
}
