use std::time::Instant;

use super::*;
use crate::fixture::{Fixture, Generated};
use crate::{Branch, Head, RemoteBranch, Signature, Tag, Worktree, history, refs};
use EdgeKind::{Ending as E, PassThrough as P, Starting as S};

fn commit(sha: &str, parents: &[&str]) -> Commit {
    let who = Signature { name: "A".into(), email: "a@x".into(), time: 0 };
    Commit { sha: sha.into(), parents: parents.iter().map(|p| p.to_string()).collect(), author: who.clone(), committer: who, message: String::new() }
}

fn no_refs() -> Refs {
    Refs { head: Head::Branch { name: "main".into(), sha: None }, branches: vec![], remotes: vec![], tags: vec![], worktrees: vec![] }
}

fn branch(name: &str, sha: &str) -> Branch {
    Branch { name: name.into(), sha: sha.into(), upstream: None }
}

fn remote(name: &str, sha: &str) -> RemoteBranch {
    RemoteBranch { name: name.into(), sha: sha.into() }
}

fn tag(name: &str, sha: &str) -> Tag {
    Tag { name: name.into(), sha: sha.into(), annotated: false, commit: Some(sha.into()) }
}

fn lay(commits: &[(&str, &[&str])]) -> Vec<Row> {
    let commits: Vec<Commit> = commits.iter().map(|(s, p)| commit(s, p)).collect();
    Layout::new(&no_refs(), false).extend(&commits)
}

type Shape = (usize, Vec<(EdgeKind, usize, usize)>);

/// Each row's lane and its edges as `(kind, lane, node)`, in the order the row lists them.
fn shape(rows: &[Row]) -> Vec<Shape> {
    rows.iter().map(|r| (r.lane, r.edges.iter().map(|e| (e.kind, e.lane, e.node)).collect())).collect()
}

#[test]
fn layout_linear() {
    let rows = lay(&[("c", &["b"]), ("b", &["a"]), ("a", &[])]);
    assert_eq!(shape(&rows), vec![(0, vec![(S, 0, 0)]), (0, vec![(E, 0, 0), (S, 0, 0)]), (0, vec![(E, 0, 0)])]);
    assert!(rows.iter().all(|r| r.max_lane == 0 && r.colour == 0));
}

#[test]
fn layout_branch_and_merge_takes_the_lower_lane() {
    let rows = lay(&[("m", &["a", "f"]), ("f", &["a"]), ("a", &["o"]), ("o", &[])]);
    assert_eq!(
        shape(&rows),
        vec![
            (0, vec![(S, 0, 0), (S, 1, 0)]),
            (1, vec![(P, 0, 0), (E, 1, 1), (S, 1, 1)]),
            (0, vec![(E, 0, 0), (E, 1, 0), (S, 0, 0)]),
            (0, vec![(E, 0, 0)]),
        ]
    );
    assert_eq!(rows.iter().map(|r| r.max_lane).collect::<Vec<_>>(), [1, 1, 1, 0]);
    assert_eq!(rows[1].colour, 1);
}

#[test]
fn layout_two_branches_off_one_commit() {
    let rows = lay(&[("x", &["a"]), ("y", &["a"]), ("a", &[])]);
    assert_eq!(shape(&rows), vec![(0, vec![(S, 0, 0)]), (1, vec![(P, 0, 0), (S, 1, 1)]), (0, vec![(E, 0, 0), (E, 1, 0)])]);
}

#[test]
fn layout_octopus_merge() {
    let rows = lay(&[("m", &["a", "b", "c"]), ("a", &["o"]), ("b", &["o"]), ("c", &["o"]), ("o", &[])]);
    assert_eq!(
        shape(&rows),
        vec![
            (0, vec![(S, 0, 0), (S, 1, 0), (S, 2, 0)]),
            (0, vec![(E, 0, 0), (P, 1, 1), (P, 2, 2), (S, 0, 0)]),
            (1, vec![(P, 0, 0), (E, 1, 1), (P, 2, 2), (S, 1, 1)]),
            (2, vec![(P, 0, 0), (P, 1, 1), (E, 2, 2), (S, 2, 2)]),
            (0, vec![(E, 0, 0), (E, 1, 0), (E, 2, 0)]),
        ]
    );
}

#[test]
fn layout_criss_cross_merge() {
    let rows = lay(&[("m1", &["a", "b"]), ("m2", &["b", "a"]), ("a", &["o"]), ("b", &["o"]), ("o", &[])]);
    assert_eq!(
        shape(&rows),
        vec![
            (0, vec![(S, 0, 0), (S, 1, 0)]),
            (2, vec![(P, 0, 0), (P, 1, 1), (S, 2, 2), (S, 0, 2)]),
            (0, vec![(E, 0, 0), (P, 1, 1), (P, 2, 2), (S, 0, 0)]),
            (1, vec![(P, 0, 0), (E, 1, 1), (E, 2, 1), (S, 1, 1)]),
            (0, vec![(E, 0, 0), (E, 1, 0)]),
        ]
    );
    assert_eq!(rows[1].max_lane, 2);
}

