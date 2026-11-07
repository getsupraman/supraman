//! The ref menus (docs/research/gitkraken-12.6.1.md §B): one builder per kind, each filling a [`RefMenu`]'s groups
//! in GitKraken's G1–G13 order, a rule between the non-empty ones. Another unit adds its items by pushing them into
//! the group they belong to ([`RefMenu::add`]) in the builder of their kind.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use supraman_gitman::write::branch::{Relation, Target, current_branch, held_elsewhere};
use supraman_gitman::{Head, Refs, short_sha};
use supraman_ui::menu::{MenuItem, item};

use crate::git_view::ref_panel::RefAction;

/// A menu's groups in the order they show; `Sync`, `Worktree`, `Rewrite` and `Compare` are filled by
/// gitman-actions/remote and gitman-actions/commit-menus.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// Pull, push, set upstream (gitman-actions/remote).
    Sync,
    Merge,
    Checkout,
    /// Create worktree from (gitman-actions/commit-menus).
    Worktree,
    /// Create branch here; cherry-pick, reset, revert (commit-menus).
    Commit,
    /// Interactive rebase and its shortcuts (commit-menus).
    Rewrite,
    Manage,
    Copy,
    Graph,
    /// Compare against the working directory (commit-menus).
    Compare,
    Tags,
}

pub struct RefMenu<A> {
    groups: BTreeMap<Group, Vec<MenuItem<A>>>,
}

impl<A> Default for RefMenu<A> {
    fn default() -> Self {
        RefMenu { groups: BTreeMap::new() }
    }
}

impl<A> RefMenu<A> {
    pub fn add(&mut self, group: Group, item: MenuItem<A>) {
        self.groups.entry(group).or_default().push(item);
    }

    /// The items group by group, each group after the first starting with a rule.
    pub fn items(self) -> Vec<MenuItem<A>> {
        let mut out = Vec::new();
        for (i, (_, items)) in self.groups.into_iter().filter(|(_, items)| !items.is_empty()).enumerate() {
            for (j, item) in items.into_iter().enumerate() {
                out.push(if i > 0 && j == 0 && item.section.is_none() { item.rule() } else { item });
            }
        }
        out
    }
}

/// What a ref menu's item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefAct {
    Checkout(Target),
    /// The Git tab of that folder.
    OpenWorktree(PathBuf),
    FastForward { branch: String, to: String },
    Merge { source: String, into: String },
    Rebase { branch: String, onto: String },
    CreateBranch { at: String },
    CreateTag { at: String, annotated: bool },
    /// A lightweight tag made again annotated.
    Annotate { name: String, at: String },
    Rename(String),
    DeleteBranch(String),
    DeleteTag(String),
    Copy { text: String, what: &'static str },
    RemoveWorktree { path: PathBuf, branch: Option<String> },
    LockWorktree { path: PathBuf, lock: bool },
    PruneWorktrees,
    /// The ref panel's own: hide, solo, a stash's.
    Panel(RefAction),
    /// A commit's own: cherry-pick, reset, revert, …
    Commit(crate::git_view::commit::CommitAct),
}

/// What a menu was opened on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// A branch row, a remote branch row, or a pill's group: its local and remote branches at `sha`.
    Refs { locals: Vec<String>, remotes: Vec<String>, sha: String },
    Tag { name: String, sha: String, annotated: bool },
    Detached { sha: String },
    Worktree { path: PathBuf, label: String, branch: Option<String>, main: bool, current: bool, locked: bool, prunable: bool },
}

impl Subject {
    /// The commit whose relation to HEAD the menu needs.
    pub fn sha(&self) -> Option<&str> {
        match self {
            Subject::Refs { sha, .. } | Subject::Tag { sha, .. } | Subject::Detached { sha } => Some(sha),
            Subject::Worktree { .. } => None,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Subject::Refs { locals, remotes, .. } => locals.first().or(remotes.first()).cloned().unwrap_or_default(),
            Subject::Tag { name, .. } => name.clone(),
            Subject::Detached { sha } => format!("HEAD ({})", short_sha(sha)),
            Subject::Worktree { label, .. } => label.clone(),
        }
    }
}

/// What the builders read besides the subject.
pub struct Facts<'a> {
    pub refs: &'a Refs,
    pub top: &'a Path,
    /// The subject's commit to HEAD's; `None` when unknown or there is no HEAD.
    pub relation: Option<Relation>,
    /// A merge, rebase, cherry-pick, revert or am is under way.
    pub under_way: bool,
    /// The ref panel's items for the subject: Solo, Hide.
    pub graph: Vec<MenuItem<RefAct>>,
}

