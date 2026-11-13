use std::path::PathBuf;

fn plugin(path: &str) -> String {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/supraman").join(path);
    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_hand_role_names_the_scheduler_skill_and_no_controller_duty() {
    let hand = plugin("roles/hand.md");
    assert_eq!(hand.matches("/supraman:supraman-scheduler").count(), 1);
    for duty in ["sm tell", "relay", "work_brief", "controller", "done:"] {
        assert!(!hand.contains(duty), "the hand role names {duty}");
    }
}

/// The scheduler skill runs only these `sm schedule` verbs, and pre-allows nothing else.
#[test]
fn the_scheduler_skill_runs_only_the_allowed_fm_schedule_verbs() {
    const ALLOWED: [&str; 7] = ["add", "edit", "list", "show", "pause", "resume", "dry-run"];
    let raw = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/supraman/skills/supraman-scheduler/SKILL.md")).unwrap();
    let text = plugin("skills/supraman-scheduler/SKILL.md");
    let never = text.split("## 4. Never").nth(1).expect("a Never section");
    let body = text.split("## 4. Never").next().unwrap();
    let verbs: Vec<&str> = body
        .split("sm schedule ")
        .skip(1)
        .map(|rest| rest.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).next().unwrap())
        .collect();
    assert!(verbs.len() > 10, "{verbs:?}");
    for verb in &verbs {
        assert!(ALLOWED.contains(verb), "the skill runs `sm schedule {verb}`");
    }
    for verb in ALLOWED {
        assert!(verbs.contains(&verb), "the skill does not use `sm schedule {verb}`");
    }
    let allowed_tools = raw.lines().find_map(|l| l.strip_prefix("allowed-tools: ")).expect("allowed-tools in the header");
    let rules: Vec<&str> = allowed_tools.split(", ").collect();
    assert_eq!(rules, ALLOWED.map(|v| format!("Bash(sm schedule {v}:*)")).iter().map(String::as_str).collect::<Vec<_>>());
    for word in ["approve", "TOML", "`sm schedule remove`, or `sm schedule edit` of a schedule you did not add in this conversation at the human's request", "the built-in schedule skill", "`schtasks`", "`edit` or `auto` without the human's answer"] {
        assert!(never.contains(word), "the Never table names {word}");
    }
    for word in ["schtasks", "Register-ScheduledTask", ".toml"] {
        assert!(!body.contains(word), "only the Never table names {word}");
    }
    assert!(body.contains("only the human picks it: ask (§2), never choose it yourself"));
    assert!(body.contains("**Pass the hint on; never add the rule yourself.**"), "a dry-run hint goes to the human");
    assert!(!body.contains("worktree-edit"), "the profile is edit");
    assert!(body.contains("It is pending: it runs only after you Approve it on its row in the Supraman window."));
    assert!(!body.contains("sm schedule remove"), "remove stays the human's");
    let verb_table = body.split("| Verb | Use |").nth(1).and_then(|t| t.split("`<w>` is the workspace").next()).expect("a verb table");
    assert!(verb_table.contains("| `sm schedule edit [<w>] <id> <options>` | change a schedule **you added in this conversation at the human's request**"), "{verb_table}");
    assert!(body.contains("Only for a schedule **you added in this conversation at the human's request**: `sm schedule edit"));
}
