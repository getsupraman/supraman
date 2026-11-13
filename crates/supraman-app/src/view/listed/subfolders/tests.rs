use std::sync::mpsc;

use supraman_core::folders::subfolders;
use supraman_core::paths::folder_name;
use supraman_core::testing::eventually;

use super::*;

fn names(parent: &std::path::Path) -> Vec<String> {
    subfolders(parent).iter().map(|p| folder_name(p)).collect()
}

#[test]
fn a_child_folder_made_or_removed_under_a_watched_folder_is_a_change_and_reads_in_or_out() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().canonicalize().unwrap();
    std::fs::create_dir(parent.join("api")).unwrap();
    let (tx, rx) = mpsc::channel::<()>();
    let tx = Mutex::new(tx);
    let mut watchers = Watchers::default();
    watchers.sync(&[parent.clone()], Arc::new(move || {
        tx.lock().unwrap().send(()).ok();
    }));
    assert_eq!(watchers.watched(), [path_key(&parent)]);
    let batch = |what: &str| eventually(Duration::from_secs(10), what, |_| (), || rx.recv_timeout(Duration::from_secs(10)).ok());

    std::fs::create_dir(parent.join("web")).unwrap();
    batch("a change for web made");
    assert!(names(&parent).contains(&"web".to_string()), "read in once its change came");
    while rx.try_recv().is_ok() {}
    std::fs::remove_dir(parent.join("api")).unwrap();
    batch("a change for api removed");
    assert_eq!(names(&parent), ["web"]);

    watchers.sync(&[], Arc::new(|| ()));
    assert!(watchers.watched().is_empty(), "a folder no longer showing them is not watched");
}
