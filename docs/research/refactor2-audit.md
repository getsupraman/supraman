# refactor2 audit: findings, test verdicts and proposed units

Unit refactor2/1, 2026-11-03, worktree `refactor2-audit` at `049f8d6` (release 0.1.4). Read-only: nothing under `crates/`, `plugins/`, `scripts/` or `Cargo.*` changed. The only output is this file and the findings recorded on `refactor2` in Workman.

**Method.**
- Five read-only reviewers ran in parallel, each briefed on CLAUDE.md (C56 shared components, C57 comments, C15/C17/C21):
  - supraman-app outside the selftests, plus the reuse inventory;
  - the other crates and `plugins/supraman`, plus F-01;
  - every `--selftest` step;
  - every `--selftest-supervise` and `--selftest-detach-shutdown` step;
  - every `crates/*/tests/*.rs` file and the unit-test totals.
- I checked the highest-risk claims against the code myself: R-101, R-201 and F-01's cause.
- I built the release at 049f8d6 and ran `cargo test --release --workspace` once: 929 passed, 0 failed, every test binary under 17 s.
- I ran each selftest once at Normal priority with every output line timestamped. All three passed, with 0 leftover sessions:

| mode | wall time | slowest steps |
|---|---|---|
| `--selftest` | 224 s (213 s to PASS, 11 s cleanup) | 7j 29.7 s, 7y 25.5 s, 7+r 22.7 s, 7b 19.1 s, 7 13.1 s, 7n 9.4 s, 7l 8.3 s, 8 7.0 s, 7d 6.7 s, 7z 6.5 s |
| `--selftest-supervise` | 400 s (385 s to PASS) | r 102.3 s, v 56.8 s, c 36.3 s, t 36.4 s, f 31.7 s, y 28.1 s, w 24.8 s, x 14.5 s |
| `--selftest-detach-shutdown` | 14 s | – |

A step's time is the gap between its printed line and the previous one. Each step prints when it ends, so the setup of the next step is counted with it.

**Columns** are those of the m5/44 audit:
- **Risk** is the risk of the fix.
- **Prot.** marks protected logic: WS (Workman contract), OAuth, Mint, CLI (claude protocol, `--bg`, resume, rm), Del (deletion confinement), C15/17/21, Sched (C100/C101), Update (signed self-update).
- **Sev.** is *blocker* only for a security hole, a UI freeze or data loss in normal use. No finding here meets that bar. R-201 and R-101 are the most urgent.

**Workman refs.** The code findings are recorded on `refactor2`:

| audit ids | Workman refs |
|---|---|
| R-101 … R-121 | refactor2/F-02 … F-22 |
| R-202 … R-220 | F-23 … F-41 |
| R-222 … R-232 | F-42 … F-52 |
| R-233 (the test spawns left from F-01) | F-53 |

R-201 and R-221 have no record: R-201 is fixed by the hotfix, and R-221 is kept as API.

## Summary

- **Live breakage, both already fixed by supraman's hotfix on branch `skills-workman`:**
  - R-201: the supraman plugin tells every repo-controller to run `/rlarch:controller` and `/rlarch:nested-claude`. rlarch 3.0.x no longer has either skill; they are `workman:controller` and `workman:agents` now. A repo-controller Supraman starts fails its first Skill call and goes BLOCKED.
  - F-01: the ping console windows come from one supraman-core unit test. The 17 sm test spawns without `no_window` remain (U1).
- **Correctness:**
  - R-101: the menu's Enable/Disable bypasses `Registry::edit`, so one folder can get two supervisors.
  - R-102: Send and Approve deliver on the UI thread, up to a 5 s freeze.
  - R-211, R-213, R-215: fragile paths on Sched, CLI and Mint.
- **Overdue compat:** R-223/R-224. The `fm.exe`/`fmw.exe` copies and `FOREMAN_HOME` writes were due to go in 0.1.3.
- **Tests:** the gates take 638 s of machine time per merge (224 + 400 + 14) and start 13 real claude sessions. The proposal below brings that to about 320 s and 8 sessions without losing a product behaviour that nothing else covers. The changes:
  - cut 12 `--selftest` steps or parts, merge 9, move 11 checks down to unit tests;
  - cut 5 supervise steps and move 5;
  - cut about 14 cargo tests and merge about 10 pairs.

| Area | Count | Highest risk |
|---|---|---|
| Live breakage (R-201, F-01) | 2 | med (skill text drives sessions) |
| App code R-101 … R-121 | 21 | med (R-102 async compose) |
| Other crates R-201 … R-231 | 31 | med (R-202 mint helper, R-211, R-213, R-215, R-223) |
| `--selftest` steps | 46 ids | – |
| `--selftest-supervise` steps | 18 | – |
| `--selftest-detach-shutdown` phases | 5 | – |
| `tests/*.rs` files | 19 (129 tests) | – |

## F-01: ping.exe console windows (refactor2/F-01)

**Cause, verified:**
- `crates/supraman-core/src/process.rs:345`, the unit test `a_timeout_kills_the_tree_or_only_the_child`, runs `[System.Diagnostics.Process]::Start('ping', '-n 60 127.0.0.1')` inside Windows PowerShell 5.1 (`powershell()` at :301).
- On .NET Framework, `Process.Start(file, args)` defaults to `UseShellExecute = true`, so ping is started through ShellExecuteEx and gets a new, visible console. CREATE_NO_WINDOW on powershell does not reach that grandchild.
- The test loops twice (`Kill::Tree`, `Kill::Child`), so every `cargo test` of supraman-core shows two ping windows for about 8 s.
- With Windows Terminal as the default console host they open as new WT windows in the foreground, which fits "sometimes TOPMOST".

**Fixed by supraman's hotfix on branch `skills-workman`, not by a unit of this run:** `Start-Process ping -NoNewWindow` with stdout and stderr redirected to files, so the grandchild does not keep the pipe.

**The other candidates in the brief open no window:**
- `sm/tests/fire.rs:20,26` (`slow.cmd`, `once.cmd`) run through `process::command("cmd")` (`sm/src/fire.rs:695`, CREATE_NO_WINDOW), and ping inherits cmd's hidden console.
- `selftest_refresh.rs:42` uses `process::command` on the hidden desktop.

Production spawns all go through `supraman_core::process`.

