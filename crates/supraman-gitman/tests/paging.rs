use std::time::Instant;

use supraman_gitman::fixture::Generated;
use supraman_gitman::{FIRST_PAGE, NEXT_PAGE, history, refs};

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
