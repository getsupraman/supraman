use std::fs;
use std::path::Path;
use std::process::Output;

fn sm(home: &Path, args: &[&str]) -> Output {
    supraman_core::process::command(env!("CARGO_BIN_EXE_sm"))
        .args(args)
        .env("SUPRAMAN_HOME", home)
        .env_remove("APPDATA")
        .output()
        .unwrap()
}

fn ok(home: &Path, args: &[&str]) -> String {
    let out = sm(home, args);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn folders_and_groups_are_added_listed_moved_out_and_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let (a_s, b_s) = (a.to_str().unwrap(), b.to_str().unwrap());

    assert_eq!(ok(&home, &["folder", "list"]), "no folders listed\n");
    ok(&home, &["folder", "add", a_s]);
    ok(&home, &["group", "add", "tools"]);
    ok(&home, &["folder", "add", b_s, "--group", "tools"]);
    ok(&home, &["folder", "add", a_s, "--group", "tools", "--at", "0"]);
    assert_eq!(ok(&home, &["folder", "list"]), format!("{a_s}\ntools/\n  {a_s}\n  {b_s}\n"), "the same path in a group and at the top level");

    let twice = sm(&home, &["folder", "add", a_s]);
    assert_eq!(twice.status.code(), Some(1), "the top level lists it already: {twice:?}");
    assert_eq!(sm(&home, &["group", "add", "Bad Name"]).status.code(), Some(1));
    assert_eq!(sm(&home, &["group", "add", "tools"]).status.code(), Some(1), "a taken name");
    assert_eq!(sm(&home, &["folder", "add", &tmp.path().join("missing").display().to_string()]).status.code(), Some(1));

    ok(&home, &["folder", "remove", a_s]);
    ok(&home, &["group", "rename", "tools", "kit"]);
    assert_eq!(ok(&home, &["folder", "list"]), format!("kit/\n  {a_s}\n  {b_s}\n"));
    ok(&home, &["group", "remove", "kit"]);
    assert_eq!(ok(&home, &["folder", "list"]), format!("{a_s}\n{b_s}\n"), "its folders stay at the top level");
    ok(&home, &["group", "add", "gone", "--at", "0"]);
    ok(&home, &["folder", "add", b_s, "--group", "gone"]);
    ok(&home, &["group", "remove", "gone", "--with-folders"]);
    assert_eq!(ok(&home, &["folder", "list"]), format!("{a_s}\n{b_s}\n"));
    assert!(a.is_dir() && b.is_dir(), "no folder on disk is touched");
}

#[test]
fn a_broken_folders_toml_is_refused_and_kept() {
    let home = tempfile::tempdir().unwrap();
    let file = home.path().join("folders.toml");
    fs::write(&file, "entry = ").unwrap();
    let out = sm(home.path(), &["group", "add", "x"]);
    assert_eq!(out.status.code(), Some(2), "a file that does not parse: {out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("folders.toml"), "{out:?}");
    assert_eq!(fs::read_to_string(&file).unwrap(), "entry = ");
}
