use supraman_gitman::write::branch::current_branch;
use supraman_gitman::write::remote::{PullChoice, PullMode};
use supraman_gitman::{Link, Refs, Remote, web_link};
use supraman_ui::menu::{MenuItem, item};

use super::RemoteAct;
use super::bar::words;
use crate::git_view::branch::menu::{Dragged, Group, RefAct, RefMenu, Subject};

/// What the remote items read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RemoteFacts {
    pub(crate) remotes: Vec<Remote>,
    pub(crate) pull: PullChoice,
    /// HEAD's branch.
    pub(crate) current: Option<String>,
}

impl RemoteFacts {
    pub(crate) fn new(remotes: Vec<Remote>, refs: &Refs, pull: PullChoice) -> RemoteFacts {
        RemoteFacts { remotes, pull, current: current_branch(refs).map(str::to_string) }
    }

    fn url(&self, remote: &str) -> Option<&str> {
        self.remotes.iter().find(|r| r.name == remote).map(|r| r.url.as_str())
    }
}

/// `origin/feat/x` as `(origin, feat/x)`: the longest remote name it starts with.
pub(crate) fn split_remote<'a>(remotes: &[Remote], name: &'a str) -> Option<(&'a str, &'a str)> {
    remotes
        .iter()
        .filter_map(|r| name.strip_prefix(r.name.as_str()).and_then(|rest| rest.strip_prefix('/')).map(|rest| (&name[..r.name.len()], rest)))
        .max_by_key(|(remote, _)| remote.len())
}

fn remote(key: String, label: String, act: RemoteAct) -> MenuItem<RefAct> {
    item(key, label, RefAct::Remote(act))
}

fn copy(key: String, label: String, url: String) -> MenuItem<RefAct> {
    item(key, label, RefAct::Copy { text: url, what: "the link" })
}

/// `subject`'s remote items, nothing without a remote.
pub(crate) fn add(menu: &mut RefMenu<RefAct>, subject: &Subject, f: &RemoteFacts) {
    if f.remotes.is_empty() {
        return;
    }
    match subject {
        Subject::Refs { locals, remotes, .. } => {
            for local in locals.iter().filter(|l| Some(l.as_str()) == f.current.as_deref()) {
                sync_items(menu, local, f);
            }
            for name in remotes {
                let Some((r, branch)) = split_remote(&f.remotes, name) else { continue };
                menu.add(Group::Manage, remote(format!("remote-delete-{name}"), format!("Delete {name} from {r}"), RemoteAct::DeleteOnRemote { remote: r.into(), refname: format!("refs/heads/{branch}") }));
                if let Some(url) = f.url(r).and_then(|u| web_link(u, Link::Branch(branch))) {
                    menu.add(Group::Copy, copy(format!("remote-link-{name}"), format!("Copy link to branch {name}"), url));
                }
            }
        }
        Subject::Tag { name, .. } => tag_items(menu, name, f),
        Subject::Remote { name } => {
            menu.add(Group::Sync, remote(format!("remote-fetch-{name}"), format!("Fetch {name}"), RemoteAct::Fetch(name.clone())));
            menu.add(Group::Manage, remote(format!("remote-edit-{name}"), format!("Edit {name}"), RemoteAct::EditRemote(name.clone())));
            menu.add(Group::Manage, remote(format!("remote-remove-{name}"), format!("Remove {name}"), RemoteAct::RemoveRemote(name.clone())));
        }
        _ => {}
    }
}

/// HEAD's branch's pull (each mode while the default is a fetch), push and set upstream.
fn sync_items(menu: &mut RefMenu<RefAct>, local: &str, f: &RemoteFacts) {
    match f.pull.mode() {
        Some(mode) => menu.add(Group::Sync, remote("remote-pull".into(), "Pull".into(), RemoteAct::Pull(mode))),
        None => {
            for (key, mode) in [("ff", PullMode::FastForwardIfPossible), ("ff-only", PullMode::FastForwardOnly), ("rebase", PullMode::Rebase)] {
                let choice = PullChoice::ALL.into_iter().find(|c| c.mode() == Some(mode)).unwrap_or_default();
                menu.add(Group::Sync, remote(format!("remote-pull-{key}"), words(choice).into(), RemoteAct::Pull(mode)));
            }
        }
    }
    menu.add(Group::Sync, remote("remote-push".into(), "Push".into(), RemoteAct::Push(local.into())));
    menu.add(Group::Sync, remote("remote-set-upstream".into(), "Set upstream".into(), RemoteAct::SetUpstream(local.into())));
}

/// The tag pushed to each remote (and all at once with several), deleted from each.
fn tag_items(menu: &mut RefMenu<RefAct>, tag: &str, f: &RemoteFacts) {
    let names: Vec<String> = f.remotes.iter().map(|r| r.name.clone()).collect();
    for (i, r) in names.iter().enumerate() {
        let push = remote(format!("remote-push-tag-{r}"), format!("Push {tag} to {r}"), RemoteAct::PushTag { tag: tag.into(), remotes: vec![r.clone()] });
        menu.add(Group::Sync, if i == 0 && names.len() > 1 { push.under(format!("Push {tag}")) } else { push });
    }
    if names.len() > 1 {
        menu.add(Group::Sync, remote("remote-push-tag-all".into(), format!("Push {tag} to all remotes"), RemoteAct::PushTag { tag: tag.into(), remotes: names.clone() }));
    }
    for r in &names {
        menu.add(Group::Manage, remote(format!("remote-delete-tag-{r}"), format!("Delete {tag} from {r}"), RemoteAct::DeleteOnRemote { remote: r.clone(), refname: format!("refs/tags/{tag}") }));
    }
}

