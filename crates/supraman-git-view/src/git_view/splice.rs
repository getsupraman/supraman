//! A graph refresh reads the first page only; the commits loaded below it stay when the walk below is the same.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::SharedString;
use supraman_gitman::Commit;

/// The commits a tab has loaded, for a refresh to keep below its first page.
pub(crate) struct Loaded {
    pub(crate) tips: Arc<[String]>,
    /// Each loaded commit's sha and parents, in the graph's order.
    pub(crate) commits: Vec<(SharedString, Vec<SharedString>)>,
    pub(crate) more: bool,
}

/// Where in `loaded.commits` the commits below `page`, the first page read now from `tips`, carry on: `None`
/// when the page reaches past them or the walk changed (a commit gone, a tip moved off the page or added below it).
pub(crate) fn kept_from(loaded: &Loaded, tips: &[String], page: &[Commit]) -> Option<usize> {
    let old = &loaded.commits;
    let at: HashMap<&str, usize> = old.iter().enumerate().map(|(i, (sha, _))| (sha.as_ref(), i)).collect();
    let first = page.iter().position(|c| at.contains_key(c.sha.as_str()))?;
    let from = at[page[first].sha.as_str()];
    let below = from + page.len() - first;
    if below >= old.len() || page[first..].iter().zip(&old[from..below]).any(|(c, (sha, _))| c.sha != sha.as_ref()) {
        return None;
    }
    let read: HashSet<&str> = page.iter().map(|c| c.sha.as_str()).collect();
    let known = |sha: &str| read.contains(sha) || at.contains_key(sha);
    let was: HashSet<&str> = loaded.tips.iter().map(String::as_str).collect();
    let now: HashSet<&str> = tips.iter().map(String::as_str).collect();
    let unchanged = old[..from].iter().all(|(sha, _)| read.contains(sha.as_ref()))
        && page[..first].iter().flat_map(|c| &c.parents).all(|p| known(p))
        && was.difference(&now).all(|t| read.contains(t))
        && now.difference(&was).all(|t| known(t));
    unchanged.then_some(below)
}

#[cfg(test)]
mod tests {
    use supraman_gitman::Signature;

    use super::*;

    fn commit(sha: &str, parents: &[&str]) -> Commit {
        let who = Signature { name: "A".into(), email: "a@x".into(), time: 0 };
        Commit { sha: sha.into(), parents: parents.iter().map(|p| p.to_string()).collect(), author: who.clone(), committer: who, message: String::new() }
    }

    /// `c5 … c0` in a line, `c0` the root.
    fn line(n: usize) -> Vec<Commit> {
        (0..n).rev().map(|i| if i == 0 { commit("c0", &[]) } else { commit(&format!("c{i}"), &[&format!("c{}", i - 1)]) }).collect()
    }

    fn loaded(commits: &[Commit], tips: &[&str]) -> Loaded {
        let commits = commits.iter().map(|c| (SharedString::from(c.sha.clone()), c.parents.iter().map(|p| SharedString::from(p.clone())).collect())).collect();
        Loaded { tips: tips.iter().map(|t| t.to_string()).collect(), commits, more: true }
    }

    fn tips(t: &[&str]) -> Vec<String> {
        t.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn splice_keeps_what_lies_below_a_page_with_new_commits_on_top() {
        let old = line(10);
        let was = loaded(&old, &["c9"]);
        let mut page = vec![commit("n1", &["n0"]), commit("n0", &["c9"])];
        page.extend(old[..4].iter().cloned());
        assert_eq!(kept_from(&was, &tips(&["n1"]), &page), Some(4), "c5 down to c0 stay");
        assert_eq!(kept_from(&was, &tips(&["c9"]), &old[..4]), Some(4), "nothing new: the rows below the page stay");
        assert_eq!(kept_from(&was, &tips(&["n1", "c9", "c2"]), &page), Some(4), "a new tip on a loaded commit");
    }

    #[test]
    fn splice_reads_afresh_when_the_walk_below_may_differ() {
        let old = line(10);
        let was = loaded(&old, &["c9", "c1"]);
        let mut page = vec![commit("x9", &["c8"])];
        page.extend(old[1..5].iter().cloned());
        assert_eq!(kept_from(&was, &tips(&["x9", "c1"]), &page), None, "c9 rewritten away");
        let mut page = vec![commit("n0", &["old"])];
        page.extend(old[..4].iter().cloned());
        assert_eq!(kept_from(&was, &tips(&["n0", "c9", "c1"]), &page), None, "a new commit whose parent may lie below the page");
        assert_eq!(kept_from(&was, &tips(&["c9"]), &old[..4]), None, "the tip c1 is gone and its commits may be too");
        assert_eq!(kept_from(&was, &tips(&["c9", "c1", "far"]), &old[..4]), None, "a tip nowhere loaded");
        assert_eq!(kept_from(&was, &tips(&["c9", "c1"]), &old), None, "the page holds everything");
        let page = [old[0].clone(), old[2].clone(), old[3].clone()];
        assert_eq!(kept_from(&was, &tips(&["c9", "c1"]), &page), None, "the order changed");
    }
}
