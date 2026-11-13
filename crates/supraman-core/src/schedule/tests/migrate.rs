use std::path::Path;

use super::*;

/// The doc example as the earlier per-workspace file held it: `id` in `folder`, active.
fn old_table(id: &str, folder: &str) -> String {
    let table = &DOC_EXAMPLE[DOC_EXAMPLE.find("[[schedule]]").unwrap()..];
    table.replace(r#"id = "autofix-prod-bugs""#, &format!(r#"id = "{id}""#)).replace(r#"folder = "Nomos""#, &format!(r#"folder = "{folder}""#)).replace(r#"status = "pending""#, r#"status = "active""#)
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn two_workspaces_schedules_move_to_their_folders_files_pending_with_their_runs_files_and_secrets_once() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let (a, b, c) = (tmp.path().join("A"), tmp.path().join("B"), tmp.path().join("C"));
    crate::folders::migrate::write_workspace(&home, "alpha", &[&a, &b]);
    crate::folders::migrate::write_workspace(&home, "beta", &[&c]);
    let dir = home.join("schedules");
    write(&dir.join("alpha.toml"), &format!("version = 1\n\n{}\n{}", old_table("s1", "A"), old_table("s2", "Lost")));
    write(&dir.join("beta.toml"), &format!("version = 1\n\n{}", old_table("s3", "C")));
    let started = |run: &str, id: &str| RunRecord { ts: 1, run: run.into(), schedule: id.into(), event: RunEvent::Started { trigger: Fired::Task, fingerprint: "f".into(), fm_pid: 1 } };
    append_run(&home, "alpha", &started("r1", "s1")).unwrap();
    append_run(&home, "alpha", &started("r2", "s2")).unwrap();
    append_run(&home, "beta", &started("r3", "s3")).unwrap();
    write(&dir.join("live").join("alpha").join("s1.json"), "{}");
    write(&dir.join("live").join("alpha").join("s1-x.json"), "{}");
    write(&dir.join("state").join("beta").join("s3").join("seen.txt"), "1");
    let store = MemoryStore::default();
    store.set(&secret_key("beta", "s3", "SENTRY_TOKEN"), "v").unwrap();

    let done = migrate(&home, &store).unwrap();
    let (ka, kc) = (folder_key(&a), folder_key(&c));
    assert_eq!(done.moved, [("alpha".into(), "s1".into(), a.clone()), ("beta".into(), "s3".into(), c.clone())]);
    assert_eq!(done.unplaced.len(), 1, "{:?}", done.unplaced);
    for (folder, id) in [(&a, "s1"), (&c, "s3")] {
        let moved = load(&home, folder).unwrap();
        assert_eq!(moved.iter().map(|s| (s.id.as_str(), s.status)).collect::<Vec<_>>(), [(id, Status::Pending)], "pending until Approve");
        assert_eq!(effective_status(&store, folder, &moved[0]), Status::Pending);
    }
    assert!(load(&home, &b).unwrap().is_empty());
    assert_eq!(runs(&home, &ka).unwrap().iter().map(|r| r.run.as_str()).collect::<Vec<_>>(), ["r1"]);
    assert_eq!(runs(&home, &kc).unwrap().iter().map(|r| r.run.as_str()).collect::<Vec<_>>(), ["r3"]);
    assert!(runs_path(&home, "alpha").unwrap().exists(), "the records of the schedule left behind stay");
    assert!(!runs_path(&home, "beta").unwrap().exists());
    assert!(dir.join("live").join(&ka).join("s1.json").exists() && dir.join("live").join("alpha").join("s1-x.json").exists(), "its own files only");
    assert!(dir.join("state").join(&kc).join("s3").join("seen.txt").exists() && !dir.join("state").join("beta").exists());
    assert_eq!(store.get(&secret_key(&kc, "s3", "SENTRY_TOKEN")).unwrap().as_deref(), Some("v"));
    assert_eq!(store.get(&secret_key("beta", "s3", "SENTRY_TOKEN")).unwrap(), None);
    assert!(!dir.join("alpha.toml").exists() && !dir.join("beta.toml").exists());
    assert!(dir.join(RESYNC).exists(), "the app's next sync removes the former task folders");

    let (all, notices) = load_all(&home);
    assert_eq!(all.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), { let mut k = vec![ka.as_str(), kc.as_str()]; k.sort(); k });
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("s2") && notices[0].contains("alpha") && notices[0].contains("'Lost'") && notices[0].contains("unplaced"), "{}", notices[0]);

    fs::remove_file(dir.join(RESYNC)).unwrap();
    assert_eq!(migrate(&home, &store).unwrap(), Migrated::default(), "it runs once");
    assert!(!dir.join(RESYNC).exists());
}

#[test]
fn a_home_without_the_earlier_files_migrates_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("A");
    edit(tmp.path(), &folder, |all| {
        all.push(example());
        Ok(())
    })
    .unwrap();
    assert_eq!(migrate(tmp.path(), &MemoryStore::default()).unwrap(), Migrated::default());
    assert_eq!(load(tmp.path(), &folder).unwrap().len(), 1);
}