impl Facts<'_> {
    fn current(&self) -> Option<&str> {
        current_branch(self.refs)
    }

    fn head_sha(&self) -> Option<&str> {
        match &self.refs.head {
            Head::Branch { sha, .. } => sha.as_deref(),
            Head::Detached { sha } => Some(sha),
        }
    }

    fn elsewhere(&self, branch: &str) -> Option<PathBuf> {
        held_elsewhere(self.refs, self.top, branch)
    }
}

fn act(key: String, label: String, action: RefAct) -> MenuItem<RefAct> {
    item(key, label, action)
}

fn in_worktree(path: &Path) -> String {
    format!("It is checked out in the worktree {}", path.display())
}

/// The menu of `subject`.
pub fn build(subject: &Subject, facts: &Facts) -> RefMenu<RefAct> {
    let mut menu = RefMenu::default();
    match subject {
        Subject::Refs { locals, remotes, sha } => refs_menu(&mut menu, locals, remotes, sha, facts),
        Subject::Tag { name, sha, annotated } => tag_menu(&mut menu, name, sha, *annotated, facts),
        Subject::Detached { sha } => commit_items(&mut menu, sha),
        Subject::Worktree { .. } => worktree_menu(&mut menu, subject),
    }
    for item in facts.graph.iter().cloned() {
        menu.add(Group::Graph, item);
    }
    menu
}

/// B3 G3: merge, fast-forward and rebase between `clicked` and HEAD's branch.
fn merge_items(menu: &mut RefMenu<RefAct>, clicked: &str, local: bool, sha: &str, facts: &Facts) {
    let Some(current) = facts.current() else { return };
    if clicked == current || facts.head_sha() == Some(sha) || facts.under_way {
        return;
    }
    let held = local.then(|| facts.elsewhere(clicked)).flatten();
    let into_clicked = |item: MenuItem<RefAct>| match &held {
        Some(path) => item.dimmed(in_worktree(path)),
        None => item,
    };
    let merge_in = || act(format!("ref-merge-{clicked}"), format!("Merge {clicked} into {current}"), RefAct::Merge { source: clicked.into(), into: current.into() });
    let rebase = || act(format!("ref-rebase-{clicked}"), format!("Rebase {current} onto {clicked}"), RefAct::Rebase { branch: current.into(), onto: clicked.into() });
    match facts.relation {
        Some(Relation::Behind) if local => {
            menu.add(Group::Merge, into_clicked(act(format!("ref-ff-{clicked}"), format!("Fast-forward {clicked} to {current}"), RefAct::FastForward { branch: clicked.into(), to: current.into() })));
            menu.add(Group::Merge, into_clicked(act(format!("ref-merge-into-{clicked}"), format!("Merge {current} into {clicked}"), RefAct::Merge { source: current.into(), into: clicked.into() })));
        }
        Some(Relation::Behind) | Some(Relation::Same) | None => {}
        Some(Relation::Ahead) => {
            menu.add(Group::Merge, act(format!("ref-ff-head-{clicked}"), format!("Fast-forward {current} to {clicked}"), RefAct::FastForward { branch: current.into(), to: clicked.into() }));
            menu.add(Group::Merge, merge_in());
            menu.add(Group::Merge, rebase());
        }
        Some(Relation::Diverged) => {
            menu.add(Group::Merge, merge_in());
            menu.add(Group::Merge, rebase());
        }
    }
}

/// Create branch here, and the tags group's Create tag here and Create annotated tag here.
pub(crate) fn commit_items(menu: &mut RefMenu<RefAct>, sha: &str) {
    menu.add(Group::Commit, item("ref-create-branch", "Create branch here", RefAct::CreateBranch { at: sha.into() }));
    menu.add(Group::Copy, item("ref-copy-sha", "Copy commit sha", RefAct::Copy { text: sha.into(), what: "the commit sha" }));
    menu.add(Group::Tags, item("ref-create-tag", "Create tag here", RefAct::CreateTag { at: sha.into(), annotated: false }));
    menu.add(Group::Tags, item("ref-create-annotated", "Create annotated tag here", RefAct::CreateTag { at: sha.into(), annotated: true }));
}