**Test spawns that skip `no_window`.** Each can open a console when the test runner has none (Git Bash's MSYS runtime usually hides them; not run):
- 17 sites start `sm.exe` with a bare `Command::new`: `sm/tests/{hook.rs:15, relay.rs:10,61, schedule.rs:46,163, sessions.rs:4, want.rs:8, work.rs:30,189, workspace.rs:6,189, fire.rs:562, fire_session.rs:314, update.rs:281,287}`. `update.rs:103,253` already use `no_window`.
- Unit tests with a bare `Command::new`: `process.rs:359` taskkill, `claude/forget.rs:186` `cmd /C mklink`, `claude/launch.rs:508` and `app/hand.rs:623` icacls, `app/hand.rs:899,912,928,941` pwsh, `supervisor/mint.rs:385` git.
- `runas` opens its own console by design. It is used in `claude/daemon.rs:209` and `app/tests/elevation_handoff.rs:50`, which run only when elevated.

**Fix:** one `sm_cmd()` test helper over `process::command`, and `process::command` in every unit test above. A guard test that no visible top-level window appears is hard to write for cargo test. A grep lint is enough: no `Command::new` outside `supraman_core::process`, except `runas` (see U1).

## Findings

### 0. Live breakage

| ID | Where | Problem | Fix | Risk | Prot. |
|---|---|---|---|---|---|
| R-201 | `plugins/supraman/roles/repo.md:5,12`; `skills/repo-controller/SKILL.md:3,8-9,35-41,118`; `skills/workspace-controller/SKILL.md:3,131,172,235`; `README.md:19`; `.claude-plugin/plugin.json:4`; `crates/supraman-core/src/settings.rs:36-37`; asserted in `crates/supraman-app/src/supervise.rs:462` and `crates/sm/tests/skills.rs` | The roles and skills name `/rlarch:controller` and `/rlarch:nested-claude`. rlarch 3.0.1's skills are code-comments, dotnet-reflect, dotnet-source and ts-source; the two moved to workman (2.0.1: `controller`, `agents`). Verified in `~/.claude/plugins/cache`. | **Fixed by supraman's hotfix on branch `skills-workman`:** every `rlarch:controller` → `workman:controller` and `rlarch:nested-claude` → `workman:agents`, in the roles, both skills, README, plugin.json, the settings comments, `supervise.rs:462` and `sm/tests/skills.rs`. The § numbers match (§0, §1, §6, §8 checked). | – | none |

### 1. Duplicates

| ID | Where | Problem | Fix | Risk | Prot. |
|---|---|---|---|---|---|
| R-109 | `app/view/manage.rs:114-137`, `app/view/folders.rs:134-157` | Two copies of the folder picker; only one calls `activate_window` | one `pick_folder` in supraman-ui (FileMan needs it) | low | none |
| R-110 | `app/schedules.rs:107-119, 387-390`; `tab_card.rs:64-69`; `work_view.rs:487-499`; `updates.rs:202`; `fleet.rs:138-145`; `row_details.rs:103` | Seven time formatters | `supraman_core::time::{local, ago}` | low (selftests assert texts) | none |
| R-111 | `app/view.rs:2601-2622`, `updates.rs:218-225`, `signin.rs:45-51`; openers in `view.rs:802-805`, `manage.rs:596-610`, `settings_window.rs:309,342` | Three URL-scheme checks, four ways to open a link or path | `supraman_ui::open::{url, path}` with the allowlist and the off-thread rule | low | OAuth (browser open) |
| R-112 | `app/supervise.rs:37-51`, `view.rs:1957`, `view/schedules.rs:383`, `updates.rs:72,109`, `elevate.rs:57` | The exe dir / `sm.exe` lookup is built six times; core has `schedule::smw_exe` | `supraman_core::install::{exe_dir, sm_exe}` | low | none |
| R-113 | `app/work_view/inbox.rs:16-57`, `dismissed_messages.rs`, `stars.rs:35-43` | Three keyed-set stores; `dismissed_messages` saves on the UI thread (`view.rs:1114-1120`) | one store over `load_json_or_notice` + `write_atomic`, saved off-thread; file formats kept | low | C21 |
| R-114 | `app/row_details.rs:264-273`; `core/paths.rs:70-86`; `core/scan.rs:90-94` | The `.git` ancestor walk is repeated | `paths::git_at(dir)` used by all three (GitMan needs it) | low | none |
| R-115 | `app/work_view/item.rs:630-650`, `work_view.rs:510-537`, `view/left.rs:1902` | Work-tab section/badge helpers beside `supraman_ui::details` | inside the Work-tab redesign: `details::Section` and a `supraman_ui::badge` | low–med | C21 |
| R-202 | `supervisor/executor.rs:481-516, 585-616`; `sm/run.rs:232-282`; `sm/fire.rs:124-138`; `app/hand.rs:~220` | The session-token lifecycle (build, mint, revoke with the right origin's bearer, delete the config) exists three or four times; `run.rs:276` copies `executor.rs:605-612` | one `supraman_supervisor::mint::{mint_session, end_session}` | med | Mint |
| R-203 | `supervisor/mint.rs:229-240`, `claude/human.rs:88-92` | Hand-written JSON state loaders | `fs::load_json_or_notice` (see R-216) | low | Mint |
| R-204 | `claude/peer.rs:84,164`, `core/events.rs:110`, `home.rs:122`, `schedule/approval.rs:325`, `update.rs:153`, `workman/fake.rs:278`, `app/brand.rs:79` | Eight copies of bytes → lowercase hex | `supraman_core::hex`, byte-identical output | low | Sched, Update (same output) |
| R-205 | `claude/launch.rs:159-166`, `core/events.rs:59`, `core/slug.rs:23`, `app/hand.rs:103`, `app/view/manage.rs:35` | Four or five ad-hoc name validators; the app's two are near-copies of `slugify` | named rules in `supraman_core::slug`; `slugify` in the app | low | none |
| R-206 | `workman/stream.rs:136-147`, `workman/auth.rs:623` | The stream parses scheme, host and port beside `url::origin` | derive from `url::origin` plus an `authority_port` helper | low | WS |

### 2. Hidden hand-rolled components (C56)

| ID | Where | Problem | Fix | Risk | Prot. |
|---|---|---|---|---|---|
| R-106 | `app/titlebar.rs:43-109`, `settings_window.rs:871-883` | Buttons with no hit key ("open-settings", the caption buttons, "Open settings.toml"), so selftests cannot click them | `button::ghost`/`icon` with keys; the captions keep their non-client hit test | low | none |
| R-107 | `app/view/folders.rs:265-275`, `view/left.rs:1765-1775` | Two hand-made ☑/☐ checkboxes | `supraman_ui::button::checkbox` (GitMan staging needs it) | low | none |
| R-108 | `app/view/icon.rs:1-163`, `clipped.rs:1-79` | `Icon`, the text `Tooltip` and the clipped label are generic but app-only | move to supraman-ui | low–med (`include_bytes!` paths; selftests read `LAST_SHOWN`, `cut_at`) | none |
| R-117 | `app/view.rs:404-424` | `PollHold::wait` polls a flag in a 10 ms sleep loop, against "nudge, never a timer polling a flag"; only selftests arm it | block on a `Nudge` with `MAX` as the timeout | low | C15/17 |
| R-207 | `workman/stream.rs:136-330` | `/api/stream` is its own HTTP/1.1 client (`TcpStream`, `native_tls`, a chunked decoder) beside `http_agent`. It dials only the first resolved address (`:149`), so a host resolving to `::1` first while the server listens on IPv4 breaks the stream while REST works | try every address; say in one line why not ureq (short read timeouts for the stop flag), or move to ureq with a per-read timeout; list the component in CLAUDE.md | med | WS |
| R-208 | `core/settings.rs:346-363` | `Settings::update` is an optimistic check-then-write loop with a gap between the re-read and the save, and it saves anyway after 10 tries; CLAUDE.md's `lock_file` rule forbids that pattern | hold `lock_file(<home>/settings.lock)` across read, edit and save | low | none |

### 3. Weak or fragile code

| ID | Where | Problem | Fix | Risk | Prot. | Sev. |
|---|---|---|---|---|---|---|
| R-101 | `app/view/manage.rs:585-594` (`set_repo_enabled`), from `view/menu.rs:192`, `manage.rs:424` | Verified. The menu's Enable/Disable calls the unlocked `Registry::update` (`core/discover.rs:94-100`), not `Registry::edit(Edit::SetEnabled)`. Enable skips the `SupervisedElsewhere` check (`edit.rs:98-106`), so a folder can be supervised by two workspaces. Disable skips `DesiredState::release` and the "stops a session?" question. It also races `sm workspace enable/disable` | route the action through `request_edit(w, Edit::SetEnabled{..})`; delete `set_repo_enabled` | low (a toggle now asks first) | none | minor, first |
| R-102 | `app/view.rs:1616-1657` (`send_compose` → `answer` → `send_to_workspace`) | `send_from_human` reads `~/.claude/sessions`, the ledger, and `peer::deliver` waits up to `DELIVER_TIMEOUT` = 5 s (`claude/peer.rs:22,282`), all inside the click | run `answer` on the background executor; apply `answered`, the route and the error on completion | med (compose closes async; selftests need an `until`) | CLI | minor (borderline: a 5 s freeze on a stalled pipe) |
| R-103 | `app/view/details.rs:95-128` | Every poll (2 s), each open session Details tab runs `OnDisk::read` on the UI thread: `find_transcript` can fall back to `read_dir` of all of `projects/` (`claude/forget.rs:104`), plus `branch_at` and metadata | compute `OnDisk` in `Poller::read` or a spawned task per tab | low | none | minor |
| R-104 | `app/view.rs:364-365, 1419-1422`; `view/notices.rs:6-58` | `schedules::read`'s "Cannot read the schedules / run records of …" land in `schedule_errors` and are never shown; `schedule_errors_dismissed` is never filled | add `Notice::Schedule` to `plain_notices` and its dismissal | low | Sched (display) | minor |
| R-105 | `app/view.rs:1685-1695` (`start_fresh`), `view/schedules.rs:472-484` (`save_schedule_edit`), `view.rs:1659-1673, 1273-1276, 1955-1957`, `view/manage.rs:226-250, 273-286`, `view/start.rs:178-185` | File IO and cross-process lock waits on the UI thread in click handlers. Approve, Pause and Remove already do their edits off-thread (`view/schedules.rs:302-341`) | the `change_schedule`/`desired_writer` pattern; `start_fresh` and `save_schedule_edit` first (they wait on locks) | low–med | Sched (C100 kept) | minor |
| R-116 | `app/main.rs:378-387`, `tray.rs:103-114` | `Tray::follow_taskbar` reads the registry (`RegGetValueW`) on every FleetView notify; every terminal Wakeup notifies | a slow timer or a settings change | low | none | minor |
| R-118 | `app/view.rs:1972-1993` | The cursor-blink loop wakes every interval even with no terminal tab active | park on a nudge that tab activation wakes | low | none | minor |
| R-119 | `app/view.rs:87, 725` | `FOREMAN_CLAUDE_DIR` (a selftest override) is read ahead of `claude_dir()`, and it is also the root Delete confines to (`view/delete.rs:168-172`): a stray user env var redirects the transcript scan and the Delete root | a selftest-only setter, or one `SUPRAMAN_`-prefixed test var read in one place | low | Del | minor |
| R-209 | `core/process.rs:109-118` | On a normal exit `joined()` joins the pipe readers with no bound. With `Kill::Child` (every `claude` call, `launch.rs:335`), a descendant that keeps the pipes blocks the call past its timeout; on a timeout the reader threads leak. Not run | the same bounded wait plus `taken()` on the exit path | low | CLI | minor |
| R-210 | `core/process.rs:93, 172-182` | The job object is made after `spawn` and has no `KILL_ON_JOB_CLOSE`, so a schedule script's tree survives a crashed or timed-out sm/smw | set KILL_ON_JOB_CLOSE for `Kill::Tree` | low–med | Sched | minor |
| R-211 | `sm/fire.rs:551-573` | `recover_dead_run` deletes the live file even when `end_dead_session` failed (stop, revoke, config delete): the orphaned session and token are never retried. `append_run`'s error is dropped; an unparsable live file is deleted silently | keep it as `<id>.dead.json` for the next poll; log failures to `fire.log` | med | Sched, Mint | minor |
| R-212 | `sm/fire.rs:314-316` | `LiveFile::write` drops write errors; a run whose live file never landed is invisible to `recover_if_dead` | log the first failure to `fire.log` | low | Sched | minor |
| R-213 | `claude/peer.rs:262-287`, `human.rs:154-173` | On a pipe-write timeout the writer thread keeps going and `Sender::send` falls back to the relay; a late pipe write plus the relay delivers twice | no fallback on `Timeout` ("may have been delivered"), or carry the `m-id` through the relay for dedupe | med | CLI | minor |
| R-214 | `claude/human.rs:101-106, 179-196` | Pacing is read outside the lock, and `record` ignores a failed `lock_file`; two senders (the app and `sm send`) both pass | check and record under one lock | low | CLI | minor |
| R-215 | `claude/launch.rs:149-156` (remint at `executor.rs:520-525`) | `write_mcp_config` truncates the live config, then fills it; a `claude` reading `--mcp-config` in between sees an empty file | write a sibling, restrict its ACL, rename over | med (owner-only ACL on the temp) | Mint | minor |
| R-216 | `supervisor/executor.rs:181-185`, `mint.rs:229` | A damaged `<ws>.sessions.json` reads as empty, and the next save overwrites it; earlier tokens are never revoked (they lapse) | as `auth.rs:168`: refuse to save over it, keep a `.damaged` copy | low | Mint | minor |
| R-217 | `workman/auth.rs:356-358` | `sign_in`'s first load → insert → save runs outside `refresh_lock`, unlike :397-401 | take `refresh_lock` there | low | OAuth | minor |
| R-218 | `workman/updates.rs:27, 314, 330-336` | Cancel is checked only between reads, and the agent's one timeout is 30 min, so a stalled download ignores Cancel | a per-read timeout (60 s) | low | Update | minor |

### 4. Dead or misplaced code

| ID | Where | Problem | Fix | Risk | Prot. |
|---|---|---|---|---|---|
| R-120 | `app/view.rs:348, 850, 2030` `tab_resolves` (written, never read); `schedule_errors_dismissed` (R-104); `app/supervise.rs:73-77` a `#[cfg(test)] fn` in production code; `app/tokens.rs` a test-only module named like production code | dead field, misplaced test code | delete; move into `mod tests`; rename `tokens.rs` → `test_auth.rs` | low | none |
| R-219 | `term/alacritty/hyperlinks.rs:1149,1167,1185,1204,1267` | five vendored Zed benchmark fns, never called | delete | low | none |
| R-220 | `core/events.rs:84-86` | `role_injection` is used only by tests | fold into the tests | low | none |
| R-221 | `core/settings.rs:365-375` | `Settings::section`/`set_section` have no production caller yet; they are the documented API for `[gitman]`/`[fileman]` | keep (GitMan uses them); note it | – | none |
| R-222 | `sm/src/probe*.rs` (~1,100 lines), `sm/lib.rs:50-53,224-250`; `workman/auth.rs:709-726`; `app/Cargo.toml:24` | Developer probes that start real claude sessions ship in the user's sm.exe; `http_browser*` (selftest-only) is in production builds; the release app links the `fake` Workman | feature-gate the probes; `cfg(feature = "fake")` on `http_browser*`; decide whether the release app should link `fake` | low | CLI, C15/17/21 |

### 5. Comments (C57)

About 175 comments remain to delete or rewrite. The app has about 75, mostly in `view.rs`, `fleet.rs`, `view/schedules.rs` and `schedules.rs`. The other crates have about 100. There are 66 `mX/Y` history tags. The worst kinds:
- **bare history tags:** `supervisor/executor.rs:2142` `// m5/61`, `app/fleet.rs:1134`;
- **"Foreman" after the rename:** `app/signin.rs:1`, `view/delete.rs:1`, `work_view/inbox.rs:1`, `view.rs:1702`, `tab_card.rs:34`;
- **provenance:** `claude/launch.rs:560` "René's Inbox row on 2ae06cd (m5/25), verbatim", `term/mouse.rs:365` "(m5/44 decision 5)";
- **vendored Zed narration:** `term/links.rs:942-944, 949, 1057`;
- **a README narrating a failed attempt:** `plugins/supraman/README.md:6, 10`.

One decision is needed first: whether the vendored Zed copy is still re-synced (A-53). That decides whether its comments and the 162 `// U+25xx` labels in `ui/builtin_glyphs.rs` stay as they are.

Recorded as R-121 (app) and R-232 (other crates). The per-file counts are in the reviewers' tables, summarised in U10.

### 6. Rename leftovers

**Can go now:**
- **R-223:** the `fm.exe`/`fmw.exe` copies and the `Bash(fm:*)` rules.
  - Where: `core/plugin.rs:11,16,56,70`, `scripts/release.ps1:10-12,65-66`, `README.md:14`, test `app/supervise.rs:438`.
  - Why now: RELEASE.md:142 removes them "in the release after the first one that ships sm.exe". That was v0.1.2, so they were due in 0.1.3.
  - Risk: med. A task registered by ≤0.1.1 that was never re-synced still calls `fmw.exe`. Run one forced `sync` at app start first. Prot.: Sched.
- **R-224:** `FOREMAN_HOME` written into every child, only for an older fm (`core/home.rs:50`, `supervisor/executor.rs:65`). Stop writing it with R-223; keep reading it one more release.
- **R-225:** internal identifiers, a pure rename. Names: `FOREMAN_DARK`, `foreman_home`, `foreman_role`, `workstate_mcp_config`, `workstate_role`, `WORKSTATE_EXTERNAL_ROLE`, `refuse_inside_foreman`, `fm_alive`, the `workstate-stream` thread, the `foreman::panic` log target, the `foremantest` pipe prefix, and the `<FOREMAN_HOME>` placeholders in docs (list in the reviewer's table). Also the app-internal `FleetView::workstate` / `WsStatus` / `workstate_setup` / `page::WORKSTATE`, which workman-2-rename left.
- **R-226:** `foreman.log` → `supraman.log` (`ui/lib.rs:53-102`), together with the selftests and `elevation_handoff.rs` that read it. Prot.: C15/17.
- **R-227:** the sample origin `127.0.0.1:5290` survives only in tests. Use a neutral origin.
- **R-228:** `.foreman/outbox` and `.foreman/prompts` in the workspace-controller skill text. No code reads them.

**Need a one-release compatibility window:**
- **R-229:** the session env `FOREMAN_ROLE/WORKSPACE/SESSION/REPO`. sm reads `SUPRAMAN_*` first, then the old names. Prot.: CLI.
- **R-230:** the schedule-script env `FOREMAN_WORKSPACE/_SCHEDULE/_RUN/_FOLDER/_LAST_RUN/_LAST_OUTCOME/_STATE_DIR`, a documented user contract. Set `SUPRAMAN_*` alongside.
- **R-231:** the `FOREMAN-ROLE` tag and `FROM_NAME = "foreman"`, renamed in lockstep with the plugin text. Prot.: CLI.

**Keep: stored or contract names.**
- `launchedBy`/`origin` = `foreman/…`: the Workman contract (WS).
- The DPAPI entropy (OAuth).
- `APPROVAL_KEY` and the `foreman/schedule/…` secret keys (Sched).
- `workstate-<agent>.json`, the run records' `fm_pid`/`workstate_session`/`without_workstate`, and the `fm-` session prefix.
- The one-time migrations, until a release named for their removal: `%APPDATA%\Foreman`, `\Foreman\<ws>`, "Foreman Dark", `[workstate]`.

## Test inventory and verdicts (c2, c7)

**Totals at 049f8d6:**
- 765 unit `#[test]`, 41 `#[gpui::test]` and 129 integration tests: 935, of which 929 ran (6 are ignored).
- About 18,400 lines of unit tests and 4,195 of integration tests.
- About 11,700 lines of selftest code:
  - about 9,100 reachable from `--selftest`;
  - 1,920 in `selftest_supervise.rs`, plus about 330 in `selftest_left.rs` and `selftest.rs` used only by supervise;
  - 161 in `selftest_shots.rs`.
- `cargo test --release --workspace` takes about 60 s of test time after the build. The slowest binaries take 16.8 s (117 tests) and 16.5 s (187 tests), set by their polls and timeouts, not their count.

**How a verdict reads.** Each row names the assumption the test rests on and whether it still holds; where nothing changed, the assumption cell says so.
- **keep:** it guards product behaviour nothing else covers.
- **merge:** another step or test covers the same setup, so one does both.
- **down:** the behaviour is wanted, but a unit test proves it without a window or a real session.
- **cut:** the behaviour is no longer wanted, or something else covers it, or it tests only a past fix's mechanics.

### `--selftest` (224 s, 46 step ids)

| step | s | guards | rests on (holds?) | verdict |
|---|---|---|---|---|
| 1 | 0.1 | scratch home under `target/`; read-only scan of the user's parent folder registered as `larchsys`; Workman at a closed port | the user's real `P:\Projects\LarchSys` and every real `claude agents --all` row (about 400): holds, but the gate is not hermetic, and its size and timing depend on the machine | keep; a fixture workspace instead of the real scan (U3) |
| 1w | 0.1 | FakeWorkman with OAuth, signed in | hosted Workman with OAuth, faked (C21): holds; the 127.0.0.1:5290 stack is gone and no step uses it | keep |
| 1h, 1o | 0.0 | fake claude dir transcripts; Delete fixture | `FOREMAN_CLAUDE_DIR` (see R-119): holds | keep (1o goes with 7p part 1) |
| 2 | 1.0 | `claude --bg --name fm-selftest-<hex>` launch | real CLI: holds | keep (every real-session step needs it) |
| 3a | 1.3 | loading line and turning Refresh icon before the first list | holds | keep; absorbs 3s |
| 3 | 3.4 | own row listed under `larchsys/<repo>`, attachable, pid, id = CLI id | virtualized sidebar via `shown()`: holds | keep |
| 3s | 0.0 | startup phase lines in `foreman.log`, first list ≤150 ms after `claude agents` | a log-text parse and a timing bound from m5/50 (past-fix mechanics) | merge into 3a ("a startup line per source"); the 150 ms bound → `--selftest-perf` |
| 4 | 0.9 | double-click opens one attach tab that paints claude output | real CLI attach: holds | keep; cut `double_click_shown` (unit `a_rows_one_open_action…`, plus a fixed 500 ms) |
| 5 | 3.7 | closing the tab detaches (Ctrl+Z); the session keeps its pid | the app's core promise: holds | keep; the fixed 2 s → a poll |
| 6 | (in 6b) | plain wheel scroll in a pwsh tab | holds | keep, or down to a `Terminal::scroll_wheel` unit test |
| 6b | 1.7 | drag-select, Ctrl+Shift+C, Ctrl+V through the system clipboard | shared clipboard behind the cross-selftest mutex: holds | keep (the one clipboard end-to-end); absorbs 6c's two survivors |
| 6c | 4.9 | Windows Terminal chords, AltGr, blink, find, selection colour, palette, font keys | nearly all unit-tested (`view/term.rs`, `keys.rs`, `content.rs`, `term_element`; font keys = 7q, palette = 7m) | down; keep only "^C never reaches the shell with a selection" and right-click paste, in 6b |
| 6d | 1.1 | OSC 8 link underline, hover, Ctrl hand, Ctrl+click opens exactly it, `ms-settings:` refused | depends on 6c's prompt state | keep, trimmed to underline, hand and refusal; independent of 6c |
| 7 | 13.1 | Work tab on the fake: Inbox, runs, item page, live event, Attach from a claim, dismissal, journal, GET-only with bearer, middle-click close | holds. Its `board_reads`/`overview_reads` counters are debounce mechanics, and its 3 frame judges each wait up to 60 s for a quiet CPU (`frames::baseline`) | keep the painted flow; counters → a debounce unit test; frame judges → `--selftest-perf` |
| 7b | 19.1 | Add workspace, cut-path tooltip, same folder twice disabled, Edit folders, root-is-a-repo, Details, Star, menus, Open folder, Remove keeps files | holds. It includes past-fix mechanics: the m5/21 PollHold across Save, the m5/68 ShellExecute thread, the caret blink count, and 4.8 s of fixed `no_card_on` rest | halve: root-repo, menu list and caret → unit; PollHold → a FleetView unit test; the fixed rests → waits |
| 7c | 3.2 | icon bar; Collapse/Expand all by `folder_rows`; show-stopped; Settings on About; no teardown error | virtualized sidebar: holds since 3b2b075 | keep; merge with 7f and 7v into one left-panel step; its 900 ms sleep only for shots |
| 7d | 6.7 | Settings → Workman: Ready, Sign out, Sign in, switch server, closed port, http refused, unreachable notice | hosted OAuth: holds; "first run" is a stale name | keep the UI parts; cut the revoke-count and https checks (`workman/tests/oauth.rs`, `settings.rs`); rename |
| 7e | 1.8 | Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+PgUp/PgDn both ways; shells never see them | writes `fm-selftest-keys-*.ps1` to %TEMP%, not `target/` (bends C15) | keep; absorbs 7u; script under `target/` |
| 7f | 1.8 | no tree row paints over the Inbox at 6 offsets | keyed_list: holds | merge into the left-panel step |
| 7v | 0.1 | rows on one icon/label grid at 3 levels | weak: it needs a level-2 row painted without scrolling, which depends on where rows sort | merge into the left-panel step; scroll the own row into view first |
| 7g | 1.1 | a new shell's cwd follows the active tab's workspace | unit `fleet.rs:635 shell_cwd_is_the_active_tabs_workspace…` | merge into 7i (one shell) |
| 7h | 1.1 | closed rows, custom title, double-click resumes `--resume <id>` in its cwd, star saved | fake claude: holds | keep; cut the `starred.json` reload part (l.159-170, data only) |
| 7i | 2.4 | OSC 2 title through ConPTY becomes the tab label; empty restores; hover card ≥400 ms | holds; the 400 ms check is mechanics | keep; drop the 400 ms check; the 1.5 s sleep → a wait |
| 7j | 29.7 | a dead pid's row closes ≤2.3 s; Refresh shows a new transcript ≤3 s; icon state; F5 on Work; F5 in a shell goes to the program | bounds tied to the real `claude agents` poll (≥2.5 s a call) that hold only on an idle machine (F-96 "7j watchdog under load") | trim: keep F5 routing and the icon; the rest "eventually" with no bound |
| 7w | 0.2 | warm start shows cached rows without the loading line; cold start keeps it | drives test hooks, not a real restart | down to a view unit test |
| 7k | 1.0 | Ctrl+Shift+E search keys, matches, Esc twice, result grid, `text_inset` | `text_inset` is a past padding fix | keep keys and Esc; cut `text_inset` and the result-set checks (`search.rs` 4 tests) |
| 7l | 8.3 | a real interactive `claude` in pwsh is detected and marks its row; tab link; Copy SendMessage name; drag-and-drop; `/exit` clears | claude's `~/.claude/sessions` format (the peerProtocol drift watch): holds | keep the core (the only live test of detecting claude in a shell); cut the toast geometry (F-06a); drop → unit |
| 7m | 0.7 | default theme; switching paints and saves | calls `set_theme`, no clicks; `themes.rs` 12 tests | down to a gpui unit test |
| 7n placeholder | – | screenshots only; asserts nothing | – | cut (shots mode) |
| 7n compose | – | Reply… opens the box; checks only that it painted | screenshot material | cut (shots mode) |
| 7n run | 9.4 (all 3) | × on an outbox note hides only it, kept; a newer one shows again | `dismissed_messages.rs` 2 tests | merge with 7o |
| 7o | 0.2 | × on a hand notice removes only it | `notices.rs` tests | merge with 7n run |
| 7+u run | 3.9 | signed fake v99: banner, What's new, Update, About line, × kept; 304 hidden; check off sends nothing | GitHub at a closed port (C109): holds; request count, ETag and check-off are HTTP mechanics in `workman/updates.rs` tests | keep banner, buttons and ×; cut the HTTP parts |
| 7+u install | (in 7+u) | bad-hash zip refused; good zip downloads with progress; `sm update apply` swaps a temp install and starts the stub | holds | keep part 2 (the only updater end-to-end); cut part 1 (`updates.rs:636`) |
| 7x | 5.0 | 5 saved tabs come back as 4 in order after `restart_tabs`; a notice names the dropped one | simulated restart; writes %TEMP% | keep; the 1 s sleep → a check |
| 7+ | 5.8 | a [+] tab gets the plugin dir, `--allowedTools`, env and this build's sm first; the real hook injects the role; Notification marks "permission prompt" | `FOREMAN_*` env is still the live protocol (R-229): holds | keep; drop the argv checks (`hand.rs:536`) |
| 7p | 5.8 | (1) Delete… removes a closed fixture's transcript and folder only; (2) the live session has no Delete, Stop, then Delete runs `claude rm` | holds | keep part 2; cut part 1 (`view/delete.rs:460, :364`) and the 1.5 s sleep |
| 7q | 3.4 | one font size for every tab; Ctrl+= / wheel / 0; 14 saved as unset; Settings in sync; debounced write | holds; the debounce check is mechanics | merge with 7s; drop the debounce check |
| 7r | 1.3 | claude's Alt+V image check sees an image the selftest borrowed and restored | embeds claude 2.1.312's `checkImage` verbatim (drifts); no product path writes an image, so it tests the selftest's own Snapshot | cut (or down to a `selftest_clipboard` round-trip unit test) |
| 7s | 0.9 | Cascadia default, fallback glyphs fit, `->` ligates, Consolas switch saves, Default unsets | installed fonts: holds | merge with 7q; cut `probe` (asserts nothing) |
| 7t | 1.3 | box-drawing strokes unbroken in a captured frame at 14 and 23 px | a PrintWindow capture with real pixels: fragile (F-96 "no ink"); geometry unit-tested in `builtin_glyphs.rs:499-576`, `term_element.rs:1003` | cut |
| 7u | 0.8 | tab drag-to-reorder; Ctrl+Tab follows the new order | `tabs.rs` reorder tests | merge into 7e |
| 7y 1,2,4,5,7,8 | 25.5 (all of 7y) | `sm add` → pending, no task; Approve only from the page (C100); Pause/Resume sync; Run now through real `smw`; Edit; Remove leaves the folder | FakeRegistrar (C101): holds | keep (the only end-to-end of C100 and Run now) |
| 7y 3 / 3c | (in 7y) | `sm` edit → pending, no registrar write; the dismissal file; edited command file re-asks | holds | keep edit → pending → re-approve; merge 3c into 3; the dismissal file → unit |
| 7y 5d, 6, others | (in 7y) | dry run; failed run in Inbox and tray; read-only other schedules | `sm/tests/schedule.rs:262`, inbox and `schedules.rs` unit tests | cut 5d; 6 and "others" → unit (keep "no Task Scheduler write") |
| 7+t | 4.8 | Task icon and menus; `claude.cmd` refused; a two-line prompt reaches claude's argv byte for byte after `--` with the flags | holds; `view/start.rs` 5 tests | keep only the argv-through-pwsh part |
| 7+r | 22.7 | keystroke-to-paint p95 <100 ms, frames p95 <50 ms with 460 live + 2,800 closed fixtures | a measurement (responsive/1); ~21 s of paced typing | → `--selftest-perf` |
| 7z | 6.5 | OCR of the window and 5 Settings pages reads Supraman/Workman, never Foreman; tray tooltip | a one-time rename (2026-10-31, done); needs Windows OCR | cut; a source scan for "Foreman" in UI string literals instead |
| 7+c | 0.0 | every cached sidebar tree equals a fresh build across the run | holds; cheap | keep; add a `fleet::next_change` unit test |
| 8 | 7.0 | 6 s idle window: own work p10 ≤16 ms, no frame >50 ms when quiet; panel refreshed; `main_repo` walks ≤ polls × sessions | quiet-machine heuristics; `baseline` waits up to 60 s (F-97) | keep refresh and walks; frame budget → `--selftest-perf` |
| watchdog | – | the whole-run budget (270 s) | the baselines alone can use 240 s under load, so a FAIL names whichever step runs, not the slow one | per-step budgets, or the baselines capped and SKIP; LEARNINGS still says 210 s |
| cleanup | 10.9 | own pids, own session by name, own transcripts, selftest credential, scratch dirs | holds C15 | keep |

### `--selftest-supervise` (400 s, 18 steps, 12 real claude sessions, 3 with the heavy workspace-controller role)

| step | s | guards | rests on (holds?) | verdict |
|---|---|---|---|---|
| a | 0.2 | the testbed under `target/`, Workman at a closed port, relay idle 60 s; a pre-m5/32 desired state with the relay listed | the pre-m5/32 entry: no user has one (first release 0.1.2, after m5/32) | keep; drop the legacy relay entry |
| b | 10.7 | Start launches `fm-<w>-workspace`; `SessionStart(startup)` hook; role tag in the transcript; the plugin's skills listed; the workspace row is the session's | the real CLI's hook and skill listing: holds. `workspace_row_step` is unit-covered (`view::menu::tests`) | keep launch, hook, role and skills; cut `workspace_row_step`, the `no_relay` check here, and the unasserted first-turn report |
| e | 10.7 | a default-mode session parks on a permission prompt: Notification hook, Inbox Blocked with Attach | real CLI `blocked/waiting/waitingFor`: holds | keep (the only end-to-end of the hook and the Inbox) |
| e2 | 4.9 | another home's `fm-` row is muted, has no actions, raises no Inbox row | injected row, no claude; needs e's park still Blocked | down (`fleet::another_homes_fm_rows…`, `reconcile::another_homes_parked…`), or into `--selftest` |
| t0 | 2.8 | a Messages row under a resting pointer does not move the Task icon; a table paints unwrapped | timing against HOLD_STILL 2 s (F-02); here only because the gate's Messages section was full | → `--selftest` with the hold logic already there, or down to a hold-timer unit test; drop the duplicate Task-icon check |
| r | 102.3 | (1) 30 sends straight into a real session's pipe, all 30 nonces back; (2) forced fallback starts the on-demand relay, labels, reply in Messages; (3) the relay is retired after 60 s idle | (1) the peerProtocol 1 drift guard: holds; (2) the relay is the live fallback: holds; (3) re-asserts a timer, unit-covered (`supervise::tests::the_relay_is_wanted…`); the target's env is hand-built (`SS:126-131`) beside `LaunchConfig::session_env` (C56) | keep (1) with 3 sends; keep (2) once; cut (3), which saves ≥60 s; use `session_env` |
| c (+h) | 36.3 + 2.5 | `claude stop` on the idle controller, the supervisor resumes it with the same id and sessionId; WM_CLOSE hides, ticks continue hidden, Open repaints | the real `--bg --resume <uuid>`: holds; `supervised_health` reads blocked/idle as Idle (7b52e29); the 30 s respawn grace is irreducible. F-95: one `alive(pid)` 3 s after the resume (`SS:382, 391`) | keep, h merged; a bounded pid poll (fixes F-95) |
| t | 36.4 | Reply… on DECIDE:, Ctrl+Enter sends direct, framed in the transcript; APPROVE? sends `approve`; `answered.json`; Escape sends nothing | holds. The editing keys are unit-tested (`text_input` tests). The 15 s settle exists for f's budget of 1, and its comment (`SS:1858`) is wrong | keep Reply and Approve delivery; cut the keystroke editing (`1771-1795`, `1836-1842`); drop the settle with f |
| w | 24.8 | `sm want` → the supervisor launches `fm-<w>-repo`; its own `sm roster` runs without parking (`--allowedTools`); `sm unwant` stops it, not resumed | the real CLI's permission matching of `sm_allowed_tools`: holds | keep (the only real check that `sm` does not park); absorbs x's "a removed folder's session stops" |
| d | 7.9 | Start on a `git init` folder: claude refuses, TrustNeeded with Run in terminal, a pwsh tab with the command typed | claude's trust refusal text: holds (unit tests pin the parse) | keep (cheap; guards drift in the refusal text) |
| f | 31.7 | with limit 1 two workspaces never work at once | unit-covered (`executor::another_homes_busy_sessions_take_no_slot` "step f on fixtures", `two_started_workspaces_never_exceed…`, `reconcile::cap_counts…`); 2 heavy sessions | cut (down already) |
| s | 2.3 | in a 480 px window the tree's lowest row scrolls fully into view | predates the virtualized sidebar; `scrolled_into_view` exercises `KeyedList::reveal` in every step | cut |
| v | 56.8 | a git and a plain folder each run their session under their folder; not started → stopped; double-click on a stopped `fm-` row gives the toast | fragile: its git folder is the checkout itself, so a session runs outside `target/` (C15's edge). It fails on a detached HEAD (`branch.is_none()`) and on a `sched-…` checkout name, and it makes every later testbed folder `also_in` it (the 2026-11-02 key workarounds in x and y). Claims unit-covered (`fleet::a_folder_enabled_in_two…`, `wanted_sessions_sit_under…`, `reconcile::not_started_stops…`) | cut (removes the x/y workarounds) |
| x | 14.5 | Add workspace → Empty, two folders by the picker, home set, session under XPlain, XGit disabled leaves the tree, removing XPlain asks and stops its session | the dialog needs no claude; `selftest_manage` already drives Add and Edit folders | dialog part → `--selftest`; "removing a folder stops its live session (stop, never rm)" → w |
| y | 28.1 | Adopt: the unmanaged session is stopped and stays listed; `fm-<ws>-<folder>` runs as a fork under a new id, transcript carries the turn, no second Adopt | the only check of `--resume <uuid> --fork-session` against the real CLI: holds. The adoptee is named `selftest-<hex>-adoptee`, not `fm-selftest-…` (C15's edge); UI checks unit-covered | keep the real fork in a release-only tier (`--selftest-supervise --release`), not the merge gate; cut the question checks |
| g | 8.9 | Stop stops controller, park and relay; they stay listed; none restarts | holds | keep; becomes Stop all (q) |
| q | 3.0 | tray Stop all saves every workspace as not started | no unit test of `tray::Command::StopAll` | merge into g |

### `--selftest-detach-shutdown` (14 s, 1 real session)

| phase | guards | verdict |
|---|---|---|
| register, launch | shared with `--selftest` steps 1 and 2 | keep |
| attach | duplicates gate steps 3 and 4 | merge (shared helpers already) |
| WM_CLOSE hides with an attach tab open | supervise h covers the hide, not with an attach tab | keep |
| Quit detaches | tray Quit closes the attach tab (Ctrl+Z), then the window; the session keeps its pid; no teardown errors. The only test of `should_close`/`quit_step` with an attach tab | keep |

`--selftest-detach <n>` is a manual soak (n attach/close cycles) that no gate runs. Keep it as a tool and list it in ARCHITECTURE as one.

### `crates/*/tests/*.rs` (19 files, 129 tests)

Every test below starts no real claude and uses FakeWorkman or ephemeral ports. None uses 127.0.0.1:5290, the tag-triggered release or the fm binary.

| file | tests | verdict, with the assumption |
|---|---|---|
| `sm/tests/fire.rs` | 19 | keep 16. `fire_records_a_dead_run…` + `the_app_recovers_a_run…` → one test. `fire_through_fm_and_fmw…` + `fmw_runs_now_for_the_window` → one test; it also reaches the real `claude` on PATH because `sm_bin` sets no `CLAUDE_CONFIG_DIR`, so set it. `fire_restart_interrupts_the_running_script` (flakes/F-01): replace the <15 s elapsed assert (`:419`) with the outcome asserts it already has |
| `sm/tests/fire_session.rs` | 12 (1 ignored) | keep 11 (the only end-to-end of the session stage, C1–C4); cut `fire_session_real_run_scratch_home`: ignored, writes outside a tempdir, its doc says `FOREMAN_HOME`, a manual tool |
| `sm/tests/hook.rs` | 4 | keep 3; cut the `FOREMAN_PLATFORM` half of `the_workspace_role_text…`: no product code reads it (a past rename's mechanics); `…only_foreman_home_still_logs` stays while `FOREMAN_HOME` is read (R-224) |
| `sm/tests/relay.rs` | 5 | keep; `inbox_prints_each_send…within_a_second`: drop the 1 s assert (`:88`) |
| `sm/tests/schedule.rs` | 14 | keep 11. `read_runs_in_the_main_checkout…` → into the dry-run test. Cut `add_and_edit_take_no_worktree_option`: it guards a removed flag and never tests `edit`. `covered_edits_pause_and_resume…`: shrink, since the edit loop repeats core tests. `an_earlier_worktree_schedule_keeps_its_worktree…`: keep until those files are out of use |
| `sm/tests/sessions.rs` | 1 | merge into `workspace.rs` `user_errors_exit_1` |
| `sm/tests/skills.rs` | 9 | keep 2: the human-channel test, which compares skill text with `human::framed`/`peer::FROM_NAME`, and the scheduler `allowed-tools` check (a permission boundary). Cut 7 exact-phrase checks or fold them into one lint; one pins the stale `rlarch:controller` text (R-201) |
| `sm/tests/update.rs` | 8 | keep all (the strongest updater tests; core covers only unpacking); `:281,287` → `no_window` |
| `sm/tests/want.rs` | 4 | keep |
| `sm/tests/work.rs` | 4 | keep 3. Cut `work_is_in_the_help`: trivial, and it points at `127.0.0.1:9`. `work_errors_are_one_line_without_the_token` (flakes/F-05): the explanation on record (Credential Manager, `CREDENTIAL_WRITERS`) no longer fits the code, because the store has been the DPAPI FileStore since m5/44.18 and `CREDENTIAL_WRITERS` is gone. It needs a fresh diagnosis: the test reads the store without `refresh.lock` while three `sm` children refresh |
| `sm/tests/workspace.rs` | 6 | keep; `io_errors_exit_2`'s comment says `FOREMAN_HOME` (should say `SUPRAMAN_HOME`); `the_default_home_is_refused_until_the_foreman_home_is_copied` until the migration window ends |
| `supraman-app/tests/elevation_handoff.rs` | 1 | move out of `cargo test`: it returns at once unless elevated (vacuous), and when elevated it starts raw `supraman.exe`, `runas` and `cmd` for 35+ s; `instance.rs`/`elevate.rs` unit tests cover the logic |
| `supraman-claude/tests/fake_claude.rs` | 6 | merge into 2–3 (one launch, not installed, stop/rm/resume); two repeat `launch.rs` unit tests, and `launch_checks_the_list_then_launches` asserts almost nothing |
| `supraman-supervisor/tests/workspace_files.rs` | 2 | keep the root-based file round trip; cut `the_users_workspace_files_keep_their_session_names`: it reads untracked `.claude/briefs/*.toml`, absent here, so it passes having checked nothing |
| `supraman-ui/tests/panic_log.rs` | 1 | keep (needs its own process: the logger and hook are global) |
| `supraman-workman/tests/client.rs` | 7 | keep 6; merge `client_reads_x_workman_api…` into `client_maps_errors…` (both, and `stream.rs` unit, assert "without X-Workman-Api") |
| `supraman-workman/tests/fake.rs` | 3 | cut `a_changed_route_answers_the_next_request` (tests the fake) and `a_stream_answer_without_the_v1_header_is_refused` (= `stream.rs` unit); keep one of the reconnect tests (this or `stream.rs` test 1) |
| `supraman-workman/tests/stream.rs` | 2 | its hand-rolled TCP SSE server breaks the CLAUDE.md rule "FakeWorkman, never a hand-rolled server". Keep the Last-Event-ID reconnect, the fast stop and the backoff, on FakeWorkman if it can serve them |
| `supraman-workman/tests/oauth.rs` | 21 | keep 19 (real product behaviour); merge `sign_in_asks_for_the_workman_scopes` into `sign_in_discovers…` (`:68`); `the_fake_names_why_it_refuses…` → the fake's own tests; `a_grant_under_the_old_scope_names…` is time-bound (drop with Workman's old-scope aliases) |

**Unit tests worth a look**:
- `term/alacritty/hyperlinks.rs` has 1,484 lines of vendored Zed tests. Keep or drop them with the A-53 decision.
- These are kept: `supervisor/executor.rs` (1,356), `core/schedule/tests.rs` (829), `app/hand.rs` (604), `app/fleet.rs` (553).
- `transcript.rs` has three ignored benchmark tests. Move them to `sm probe` or delete them.

### Flakes, folded in (c5)

| flake | where | cause, as far as known | handled by |
|---|---|---|---|
| flakes/1 | – | heavy jobs overlap on one machine | U2, first: the machine-wide lock, before any selftest change is judged |
| F-02 | supervise t0 | a Messages row arriving after the 2 s HOLD_STILL moves the row under the pointer | U4: t0 → `--selftest` with the hold logic, or a hold-timer unit test |
| F-95 | supervise c | one `alive(pid)` check 3 s after the resume | U4: a bounded pid poll |
| F-96 | `--selftest` 7t, 7j | a pixel capture and poll-tied bounds under load | U3: 7t cut, 7j trimmed to "eventually" |
| F-97 | `--selftest` 8 | `frames::baseline` waits up to 60 s, times 4, inside the 270 s watchdog | U3: frame budgets → `--selftest-perf`; capped baselines |
| F-05 | `sm/tests/work.rs` | the record's explanation no longer fits the code (see above) | U5: fresh diagnosis; fix or rewrite the test |
| flakes/F-01 | `sm/tests/fire.rs:419` | a <15 s wall-clock assert | U5: the outcome asserts only |

## Targets (c3)

| measure | now | target | how |
|---|---|---|---|
| `--selftest` wall time | 224 s | ≤ 130 s, and no step waits on a quiet machine | 7+r, frame budgets and 3s bound → perf (−30 s); 7j trim (−25 s); 7t, 7z, 7n parts, 7r, 7y 5d/6 cut (−20 s); 7b halved (−9 s); 6c down (−4 s); fixed sleeps → waits (−5 s) |
| `--selftest-supervise` wall time | 400 s | ≤ 180 s (merge gate); y in a release-only tier | r retire and 27 sends (−85 s), v (−57 s), f (−32 s), t settle and editing (−20 s), x dialog moved (−10 s), y to the release tier (−28 s), s (−2 s) |
| real claude sessions per gate | 13 (12 supervise + 1 gate; +1 detach) | 8 | supervise keeps controller, park, target, relay, repo (w), trust refusal; y's 2 only in the release tier |
| `--selftest-detach-shutdown` | 14 s | 14 s | unchanged |
| selftest lines | ~11,700 | ~9,000 (−23 %) | `--selftest` −1,800 (cuts and moves), supervise −500, the `also_in` workarounds gone |
| test count | 935 | ~925 | −14 cut, −10 merged, +15 unit tests for moved-down checks |
| merge-gate machine time | 638 s | ≈ 325 s | the three modes above |

**Coverage given up.** Each item below is covered by unit tests or another step, so no product behaviour is left without a test:
- The relay's idle retire, end to end; unit-covered.
- The busy budget with two real heavy sessions; unit-covered with fixtures ("step f on fixtures").
- Two folders of one workspace each running a real session; unit-covered placement and stop.
- The box-drawing pixel capture; the geometry is unit-tested.
- The brand OCR; replaced by a source scan.
- The keystroke and frame budgets on every merge. They move to `--selftest-perf`, which the release gate (×5) runs.
- The real `--fork-session` on every merge. It moves to the release tier, so a CLI change to fork shows at the next release, not the next merge.

## Reuse inventory for GitMan, FileMan and the Work tab (c4)

Sources:
- roadmap/3's notes 3179 and 3183: plugins are workspace crates linked into Supraman, switched on in Settings, each with a standalone binary, with their views in Supraman's tab view;
- note 3270: the shared component plan;
- note 3272: Zed's git crate first, gix only if measured;
- r2: the file explorer follows the active tab;
- work-tab-redesign.

Legend: **shared** = exists in a shared crate; **app** = exists only in supraman-app; **missing** = does not exist.

| piece | status and where | coupling | target |
|---|---|---|---|
| TextInput, tab strip + hover card, context menu, confirm, toast/notices/banner, matcher, details page, settings pages, nudge, stall, themes, fonts, hit targets, buttons, keyed list | shared (`supraman_ui::*`) | – | use as is |
| Split panels | shared `split::SplitPane`: horizontal, two panes | – | extend in ui: an axis and n panes (explorer \| editor \| terminal) |
| Tab model (`Tab`, `TabKind`, push/open/close/active/cycle, saved tabs) | app `view.rs:118-223, 2193-2214, 2301-2386, 2654-2779`; `tab_card.rs`; `saved_tabs.rs`; `view/reopen.rs` | `Tab` holds `Entity<Terminal>`, find state, hand/mint fields; `TabKind` holds `WorkView`, `SchedulePage`, `DetailsPage`, `StartPage`; all on FleetView | ui: a generic `TabSet` (ids, active, reorder, cycle) plus `TabKind::Plugin(AnyView + {title, cwd, closable})`; `SavedTab::Plugin{kind, path}`; detach and quit stay in the app |
| Tree view | app `view/left.rs:28-42, 162-212, 511-614` (row model, grid, disclosure, flatten), pointer hold `47-53, 323-365`; collapsed set `view.rs:239` | FleetView (`left_list`, `left_rows`, `collapsed`), `fleet::Tree` | ui `tree`: generic nodes → keyed rows with level, disclosure, icon, label and hover slots, on KeyedList (file tree, ref tree); the fleet model stays in the app |
| Virtualised table with columns | missing (KeyedList has no columns) | – | ui `table` on `uniform_list` (commit graph, file lists) |
| Modal form dialog | missing; app examples: Edit folders (`view/folders.rs`), schedule Edit (`view/schedules.rs:415-520`) | FleetView fields | ui `dialog` (title, form, OK/Cancel, Esc) for commit/branch/stash dialogs |
| Fuzzy palette | missing; parts: `matcher` (substring/prefix), app `search.rs`, `view/results.rs` | `search.rs` on `fleet::Tree` | ui `palette` (input + matcher + KeyedList); a fuzzy scorer in `matcher` |
| Key-context binding helpers | app `view.rs:69-82`, `text_input::bind_keys` | – | ui `keys` |
| Icons, text tooltip, clipped label, checkbox, folder picker, open-with/links | app (R-107, R-108, R-109, R-111) | theme only | ui |
| Custom titlebar and window lifecycle | app `titlebar.rs:1-109`, `tray.rs:187-299` | fonts, theme | ui `window`, for `gitman.exe`/`fileman.exe` |
| Debounce | app `work_view.rs:32-40` | – | ui `nudge::debounce` (the watcher needs it) |
| Process helper | shared `supraman_core::process` | – | use as is |
| File watcher with event classes (2 s workdir / 200 ms `.git`) | missing; the app polls (Poller 2 s, transcripts 10 s) | – | core `watch` (notify + classes) + ui debounce |
| SecretStore; namespaced settings tables | shared (`secret`; `Settings::section`/`set_section`, R-221) | – | use as is |
| Settings window host | shared `settings_page`; app host `settings_window.rs:20-98, 855-921` | pages are typed over `SettingsView`, which holds `WeakEntity<FleetView>`, so a plugin crate cannot register a page | ui: a host generic over a trait / `AnyView` pages |
| Git helpers | shared `scan::branch_at`, `paths::{main_checkout, linked_git_dir}`; app `row_details.rs:264-273`, `fleet.rs:174-181` | – | core `paths::git_at` (R-114); real git work in gitman via Zed's git crate |
| Active tab's cwd/repo context | app `Tab.cwd` (`view.rs:146`), `new_shell_cwd` (`:2188-2191`), pure `fleet::shell_cwd` (`fleet.rs:617-625`), `tab_card::place` (`:76-82`); the change event is missing | FleetView `active`/`tabs` | pure parts → core; a host API (`active_context() → {cwd, repo, workspace}` plus a change event) so FileMan's explorer and GitMan follow the tab (r2) |
| Time formatting; exe dir / sm.exe | app (R-110, R-112) | – | core `time`, `install` |
| supraman-editor (headless host, editor, read-only diff) | missing | – | new GPL crate (note 3270) |
| Terminal panel | shared supraman-term + `term_element`; app host `view.rs:2220-2299, 2527-2591, 2715-2760`, `view/term.rs`, `view/drop.rs` | FleetView (tabs, font, toast, settings); attach detach (Ctrl+Z) is app policy (CLI) | ui `TerminalPane` entity emitting Exited/Title/Open to its host; detach, hand mint and resume stay in the app |
| Work tab: WorkHub (client and stream lifecycle) | app `work_view.rs:42-200` | `signin::auth`, `WsStatus` | stays in the app (or a `supraman-work` crate); gpui-free parts → workman |
| Work tab: views and pure helpers | app `work_view.rs:202-751`, `work_view/{item,journal,inbox}.rs`; only FleetView link is `FleetLink` (`item.rs:9-20`) | – | pure helpers (`pick_project`, `item_rows`, `board_to_reload`, `item_to_reload`, `inbox::touched`, …) → workman, shared with `sm work`; the redesign uses KeyedList or the new table instead of plain scroll divs; C21 (GET only, local dismissals) |

## Proposed units (c5)

There are four rules:
- Units in the same wave touch disjoint files. Waves run in order.
- Every unit is gated by build, test and selftest once, run at the priorities of `builds-below-normal`.
- No unit starts before René approves the test verdicts above.
- Protected paths are named per unit.

| wave | unit | content | files | prot. |
|---|---|---|---|---|
| 1 | **U1 Hidden test spawns** | the rest of F-01: an `sm_cmd()` helper over `process::command` for the 17 sm test spawns, `process::command` in the unit tests listed under F-01, a grep lint (no `Command::new` outside `supraman_core::process` except `runas`). R-201 and the `process.rs:345` cause are already fixed by the `skills-workman` hotfix | `crates/sm/tests/*`, `core/process.rs` (test only), `claude/{forget,launch}.rs` tests, `app/hand.rs` tests, `supervisor/mint.rs` test | none |
| 1 | **U2 flakes/1** | the machine-wide lock for `--selftest*` (`lock_file` under %TEMP%) and the scratchpad gate/build scripts; the teammate protocol | `app/main.rs`, `app/selftest_clipboard.rs` (`hold_machine`), `core/fs.rs` (if a helper is added), scratchpad `gate.sh` | C15/17 |
| 2 | **U3 `--selftest` trim** | the `--selftest` verdicts above, F-96 and F-97; per-step watchdog budgets; a fixture workspace instead of the real parent scan; `%TEMP%` scripts → `target/` | `app/selftest*.rs` except `selftest_supervise.rs`; `app/selftest/frames.rs`; the unit tests the moved checks land in (`app/fleet.rs`, `view/*.rs` test modules, `selftest_clipboard.rs` test) | C15/17/21, Sched (7y) |
| 2 | **U5 cargo tests** | the `tests/*.rs` verdicts, F-05 (diagnose first), flakes/F-01, `elevation_handoff` out of cargo test, `stream.rs` onto FakeWorkman | `crates/*/tests/*.rs`, `workman/src/fake.rs`, `claude/src/transcript.rs` (ignored benches) | OAuth, WS (tests only) |
| 2 | **U7 Core fragility** | R-208…R-218, R-203, R-202 (one mint/end helper), R-206, R-207 | `core/{process,settings}.rs`, `claude/{peer,human,launch}.rs`, `supervisor/{executor,mint}.rs`, `workman/{auth,stream,updates}.rs`, `sm/src/{fire,run}.rs` | CLI, Mint, OAuth, Sched, Update, WS |
| 3 | **U4 `--selftest-supervise` trim** | the supervise verdicts, F-02, F-95; the release-only tier for y; Stop all as the teardown; `session_env` in r | `app/selftest_supervise.rs`, `app/selftest_left.rs`, the `--selftest` steps that receive t0, e2 and the x dialog (after U3 lands) | CLI, C15/17 |
| 3 | **U6 App correctness** | R-101 (first), R-102, R-103, R-104, R-105, R-116, R-117, R-118, R-119, R-120 | `app/view.rs`, `app/view/{manage,menu,details,notices,schedules,start,delete}.rs`, `app/main.rs`, `app/tray.rs`, `app/supervise.rs`, `app/tokens.rs` | Del (R-119), Sched (R-105), CLI (R-102) |
| 4 | **U8a Small shared extractions** | R-106…R-114, R-204, R-205 (icons, tooltip, clipped, checkbox, picker, open, time, exe dir, git_at, hex, slug rules) and their CLAUDE.md/ARCHITECTURE.md entries | `supraman-ui/src/*` (new modules), `core/{time,install,paths,slug}.rs`, the app callers | none |
| 5 | **U8b GitMan/FileMan groundwork** | `TabSet` + `TabKind::Plugin`, the ui `tree`, `table`, `dialog`, `palette`, `keys`, `window`, a generic settings host, `TerminalPane`, the active-context API, core `watch`. Each piece can be its own unit; plan them with roadmap/3, not inside this run | `supraman-ui`, `supraman-core`, `app/view.rs` and the tab/settings files | CLI (terminal detach) |
| 6 | **U9 Rename leftovers** | R-223 (after a forced `sync` at start), R-224, R-225, R-226, R-227, R-228; R-229/R-230/R-231 with a one-release compat window; R-222 (feature-gate probes, `http_browser*`, the `fake` link) | many files, runs alone | Sched, CLI, C15/17 |
| 7 | **U10 Comment passes** | R-121, R-232 per crate group (app; core+claude+supervisor; workman+sm+plugins; term+ui after the A-53 decision) | per group, after everything else | – |

The Work-tab redesign (roadmap/work-tab-redesign) starts after U8a. It is its own proposal for René, per its brief. It reuses U8a's badge, time and table pieces, and fixes R-115 there.
