use std::path::PathBuf;

fn plugin(path: &str) -> String {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/supraman").join(path);
    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_roles_and_skills_name_the_skills_and_channels_they_rely_on() {
    let repo = plugin("skills/repo-controller/SKILL.md");
    assert!(repo.contains("`workman:controller` and `workman:agents` even when the skill listing does not show them"));
    assert!(repo.contains("You never message `fm-<w>-relay` and never run `sm tell`"));
    assert!(repo.contains("`work_brief(scope: \"<run ref>\")`") && !repo.contains("work_brief()"), "a repo-controller scopes its run");
    for path in ["skills/repo-controller/SKILL.md", "skills/workspace-controller/SKILL.md", "roles/repo.md", "roles/workspace.md"] {
        assert!(plugin(path).contains("Skill tool"), "{path}");
    }
    for path in ["skills/repo-controller/SKILL.md", "skills/workspace-controller/SKILL.md", "roles/repo.md"] {
        assert!(plugin(path).contains("`done: waiting on <who> for <what>`"), "{path}");
    }
    for path in ["roles/workspace.md", "roles/repo.md", "roles/hand.md"] {
        assert_eq!(plugin(path).matches("/supraman:supraman-scheduler").count(), 1, "{path}");
    }
    let hand = plugin("roles/hand.md");
    for duty in ["sm tell", "relay", "work_brief", "controller", "done:"] {
        assert!(!hand.contains(duty), "the hand role names {duty}");
    }
}

#[test]
fn the_workspace_controller_reaches_the_human_with_fm_tell_not_the_relay() {
    let skill = plugin("skills/workspace-controller/SKILL.md");
    let role = plugin("roles/workspace.md");
    assert!(skill.contains("**The human is reached only with `sm tell`**"));
    assert!(skill.contains("`sm tell --file <path>`"));
    assert!(!skill.contains("SendMessage(to: \"fm-<w>-relay\""), "the relay is not the human's mailbox any more");
    assert!(!skill.contains("Send the plan to the relay") && !skill.contains("Report to the relay"));
    assert!(role.contains("you reach the human with `sm tell` (never SendMessage to the relay"));
    assert!(role.contains("announce ready to the human with `sm tell` once"));
    // both shapes of the human's message: Supraman's own frame, and the relay as fallback
    assert!(skill.contains("<cross-session-message from-name=\"supraman\">` whose first line is `From the human (via Supraman, workspace <w>) [m-<id>]:`"));
    assert!(skill.contains("from-name=\"fm-<w>-relay\">`: the relay carried it"));
    assert!(skill.contains("**both are the human's voice**"));
    let first_line = supraman_claude::human::framed("<w>", "", "m-<id>");
    assert!(skill.contains(&format!("`{}`", first_line.trim_end())), "the skill quotes the first line Supraman writes: {first_line:?}");
    assert!(skill.contains(&format!("from-name=\"{}\"", supraman_claude::peer::FROM_NAME)));
    assert!(skill.contains("**Never run `sm send`.**"));
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