#[test]
fn layout_two_roots_joined() {
    let rows = lay(&[("m", &["a", "b"]), ("a", &[]), ("b", &[]), ("c", &[])]);
    assert_eq!(shape(&rows), vec![(0, vec![(S, 0, 0), (S, 1, 0)]), (0, vec![(E, 0, 0), (P, 1, 1)]), (1, vec![(E, 1, 1)]), (0, vec![])]);
}

#[test]
fn layout_wip_row_on_head() {
    let mut refs = no_refs();
    refs.head = Head::Branch { name: "main".into(), sha: Some("b".into()) };
    let rows = Layout::new(&refs, true).extend(&[commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);
    assert_eq!(rows[0].sha, None);
    assert_eq!(
        shape(&rows),
        vec![(0, vec![(S, 0, 0)]), (1, vec![(P, 0, 0), (S, 1, 1)]), (0, vec![(E, 0, 0), (E, 1, 0), (S, 0, 0)]), (0, vec![(E, 0, 0)])]
    );
    let dashed: Vec<Vec<bool>> = rows.iter().map(|r| r.edges.iter().map(|e| e.dashed).collect()).collect();
    assert_eq!(dashed, vec![vec![true], vec![true, false], vec![true, false, false], vec![false]]);
}

#[test]
fn layout_of_the_fixture_repo() {
    let f = Fixture::new();
    let repo = f.repo();
    let refs = refs(&repo).unwrap();
    let page = history(&repo, &refs.tips(), 0, 100).unwrap();
    let rows = Layout::new(&refs, true).extend(&page.commits);
    let order: Vec<Option<&str>> = rows.iter().map(|r| r.sha.as_deref()).collect();
    let labels = ["ahead", "merge", "main_work", "rename", "second", "root"];
    let expected: Vec<Option<&str>> = [None].into_iter().chain(labels.iter().map(|l| Some(f.sha(l)))).collect();
    assert_eq!(order, expected);
    assert_eq!(
        shape(&rows),
        vec![
            (0, vec![(S, 0, 0)]),
            (0, vec![(E, 0, 0), (S, 0, 0)]),
            (0, vec![(E, 0, 0), (S, 0, 0), (S, 1, 0)]),
            (0, vec![(E, 0, 0), (P, 1, 1), (S, 0, 0)]),
            (1, vec![(P, 0, 0), (E, 1, 1), (S, 1, 1)]),
            (0, vec![(E, 0, 0), (E, 1, 0), (S, 0, 0)]),
            (0, vec![(E, 0, 0)]),
        ]
    );
    let names = |r: &Row| r.refs.iter().map(|g| g.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&rows[1]), ["main"]);
    assert_eq!(names(&rows[2]), ["main"]);
    assert_eq!(names(&rows[3]), ["stale", "light"]);
    assert_eq!(names(&rows[4]), ["feature"]);
    assert_eq!(names(&rows[5]), ["side", "v1"]);
    assert_eq!(rows[5].refs[0].kind(), RefKind::Worktree);
    assert_eq!(rows[6].ghost.as_ref().unwrap()[0].name, "side");
}

#[test]
fn layout_pages_extend_one_pass() {
    let g = Generated::new(700);
    let repo = g.repo();
    let refs = refs(&repo).unwrap();
    let tips = refs.tips();
    let all = history(&repo, &tips, 0, 1000).unwrap();
    assert_eq!(all.commits.len(), 700);
    let whole = Layout::new(&refs, true).extend(&all.commits);
    let mut paged = Layout::new(&refs, true);
    let mut rows = Vec::new();
    for (skip, count) in [(0, 250), (250, 200), (450, 300)] {
        let page = history(&repo, &tips, skip, count).unwrap();
        rows.extend(paged.extend(&page.commits));
    }
    assert_eq!(rows, whole);
    assert!(whole.iter().any(|r| r.max_lane > 0));
}

#[test]
fn layout_an_unloaded_parent_keeps_its_lane_open() {
    let mut layout = Layout::new(&no_refs(), false);
    let first = layout.extend(&[commit("m", &["a", "f"]), commit("f", &["x"])]);
    assert_eq!(shape(&first)[1], (1, vec![(P, 0, 0), (E, 1, 1), (S, 1, 1)]));
    let next = layout.extend(&[commit("a", &["x"]), commit("x", &[])]);
    assert_eq!(shape(&next), vec![(0, vec![(E, 0, 0), (P, 1, 1), (S, 0, 0)]), (0, vec![(E, 0, 0), (E, 1, 0)])]);
}

