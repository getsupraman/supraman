//! Its own process: the set-once overrides of `paths::workman_server_json` and `paths::claude_json` hold for every later read.

use std::path::PathBuf;

use supraman_core::paths::{claude_json, set_claude_json_for_selftest, set_workman_server_json_for_selftest, workman_server_json};

#[test]
fn the_selftests_server_json_is_set_once_before_any_read_and_then_read_everywhere() {
    let own = PathBuf::from(r"C:\scratch\selftest-home\user\.workman\server.json");
    set_workman_server_json_for_selftest(own.clone()).unwrap();
    let twice = set_workman_server_json_for_selftest(PathBuf::from(r"C:\other\server.json")).unwrap_err();
    assert!(twice.contains("set already"), "{twice}");
    assert_eq!(workman_server_json(), Some(own.clone()));
    let again = set_workman_server_json_for_selftest(PathBuf::from(r"C:\other\server.json")).unwrap_err();
    assert!(again.contains("read before"), "{again}");
    assert_eq!(workman_server_json(), Some(own));
}

#[test]
fn the_selftests_claude_json_is_set_once_before_any_read_and_then_read_everywhere() {
    let own = PathBuf::from(r"C:\scratch\selftest-home\user\.claude.json");
    set_claude_json_for_selftest(own.clone()).unwrap();
    assert!(set_claude_json_for_selftest(PathBuf::from(r"C:\other\.claude.json")).unwrap_err().contains("set already"));
    assert_eq!(claude_json(), Some(own.clone()));
    assert!(set_claude_json_for_selftest(PathBuf::from(r"C:\other\.claude.json")).unwrap_err().contains("read before"));
    assert_eq!(claude_json(), Some(own));
}