/// B3: a branch row, a remote branch row, or a pill's group of a local branch and its remotes.
fn refs_menu(menu: &mut RefMenu<RefAct>, locals: &[String], remotes: &[String], sha: &str, facts: &Facts) {
    let current = facts.current();
    let (clicked, local) = match (locals.first(), remotes.first()) {
        (Some(l), _) => (l.as_str(), true),
        (None, Some(r)) => (r.as_str(), false),
        (None, None) => return,
    };
    merge_items(menu, clicked, local, sha, facts);
    for name in locals.iter().filter(|l| Some(l.as_str()) != current) {
        menu.add(Group::Checkout, match facts.elsewhere(name) {
            Some(path) => act(format!("ref-open-{name}"), format!("Open the worktree of {name}"), RefAct::OpenWorktree(path)),
            None => act(format!("ref-checkout-{name}"), format!("Checkout {name}"), RefAct::Checkout(Target::Branch(name.clone()))),
        });
    }
    for name in remotes {
        menu.add(Group::Checkout, act(format!("ref-checkout-{name}"), format!("Checkout {name}"), RefAct::Checkout(Target::Remote(name.clone()))));
    }
    commit_items(menu, sha);
    for name in locals {
        menu.add(Group::Manage, act(format!("ref-rename-{name}"), format!("Rename {name}"), RefAct::Rename(name.clone())));
        let delete = act(format!("ref-delete-{name}"), format!("Delete {name}"), RefAct::DeleteBranch(name.clone()));
        menu.add(Group::Manage, match (Some(name.as_str()) == current, facts.elsewhere(name)) {
            (true, _) => delete.dimmed(format!("{name} is checked out: check out another branch first")),
            (false, Some(path)) => delete.dimmed(in_worktree(&path)),
            (false, None) => delete,
        });
    }
    menu.add(Group::Copy, act("ref-copy-name".into(), "Copy branch name".into(), RefAct::Copy { text: clicked.into(), what: "the branch name" }));
}

/// B4: a tag.
fn tag_menu(menu: &mut RefMenu<RefAct>, name: &str, sha: &str, annotated: bool, facts: &Facts) {
    if let Some(current) = facts.current().filter(|_| facts.head_sha() != Some(sha) && !facts.under_way)
        && matches!(facts.relation, Some(Relation::Ahead | Relation::Diverged))
    {
        menu.add(Group::Merge, act(format!("ref-merge-{name}"), format!("Merge {name} into {current}"), RefAct::Merge { source: name.into(), into: current.into() }));
        menu.add(Group::Merge, act(format!("ref-rebase-{name}"), format!("Rebase {current} onto {name}"), RefAct::Rebase { branch: current.into(), onto: name.into() }));
    }
    let detached_here = matches!(&facts.refs.head, Head::Detached { sha: head } if head == sha);
    if !detached_here {
        menu.add(Group::Checkout, act(format!("ref-checkout-{name}"), format!("Checkout the commit at {name}"), RefAct::Checkout(Target::Commit(sha.into()))));
    }
    commit_items(menu, sha);
    menu.add(Group::Manage, act(format!("ref-delete-tag-{name}"), format!("Delete {name} locally"), RefAct::DeleteTag(name.into())));
    if !annotated {
        menu.add(Group::Manage, act(format!("ref-annotate-{name}"), format!("Annotate {name}"), RefAct::Annotate { name: name.into(), at: sha.into() }));
    }
    menu.add(Group::Copy, act("ref-copy-name".into(), "Copy tag name".into(), RefAct::Copy { text: name.into(), what: "the tag name" }));
}

/// §B part 1 item 5: a worktree's row.
fn worktree_menu(menu: &mut RefMenu<RefAct>, subject: &Subject) {
    let Subject::Worktree { path, label, branch, main, current, locked, prunable } = subject else { return };
    if !*current && !*prunable {
        menu.add(Group::Checkout, item("worktree-open", format!("Open {label}"), RefAct::OpenWorktree(path.clone())));
    }
    if !*main {
        let unlock_first = |item: MenuItem<RefAct>| if *locked { item.dimmed("It is locked: unlock it first") } else { item };
        if !*current {
            menu.add(Group::Manage, unlock_first(item("worktree-remove", "Remove worktree", RefAct::RemoveWorktree { path: path.clone(), branch: None })));
            if let Some(branch) = branch {
                menu.add(Group::Manage, unlock_first(item("worktree-remove-branch", format!("Remove worktree and delete {branch}"), RefAct::RemoveWorktree { path: path.clone(), branch: Some(branch.clone()) })));
            }
        }
        let (key, words) = if *locked { ("worktree-unlock", "Unlock worktree") } else { ("worktree-lock", "Lock worktree") };
        menu.add(Group::Manage, item(key, words, RefAct::LockWorktree { path: path.clone(), lock: !*locked }));
    }
    if *prunable {
        menu.add(Group::Manage, item("worktree-prune", "Prune worktrees", RefAct::PruneWorktrees));
    }
    menu.add(Group::Copy, item("worktree-copy-path", "Copy path", RefAct::Copy { text: path.display().to_string(), what: "the path" }));
}