/// A link to the commit on each remote whose host makes one.
pub(crate) fn commit_links(menu: &mut RefMenu<RefAct>, sha: &str, f: &RemoteFacts) {
    for r in &f.remotes {
        if let Some(url) = web_link(&r.url, Link::Commit(sha)) {
            menu.add(Group::Copy, copy(format!("remote-commit-link-{}", r.name), format!("Copy link to this commit on {}", r.name), url));
        }
    }
}

/// A local branch dropped on a remote branch pushed there.
pub(crate) fn drop_items(menu: &mut RefMenu<RefAct>, dragged: &Dragged, target: &Dragged, f: &RemoteFacts) {
    if !dragged.local || target.local || dragged.sha == target.sha {
        return;
    }
    if let Some((r, branch)) = split_remote(&f.remotes, &target.name) {
        let act = RemoteAct::PushTo { local: dragged.name.clone(), remote: r.into(), branch: branch.into() };
        menu.add(Group::Sync, remote(format!("drop-push-{}", target.name), format!("Push {} to {}", dragged.name, target.name), act));
    }
}

#[cfg(test)]
mod tests {
    use supraman_gitman::{Branch, Head};

    use super::*;

    fn facts(remotes: &[(&str, &str)], pull: PullChoice) -> RemoteFacts {
        let refs = Refs {
            head: Head::Branch { name: "main".into(), sha: Some("h".into()) },
            branches: vec![Branch { name: "main".into(), sha: "h".into(), upstream: None }],
            remotes: Vec::new(),
            tags: Vec::new(),
            worktrees: Vec::new(),
            stashes: Vec::new(),
        };
        RemoteFacts::new(remotes.iter().map(|(n, u)| Remote { name: n.to_string(), url: u.to_string(), push_url: None }).collect(), &refs, pull)
    }

    fn labels(menu: RefMenu<RefAct>) -> Vec<String> {
        menu.items().into_iter().map(|i| format!("{}{}", if i.section.is_some() { "| " } else { "" }, i.label)).collect()
    }

    fn of(locals: &[&str], remotes: &[&str]) -> Subject {
        Subject::Refs { locals: locals.iter().map(|s| s.to_string()).collect(), remotes: remotes.iter().map(|s| s.to_string()).collect(), sha: "h".into() }
    }

    #[test]
    fn heads_branch_syncs_a_remote_branch_deletes_there_and_links_on_a_known_host() {
        let f = facts(&[("origin", "git@github.com:a/b.git"), ("origin/x", "C:/x.git")], PullChoice::Rebase);
        let mut menu = RefMenu::default();
        add(&mut menu, &of(&["main"], &["origin/main", "origin/x/dev"]), &f);
        assert_eq!(labels(menu), ["Pull", "Push", "Set upstream", "| Delete origin/main from origin", "Delete origin/x/dev from origin/x", "| Copy link to branch origin/main"]);
        let mut other = RefMenu::default();
        add(&mut other, &of(&["feature"], &[]), &f);
        assert!(other.items().is_empty(), "another branch: no pull or push");
        let mut fetching = RefMenu::default();
        add(&mut fetching, &of(&["main"], &[]), &facts(&[("origin", "C:/o.git")], PullChoice::Fetch));
        assert_eq!(labels(fetching)[..3], ["Pull (fast-forward if possible)", "Pull (fast-forward only)", "Pull (rebase)"], "the default is a fetch: each pull");
    }

    #[test]
    fn a_tag_a_remote_row_a_drop_and_a_commit_have_their_remote_items() {
        let f = facts(&[("origin", "https://u:t@github.com/a/b.git"), ("mirror", "C:/m.git")], PullChoice::default());
        let mut tag = RefMenu::default();
        add(&mut tag, &Subject::Tag { name: "v1".into(), sha: "h".into(), annotated: false }, &f);
        assert_eq!(labels(tag), ["| Push v1 to origin", "Push v1 to mirror", "Push v1 to all remotes", "| Delete v1 from origin", "Delete v1 from mirror"]);
        let mut row = RefMenu::default();
        add(&mut row, &Subject::Remote { name: "origin".into() }, &f);
        assert_eq!(labels(row), ["Fetch origin", "| Edit origin", "Remove origin"]);
        let mut dropped = RefMenu::default();
        drop_items(&mut dropped, &Dragged { name: "main".into(), local: true, sha: "a".into() }, &Dragged { name: "mirror/main".into(), local: false, sha: "b".into() }, &f);
        assert_eq!(labels(dropped), ["Push main to mirror/main"]);
        let mut commit = RefMenu::default();
        commit_links(&mut commit, "abc", &f);
        let items = commit.items();
        assert_eq!(items.len(), 1, "only the host that makes a link");
        assert!(matches!(&items[0].action, RefAct::Copy { text, .. } if text == "https://github.com/a/b/commit/abc"), "no userinfo in the link");
        assert!(labels(RefMenu::default()).is_empty());
        let mut none = RefMenu::default();
        add(&mut none, &of(&["main"], &[]), &facts(&[], PullChoice::default()));
        assert!(none.items().is_empty(), "no remote: no items");
    }

    #[test]
    fn a_remote_branch_name_splits_at_its_longest_remote() {
        let remotes: Vec<Remote> = ["origin", "origin/x"].iter().map(|n| Remote { name: n.to_string(), url: String::new(), push_url: None }).collect();
        assert_eq!(split_remote(&remotes, "origin/x/dev"), Some(("origin/x", "dev")));
        assert_eq!(split_remote(&remotes, "origin/main"), Some(("origin", "main")));
        assert_eq!(split_remote(&remotes, "fork/main"), None);
    }
}
