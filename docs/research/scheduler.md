# Scheduler: design and probes (sched/1)

The design every later `sched` unit builds on. It comes from the analysis René approved (run `sched`, 2026-10-30) and from reading the code at `6546be4`. Binding rules: C100 (only René's action in the window approves), C101 (no real Windows task outside a unit's own probe folder), C102 (unattended runs: profile, caps, stop never rm, token revoked, no secret in any file). Approval follows sched/10's answer **(b)+(c)**.

Each section ends with what it decides for the unit that builds it. The choices that were open are settled under [Decisions](#decisions) (René and the controller, 2026-10-30).

## Where things live

Everything is in Foreman's home (`Registry::open`: `FOREMAN_HOME`, else `%APPDATA%\Foreman`). Nothing goes in a workspace folder except a run's git worktree.

| Path | What | Written by |
|---|---|---|
| `schedules/<workspace>.toml` | the schedules of one workspace | `foreman_core::schedule` (fm and app), under `lock_file(schedules/<workspace>.lock)` + `write_atomic` |
| `schedules/runs/<workspace>.jsonl` (+ `.1`) | the run records | `append_capped` (1 MiB), `prune` keeps the `.1` for 30 days |
| `schedules/live/<workspace>/<id>.json` | the run in flight: fm's pid and start time, the session, the minted WorkState session id (never the token) | fm, removed at the end of the run |
| `schedules/live/<workspace>/<id>.lock` | the overlap lock | fm (held while the run lives) |
| `schedules/state/<workspace>/<id>/` | the script's own state dir, kept between runs | the script |
| `schedules/fire.log` (+ `.1`) | fmw's diagnostics (it has no console) | `append_capped` (1 MiB) |
| `oauth/secrets/<sha256(key)>.bin` | the approval key and per-schedule secrets (DPAPI) | `FileStore` |
| `mcp/workstate-<session name>.json` | the run session's minted-token config | `LaunchSpec::with_minted_token` / `mcp_config_path`, deleted at the end |

## The schedule TOML

`<home>/schedules/<workspace>.toml`, one table per schedule. Durations use the `humantime`-style forms `30s`, `10m`, `2h`; times are local wall-clock `HH:MM`.

```toml
version = 1

[[schedule]]
id = "autofix-prod-bugs"            # validate_name, unique in the workspace, at most 32 chars
name = "Autofix prod bugs"           # free text; not covered
description = ""                     # free text; not covered
folder = "Nomos"                     # a Folder of the workspace, by name (Workspace::folders)
status = "pending"                   # pending | active | paused
created_by = "fm"                    # fm | app
created_at = 1793337600000           # epoch ms

[schedule.trigger]                   # covered
kind = "weekly"                      # weekly | daily | interval
days = ["mon"]                       # weekly only: mon..sun
at = "09:00"                         # weekly, daily; interval: the first start of the day
# every = "2h"                       # interval only; at least 15m

[schedule.action]                    # covered
kind = "session"                     # session | script_then_session | script
prompt = { command = "/autofix-prod-bugs" }   # or { file = "prompts/x.md" } or { text = "…" }; not for kind = "script"
script = "scripts/gate.ps1"          # relative to the folder; script_then_session and script only
script_args = []
script_timeout = "10m"
worktree = true                      # the session runs in a fresh worktree of the folder
model = ""                           # empty: claude's default

[schedule.permissions]               # covered
profile = "worktree-edit"            # read | worktree-edit | auto (see "Permission profile and caps")
allow_tools = ["Bash(dotnet test:*)"]  # added to the profile's
deny_tools = []                      # added to the profile's
add_dirs = []
secrets = ["SENTRY_TOKEN"]           # names only; values in SecretStore

[schedule.caps]                      # covered
max_runtime = "1h"
max_tokens = 3000000

[schedule.policy]                    # covered
overlap = "skip"                     # skip | queue | restart
catch_up = "once"                    # once | skip
wake = false

[schedule.approval]                  # written only by the app (C100)
fingerprint = "9f2c…"                # hex sha256, see below
mac = "41ab…"                        # hex HMAC-SHA256, see below
approved_at = 1793341200000
covered = '{"action":…}'             # the canonical covered JSON as approved: what Approve diffs against
command_sha256 = "c0de…"             # the slash command file's hash at approval; not covered (shown, never refused)
```

Rules for `foreman_core::schedule` (sched/2):
- An edit through `foreman_core::schedule` that changes any covered table, or a file the fingerprint hashes, sets `status = "pending"`. The edit leaves `[schedule.approval]` alone, so Approve can show the change. `name` and `description` stay as they are.
- Removing a schedule removes its table; the app's `Registrar::sync` then removes its task.
- A schedule whose `[schedule.approval]` does not verify (see next section) counts as pending wherever it is read, whatever `status` says. The UI shows it as "changed since approval".
- The workspace name is the file name, so a schedule never names its own workspace.
- Next fire: computed from the trigger in local time (`chrono` with the `clock` feature, as `foreman-ui` has it). An `at` that a DST change skips fires at the first minute after it. One that repeats fires once.

## Approval: the fingerprint and the HMAC

**What the fingerprint covers.** `fingerprint = sha256(canonical JSON)`, where the JSON has keys in a fixed order (serde structs, no maps):

```
{ "v":1, "workspace":…, "id":…, "folder":…,
  "trigger":{…}, "action":{…}, "permissions":{…}, "caps":{…}, "policy":{…},
  "files":{ "<path relative to the folder>": "<sha256 of its bytes>", … } }
```

`files` holds the script and the prompt file: a session could change what runs by editing them, so their content is covered. A slash command (`prompt = { command = "/x" }`) is covered by its name and its resolved path (`"command":{"name":"/x","path":".claude/commands/x.md"}` in the action), not by its content (Decision 2). `approval.command_sha256` records the file's hash at approval. It is not covered: when the file no longer matches, the row shows "changed since approval" and the schedule still fires. The rest (`name`, `description`, `status`, `created_*`, `approval`) is not covered.

**The MAC.** `mac = HMAC-SHA256(key, "foreman-schedule-v1\n" + workspace + "\n" + id + "\n" + fingerprint)`, as hex.

- **The key.** 32 random bytes, kept as hex in Foreman's `SecretStore` (`foreman_core::secret::default_store(home)`, the DPAPI `FileStore`) under the name **`foreman/schedule-approval/v1`**. Only the app creates it, on the first Approve, under `lock_file(<home>/schedules/approval.lock)`. `getrandom` 0.3 is in `Cargo.lock` already (`foreman-workstate`). fm reads the key and never writes it. A missing key means nothing was ever approved, so fire refuses.
- **The crate.** `hmac` 0.12.1 with `sha2` 0.10.9. Both are in `Cargo.lock` already: `hmac` 0.12.1 comes in through `hkdf`, and its `digest` 0.10 matches `sha2` 0.10.9, which `foreman-core` already depends on. Adding `hmac = "0.12.1"` to `foreman-core` adds no package to the lock, so it needs no question to the controller. Verify with `Mac::verify_slice`, which is constant-time.
- **What fire checks before anything runs**, recording `refused` with the reason if any check fails:
  1. `status == "active"`;
  2. the fingerprint computed now (TOML + file hashes) == `approval.fingerprint`;
  3. `mac` verifies under the key;
  4. the `--fingerprint` in the task's own command line == the computed one.

  Check 4 is sched/10's (c): only the app registers the task (`Registrar::sync`, run after Approve), so a schedule edited behind the app has no task carrying its new fingerprint. The task that does exist refuses to fire.
- **What remains possible.** A process running as René can call DPAPI with the same entropy (it is in the source), and it can register tasks of its own. The HMAC and the task's fingerprint stop an agent that edits Foreman's files. They don't stop one set on running code as René, which needs no scheduler to do harm. sched/10 accepted this.

sched/2 builds the fingerprint, `verify(approval, key)` and the "covered changed → pending" rule. sched/7 builds `approve(workspace, id)`, the one call that writes `[schedule.approval]`, and calls it only from the window's Approve. No `fm` path writes the approval (C100; sched/4's c2 proves it).

