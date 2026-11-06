use std::time::{Duration, Instant};

use supraman_gitman::fixture::Generated;
use supraman_core::process;
use supraman_gitman::{FIRST_PAGE, History, NEXT_PAGE, history, refs};

#[test]
fn the_first_and_next_page_of_a_long_history() {
    let made = Instant::now();
    let generated = Generated::new(20_000);
    println!("fast-import of 20000 commits: {:?}", made.elapsed());
    let repo = generated.repo();
    let refs = refs(&repo).unwrap();
    let tips = refs.tips();
    assert!(refs.branches.len() > 10 && refs.tags.len() == 20, "{} branches, {} tags", refs.branches.len(), refs.tags.len());

    let started = Instant::now();
    let first = history(&repo, &tips, 0, FIRST_PAGE).unwrap();
    let first_took = started.elapsed();
    let started = Instant::now();
    let next = history(&repo, &tips, FIRST_PAGE, NEXT_PAGE).unwrap();
    let next_took = started.elapsed();
    println!("first page ({} + 1 asked): {first_took:?}; next page ({} + 1 asked, after {}): {next_took:?}", FIRST_PAGE, NEXT_PAGE, FIRST_PAGE);

    assert_eq!((first.commits.len(), first.more), (FIRST_PAGE, true));
    assert_eq!((next.commits.len(), next.more), (NEXT_PAGE, true));
    assert_eq!(first.commits[0].message, "Commit 20000\n");
    assert!(first.commits.iter().any(|c| c.parents.len() == 2), "merges are in the page");
    assert!(first.commits.windows(2).all(|w| w[0].committer.time >= w[1].committer.time), "date order");
    assert!(first.commits.last().unwrap().committer.time > next.commits[0].committer.time, "the next page follows on");
    let whole = history(&repo, &tips, 0, 30_000).unwrap();
    assert_eq!((whole.commits.len(), whole.more), (20_000, false));
    assert!(first_took.as_secs() < 20 && next_took.as_secs() < 20);
}

#[test]
fn a_walk_pages_on_from_one_rev_list_as_one_shot_reads_do() {
    let generated = Generated::new(FIRST_PAGE + 10 * NEXT_PAGE + 50);
    let repo = generated.repo();
    let tips = refs(&repo).unwrap().tips();
    let before = process::spawned_here();
    let mut walk = History::start(&repo, &tips, 0).unwrap();
    let mut pages = vec![walk.page(FIRST_PAGE).unwrap()];
    for _ in 0..10 {
        pages.push(walk.page(NEXT_PAGE).unwrap());
    }
    let last = walk.page(NEXT_PAGE).unwrap();
    assert_eq!(process::spawned_here() - before, 1, "one rev-list for the first page and ten more");
    assert_eq!((last.commits.len(), last.more, walk.at(), walk.pid()), (50, false, FIRST_PAGE + 10 * NEXT_PAGE + 50, None), "git ended with the walk");
    for (i, page) in pages.iter().enumerate() {
        let (skip, count) = if i == 0 { (0, FIRST_PAGE) } else { (FIRST_PAGE + (i - 1) * NEXT_PAGE, NEXT_PAGE) };
        assert_eq!(page, &history(&repo, &tips, skip, count).unwrap(), "page {i}");
    }
    let mut later = History::start(&repo, &tips, FIRST_PAGE + 3 * NEXT_PAGE).unwrap();
    assert_eq!(later.page(NEXT_PAGE).unwrap(), pages[4], "a walk started at an offset reads what the stream read there");
}

#[test]
fn a_dropped_walk_kills_its_git() {
    let generated = Generated::new(FIRST_PAGE + 20 * NEXT_PAGE);
    let repo = generated.repo();
    let tips = refs(&repo).unwrap().tips();
    let mut walk = History::start(&repo, &tips, 0).unwrap();
    assert!(walk.page(FIRST_PAGE).unwrap().more);
    let pid = walk.pid().expect("git waits to write the rest");
    assert!(process::alive(pid));
    drop(walk);
    assert!(process::wait_any_exit(&[pid], Duration::from_secs(10)), "git is gone with its walk");
}
