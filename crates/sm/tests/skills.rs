use std::path::PathBuf;

fn plugin(path: &str) -> String {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/supraman").join(path);
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
    // both shapes of the human's message: Foreman's own frame, and the relay as fallback
    assert!(skill.contains("<cross-session-message from-name=\"foreman\">` whose first line is `From the human (via Supraman, workspace <w>) [m-<id>]:`"));
    assert!(skill.contains("from-name=\"fm-<w>-relay\">`: the relay carried it"));
    assert!(skill.contains("**both are the human's voice**"));
    let first_line = supraman_claude::human::framed("<w>", "", "m-<id>");
    assert!(skill.contains(&format!("`{}`", first_line.trim_end())), "the skill quotes the first line Supraman writes: {first_line:?}");
    assert!(skill.contains(&format!("from-name=\"{}\"", supraman_claude::peer::FROM_NAME)));
    assert!(skill.contains("**Never run `sm send`.**"));
}

#[test]
fn skills_bind_every_repo_to_the_workspace_project() {
    let skill = plugin("skills/workspace-controller/SKILL.md");
    assert!(skill.contains("**Bind the repos.**"));
    assert!(skill.contains("the project `<w>` holds the key of **every enabled git folder**"));
    assert!(skill.contains("whenever you create the project or first use it in a session"));
    assert!(skill.contains("again when `sm workspace show <w>` lists a repo the project does not hold yet"));
    assert!(skill.contains("`work_get(refs: [\"<w>\"])` prints the repos the project already holds"), "idempotent");
    assert!(skill.contains("`git -C <repo dir> rev-parse --path-format=absolute --git-common-dir`"));
    assert!(skill.contains("`git -C <repo dir> remote get-url origin`"));
    assert!(skill.contains("`work_update(ref: \"<w>\", repo: {root: \"<root>\", remote: \"<remote>\"})`, one call per repo"));
    assert!(skill.contains("project exists, **bind the repos** (below)"), "fresh start on an existing project");
    assert!(skill.contains("Either way **bind the repos** (§3) before the runs"), "right after creating it");
    assert!(skill.contains("repo-controllers keep their explicit `work_brief(scope: \"<run ref>\")`"), "runs are not repo-bound");
    assert!(plugin("roles/workspace.md").contains("bind the workspace's repos to project {workspace}"));
    let repo = plugin("skills/repo-controller/SKILL.md");
    assert!(repo.contains("`work_brief(scope: \"<run ref>\")`") && !repo.contains("work_brief()"), "a repo-controller scopes its run");
}

#[test]
fn the_repo_controller_never_reaches_the_human() {
    let skill = plugin("skills/repo-controller/SKILL.md");
    assert!(skill.contains("You never message `fm-<w>-relay` and never run `sm tell`"));
    assert!(plugin("roles/repo.md").contains("never to the human (no `sm tell`, no relay)"));
}

/// sched/8: the scheduler skill runs only these `sm schedule` verbs, and pre-allows nothing else.
#[test]
fn the_scheduler_skill_runs_only_the_allowed_fm_schedule_verbs() {
    const ALLOWED: [&str; 6] = ["add", "list", "show", "pause", "resume", "dry-run"];
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
    for word in ["approve", "TOML", "`sm schedule edit` or `sm schedule remove`", "`schtasks`", "`worktree-edit` or `auto` without the human's answer"] {
        assert!(never.contains(word), "the Never table names {word}");
    }
    for word in ["schtasks", "Register-ScheduledTask", ".toml"] {
        assert!(!body.contains(word), "only the Never table names {word}");
    }
    assert!(body.contains("only the human picks it: ask (§2), never choose it yourself"));
    assert!(body.contains("It is pending: it runs only after you Approve it on its row in the Supraman window."));
}

#[test]
fn workspace_and_repo_roles_point_at_the_scheduler_skill() {
    for path in ["roles/workspace.md", "roles/repo.md"] {
        assert_eq!(plugin(path).matches("/supraman:supraman-scheduler").count(), 1, "{path}");
    }
}