/// A ref dragged onto another, as B7 names them: its branch name, whether it is local, its commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dragged {
    pub name: String,
    pub local: bool,
    pub sha: String,
}

/// B7: what dropping `dragged` on `target` offers; `relation` is the dragged commit's to the target's. Empty when
/// the two are one commit or neither is a local branch.
pub fn drop_menu(dragged: &Dragged, target: &Dragged, relation: Relation, facts: &Facts) -> RefMenu<RefAct> {
    let mut menu = RefMenu::default();
    if dragged.sha == target.sha || dragged.name == target.name || facts.under_way {
        return menu;
    }
    let (d, t) = (dragged.name.as_str(), target.name.as_str());
    let dim_held = |name: &str, item: MenuItem<RefAct>| match facts.elsewhere(name).filter(|_| Some(name) != facts.current()) {
        Some(path) => item.dimmed(in_worktree(&path)),
        None => item,
    };
    if target.local {
        if relation == Relation::Ahead && dragged.local {
            menu.add(Group::Merge, dim_held(t, act(format!("drop-ff-{t}"), format!("Fast-forward {t} to {d}"), RefAct::FastForward { branch: t.into(), to: d.into() })));
        }
        if relation != Relation::Behind {
            menu.add(Group::Merge, dim_held(t, act(format!("drop-merge-{t}"), format!("Merge {d} into {t}"), RefAct::Merge { source: d.into(), into: t.into() })));
        }
    }
    if dragged.local && relation == Relation::Behind {
        menu.add(Group::Merge, dim_held(d, act(format!("drop-ff-{d}"), format!("Fast-forward {d} to {t}"), RefAct::FastForward { branch: d.into(), to: t.into() })));
        menu.add(Group::Merge, dim_held(d, act(format!("drop-merge-{d}"), format!("Merge {t} into {d}"), RefAct::Merge { source: t.into(), into: d.into() })));
    }
    if dragged.local && relation != Relation::Ahead {
        menu.add(Group::Merge, dim_held(d, act(format!("drop-rebase-{d}"), format!("Rebase {d} onto {t}"), RefAct::Rebase { branch: d.into(), onto: t.into() })));
    }
    menu
}

#[cfg(test)]
mod tests {
    use supraman_gitman::{Branch, RemoteBranch, Worktree};

    use super::*;

    fn refs() -> Refs {
        let branch = |name: &str, sha: &str| Branch { name: name.into(), sha: sha.into(), upstream: None };
        Refs {
            head: Head::Branch { name: "main".into(), sha: Some("h".into()) },
            branches: vec![branch("main", "h"), branch("old", "o"), branch("held", "w")],
            remotes: vec![RemoteBranch { name: "origin/old".into(), sha: "o".into() }],
            tags: Vec::new(),
            worktrees: vec![
                Worktree { path: "C:/repo".into(), branch: Some("main".into()), ..Worktree::default() },
                Worktree { path: "C:/wt".into(), branch: Some("held".into()), ..Worktree::default() },
            ],
            stashes: Vec::new(),
        }
    }

    fn labels(menu: RefMenu<RefAct>) -> Vec<String> {
        menu.items().into_iter().map(|i| format!("{}{}{}", if i.section.is_some() { "| " } else { "" }, i.label, if i.dimmed.is_some() { " (dimmed)" } else { "" })).collect()
    }

