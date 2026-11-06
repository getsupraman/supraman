use std::path::{Path, PathBuf};

use supraman_core::Settings;
use supraman_gitman::{FLOOR, GitmanSettings, Located, SECTION, Version, check, judge, locate, located};

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

#[test]
fn a_found_git_is_kept_per_setting_while_its_file_is_there() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("git.cmd");
    std::fs::write(&fake, "@echo git version 2.40.0\r\n").unwrap();
    let mut settings = Settings::default();
    settings.set_section(SECTION, &GitmanSettings { git: Some(fake.clone()) }).unwrap();
    let Located::Found(first) = located(&settings) else { panic!("the fake git answers 2.40.0") };
    std::fs::write(&fake, "@echo git version 2.1.0\r\n").unwrap();
    assert_eq!(located(&settings), Located::Found(first), "not run again for the same setting");
    assert!(matches!(locate(&settings), Located::TooOld { .. }), "locate itself runs it");
    std::fs::remove_file(&fake).unwrap();
    assert_eq!(located(&settings), Located::Missing { at: Some(fake) }, "a kept git whose file is gone is looked for again");
}
