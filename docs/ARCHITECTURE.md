# Supraman architecture

For whoever is about to change Supraman, or to build another gpui tool (GitMan, FileMan) on its crates. What was
measured and why is in `docs/LEARNINGS.md`; the plan is `docs/PLAN.md`; the keys are `docs/KEYS.md`.

## Crates

Dependencies point down this list only; nothing below depends on anything above it.

| Crate | Owns | Depends on |
|---|---|---|
| `crates/supraman-app` | The `supraman.exe` binary: `FleetView` (left tree, tabs, Work tab, Inbox), Settings window, tray, the supervisor's driver, hand sessions, the selftests. | every crate below |
| `crates/sm` | The `sm` CLI (`lib.rs`, shared by `sm.exe` and the console-less `smw.exe` a schedule's task runs): `workspace`, `sessions`, `want`/`unwant`/`roster`, `inbox`/`send`/`tell`/`messages`, `schedule` (with `fire`, `run`, `report` and `secret set`; `fire.rs` the checks and the script stage, `run.rs` the session stage), `work`, `hook`, `plugin install`, `probe`. Edits files the app acts on; drives no supervisor, but owns a scheduled run end to end (launch, watch, stop, revoke). | claude, core, supervisor, workstate |
| `crates/supraman-supervisor` | Desired state per workspace, the pure `reconcile`, the executor (launch, resume, stop, mint, revoke), the shared busy budget, the Inbox rows (`NeedsYou`). No UI. | claude, core, workstate |
| `crates/supraman-claude` | The only code that knows the `claude` CLI: `agents --json --all` and health, `--bg` launch, stop, rm, resume, the peer pipe, transcripts, the background-service check. | core |
| `crates/supraman-workstate` | Every WorkState read: the read API v1 client, `/api/stream`, the OAuth client, the HTTP agent. Read only (C21). A test fake behind the `fake` feature. | core |
| `crates/supraman-ui` | The gpui widgets and painters every Supraman binary shares: text input, buttons, banners, menu, confirm, toast, notices, tabs, hover card, split pane, matcher, settings pages, themes, the terminal grid painter, hit targets, logger, frame probes. | core, term |
| `crates/supraman-term` | The terminal model: Zed's `terminal` crate vendored onto alacritty (PTY, grid, input, mouse, links, scroll, search). No gpui views, no Supraman knowledge. A tab's env (`builder.rs` `insert_terminal_env`) is `TERM_PROGRAM=supraman` with `FORCE_HYPERLINK=1`, so claude writes a markdown link as OSC 8; the app opens a Ctrl+clicked link only through `view.rs` `link_target` (`http`, `https`, `mailto`, `file`). | — |
| `crates/supraman-core` | Plain Rust shared by all: the workspace registry, settings, files written safely, paths, time, process spawning, secrets at rest, the relay's files, hook events, names. No gpui, no claude, no HTTP. | — |

**Boundaries.**
- Only `supraman-claude` runs `claude` outside a terminal tab or parses its output; a tab runs `claude` in its PTY (`hand.rs`, `view.rs` `attach`).
- Only `supraman-workstate` talks HTTP to WorkState (the supervisor's `mint.rs` uses its `http_agent`). Nothing in Supraman writes WorkState items; the MCP tools are the agents' write path (C21).
- `supraman-supervisor` takes a snapshot and returns actions; it never polls, never paints, and is ticked by the app.
- `supraman-core` and `supraman-term` have no Supraman crate dependencies; `supraman-ui` has no `claude` or WorkState knowledge.
- `plugins/supraman` is not a crate: the Claude Code plugin loaded with `--plugin-dir` on sessions Supraman launches — `plugins/supraman/hooks/hooks.json` (exec-form `sm.exe hook <event>`), `plugins/supraman/roles/` (workspace, relay, repo), and the skills `workspace-controller` and `repo-controller`. See `plugins/supraman/README.md`.
- `spikes/term-spike` is not product: the m0/5 terminal spike, kept building against `supraman-term` and `supraman-ui`. Nothing depends on it.
- `probes/` holds the m0 measurement scripts; `scripts/` holds `scripts/release.ps1`, `scripts/shot.ps1` and `scripts/icon-gen/`.

## Shared components and helpers

Use these; do not grow a private copy (CLAUDE.md, C56). If none fits, extract one into the crate it belongs to and add it here and to CLAUDE.md.

### supraman-core (`crates/supraman-core/src/`)

- `fs::write_atomic` — every write of a file another process or thread may read (settings, registry, desired state, minted sessions, relay files). A unique temp file renamed over the target.
- `fs::lock_file` — an exclusive cross-process lock held while the returned file lives, around a read-modify-write (`DesiredState::update`, `refresh.lock`). `fs::try_lock_file` takes it only when free (`None` while held): a schedule's overlap lock.
- `fs::load_json_or_notice` — a JSON state file whose loss is a notice, not an error (stars, dismissals).
- `fs::append_capped` (with `fs::rotated`) — an append-only log rotated to `<file>.1` at a cap (event logs, `foreman.log`).
- `paths::{user_home, claude_dir}` — the user's home and claude's config dir (`CLAUDE_CONFIG_DIR`-aware); never read `USERPROFILE`/`HOME` yourself.
- `paths::{normalize, path_key, within, key_within, main_checkout, linked_git_dir}` — path comparison (`within`: a folder or below it, never a sibling sharing a prefix) and the `.git` file/dir walk without running git.
- `time::{now_ms, epoch_ms, modified_ms, hex6}` — epoch milliseconds and a scratch-name suffix.
- `update::{Manifest, sign, verify, release_keys, sha256_file, signature_path}` — a release's `supraman-update.json` (its bytes as written are what is signed), its ed25519 signature file and the release keys compiled in from `update.pub`; `Manifest::check_zip` checks a downloaded zip. `sm release` writes and signs it; the app's update check and install use it.
- `prune::{prune, KEEP_MS}` — the one retention rule for what sessions leave in the home: event logs, relay outbox messages, sent and rejected drops, the `answered.json` entries of gone messages, and the schedules' rotated run records and spent run worktrees, after 30 days unmodified, and the state dirs of removed schedules; a drop still waiting is never removed. The supervision runs it hourly.
- `url::origin` (re-exported as `url_origin`; `supraman_workstate::api_base`) — the one origin rule; origins key `servers.json` and the minted records.
- `process::{command, run, run_watched, git, shell_execute, alive, started, wait_any_exit}` — every child process outside the PTY: no console window, both pipes drained, a timeout that kills the child or its tree (`Kill::Child` for `claude`, whose tree holds the background service), ShellExecute, pid liveness. `run_watched` also kills the child when its `stop` returns true and returns what a killed child wrote (`Ended::{Exited, TimedOut, Stopped}`). Never `Command::new(..).output()` by hand.
- `secret::{SecretStore, FileStore, MemoryStore, default_store}` — every secret at rest: one DPAPI-encrypted file per key under `<home>/oauth/secrets`; `MemoryStore` in tests.
- `slug::{slugify, validate_name, FM_PREFIX}` — a repo or workspace name as a session-name part; every Supraman session name starts with `fm-`.
- `Settings::update` — every change to `settings.toml`: load, edit, save, retried against other writers. `Settings::load_with_notices` when a bad key should be shown.
- `Settings` sections (`sessions`, `workstate`, `appearance`, `terminal`, `window`, each a `Section`) — a key belongs in its table; another owner's table (a plugin's `[gitman]`) goes through `Settings::section`/`set_section` and survives every save.
- `Registry` (`load_all`, `list`, `home_from`) — the workspaces at `<home>/workspaces/<name>.toml`; `load_all` where one corrupt file must not hide the rest (the supervisor), `list` where it should stop the caller.
- `Workspace` (`model.rs`: `home`, `folders`, `scans`; `home()`, `contains`, `other_dirs`, `folder_at`, `add_folder`, `unique_folder_name`), `Folder` (`kind`: `git` or `plain`), `Scan`, `owners` — a workspace is a name, a home and a set of folders; a root-based file loads as one scan of its root. `owners` says which workspace a folder's supervised session belongs to (the first that has it enabled); `Registry::claim_folders` disables what another workspace owns.
- `discover`, `rescan`, `scan`, `branch_at` — a folder to a workspace and its repos, and each scan run again; shared by `sm workspace add` and the app's Add workspace and Rescan.
- `Registry::edit` (`edit.rs`: `Edit`, `Edited`; `Workspace::find_folder`) — one change to a registered workspace (add or remove a folder, enable, home, add or remove a scan; a removed scanned folder goes on its scan's `exclude`), read, checked against `owners` and saved under `<name>.toml.lock`; shared by `sm workspace new`/`add-folder`/… and the app's Edit folders… (`view/folders.rs`).
- `schedule` (`schedule/`: `load`, `edit`, `approve`, `covered`, `verify_approval`, `effective_status`, `next_fire`, `previous_fire`, `next_fire_local`, `append_run`, `read_runs`, `runs`, `run_worktrees`, `command_file`, `base_prompt`, `preamble`, `run_stamp`, `run_id`, `session_name`, `run_worktree`, `DEFAULT_DENY`, `Profile::permission_mode`, `Permissions::{allowed, denied}`) — the scheduler's model (`docs/research/scheduler.md`): one TOML per workspace at `<home>/schedules/<ws>.toml`, changed only through `edit` under `<ws>.lock` + `write_atomic`. `edit` keeps `[schedule.approval]` as it was, sets `pending` and clears `action.worktree` when a covered field changes, and keeps `active` only while the approval's fingerprint is the current one. The fingerprint is the sha256 of the canonical covered JSON (trigger, action with a slash command's name and path, permissions, caps, policy, and the script's and prompt file's sha256); the approval's MAC is HMAC-SHA256 under `APPROVAL_KEY` in the `SecretStore`, created on the first `approve`. `approve` is called only from the window's Approve (C100). Next fires take the zone as a parameter: a wall time a DST change skips fires at the first minute after the gap, a repeated one fires once. Run records go through `append_capped` (1 MiB); `prune` removes the rotated ones, the state of removed schedules, and run worktrees with nothing uncommitted and no commit of their own. `SCHED_SLUG_PREFIX` keeps `repo_session_name` off the run names. A run works in its folder's checkout; only a schedule an earlier release saved with `worktree = true` gets a run worktree (`run_worktree`, `run_branch`), and only there may an `edit` run push its branch (`Permissions::allowed(always, run_branch)`). `Profile::Edit` reads `edit` and `worktree-edit` and fingerprints as `worktree-edit`, so earlier approvals verify. A run's names, its preamble (which says where the run works) and base prompt (a missing slash command or prompt file is `NoPrompt`, naming the folder), and a profile's `--permission-mode`, `--allowedTools` and `--disallowedTools` live here, so `sm schedule dry-run`, fire and the app agree; `sm`'s `schedule::run_spec` turns them into the run's `LaunchSpec`. `sync(&dyn Registrar, &SyncTarget, &Workspace, &[Schedule])` registers one Windows task per schedule that is `active` with a verifying approval, at `<task_root(home)>\<ws>\<id>` (`\Foreman` for `%APPDATA%\Foreman`, `\Foreman-<hex8 of sha256(path_key(home))>` for any other home), removes the rest of that folder and deletes it, then the root, when empty; it reads and writes nothing outside the folder. `task_xml` builds the definition (InteractiveToken, the run level, StartWhenAvailable for `catch_up = once`, WakeToRun, `smw.exe schedule fire --home … <ws> <id> --fingerprint …`); its `<Version>` is `definition_digest`, everything but the start date, so an unchanged task is left alone and a changed one (run level included) is registered again. `WindowsRegistrar` is COM `ITaskService`, each call on its own MTA thread; `FakeRegistrar` keeps the XML per path for tests and selftests (C101). The app is the only caller (C100). `covered_fields`/`covered_diff` flatten a covered JSON to sorted `field: value` leaves and diff two, which the window's Approve shows; `schedule_of_session` parses a run session's name back to its schedule id.
- `relay` (`send`, `tell`, `inbox_dir`, `outbox_dir`, `InboxWatcher`, …) — the relay's `inbox`/`outbox` files per workspace.
- `events` (`append_event`, `EventReader`, `role_injection`) — the hook events `sm hook` appends per session.
- `install_hook_exe`, `find_plugin_dir`, `sm_allowed_tools` — placing `sm.exe` and its compatibility copy `fm.exe` (`COMPAT_HOOK_EXE`, docs/RELEASE.md) in the plugin, and the `--allowedTools` rules that keep `sm` from parking (every supervised session and every scheduled run).

### supraman-ui (`crates/supraman-ui/src/`)

- `text_input` (`TextInput`, `LineText`, `bind_keys`) — the one text input, single- or multi-line, on gpui's `EntityInputHandler`: selection, IME, dead keys, clipboard. Every text field uses it.
- `hit::{HitTargets, hit_target}` — every clickable element records its painted bounds under its key; the selftests click through these.
- `button::{bordered, outline, pill, icon, revealed, toolbar, toolbar_turning, ghost, small, choice}` — every clickable button; the key is its element id, hover group and hit key; `toolbar_turning` turns its icon while work it started runs; `revealed` is a row's hover icon in a fixed slot: always laid out and painted, so nothing shifts and its hit key exists, shown while the row's group is hovered or the row is selected.
- `banner::{tinted, plain, dismissable, tint}` — every notice box: a status colour on its 0.12 tint.
- `fonts::{UI, MONO, icons}` — the UI's font families; never spell a family name in a view. The terminal's font is the user's (`Settings`).
- `menu::{ContextMenu, MenuItem, item}` — every right-click menu, generic over the owner's action type; items paint under `menu-<key>`; `MenuItem::under` starts a captioned section.
- `confirm::Confirm` — a question before an action that cannot be taken back (`ask`), or a report with only its OK (`report`); the owner places it.
- `toast::ToastSlot` — short feedback floating over a panel that goes by itself (`TOAST_FOR`); `show` marks a done action with ✓, `show_why` says why nothing was done.
- `notices::{NoticeKind, render, keep_dismissed, remove_one}` — a list of plain notices with a × each; the owner keeps the kinds and sources.
- `tabs::{tab_strip, TabHeader, DraggedTab, DraggedSession, reorder, cycled, sanitize}` — the tab strip with drag-to-reorder (`on_move(from, to)`; `DraggedTab { id }` is the payload any other drop target takes, `DraggedSession { session_id }` a session row's; a terminal pane takes both and pastes the session's SendMessage name), tab cycling, and a program's OSC title as a tab label; tab state and order stay with the owner.
- `hover_card::{HoverCard, tooltip}` — a hover card of a title and `label  value` rows.
- `split::SplitPane` — a resizable side-by-side split; the owner keeps and persists the width.
- `matcher::{find_all, prefix, highlighted, count}` — case-insensitive substring/prefix matching, a result's highlighted label and a count badge.
- `settings_page::{SettingsPages, SettingsPage, row}` — a settings window's pages, registered by id with a title, an icon and a render fn, and one setting's row.
- `nudge::{nudge, Nudge, Nudged}` — a coalescing wake-up for a loop on a gpui executor, instead of polling a flag.
- `themes` — Zed theme JSON over a base, the bundled themes, the terminal palette, `contrast` and the contrast floors (`fit_floors`).
- `term_element` (`terminal_grid`, `TermFont`, `cell_metrics`) — the terminal grid painter for a `supraman_term::Terminal`; OSC 8 cells are underlined, and the pointer is a hand while Ctrl is held over a link.
- `lib.rs`: `init_globals`, `init_logger`/`Logger` (`foreman.log` via `append_capped`), `FrameProbe`, `memory` — global setup and the probes the selftests print.

### supraman-workstate (`crates/supraman-workstate/src/`)

- `http_agent`, `read_reply`, `pct_encode`, `server_reason` (`http.rs`) — the HTTP agent, an answer read whole (status, headers, body; a body that cannot be read is a transport failure), path/query encoding and a refusal's reason for every WorkState call, the mint and OAuth included.
- `Client` (`client.rs`) — the blocking read API v1 client (`overview`, `board`, `item`, `brief`, `inbox`, `journal`, `backlog`, `sessions`, `resolve`); GET only; its bearer comes from a `BearerSource` at each read; no error carries the token.
- `Stream` (`stream.rs`) — the `/api/stream` subscriber on its own thread: `StreamSignal::{Connected, Event, Lost}`; an event means "re-read".
- `Auth` (`auth.rs`) — the OAuth 2.1 native client: sign-in, `access_token`, refresh under `refresh.lock`, `bearer_for(origin)`.
- `health` — the unauthenticated `GET /health`.
- `fake::FakeWorkState` (feature `fake`) — the one test server for anything that talks HTTP to WorkState: routes by `<METHOD> <target>` or by target, `set` to change an answer, `seen()` for each request's method, target, Authorization, Content-Type and body.
- `model` — the read API v1 shapes, lenient (v1 grows additively).
- `fake::FakeWorkState` (feature `fake`) — the read API and its OAuth server on 127.0.0.1 for tests and selftests.
- `secret` re-exports `supraman_core::secret`.

### supraman-claude (`crates/supraman-claude/src/`)

- `list_sessions`, `parse`, `Session::health`/`supervised_health` — `claude agents --json --all` and the health table (crate docs in `lib.rs`).
- `Claude` (`launch.rs`: `launch`, `resume`, `stop`, `rm`, `remove`, `find_by_name`), `LaunchSpec`, `resume_argv`, `new_session_id`, `name_taken` — the write side; every run goes through `supraman_core::process::run`.
- `mcp_config_path`, `write_mcp_config`, `workstate_mcp_config` — a session's WorkState config file under `<home>/mcp/`, with the session-name check; write, read and delete it through these.
- `check_supported`, `MIN_VERSION`, `TESTED_VERSION` (`cli.rs`) — the CLI version gate.
- `daemon_status`, `daemon_mismatch`, `is_elevated` (`daemon.rs`) — whether this process can reach claude's background service. `service_pid` (the roster's `supervisorPid`, since `claude daemon status` reads `NotRunning` from an unelevated process facing an elevated service), `pid_elevated`, `service_elevated` and `task_run_level` give a scheduled task's run level.
- `peer` — writing into a session's own message pipe (`docs/peer-messaging.md`); `human::send_from_human` picks the pipe or the relay.
- `proc_tree` — which claude session runs under a tab's shell.
- `transcript` (`Transcripts`: `scan_with` with progress, `with_cache`, `cached`; `first_prompt`; `Usage`, a session's input/output/cache tokens, each message once) — closed sessions from the transcript files, read only: head and tail of a new file, the grown part of a known one, on parallel readers; the file cache persists at `<home>/cache/transcripts.json` so a start shows its rows before any file is read.
- `forget::forget_transcript` — the one write under the claude dir (see delete confinement).
- `assign` — mapping sessions onto a workspace's folders (deepest folder, then a worktree's main checkout), then its home.

### supraman-supervisor (`crates/supraman-supervisor/src/`)

- `DesiredState` (`desired.rs`) — `<home>/supervisor/<workspace>.toml`; change it only through `update` (locked read-modify-write); `release` unwants the repo sessions of folders taken away, so the supervisor stops them.
- `Supervisor`, `tick_all`, `Budget` — one tick per snapshot; the rule table is the crate docs in `lib.rs`.
- `HttpMinter`, `Minter`, `MintedSessions` (`mint.rs`) — per-session WorkState tokens.
- `AppTokens`, `bearer`, `workstate_role` (`tokens.rs`) — the `TokenProvider` of the app and of `sm`: Supraman's OAuth sign-in as the mint's bearer.
- `LaunchConfig::launch_spec` (`executor.rs`) — a wanted session's `claude --bg` spec before its token; `Supervisor::launch` and a scheduled run (`sm`'s `schedule::run_spec`) both build theirs with it. `LaunchConfig::session_env` (the `FOREMAN_*` and home vars of its `--settings`) and `interactive_args` (`--plugin-dir`, `--allowedTools`) are the same pieces for a [+] session.

## Shared vs app-only

**GitMan and FileMan may reuse:** all of `supraman-core`, `supraman-ui`, `supraman-term`, `supraman-workstate`, and `supraman-claude` if they run claude. Their settings live in their own `Settings` table (`Settings::section`/`set_section`) and their settings window registers its pages with `SettingsPages`.

**App-only (in `crates/supraman-app`, not for reuse):**
- `FleetView` (`crates/supraman-app/src/view.rs`, `crates/supraman-app/src/view/`) — the left tree (design B: the workspace row is the workspace session, the relay a glyph; `row_details.rs` the sections of a session's and a workspace's Details tab, `view/details.rs` that tab), tabs, menus, delete flow; `crates/supraman-app/src/work_view.rs` and `crates/supraman-app/src/work_view/` — the Work tab and Inbox.
- The tab lifecycle: open, attach, resume, and detach on close or quit (`view.rs` `close_tab`, `start_detach`, `quit_step`).
- The supervisor policy and its driver: `supervise.rs`, the poll loop (`view.rs` `Poller`), `supraman-supervisor` itself.
- Hand sessions (`hand.rs`), sign-in wiring (`signin.rs`), WorkState setup wording (`workstate_setup.rs`).
- The tray (`tray.rs`), the single instance (`instance.rs`), elevation (`elevate.rs`), the titlebar, stars, dismissals, the selftests.

## Key flows

### A tab starting a session
1. `crates/supraman-app/src/view.rs` `FleetView::new_claude` → `hand_claude`: pushes a shell tab, then `Hands::launch_for` off the UI thread.
2. `crates/supraman-app/src/hand.rs` `Hands::launch_for` picks the name (`session_name`, `next_name`), mints a token (below) and writes the config through `mcp_config_path`.
3. `view.rs` `hand_extras` → `hand.rs` `extras`: from `supervise::launch_config` (the supervisor's pieces), the plugin flags and `sm_allowed_tools`, the hook env (`FOREMAN_ROLE=hand`, `FOREMAN_WORKSPACE` = the owning workspace, `FOREMAN_SESSION`, `FOREMAN_REPO` for a folder, the home vars) and the folder of `sm.exe`; with no plugin found, nothing and a notice.
4. `hand.rs` `Launch::script_with` builds `$env:Path = '<sm dir>;' + $env:Path; claude [--session-id <uuid>] --mcp-config <path> <plugin flags>` in a try/finally that deletes the config when claude exits; `shell_args_with` runs it as `pwsh -NoExit -EncodedCommand`.
5. `view.rs` `start_terminal_env` builds the PTY with `supraman_term::TerminalBuilder` and the hook env; `supraman_ui::term_element::terminal_grid` paints it. The session's hooks (`sm hook`) append to the home like a supervised session's and its SessionStart injects `roles/hand.md`; the tree reads its events by session id (`HookSignal`). The supervisor never wants, stops or counts it (it is not `fm-`).
6. When the config file is gone (`Hands::ended`), `Hands::release` revokes the token.

Attach opens `claude attach <id>` (`view.rs` `attach`); `resume` runs `claude --bg --resume <id>` then attach for a background session, or `claude --resume` through `hand_claude` for an interactive one. Supervised sessions get no tab: `crates/supraman-claude/src/launch.rs` `LaunchSpec::argv` and `resume_argv` build their `--bg` lines.

### The supervisor tick
1. `crates/supraman-app/src/view.rs` `Poller` loops every `REFRESH_EVERY` or when nudged; `read` reads the registry and one `list_sessions()` snapshot, which the app applies (and paints) before `Poller::tick` hands that same snapshot to the supervisor. At start the version check runs beside setup and the first read.
2. `crates/supraman-app/src/supervise.rs` `Supervision::tick` loads `Settings` and each `DesiredState`, syncs the relay, applies server changes and the daemon hold, then under `TICK` (and only while `ENABLED`) calls `tick_all`.
3. `crates/supraman-supervisor/src/budget.rs` `tick_all` builds one `Budget` and ticks each workspace's `Supervisor`.
4. `crates/supraman-supervisor/src/executor.rs` `Supervisor::tick`: re-mint stops, resume confirmation, memory, `reconcile` (`crates/supraman-supervisor/src/reconcile.rs`, pure), then launch / resume / stop, revocation, and the `NeedsYou` rows.

`sm want`/`unwant` (`crates/sm/src/repos.rs`) and the app's Start/Stop (`supervise.rs` `start_workspace`) only edit `DesiredState` through `update`; the next tick acts on it.

### Minting and re-minting
- The supervisor mints for `fm-` sessions (`executor.rs` `Supervisor::mint`, from `launch`, and `remint` before every Resume); hands mint for tab sessions (`hand.rs` `Hands::launch_for`).
- `crates/supraman-supervisor/src/mint.rs` `HttpMinter`: POST `/api/agent-sessions` (`replace: true`), PATCH to bind the claude session id, DELETE to revoke (404 counts as revoked). The bearer is the OAuth access token (`crates/supraman-supervisor/src/tokens.rs` `AppTokens`, role from `workstate_role`). A scheduled run mints its own token the same way (`crates/sm/src/run.rs`), keeps it out of `MintedSessions`, and revokes it when the run ends.
- Only the minted `wss_` token reaches a session, in the file `mcp_config_path` names (`crates/supraman-claude/src/launch.rs` `write_mcp_config`, owner-only ACL, `--strict-mcp-config` for supervised sessions).
- Records: `<home>/supervisor/<workspace>.sessions.json` (`MintedSessions`) and `<home>/supervisor/hands.json`.
- Re-mint: every Resume; the Re-mint remedy on a `TokenExpired` or `OtherServer` row (`Supervisor::retry` → `stop_for_remint`, then resume); a server change (`Supervisor::server_changed`). Unwanted tokens are revoked by `revoke_unwanted` and `Hands::release_all`.

### OAuth sign-in and refresh
1. `crates/supraman-app/src/view.rs` `FleetView::sign_in` → `crates/supraman-app/src/signin.rs` (one `Auth` per home).
2. `crates/supraman-workstate/src/auth.rs` `Auth::sign_in`: discovery, dynamic client registration, PKCE S256 on a loopback callback, the browser, the token request.
3. `Auth::access_token`: the held token while fresh, else a `refresh_token` grant under `<home>/oauth/refresh.lock` (shared with `sm`); `invalid_grant`/`invalid_client` forgets the stored token.
4. The refresh token lives in `crates/supraman-core/src/secret.rs` `FileStore`: `<home>/oauth/secrets/<hash>.bin`, DPAPI-encrypted for the current user, written with `write_atomic`. `<home>/oauth/servers.json` holds server metadata, no token. The access token is never logged and never written.

### The WorkState read and its stream
1. `crates/supraman-app/src/work_view.rs` `WorkHub` (a gpui global) builds a `Client` with `Auth::bearer_for(origin)` and subscribes a `Stream`.
2. `crates/supraman-workstate/src/stream.rs` `Stream::subscribe` holds `GET /api/stream` on its own thread with `Last-Event-ID` and our backoff; `WorkHub::on_signal` turns signals into `HubEvent`s.
3. `WorkView::on_hub` re-reads the open view through `crates/supraman-workstate/src/client.rs` `Client`, debounced, off the UI thread; `crates/supraman-app/src/work_view/inbox.rs` `WorkInbox` listens to the same hub. `WorkHub::reconfigure` runs after sign-in, sign-out or a URL change.
4. `sm work` (`crates/sm/src/work.rs`) reads through the same `Client` and the same sign-in.

### Quit
1. `crates/supraman-app/src/tray.rs` Quit (asking first when workspaces are started) → `view.rs` `FleetView::quit`; closing the window only hides it to the tray.
2. Every attach tab detaches: `close_tab` → `start_detach` writes Ctrl+Z (`DETACH_INPUT`) and waits `DETACH_WITHIN`; a tab that does not detach cancels the quit.
3. `quit_step`: with no attach tab left, `hand::release_all_within` revokes the hand tokens, then `close_for_quit` closes the window. `open_hands` also releases them on `on_app_quit` and releases stale ones at start.
4. Supervised `--bg` sessions keep running; the supervisor only stops ticking.

## Protected business logic

Change these only deliberately, with the owner's say and a LEARNINGS entry.

- **C21, WorkState is read only:** `crates/supraman-workstate/src/client.rs` (GET only), `crates/supraman-app/src/work_view/inbox.rs` (dismissals are local), `crates/sm/src/work.rs`. Test: `crates/supraman-workstate/tests/client.rs`.
- **C15/C17, the units' red lines in code:** selftests run on a home under `target/` (`crates/supraman-app/src/selftest.rs` `home`), act only on their own `fm-selftest-<hex>` sessions and clean them up (`cleanup`), read WorkState from `FakeWorkState`, and never print or write a token.
- **The supervisor's rules:** the R1–R14 table, the budget, failures and hold in the crate docs of `crates/supraman-supervisor/src/lib.rs`, implemented in `reconcile.rs` and `executor.rs`; the claude health table in `crates/supraman-claude/src/lib.rs`.
- **The claude CLI protocols:** `peerProtocol 1` in `crates/supraman-claude/src/peer.rs` (`PEER_PROTOCOL`); the minimum and tested CLI in `cli.rs`; the `--bg` line and its resume in `launch.rs` (`LaunchSpec::argv`, `resume_argv`: a resume carries no other flag); rm and its refusals in `launch.rs` (`Claude::remove`, `RmRefusal`); the service reach in `daemon.rs`. The facts are the `sm probe` results (`crates/sm/src/probe_rm.rs`, `crates/sm/src/probe_session_id.rs` and the other `sm probe` files, LEARNINGS m2/1, m5/38, m5/40).
- **Delete confinement:** `crates/supraman-claude/src/forget.rs` (`forget_transcript`, `confined`: a UUID id, no link or junction, inside `projects/`); its one caller is `crates/supraman-app/src/view/delete.rs` `delete_blocking`.
- **Mint and name rules:** `crates/supraman-app/src/hand.rs` `session_name`, `next_name`; `crates/supraman-supervisor/src/desired.rs` `workspace_session_name`, `relay_session_name`, `repo_session_name`; `FM_PREFIX` in `crates/supraman-core/src/slug.rs`; `mcp_config_path` and `name_taken` in `crates/supraman-claude/src/launch.rs`; `MintRequest` in `crates/supraman-supervisor/src/mint.rs`. Contract: Skills `docs/workstate/14-agent-launch.md`.
- **Contrast floors:** `crates/supraman-ui/src/themes.rs` (`NAME_FLOOR`, `SECONDARY_FLOOR`, `fit_floors`), test `the_contrast_floors_hold_under_each_bundled_theme`; notice tints in `crates/supraman-ui/src/banner.rs`.
- **The busy budget:** `crates/supraman-supervisor/src/budget.rs` (`Budget`, `mid_turn`, `tick_all`), the relay's exemption (`WantedSession::exempt_from_budget`), `max_working` in `crates/supraman-core/src/settings.rs`.

## Testing

- **cargo:** `cargo test --workspace`. Unit tests sit beside the code; integration tests in each crate's `tests/`. Fakes: `FakeWorkState` (`crates/supraman-workstate/src/fake.rs`), a fake claude (`crates/supraman-claude/tests/fake_claude.rs`), `MemoryStore` for secrets.
- **Selftests** drive the real app on a home under `target/` (`crates/supraman-app/src/main.rs` `parse_args`, `crates/supraman-app/src/selftest.rs` `start`):
  - `supraman.exe --selftest` — the gate's UI run (`selftest.rs` `gate`, steps in `selftest_*.rs`), clicking through `HitTargets`.
  - `--selftest-supervise` — the supervisor against real `claude --bg` sessions (`crates/supraman-app/src/selftest_supervise.rs`).
  - `--selftest-detach-shutdown` — attach tabs detach on quit (`detach_on_shutdown`); `--selftest-detach <n>` and `--selftest-perf` are measurement runs.
- **The clipboard mutex:** `crates/supraman-app/src/selftest_clipboard.rs` holds the named mutexes `Local\Supraman-selftest-clipboard` and `Local\Foreman-selftest-clipboard` (an older build's) while a selftest uses the clipboard, so parallel selftests on one machine do not clobber each other.
- **The gate**, run before every merge: `cargo build --release --workspace`, `cargo test --workspace`, `--selftest`, `--selftest-supervise`, `--selftest-detach-shutdown`, and 0 leftover `fm-selftest` sessions afterwards (`claude agents --json --all`).