    fn facts(refs: &Refs, relation: Option<Relation>) -> Facts<'_> {
        Facts { refs, top: Path::new("C:/repo"), relation, under_way: false, graph: vec![item("ref-solo", "Solo", RefAct::Panel(RefAction::ToggleSolo(Vec::new())))] }
    }

    fn of(locals: &[&str], remotes: &[&str], sha: &str) -> Subject {
        Subject::Refs { locals: locals.iter().map(|s| s.to_string()).collect(), remotes: remotes.iter().map(|s| s.to_string()).collect(), sha: sha.into() }
    }

    #[test]
    fn a_branch_behind_head_offers_its_fast_forward_and_the_merge_into_it_in_gitkrakens_groups() {
        let refs = refs();
        let menu = build(&of(&["old"], &["origin/old"], "o"), &facts(&refs, Some(Relation::Behind)));
        assert_eq!(labels(menu), [
            "Fast-forward old to main", "Merge main into old",
            "| Checkout old", "Checkout origin/old",
            "| Create branch here",
            "| Rename old", "Delete old",
            "| Copy commit sha", "Copy branch name",
            "| Solo",
            "| Create tag here", "Create annotated tag here",
        ]);
    }

    #[test]
    fn the_current_branch_has_no_merge_or_checkout_and_its_delete_is_dimmed() {
        let refs = refs();
        let labels = labels(build(&of(&["main"], &[], "h"), &facts(&refs, Some(Relation::Same))));
        assert!(!labels.iter().any(|l| l.starts_with("Merge") || l.starts_with("Checkout") || l.starts_with("| Checkout")));
        assert!(labels.contains(&"Delete main (dimmed)".to_string()));
    }

    #[test]
    fn a_branch_in_another_worktree_opens_it_and_merges_into_it_are_dimmed() {
        let refs = refs();
        let labels = labels(build(&of(&["held"], &[], "w"), &facts(&refs, Some(Relation::Behind))));
        assert_eq!(labels[..3], ["Fast-forward held to main (dimmed)", "Merge main into held (dimmed)", "| Open the worktree of held"]);
        assert!(labels.contains(&"Delete held (dimmed)".to_string()));
    }

    #[test]
    fn a_diverged_remote_merges_and_rebases_and_an_ahead_one_fast_forwards_head() {
        let refs = refs();
        assert_eq!(labels(build(&of(&[], &["origin/old"], "o"), &facts(&refs, Some(Relation::Diverged))))[..2], ["Merge origin/old into main", "Rebase main onto origin/old"]);
        assert_eq!(labels(build(&of(&[], &["origin/old"], "o"), &facts(&refs, Some(Relation::Ahead))))[..3], ["Fast-forward main to origin/old", "Merge origin/old into main", "Rebase main onto origin/old"]);
        assert!(!labels(build(&of(&[], &["origin/old"], "o"), &facts(&refs, Some(Relation::Behind)))).iter().any(|l| l.contains("Merge")), "a remote behind HEAD has no merge");
    }

    #[test]
    fn a_tag_a_detached_head_and_a_worktree_have_their_own_items() {
        let refs = refs();
        let tag = Subject::Tag { name: "v1".into(), sha: "o".into(), annotated: false };
        assert_eq!(labels(build(&tag, &facts(&refs, Some(Relation::Diverged)))), [
            "Merge v1 into main", "Rebase main onto v1",
            "| Checkout the commit at v1",
            "| Create branch here",
            "| Delete v1 locally", "Annotate v1",
            "| Copy commit sha", "Copy tag name",
            "| Solo",
            "| Create tag here", "Create annotated tag here",
        ]);
        let detached = labels(build(&Subject::Detached { sha: "d".into() }, &facts(&refs, None)));
        assert_eq!(detached, ["Create branch here", "| Copy commit sha", "| Solo", "| Create tag here", "Create annotated tag here"]);
        let wt = Subject::Worktree { path: "C:/wt".into(), label: "held".into(), branch: Some("held".into()), main: false, current: false, locked: true, prunable: false };
        let none = Facts { graph: Vec::new(), ..facts(&refs, None) };
        assert_eq!(labels(build(&wt, &none)), ["Open held", "| Remove worktree (dimmed)", "Remove worktree and delete held (dimmed)", "Unlock worktree", "| Copy path"]);
    }

    #[test]
    fn a_drop_merges_into_the_target_and_rebases_the_dragged() {
        let refs = refs();
        let f = facts(&refs, None);
        let old = Dragged { name: "old".into(), local: true, sha: "o".into() };
        let main = Dragged { name: "main".into(), local: true, sha: "h".into() };
        assert_eq!(labels(drop_menu(&old, &main, Relation::Behind, &f)), ["Fast-forward old to main", "Merge main into old", "Rebase old onto main"]);
        assert_eq!(labels(drop_menu(&main, &old, Relation::Ahead, &f)), ["Fast-forward old to main", "Merge main into old"]);
        assert_eq!(labels(drop_menu(&old, &main, Relation::Diverged, &f)), ["Merge old into main", "Rebase old onto main"]);
        let remote = Dragged { name: "origin/old".into(), local: false, sha: "o".into() };
        assert_eq!(labels(drop_menu(&remote, &main, Relation::Diverged, &f)), ["Merge origin/old into main"]);
        assert!(drop_menu(&old, &Dragged { sha: "o".into(), ..main.clone() }, Relation::Same, &f).items().is_empty(), "one commit: nothing");
    }
}
