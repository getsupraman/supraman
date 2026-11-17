use super::*;

#[test]
fn a_walked_line_reads_its_new_id_time_and_message() {
    let line = walked_line("1111\0refs/worktree/supraman/undo@{1700000060 -0500}\0commit: a @{x} b").unwrap_or_else(|| panic!("a line"));
    assert_eq!(line, Line { old: String::new(), new: "1111".into(), time: 1_700_000_060, message: "commit: a @{x} b".into() });
    assert_eq!(walked_line("1111\0HEAD@{1700000000 +0100}\0"), Some(Line { old: String::new(), new: "1111".into(), time: 1_700_000_000, message: String::new() }));
    assert_eq!(walked_line("1111 no fields"), None);
}
