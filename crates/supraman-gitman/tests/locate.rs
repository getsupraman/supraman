use std::path::{Path, PathBuf};

use supraman_core::Settings;
use supraman_gitman::{FLOOR, GitmanSettings, Located, SECTION, Version, check, judge, locate};

#[test]
fn an_override_to_nothing_is_missing() {
    let nowhere = std::env::temp_dir().join("supraman-no-such-dir").join("git.exe");
    assert_eq!(check(&nowhere), Located::Missing { at: Some(nowhere.clone()) });
    let mut settings = Settings::default();
    settings.set_section(SECTION, &GitmanSettings { git: Some(nowhere.clone()) }).unwrap();
    assert_eq!(locate(&settings), Located::Missing { at: Some(nowhere) });
}

#[test]
fn a_version_below_the_floor_is_too_old() {
    let path = Path::new("git.exe");
    assert_eq!(
        judge(path, "git version 2.35.8.windows.1\n"),
        Located::TooOld { path: PathBuf::from("git.exe"), version: Version { major: 2, minor: 35, patch: 8 }, floor: FLOOR }
    );
    let Located::Found(git) = judge(path, "git version 2.39.3 (Apple Git-145)") else { panic!("2.39.3 is new enough") };
    assert_eq!(git.version, Version { major: 2, minor: 39, patch: 3 });
    assert!(matches!(judge(path, "git version 2.36.0.windows.1"), Located::Found(_)), "the floor itself");
    assert!(matches!(judge(path, "hello"), Located::Unusable { .. }));
    assert!(matches!(judge(path, "git version 3"), Located::Unusable { .. }), "no minor");
}

#[test]
fn without_an_override_the_git_on_path_is_found_and_checked() {
    match locate(&Settings::default()) {
        Located::Found(git) => assert!(git.version >= FLOOR && git.path.is_file(), "{git:?}"),
        other => panic!("{other:?}"),
    }
}