## The script contract

**How it is run.** A script runs from the folder's main checkout, before any worktree exists. It goes through `foreman_core::process::run` with no console window and a `Kill::Tree` timeout of `script_timeout` (default 10m, at most 1h). The extension decides how:

| Extension | Command |
|---|---|
| `.ps1` | `pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File <script> <args>` (`powershell.exe` when there is no `pwsh`) |
| `.cmd`, `.bat` | `cmd /d /c <script> <args>` |
| `.exe` | the exe itself |

Any other extension is refused by `fm schedule add`.

**Exit codes**, for `script_then_session`:

| Exit | Meaning | Recorded |
|---|---|---|
| 0 | nothing to do | `nothing_to_do` |
| 10 | launch the session; stdout is added to the prompt | the session stage's outcome |
| anything else, or the timeout | failed | `failed`, with the last 2 KiB of stderr |

For `kind = "script"` there is no session: 0 and 10 record `ok`, anything else `failed`.

**Environment.** On top of the user's environment the script gets:
- `FOREMAN_HOME`, `FOREMAN_WORKSPACE`, `FOREMAN_SCHEDULE` (the id), `FOREMAN_RUN` (the run id);
- `FOREMAN_FOLDER` (its checkout);
- `FOREMAN_LAST_RUN` (epoch ms of the previous run's start, empty for the first) and `FOREMAN_LAST_OUTCOME`;
- `FOREMAN_STATE_DIR` (`schedules/state/<ws>/<id>/`, created before the run);
- each `secrets` name with its value from `SecretStore` key `foreman/schedule/<ws>/<id>/<NAME>`. The values exist only in the child's environment, and every stderr tail is masked for them before it is recorded.

**The prompt.** Stdout is capped at 32 KiB, with the cut marked. It goes into the prompt as:

```
<base prompt: the slash command, the file's text, or the text>

<gate-output>
…stdout…
</gate-output>
```

In front of that comes the run preamble: "this is an unattended scheduled run `<run id>` … when you are finished, run `fm schedule report <run id> done "<one line>"`; if you cannot go on without the human, run `fm schedule report <run id> needs-you "<why>"`". `Bash(fm:*)` is already in every Foreman session's `--allowedTools` (`LaunchConfig::allowed_tools`, m3/2).

sched/5 builds the script stage. sched/6 builds the prompt assembly and `fm schedule report`.

## The run record

One JSON line per event in `schedules/runs/<workspace>.jsonl`: `started`, `stage`, `ended`. A reader folds the lines by `run`. Never a token, never stdout, and stderr only masked.

```json
{"ts":1793595600000,"run":"autofix-prod-bugs-20261102-090000","schedule":"autofix-prod-bugs","event":"started",
 "trigger":"task","fingerprint":"9f2c…","fm_pid":4242}
{"ts":…,"run":"…","event":"stage","stage":"script","code":10,"ms":5321}
{"ts":…,"run":"…","event":"stage","stage":"session","name":"fm-nomos-sched-autofix-prod-bugs-20261102-090000",
 "id":"948719b4","session_id":"948719b4-291b-…","worktree":"P:\\…\\.claude\\worktrees\\fm-sched-…","workstate_session":"7f3c"}
{"ts":…,"run":"…","event":"ended","outcome":"done","why":"fixed 2 bugs, PR #41",
 "tokens":{"input":81234,"output":9120,"cache_creation":40211,"cache_read":1200443},"ms":1830000}
```

- `trigger`: `task` | `run_now` | `catch_up`.
- `outcome`: `nothing_to_do` | `ok` | `done` | `needs_you` | `failed` | `refused` | `skipped` | `capped_runtime` | `capped_tokens` | `interrupted`.
- `failed`, `needs_you`, `refused`, `capped_*` and `interrupted` go to the Inbox and the tray (sched/7).

Retention: `append_capped(…, 1 MiB)`. `foreman_core::prune::prune` gains `schedules/runs/*.jsonl.1` and `schedules/state/<ws>/<id>/` of removed schedules. It stays the one retention rule (sched/2).

## The life of `fm schedule fire`

The task runs `fmw.exe schedule fire --home "<home>" <workspace> <id> --fingerprint <hex>`, and Run now runs `fm schedule run <workspace> <id>`. Both enter the same function, with `trigger` set to `task` or `run_now`. Run now has no task fingerprint, so it skips check 4 but makes checks 1 to 3.

1. **Start.** Open `fire.log`. Load the registry from `--home` (an Exec action cannot set environment variables, and the task inherits René's environment, not the app's). Find the workspace and load the schedule. An unknown workspace or schedule writes `refused` to `fire.log` and exits 2.
2. **Approval** (see above): any failure records `refused` and exits 3.
3. **Catch-up.** With `catch_up = "skip"`, when now is more than 10 minutes after the previous scheduled time (from the trigger), record `skipped` ("missed") and exit 0. Task Scheduler's `StartWhenAvailable` fires a missed start once when the machine is back; it does not tell fm which start it was.
4. **Budget.** While this home's mid-turn `fm-` rows (`foreman_supervisor::working`) fill `max_working`, wait for a free slot, checking once a minute. After 30 minutes, record `skipped` ("busy") and exit 0 (Decision 6).
5. **Overlap**, on `schedules/live/<ws>/<id>.lock`:
   - **skip**: try the lock; when it is held, record `skipped` and exit 0.
   - **queue**: wait for the lock, at most one waiter (a `.queued` marker; a second waiter records `skipped`).
   - **restart**: write `<id>.restart`. The owner sees it within one poll, ends its run as `interrupted` ("restarted"), stops and revokes, and frees the lock, which the restarter then takes.

   `foreman_core::fs::lock_file` blocks, so sched/5 adds a non-blocking `try_lock_file` to `foreman_core::fs`, next to it (C56).
6. **Live file.** Write `schedules/live/<ws>/<id>.json`: run id, fm pid with its `process::started(pid)`, stage, and later the session and the WorkState session id. Record `started`.
7. **Script stage**, for `script*` kinds. `script` ends here.
8. **Session stage** (sched/6):
   1. Worktree, when `worktree = true`: `git worktree add -b sched/<id>/<stamp> <folder>/.claude/worktrees/fm-sched-<id>-<stamp> HEAD`. It lives inside the folder, so it falls under the folder's existing claude trust; Foreman never changes trust.
   2. Mint (see [fm without the app](#fm-without-the-app-mint-launch-stop)) and write the mcp config.
   3. `Claude::launch` (refuses a taken name), then `find_by_name` for the row's id and uuid, then `Minter::bind`.
9. **Watch.** Poll `Claude::list_sessions` every 15 s, sleeping in between with no flag polling. Read the session's hook events (`EventReader`) and its transcript's `usage` lines. Check `<id>.restart`. The run ends on the first of:
   - **`fm schedule report done`** → `done`.
   - **Idle after work** → `done` ("idle"): the row is `Idle` or state `done` for 2 minutes after a `Stop` event, with no report.
   - **Blocked** → `needs_you`, recorded at once so the Inbox shows it. Blocked means state `Blocked`, `waiting_for` set, or `fm schedule report needs-you`. The session stays parked so René can attach and answer, and is stopped at `max_runtime` (Decision 7).
   - **`max_runtime`** → `capped_runtime`.
   - **Token total over `max_tokens`** → `capped_tokens`.
   - **The row gone or dead** for 2 polls, without a report → `failed` ("the session ended").
10. **End.** `Claude::stop(id)`, never `rm`. Then `Minter::revoke`, delete the mcp config file, record `ended` with the token totals, delete the live file, release the lock. Exit 0 for `done`/`ok`/`nothing_to_do`/`skipped`, otherwise 1, so Task Scheduler's *Last Run Result* agrees.
11. **Recovery.** fm can die mid-run: sleep, logoff, the task's `ExecutionTimeLimit`, a crash. A live file whose pid is not alive with the recorded start time (`process::alive` + `started`) is a dead run. The next fire of that schedule finds it, and so does the app at start and on each refresh. Whoever finds it stops the session if it is live, revokes the WorkState session id, deletes the config file, records `interrupted`, and removes the live file. Revoking is a call to `/api/agent-sessions`, the same one the supervisor makes; it is not a write to WorkState's work data (C21).

## Task Scheduler registration

**Transport: the COM `ITaskService`**, with the task given as XML (`ITaskFolder::RegisterTask(path, xml, TASK_CREATE_OR_UPDATE, …, TASK_LOGON_INTERACTIVE_TOKEN)`). Not `schtasks /xml`. Why:
- **Folders.** `schtasks` and `Unregister-ScheduledTask` cannot delete a task folder; `ITaskFolder::DeleteFolder` can. `register-probe.ps1` has to go through COM for the same reason.
- **Reading back.** `sync` lists a folder's tasks (`ITaskFolder::GetTasks`) and reads each definition's XML back. `schtasks /query` output is localized and has to be parsed.
- **No child process.** No console to hide, errors come back as typed HRESULTs, and `windows` 0.62 is already a dependency of `foreman-core` (it needs feature `Win32_System_TaskScheduler`, not a new crate).
- **XML as the definition.** It is the format the fake `Registrar` keeps and the tests compare with golden files, and it is what a real task reads back as.

COM runs on a background thread in the MTA (`CoInitializeEx(COINIT_MULTITHREADED)`), never on the UI thread: an STA call pumps messages, the m5/68 crash.

**Folder and name.** `\Foreman\<workspace>\<schedule id>` for the default home. For any other home (a dev home, a second Foreman) it is `\Foreman-<hex8 of sha256(path_key(home))>\<workspace>\<id>`, so two homes never manage each other's tasks. `sync(workspace)` touches only its own folder, and deletes the folder when it ends up empty (sched/3 c1).

**The definition** (the probe registers the same settings):

```xml
<Principals><Principal id="Author">
  <UserId>{current user SID}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel>
</Principal></Principals>
<Settings>
  <MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy>   <!-- fm applies overlap itself and records the skip -->
  <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
  <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
  <StartWhenAvailable>{catch_up == "once"}</StartWhenAvailable>
  <WakeToRun>{wake}</WakeToRun>
  <ExecutionTimeLimit>{max_runtime + script_timeout + 30m}</ExecutionTimeLimit>  <!-- a backstop; fm ends runs itself -->
  <Enabled>true</Enabled>
</Settings>
<Triggers>  <!-- StartBoundary without a zone: local wall time, so DST follows the clock -->
  weekly:   <CalendarTrigger><StartBoundary>…T09:00:00</StartBoundary><ScheduleByWeek><DaysOfWeek><Monday/></DaysOfWeek><WeeksInterval>1</WeeksInterval></ScheduleByWeek></CalendarTrigger>
  daily:    <CalendarTrigger><StartBoundary>…</StartBoundary><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>
  interval: <CalendarTrigger><StartBoundary>…</StartBoundary><Repetition><Interval>PT2H</Interval><Duration>P1D</Duration></Repetition><ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger>
</Triggers>
<Actions Context="Author"><Exec>
  <Command>{install dir}\fmw.exe</Command>
  <Arguments>schedule fire --home "{home}" {workspace} {id} --fingerprint {fingerprint}</Arguments>
  <WorkingDirectory>{home}</WorkingDirectory>
</Exec></Actions>
```

**"Run only when the user is logged on" (`InteractiveToken`) is required, not just preferred.** DPAPI user keys are available only in a logged-on session. "Run whether logged on or not" without a stored password (S4U) has no DPAPI key and no network credentials, so the FileStore could not be opened. Storing René's password is not an option.

**No console window: `fmw.exe`.** `fm.exe` is a console program, and so is `foreman.exe`, which only `FreeConsole`s after it starts (`foreman-app/src/main.rs:139`). An `InteractiveToken` task running a console exe opens a visible console window for the whole run. That breaks the red line, so the probe deliberately did not try it. sched/5 adds a second binary, `fmw`, to `crates/fm`: `src/bin/fmw.rs` with `#![windows_subsystem = "windows"]`, calling the same `schedule` entry point (this means `fm` grows a `lib.rs`). It writes its diagnostics to `schedules/fire.log`. It ships next to `fm.exe` (`docs/RELEASE.md`). The probe is built the same way (`#![windows_subsystem = "windows"]`) and reports `console_window=false`.

**Elevation.** A `LeastPrivilege` task cannot reach a claude background service that an elevated process started (LEARNINGS m5/25). `claude --bg` then gives up after 45 s with "Couldn't reach the background service". So the task's run level follows the service (Decision 5):
- `HighestAvailable` when the service is elevated (the service's pid, from `claude daemon status`, has an elevated token; sched/3 adds that check next to `foreman_claude::is_elevated`); `LeastPrivilege` otherwise.
- `Registrar::sync` records the run level it registered and re-registers the workspace's tasks when the service's elevation changes. The app checks this at start and with each `DaemonWatch` check.
- fire still runs `foreman_claude::daemon_mismatch()` first and records `failed` with the `elevation_mismatch` text, so a task registered before a change never hangs a launch.

The `<RunLevel>` above is that value, not a constant.

sched/3 builds `Registrar { sync(workspace, &[Schedule]) }` with `WindowsRegistrar` (COM) and `FakeRegistrar` (keeps the XML per path). It provides the XML builder as a pure function, golden-tested, plus one `#[ignore]`d test that registers a real task under `\ForemanProbe-sched-3\` and removes it.

## Permission profile and caps

The profiles map onto claude's flags. Every profile also gets `LaunchConfig::allowed_tools` (`Bash(fm:*)`) and the default deny list.

| Profile | `--permission-mode` | Worktree | `--allowedTools` | Use |
|---|---|---|---|---|
| `read` | `default` | no (the main checkout) | `Read Glob Grep` + `allow_tools` | reports, triage; anything else parks → `needs_you` |
| `worktree-edit` | `acceptEdits` | required | `Read Glob Grep Edit Write` + `allow_tools` | fixes that end in a branch or PR |
| `auto` | `auto` | required | `allow_tools` | the classifier decides, like a workspace session |

- `bypassPermissions` is never a schedule's mode; `fm schedule add` refuses it.
- **Default deny list** (`--disallowedTools`, always): `Bash(git push --force:*)`, `Bash(claude rm:*)`, `Bash(schtasks:*)`, `PowerShell(Register-ScheduledTask:*)`, `PowerShell(Unregister-ScheduledTask:*)`. A run cannot schedule anything, and cannot rm.
- **sched/8's skill** asks the human whenever the action needs more than `read`.

**Caps** (C102), all covered by the fingerprint:

| Cap | Default | Hard limit |
|---|---|---|
| `max_runtime` | 1h | 8h |
| `max_tokens` | 3 000 000 | 20 000 000 |
| `script_timeout` | 10m | 1h |

- `max_tokens` is the run's input + output + cache-creation tokens from the transcript's `usage` lines; cache reads are left out.
- The cap is in tokens, not money: a price table per model rots, and the transcript holds tokens.
- `foreman_claude::transcript` has no `usage` reader yet. sched/6 adds one there (shared), not in fm.

## The session name and the supervisor

Name: **`fm-<workspace>-sched-<id>-<yyyymmdd-hhmmss>`**, e.g. `fm-nomos-sched-autofix-prod-bugs-20261102-090000`.

- **`validate_name`.** The workspace is a valid name, the id passes `validate_name`, and the stamp is digits and a hyphen, so the whole name passes `validate_name` (lowercase, digits, `-`, no leading or trailing `-`). The second-resolution stamp keeps two Run nows in one minute apart; `Claude::launch` refuses a taken name in any case.
- **Will the supervisor stop it? No.** `reconcile` acts only on names in the desired state's `sessions` or `retired` (crate docs: "Rows whose name is neither wanted nor retired are never touched"; R5, R14). fm never adds a sched name to either.
- **The supervisor must not revoke its token either.** `Supervisor::revoke_unwanted` (`executor.rs:527`) revokes every name in `supervisor/<ws>.sessions.json` that is not wanted. So fm keeps a run's minted session in the live file and the run record, **never in `MintedSessions`**.
- **It is not foreign.** It starts with `fm-<workspace>-`, so `is_foreign` is false. The app shows it as this home's own row (attach, resume work as for any row), and **it counts against the shared `max_working` budget** while it is mid-turn (`budget.rs:31`). Fire waits up to 30 minutes for a slot, then records `skipped` ("busy") (Decision 6).
- **A name to reserve.** `repo_session_name(ws, repo)` gives `fm-<ws>-<slug>`, so a repo whose slug starts with `sched-` could take a run's name. sched/2 makes `repo_session_name` return `None` for a slug starting with `sched-`, as it already does for `workspace` and `relay`.
- **For sched/7.** A row named `fm-<ws>-sched-<id>-…` belongs to that schedule's run; the fleet shows it under the schedule (design B) and with its run record.

## fm without the app: mint, launch, stop

A3 is confirmed by reading the code. The supervisor ticks only inside the app (`foreman-app/src/supervise.rs` drives `tick_all`), so a run fired while Foreman is closed has nobody else to watch it, and fm owns the run end to end:

- **The bearer.** `Auth::open(home).access_token(configured_origin)`, as `fm work` does (`crates/fm/src/work.rs:36`). It reads the refresh token from the DPAPI `FileStore`, refreshes under `oauth/refresh.lock` (cross-process, so fm and a running app never spend one refresh token twice), and keeps a rotated token. `AppTokens`/`bearer`/`workstate_role` live in `foreman-app/src/tokens.rs`. sched/6 moves them into `foreman-supervisor` (next to `TokenProvider`) so fm uses the same provider instead of a copy (C56). Not signed in → the run launches without WorkState, as the supervisor does, and the record says so.
- **The mint.** `HttpMinter::mint` with a `MintRequest`:
  - `name` = the session name; `role` = `orchestrator` (Decision 4);
  - `launched_by` = `origin` = `foreman/<ws>/schedule/<id>`; `cwd` = the worktree; `external_role` = `schedule`;
  - `replace` false (every run is a new name); `repo` = `git_repo(cwd)`; `claude_session_id` None.

  Once the row shows its uuid, `Minter::bind` sends it.
- **The launch.** `foreman_claude::Claude::launch(&LaunchSpec)`, built exactly as `Supervisor::launch` builds it (`executor.rs:414`):
  - `strict_mcp_config`, the plugin dir (`foreman_core::find_plugin_dir`), `--add-dir`s;
  - `--settings` with `FOREMAN_HOME`/`ROLE`/`WORKSPACE`/`SESSION`, plus `FOREMAN_SCHEDULE` and `FOREMAN_RUN`;
  - `bgIsolation: none`, because the run already has its own worktree;
  - `--allowedTools`/`--disallowedTools` from the profile; `--model` when set.

  sched/6 pulls the spec building out into one shared function on `LaunchConfig`, used by both.
- **The stop.** `Claude::stop(id)`, `Minter::revoke(session_id)`, delete `mcp_config_path(mcp, name)`. Never `rm`. The worktree is kept, so a run's branch survives. `prune` removes it after the 30-day retention only when it holds no uncommitted and no unpushed work; it lists the rest (Decision 8).

## Probes (a)–(d): pending, run by René

The unit's own attempt to register its probe task was refused by Claude Code's auto-mode classifier ("Unauthorized Persistence"). The controller decided the probes are run by René. Everything they need is ready:

- `docs/research/scheduler-probe/` holds the probe (`src/main.rs`, a standalone crate; `#![windows_subsystem = "windows"]`) and `register-probe.ps1`. The script:
  1. starts the probe's `serve` step interactively: a fake WorkState with OAuth on 127.0.0.1, signed in once through a scratch home's DPAPI `FileStore`;
  2. registers `\ForemanProbe-sched-1\probe` (InteractiveToken, LeastPrivilege, StartWhenAvailable, WakeToRun, a trigger an hour ahead) and reads it back;
  3. prints `powercfg /waketimers` and the Allow-wake-timers setting;
  4. starts the task, waits for the probe's log and prints it;
  5. unregisters the task, deletes the folder through COM, and shows both are gone.
- Everything it prints goes to `<worktree>\.probe-run\register-probe-output.txt`.

**The probe is already built** at `C:\fmbuild\sched-1\probe-target\release\scheduler-probe.exe`. To rebuild it:

```powershell
Copy-Item P:\Projects\LarchSys\AutoWork.worktrees\sched-1\Cargo.lock P:\Projects\LarchSys\AutoWork.worktrees\sched-1\docs\research\scheduler-probe\
$env:CARGO_TARGET_DIR='C:\fmbuild\sched-1\probe-target'; cargo build --release --manifest-path P:\Projects\LarchSys\AutoWork.worktrees\sched-1\docs\research\scheduler-probe\Cargo.toml
```

**To run the probes** (from an elevated shell or not; the task is LeastPrivilege either way):

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File P:\Projects\LarchSys\AutoWork.worktrees\sched-1\docs\research\scheduler-probe\register-probe.ps1
```

| Probe | What the task's log shows | Result |
|---|---|---|
| (a) | `elevated=false console_window=false`; `claude --version`; `claude daemon status: Reachable`; `claude agents --json --all: ok, N rows`; `claude --bg --name probe-sched-1-…` and its row; `claude stop <id>` and the row after it | pending: run by René |
| (b) | `FileStore get: refresh token read, N chars`; `access_token: token obtained, N chars` | pending: run by René |
| (c) | `git --no-optional-locks status`: exit and output in `P:\Projects\LarchSys\AutoWork`; whether `safe.directory` lists it | pending: run by René |
| (d) | the task's `WakeToRun=True StartWhenAvailable=True` read back; `powercfg /waketimers` with the task armed; Allow wake timers AC/DC | pending: run by René |

**Preliminary, not from a task.** The same probe binary was run by this unit interactively, from an elevated shell (Administrators token), against the scratch home. It shows the probe works. It does not answer A1 or A2, which are about a non-elevated task:

```
== context
user=renel session=1 elevated=true console_window=false
PATH has .local\bin: true
user_home=Some("C:\\Users\\renel") claude_dir=Some("C:\\Users\\renel\\.claude")
== (a) claude
claude --version: 2.1.314 (Claude Code)
claude daemon status: Reachable
claude agents --json --all: ok, 428 rows
claude --bg --name probe-sched-1-b51dd4: backgrounded · 948719b4 · probe-sched-1-b51dd4
row: id=948719b4 sessionId=948719b4-291b-4bdb-aaad-3247d82640f9 pid=None state=Some(Working) status=None cwd=P:\Projects\LarchSys\AutoWork.worktrees\sched-1\.probe\scratch
claude stop 948719b4: stopped 948719b4
after stop: pid=None state=Some(Done) status=None
== (b) DPAPI FileStore and OAuth refresh
configured origin: http://127.0.0.1:51257
FileStore get: refresh token read, 24 chars
access_token: token obtained, 152 chars
== (c) git status in P:\Projects\LarchSys\AutoWork
git --no-optional-locks status --short --branch: exit Some(0)
  stdout: ## master
```

The scratch session `948719b4` (`probe-sched-1-b51dd4`) was this unit's own. It was stopped (`claude stop`), and `claude agents` afterwards lists it as `done` with no pid. It was not rm'd.

**Read-only facts gathered now**, which the probes will add to:
- `powercfg /waketimers`: "There are no active wake timers in the system." That is expected: no task with WakeToRun was registered.
- `powercfg /query SCHEME_CURRENT SUB_SLEEP RTCWAKE`: plan "AMD Ryzen™ High Performance", Allow wake timers: `Current AC Power Setting Index: 0x00000001`, `Current DC Power Setting Index: 0x00000001` (1 = Enable). Wake timers are allowed on AC and DC.
- **The repo's owner.** `P:\Projects\LarchSys\AutoWork` is owned by `BUILTIN\Administrators`; its `.git` by `PCRENE\renel`. A non-elevated token holds Administrators only as deny-only, so git's ownership check sees the worktree as someone else's ("detected dubious ownership"). That is A2, seen 2026-10-29. Elevated, git accepts it, as the run above shows.
- **The fix that needs no change to workspace trust**, done once by René:

  ```
  git config --global --add safe.directory P:/Projects/LarchSys/AutoWork
  ```

  It writes `~/.gitconfig` only. Claude's trust in `~/.claude.json` and the folder's ACL stay as they are. Each workspace folder owned by Administrators needs its own entry. Taking ownership (`icacls … /setowner`) would also work, but it changes the ACL.
- **fm's own git calls hit the same refusal:** `git_repo` for the mint returns `None` silently, and `git worktree add` fails. So sched/6 checks first and records `failed` with this exact command as the fix, instead of minting a token with no repo binding.

## Decisions for the later units

| Unit | Decided here |
|---|---|
| sched/2 | the TOML above; `foreman_core::schedule` with load/edit under `lock_file` + `write_atomic`; the fingerprint (covered tables + file hashes) and `verify`; covered edit → pending; next fire in local time across DST; run records with `append_capped` 1 MiB and `prune` extended; `hmac` 0.12.1 as a direct dependency (already locked); reserve the `sched-` slug in `repo_session_name`; list the module in CLAUDE.md and ARCHITECTURE.md |
| sched/3 | COM `ITaskService` with XML; `\Foreman\<ws>\<id>` (another home: `\Foreman-<hex8>\…`); the settings above; the fake keeps the XML; sync deletes empty folders; COM on an MTA background thread; one `#[ignore]`d real-task test in the unit's own probe folder |
| sched/4 | flags as in its intent plus `--profile read\|worktree-edit\|auto`, `--max-tokens` instead of `--max-cost` (Decision 3), `--secret NAME`; refuses `bypassPermissions` and unknown script extensions; dry-run prints the resolved prompt with the preamble, the full `claude` argv, the profile's allow and deny lists, and the next three fires |
| sched/5 | `fmw.exe` (GUI subsystem) runs `schedule fire --home … <ws> <id> --fingerprint …`; checks 1–4; `try_lock_file` in `foreman_core::fs`; overlap via the live dir; the script contract above; secrets from `SecretStore` keys `foreman/schedule/<ws>/<id>/<NAME>`, masked in stderr tails |
| sched/6 | the worktree under `<folder>/.claude/worktrees/`; the bearer through the moved `AppTokens`; the mint as above, kept out of `MintedSessions`; the launch spec shared with `Supervisor::launch`; the watch loop and end conditions; a `usage` reader in `foreman_claude::transcript`; `fm schedule report`; the safe.directory check |
| sched/7 | `approve()` is the only writer of `[schedule.approval]` (with `command_sha256`), then `Registrar::sync`; "changed since approval" when the command file no longer matches; Approve diffs `approval.covered` against the current covered JSON; run rows found by the `fm-<ws>-sched-<id>-` prefix; the app recovers dead runs (live file, pid gone) |
| sched/8 | the skill runs only `fm schedule add/list/show/pause/resume/dry-run`; it never touches the TOML; it asks before any profile above `read` |

## Decisions

Answered 2026-10-30: questions 2, 4 and 5 by René, the others by the controller. They are folded into the sections above.

1. **The trigger and the policy are covered** (controller). The fingerprint includes `trigger` and `policy` (overlap, catch-up, wake) as well as the action, script, prompt source, permissions and caps. Changing any of them makes the schedule pending.
2. **A slash command is covered by its name and path, not its content** (René). Its hash at approval (`approval.command_sha256`) only drives the row's "changed since approval"; it never refuses a fire. The script's and the prompt file's content stay covered.
3. **The cost cap is in tokens** (controller). The cap is `max_tokens`, with `fm schedule add --max-tokens`; there is no price table.
4. **A run's WorkState role is `orchestrator`** (René).
5. **The task's run level matches the claude service** (René). It is `HighestAvailable` when the service is elevated and `LeastPrivilege` otherwise; `Registrar::sync` re-registers when that changes (sched/3).
6. **fire waits up to 30 minutes for a budget slot** (controller). Then it records `skipped` ("busy") (sched/5).
7. **A `needs_you` run stays parked until `max_runtime`** (controller), with an Inbox notice from the moment it is recorded (sched/6, sched/7).
8. **Worktrees of finished runs** (controller). `prune` removes one after the 30-day retention only when it has no uncommitted and no unpushed work; the others are listed for René (sched/2 extends `prune`; sched/7 shows the list).

## Assumptions

- **A1** (task can reach claude, DPAPI and refresh): open until René runs the probes. Code reading and the interactive run support it. The `InteractiveToken` principal is what makes DPAPI available, and the elevated-service case is the known exception (m5/25).
- **A2** (git refuses Administrators-owned repos non-elevated): consistent with the owners above, to be confirmed by probe (c). The fix is the per-repo `safe.directory` above.
- **A3** (fm owns the run without the app): confirmed by reading the code; see [fm without the app](#fm-without-the-app-mint-launch-stop).