fn ref_order_refs() -> Refs {
    Refs {
        head: Head::Branch { name: "main".into(), sha: Some("x".into()) },
        branches: vec![branch("main", "x"), branch("zeta", "x"), branch("alpha", "x"), branch("Beta", "x"), branch("wt", "x")],
        remotes: vec![remote("origin/main", "x"), remote("upstream/main", "x"), remote("origin/dev", "x"), remote("origin/Beta", "x")],
        tags: vec![tag("v2", "x"), tag("main", "x")],
        worktrees: vec![Worktree { branch: Some("wt".into()), ..Worktree::default() }],
    }
}

#[test]
fn layout_ref_groups_and_their_order() {
    let rows = Layout::new(&ref_order_refs(), false).extend(&[commit("x", &[])]);
    let groups = &rows[0].refs;
    let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["main", "wt", "Beta", "alpha", "zeta", "dev", "v2"]);
    let main: Vec<(RefKind, &str, bool)> = groups[0].refs.iter().map(|r| (r.kind, r.name.as_str(), r.head)).collect();
    assert_eq!(
        main,
        [(RefKind::Local, "main", true), (RefKind::Remote, "origin/main", false), (RefKind::Remote, "upstream/main", false), (RefKind::Tag, "main", false)]
    );
    assert_eq!(groups[1].kind(), RefKind::Worktree);
    assert_eq!(rows[0].more(), 6);
    assert!(!rows[0].detached_head);
}

#[test]
fn layout_detached_head_marks_its_commit() {
    let mut refs = ref_order_refs();
    refs.head = Head::Detached { sha: "y".into() };
    let rows = Layout::new(&refs, true).extend(&[commit("y", &["x"]), commit("x", &[])]);
    assert_eq!(rows.iter().map(|r| r.detached_head).collect::<Vec<_>>(), [false, true, false]);
    assert!(rows[2].refs.iter().all(|g| !g.head()));
    assert_eq!(rows[2].refs[0].name, "wt");
    assert_eq!(rows[1].more(), 0);
}

#[test]
fn layout_ghost_refs_follow_the_first_parent() {
    let mut refs = no_refs();
    refs.branches = vec![branch("main", "m"), branch("topic", "t")];
    refs.remotes = vec![remote("origin/topic", "t")];
    refs.tags = vec![tag("v1", "b")];
    let commits = [("m", &["a", "t"][..]), ("t", &["s"]), ("a", &["b"]), ("s", &["b"]), ("b", &["o"]), ("o", &[])];
    let commits: Vec<Commit> = commits.iter().map(|(s, p)| commit(s, p)).collect();
    let rows = Layout::new(&refs, false).extend(&commits);
    let ghost = |i: usize| rows[i].ghost.as_ref().map(|g| g.iter().map(|g| g.name.clone()).collect::<Vec<_>>());
    assert_eq!(ghost(0), None);
    assert_eq!(ghost(1), None);
    assert_eq!(ghost(2), Some(vec!["main".to_string()]));
    assert_eq!(ghost(3), Some(vec!["topic".to_string()]));
    assert_eq!(ghost(4), None);
    assert_eq!(ghost(5), None);
    assert_eq!(rows[3].ghost.as_ref().unwrap()[0].refs.len(), 2);

    let second_parent_first = [("m", &["x", "a"][..]), ("x", &["a"]), ("a", &[])];
    let commits: Vec<Commit> = second_parent_first.iter().map(|(s, p)| commit(s, p)).collect();
    let mut refs = no_refs();
    refs.branches = vec![branch("main", "m"), branch("topic", "x")];
    let rows = Layout::new(&refs, false).extend(&commits);
    assert_eq!(rows[2].ghost.as_ref().unwrap()[0].name, "topic");
}

/// 10,000 rows: a main line, every 10th commit a merge of a one-commit topic, every 50th of a 30-commit branch.
fn synthetic(n: usize) -> Vec<Commit> {
    let sha = |i: usize| format!("{i:040x}");
    let mut commits = Vec::with_capacity(n);
    for i in 0..n {
        let parents: Vec<String> = match i {
            _ if i + 1 >= n => vec![],
            _ if i % 50 == 0 && i + 32 < n => vec![sha(i + 1), sha(i + 31)],
            _ if i % 10 == 0 && i + 3 < n => vec![sha(i + 1), sha(i + 2)],
            _ => vec![sha(i + 1)],
        };
        commits.push(Commit { parents, ..commit(&sha(i), &[]) });
    }
    commits
}

#[test]
fn layout_ten_thousand_rows_fast() {
    let commits = synthetic(10_000);
    let start = Instant::now();
    let rows = Layout::new(&no_refs(), true).extend(&commits);
    let took = start.elapsed();
    println!("layout of {} rows: {took:?}", rows.len());
    assert_eq!(rows.len(), 10_000);
    assert!(rows.iter().any(|r| r.max_lane >= 1));
    if !cfg!(debug_assertions) {
        assert!(took.as_millis() < 50, "{took:?}");
    }
}
