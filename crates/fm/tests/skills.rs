//! The controller skills and roles as text: what m3/4's end-to-end run showed they must say.

use std::path::PathBuf;

fn plugin(path: &str) -> String {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/foreman").join(path);
    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn skills_call_the_skill_tool_even_when_the_listing_omits_it() {
    for path in [
        "skills/repo-controller/SKILL.md",
        "skills/workspace-controller/SKILL.md",
        "roles/repo.md",
        "roles/workspace.md",
    ] {
        let text = plugin(path);
        assert!(text.contains("Skill tool"), "{path}");
        assert!(text.contains("a skill can be missing from the listing"), "{path}");
        assert!(!text.contains("listing is truncated"), "the listing is not truncated (tf/7): {path}");
        assert!(text.contains("only a failed Skill call is a blocker") || text.contains("Only a failed Skill call is a blocker"), "{path}");
    }
    let repo = plugin("skills/repo-controller/SKILL.md");
    assert!(repo.contains("`rlarch:controller` and `rlarch:nested-claude` even when the skill listing does not show them"));
    assert!(!repo.contains("Not in your skill list"), "a missing listing entry is not a blocker");
    assert!(plugin("skills/workspace-controller/SKILL.md").contains("answer it yourself, never ask the human"));
}

#[test]
fn skills_end_a_waiting_turn_without_the_word_blocked() {
    for path in ["skills/repo-controller/SKILL.md", "skills/workspace-controller/SKILL.md", "roles/repo.md"] {
        let text = plugin(path);
        assert!(text.contains("`done: waiting on <who> for <what>`"), "{path}");
        assert!(!text.contains("`done: BLOCKED"), "{path}");
    }
}
