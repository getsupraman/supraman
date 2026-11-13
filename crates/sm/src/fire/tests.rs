use super::*;

#[test]
fn live_ids_lists_each_schedule_with_a_live_or_a_dead_file_once() {
    let home = tempfile::tempdir().unwrap();
    let dir = schedule::live_dir(home.path(), "demo").unwrap();
    fs::create_dir_all(&dir).unwrap();
    for name in ["gate.json", "gate.dead.json", "kept.dead.json", "run.json", "run.lock", "run.restart", "idle.lock"] {
        fs::write(dir.join(name), b"{}").unwrap();
    }
    assert_eq!(live_ids(home.path(), "demo").into_iter().collect::<Vec<_>>(), ["gate", "kept", "run"]);
}

#[test]
fn live_ids_of_a_folder_key_without_a_live_folder_is_empty() {
    let home = tempfile::tempdir().unwrap();
    assert!(live_ids(home.path(), "demo").is_empty());
}
