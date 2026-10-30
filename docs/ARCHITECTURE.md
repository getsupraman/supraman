# Foreman architecture

For whoever is about to change Foreman, or to build another gpui tool (GitMan, FileMan) on its crates. What was
measured and why is in `docs/LEARNINGS.md`; the plan is `docs/PLAN.md`; the keys are `docs/KEYS.md`.

## Crates

Dependencies point down this list only; nothing below depends on anything above it.

| Crate | Owns | Depends on |
|---|---|---|
| `crates/foreman-app` | The `foreman.exe` binary: `FleetView` (left tree, tabs, Work tab, Inbox), Settings window, tray, the supervisor's driver, hand sessions, the selftests. | every crate below |
| `crates/fm` | The `fm` CLI: `workspace`, `sessions`, `want`/`unwant`/`roster`, `inbox`/`send`/`tell`/`messages`, `work`, `hook`, `plugin install`, `probe`. Edits files the app acts on; drives no supervisor. | claude, core, supervisor, workstate |
| `crates/foreman-supervisor` | Desired state per workspace, the pure `reconcile`, the executor (launch, resume, stop, mint, revoke), the shared busy budget, the Inbox rows (`NeedsYou`). No UI. | claude, core, workstate |
| `crates/foreman-claude` | The only code that knows the `claude` CLI: `agents --json --all` and health, `--bg` launch, stop, rm, resume, the peer pipe, transcripts, the background-service check. | core |
| `crates/foreman-workstate` | Every WorkState read: the read API v1 client, `/api/stream`, the OAuth client, the HTTP agent. Read only (C21). A test fake behind the `fake` feature. | core |
| `crates/foreman-ui` | The gpui widgets and painters every Foreman binary shares: text input, buttons, banners, menu, confirm, toast, notices, tabs, hover card, split pane, matcher, settings pages, themes, the terminal grid painter, hit targets, logger, frame probes. | core, term |
| `crates/foreman-term` | The terminal model: Zed's `terminal` crate vendored onto alacritty (PTY, grid, input, mouse, links, scroll, search). No gpui views, no Foreman knowledge. | — |
| `crates/foreman-core` | Plain Rust shared by all: the workspace registry, settings, files written safely, paths, time, process spawning, secrets at rest, the relay's files, hook events, names. No gpui, no claude, no HTTP. | — |

**Boundaries.**
- Only `foreman-claude` runs `claude` outside a terminal tab or parses its output; a tab runs `claude` in its PTY (`hand.rs`, `view.rs` `attach`).
- Only `foreman-workstate` talks HTTP to WorkState (the supervisor's `mint.rs` uses its `http_agent`). Nothing in Foreman writes WorkState items; the MCP tools are the agents' write path (C21).
- `foreman-supervisor` takes a snapshot and returns actions; it never polls, never paints, and is ticked by the app.
- `foreman-core` and `foreman-term` have no Foreman crate dependencies; `foreman-ui` has no `claude` or WorkState knowledge.
- `plugins/foreman` is not a crate: the Claude Code plugin loaded with `--plugin-dir` on sessions Foreman launches — `plugins/foreman/hooks/hooks.json` (exec-form `fm.exe hook <event>`), `plugins/foreman/roles/` (workspace, relay, repo), and the skills `workspace-controller` and `repo-controller`. See `plugins/foreman/README.md`.
- `spikes/term-spike` is not product: the m0/5 terminal spike, kept building against `foreman-term` and `foreman-ui`. Nothing depends on it.
- `probes/` holds the m0 measurement scripts; `scripts/` holds `scripts/release.ps1`, `scripts/shot.ps1` and `scripts/icon-gen/`.

## Shared components and helpers

Use these; do not grow a private copy (CLAUDE.md, C56). If none fits, extract one into the crate it belongs to and add it here and to CLAUDE.md.

### foreman-core (`crates/foreman-core/src/`)

- `fs::write_atomic` — every write of a file another process or thread may read (settings, registry, desired state, minted sessions, relay files). A unique temp file renamed over the target.
- `fs::lock_file` — an exclusive cross-process lock held while the returned file lives, around a read-modify-write (`DesiredState::update`, `refresh.lock`).
- `fs::load_json_or_notice` — a JSON state file whose loss is a notice, not an error (stars, dismissals).
- `fs::append_capped` (with `fs::rotated`) — an append-only log rotated to `<file>.1` at a cap (event logs, `foreman.log`).
- `paths::{user_home, claude_dir}` — the user's home and claude's config dir (`CLAUDE_CONFIG_DIR`-aware); never read `USERPROFILE`/`HOME` yourself.
- `paths::{normalize, path_key, within, key_within, main_checkout, linked_git_dir}` — path comparison (`within`: a folder or below it, never a sibling sharing a prefix) and the `.git` file/dir walk without running git.
- `time::{now_ms, epoch_ms, modified_ms, hex6}` — epoch milliseconds and a scratch-name suffix.
- `url::origin` (re-exported as `url_origin`; `foreman_workstate::api_base`) — the one origin rule; origins key `servers.json` and the minted records.
- `process::{command, run, git, shell_execute, alive, started, wait_any_exit}` — every child process outside the PTY: no console window, both pipes drained, a timeout that kills the child or its tree (`Kill::Child` for `claude`, whose tree holds the background service), ShellExecute, pid liveness. Never `Command::new(..).output()` by hand.
- `secret::{SecretStore, FileStore, MemoryStore, default_store}` — every secret at rest: one DPAPI-encrypted file per key under `<home>/oauth/secrets`; `MemoryStore` in tests.
- `slug::{slugify, validate_name, FM_PREFIX}` — a repo or workspace name as a session-name part; every Foreman session name starts with `fm-`.
- `Settings::update` — every change to `settings.toml`: load, edit, save, retried against other writers. `Settings::load_with_notices` when a bad key should be shown.
- `Settings` sections (`sessions`, `workstate`, `appearance`, `terminal`, `window`, each a `Section`) — a key belongs in its table; another owner's table (a plugin's `[gitman]`) goes through `Settings::section`/`set_section` and survives every save.
- `Registry` (`load_all`, `list`, `home_from`) — the workspaces at `<home>/workspaces/<name>.toml`; `load_all` where one corrupt file must not hide the rest (the supervisor), `list` where it should stop the caller.
- `Workspace` (`model.rs`: `home`, `folders`, `scans`; `home()`, `contains`, `other_dirs`, `folder_at`, `add_folder`, `unique_folder_name`), `Folder` (`kind`: `git` or `plain`), `Scan`, `owners` — a workspace is a name, a home and a set of folders; a root-based file loads as one scan of its root. `owners` says which workspace a folder's supervised session belongs to (the first that has it enabled); `Registry::claim_folders` disables what another workspace owns.
- `discover`, `rescan`, `scan`, `branch_at` — a folder to a workspace and its repos, and each scan run again; shared by `fm workspace add` and the app's Add workspace and Rescan.
- `Registry::edit` (`edit.rs`: `Edit`, `Edited`; `Workspace::find_folder`) — one change to a registered workspace (add or remove a folder, enable, home, add or remove a scan; a removed scanned folder goes on its scan's `exclude`), read, checked against `owners` and saved under `<name>.toml.lock`; shared by `fm workspace new`/`add-folder`/… and the app's Edit folders… (`view/folders.rs`).
- `relay` (`send`, `tell`, `inbox_dir`, `outbox_dir`, `InboxWatcher`, …) — the relay's `inbox`/`outbox` files per workspace.
- `events` (`append_event`, `EventReader`, `role_injection`) — the hook events `fm hook` appends per session.
- `install_hook_exe`, `find_plugin_dir` — placing `fm.exe` in the plugin.

### foreman-ui (`crates/foreman-ui/src/`)

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
- `term_element` (`terminal_grid`, `TermFont`, `cell_metrics`) — the terminal grid painter for a `foreman_term::Terminal`.
- `lib.rs`: `init_globals`, `init_logger`/`Logger` (`foreman.log` via `append_capped`), `FrameProbe`, `memory` — global setup and the probes the selftests print.

### foreman-workstate (`crates/foreman-workstate/src/`)

- `http_agent`, `pct_encode`, `server_reason` (`http.rs`) — the HTTP agent, path/query encoding and a refusal's reason for every WorkState call, the mint included.
- `Client` (`client.rs`) — the blocking read API v1 client (`overview`, `board`, `item`, `brief`, `inbox`, `journal`, `backlog`, `sessions`, `resolve`); GET only; its bearer comes from a `BearerSource` at each read; no error carries the token.
- `Stream` (`stream.rs`) — the `/api/stream` subscriber on its own thread: `StreamSignal::{Connected, Event, Lost}`; an event means "re-read".
- `Auth` (`auth.rs`) — the OAuth 2.1 native client: sign-in, `access_token`, refresh under `refresh.lock`, `bearer_for(origin)`.
- `health` — the unauthenticated `GET /health`.
- `model` — the read API v1 shapes, lenient (v1 grows additively).
- `fake::FakeWorkState` (feature `fake`) — the read API and its OAuth server on 127.0.0.1 for tests and selftests.
- `secret` re-exports `foreman_core::secret`.

### foreman-claude (`crates/foreman-claude/src/`)

- `list_sessions`, `parse`, `Session::health`/`supervised_health` — `claude agents --json --all` and the health table (crate docs in `lib.rs`).
- `Claude` (`launch.rs`: `launch`, `resume`, `stop`, `rm`, `remove`, `find_by_name`), `LaunchSpec`, `resume_argv`, `new_session_id`, `name_taken` — the write side; every run goes through `foreman_core::process::run`.
- `mcp_config_path`, `write_mcp_config`, `workstate_mcp_config` — a session's WorkState config file under `<home>/mcp/`, with the session-name check; write, read and delete it through these.
- `check_supported`, `MIN_VERSION`, `TESTED_VERSION` (`cli.rs`) — the CLI version gate.
- `daemon_status`, `daemon_mismatch`, `is_elevated` (`daemon.rs`) — whether this process can reach claude's background service.
- `peer` — writing into a session's own message pipe (`docs/peer-messaging.md`); `human::send_from_human` picks the pipe or the relay.
- `proc_tree` — which claude session runs under a tab's shell.
- `transcript` (`Transcripts`: `scan_with` with progress, `with_cache`, `cached`; `first_prompt`) — closed sessions from the transcript files, read only: head and tail of a new file, the grown part of a known one, on parallel readers; the file cache persists at `<home>/cache/transcripts.json` so a start shows its rows before any file is read.
- `forget::forget_transcript` — the one write under the claude dir (see delete confinement).
- `assign` — mapping sessions onto a workspace's folders (deepest folder, then a worktree's main checkout), then its home.

### foreman-supervisor (`crates/foreman-supervisor/src/`)

- `DesiredState` (`desired.rs`) — `<home>/supervisor/<workspace>.toml`; change it only through `update` (locked read-modify-write); `release` unwants the repo sessions of folders taken away, so the supervisor stops them.
- `Supervisor`, `tick_all`, `Budget` — one tick per snapshot; the rule table is the crate docs in `lib.rs`.
- `HttpMinter`, `Minter`, `MintedSessions` (`mint.rs`) — per-session WorkState tokens.

## Shared vs app-only

**GitMan and FileMan may reuse:** all of `foreman-core`, `foreman-ui`, `foreman-term`, `foreman-workstate`, and `foreman-claude` if they run claude. Their settings live in their own `Settings` table (`Settings::section`/`set_section`) and their settings window registers its pages with `SettingsPages`.

**App-only (in `crates/foreman-app`, not for reuse):**
- `FleetView` (`crates/foreman-app/src/view.rs`, `crates/foreman-app/src/view/`) — the left tree (design B: the workspace row is the workspace session, the relay a glyph; `row_card.rs` its hover cards), tabs, menus, delete flow; `crates/foreman-app/src/work_view.rs` and `crates/foreman-app/src/work_view/` — the Work tab and Inbox.
- The tab lifecycle: open, attach, resume, and detach on close or quit (`view.rs` `close_tab`, `start_detach`, `quit_step`).
- The supervisor policy and its driver: `supervise.rs`, the poll loop (`view.rs` `Poller`), `foreman-supervisor` itself.
- Hand sessions (`hand.rs`), tokens (`tokens.rs`), sign-in wiring (`signin.rs`), WorkState setup wording (`workstate_setup.rs`).
- The tray (`tray.rs`), the single instance (`instance.rs`), elevation (`elevate.rs`), the titlebar, stars, dismissals, the selftests.

## Key flows

### A tab starting a session
1. `crates/foreman-app/src/view.rs` `FleetView::new_claude` → `hand_claude`: pushes a shell tab, then `Hands::launch_for` off the UI thread.
2. `crates/foreman-app/src/hand.rs` `Hands::launch_for` picks the name (`session_name`, `next_name`), mints a token (below) and writes the config through `mcp_config_path`.
3. `hand.rs` `Launch::script_with` builds `claude [--session-id <uuid>] --mcp-config <path>` in a try/finally that deletes the config when claude exits; `shell_args_with` runs it as `pwsh -NoExit -EncodedCommand`.
4. `view.rs` `start_terminal` builds the PTY with `foreman_term::TerminalBuilder`; `foreman_ui::term_element::terminal_grid` paints it.
5. When the config file is gone (`Hands::ended`), `Hands::release` revokes the token.

Attach opens `claude attach <id>` (`view.rs` `attach`); `resume` runs `claude --bg --resume <id>` then attach for a background session, or `claude --resume` through `hand_claude` for an interactive one. Supervised sessions get no tab: `crates/foreman-claude/src/launch.rs` `LaunchSpec::argv` and `resume_argv` build their `--bg` lines.

### The supervisor tick
1. `crates/foreman-app/src/view.rs` `Poller` loops every `REFRESH_EVERY` or when nudged; `read` reads the registry and one `list_sessions()` snapshot, which the app applies (and paints) before `Poller::tick` hands that same snapshot to the supervisor. At start the version check runs beside setup and the first read.
2. `crates/foreman-app/src/supervise.rs` `Supervision::tick` loads `Settings` and each `DesiredState`, syncs the relay, applies server changes and the daemon hold, then under `TICK` (and only while `ENABLED`) calls `tick_all`.
3. `crates/foreman-supervisor/src/budget.rs` `tick_all` builds one `Budget` and ticks each workspace's `Supervisor`.
4. `crates/foreman-supervisor/src/executor.rs` `Supervisor::tick`: re-mint stops, resume confirmation, memory, `reconcile` (`crates/foreman-supervisor/src/reconcile.rs`, pure), then launch / resume / stop, revocation, and the `NeedsYou` rows.

`fm want`/`unwant` (`crates/fm/src/repos.rs`) and the app's Start/Stop (`supervise.rs` `start_workspace`) only edit `DesiredState` through `update`; the next tick acts on it.

### Minting and re-minting
- The supervisor mints for `fm-` sessions (`executor.rs` `Supervisor::mint`, from `launch`, and `remint` before every Resume); hands mint for tab sessions (`hand.rs` `Hands::launch_for`).
- `crates/foreman-supervisor/src/mint.rs` `HttpMinter`: POST `/api/agent-sessions` (`replace: true`), PATCH to bind the claude session id, DELETE to revoke (404 counts as revoked). The bearer is the OAuth access token (`crates/foreman-app/src/tokens.rs` `AppTokens`, role from `workstate_role`).
- Only the minted `wss_` token reaches a session, in the file `mcp_config_path` names (`crates/foreman-claude/src/launch.rs` `write_mcp_config`, owner-only ACL, `--strict-mcp-config` for supervised sessions).
- Records: `<home>/supervisor/<workspace>.sessions.json` (`MintedSessions`) and `<home>/supervisor/hands.json`.
- Re-mint: every Resume; the Re-mint remedy on a `TokenExpired` or `OtherServer` row (`Supervisor::retry` → `stop_for_remint`, then resume); a server change (`Supervisor::server_changed`). Unwanted tokens are revoked by `revoke_unwanted` and `Hands::release_all`.

### OAuth sign-in and refresh
1. `crates/foreman-app/src/view.rs` `FleetView::sign_in` → `crates/foreman-app/src/signin.rs` (one `Auth` per home).
2. `crates/foreman-workstate/src/auth.rs` `Auth::sign_in`: discovery, dynamic client registration, PKCE S256 on a loopback callback, the browser, the token request.
3. `Auth::access_token`: the held token while fresh, else a `refresh_token` grant under `<home>/oauth/refresh.lock` (shared with `fm`); `invalid_grant`/`invalid_client` forgets the stored token.
4. The refresh token lives in `crates/foreman-core/src/secret.rs` `FileStore`: `<home>/oauth/secrets/<hash>.bin`, DPAPI-encrypted for the current user, written with `write_atomic`. `<home>/oauth/servers.json` holds server metadata, no token. The access token is never logged and never written.

### The WorkState read and its stream
1. `crates/foreman-app/src/work_view.rs` `WorkHub` (a gpui global) builds a `Client` with `Auth::bearer_for(origin)` and subscribes a `Stream`.
2. `crates/foreman-workstate/src/stream.rs` `Stream::subscribe` holds `GET /api/stream` on its own thread with `Last-Event-ID` and our backoff; `WorkHub::on_signal` turns signals into `HubEvent`s.
3. `WorkView::on_hub` re-reads the open view through `crates/foreman-workstate/src/client.rs` `Client`, debounced, off the UI thread; `crates/foreman-app/src/work_view/inbox.rs` `WorkInbox` listens to the same hub. `WorkHub::reconfigure` runs after sign-in, sign-out or a URL change.
4. `fm work` (`crates/fm/src/work.rs`) reads through the same `Client` and the same sign-in.

### Quit
1. `crates/foreman-app/src/tray.rs` Quit (asking first when workspaces are started) → `view.rs` `FleetView::quit`; closing the window only hides it to the tray.
2. Every attach tab detaches: `close_tab` → `start_detach` writes Ctrl+Z (`DETACH_INPUT`) and waits `DETACH_WITHIN`; a tab that does not detach cancels the quit.
3. `quit_step`: with no attach tab left, `hand::release_all_within` revokes the hand tokens, then `close_for_quit` closes the window. `open_hands` also releases them on `on_app_quit` and releases stale ones at start.
4. Supervised `--bg` sessions keep running; the supervisor only stops ticking.

## Protected business logic

Change these only deliberately, with the owner's say and a LEARNINGS entry.

- **C21, WorkState is read only:** `crates/foreman-workstate/src/client.rs` (GET only), `crates/foreman-app/src/work_view/inbox.rs` (dismissals are local), `crates/fm/src/work.rs`. Test: `crates/foreman-workstate/tests/client.rs`.
- **C15/C17, the units' red lines in code:** selftests run on a home under `target/` (`crates/foreman-app/src/selftest.rs` `home`), act only on their own `fm-selftest-<hex>` sessions and clean them up (`cleanup`), read WorkState from `FakeWorkState`, and never print or write a token.
- **The supervisor's rules:** the R1–R14 table, the budget, failures and hold in the crate docs of `crates/foreman-supervisor/src/lib.rs`, implemented in `reconcile.rs` and `executor.rs`; the claude health table in `crates/foreman-claude/src/lib.rs`.
- **The claude CLI protocols:** `peerProtocol 1` in `crates/foreman-claude/src/peer.rs` (`PEER_PROTOCOL`); the minimum and tested CLI in `cli.rs`; the `--bg` line and its resume in `launch.rs` (`LaunchSpec::argv`, `resume_argv`: a resume carries no other flag); rm and its refusals in `launch.rs` (`Claude::remove`, `RmRefusal`); the service reach in `daemon.rs`. The facts are the `fm probe` results (`crates/fm/src/probe_rm.rs`, `crates/fm/src/probe_session_id.rs` and the other `fm probe` files, LEARNINGS m2/1, m5/38, m5/40).
- **Delete confinement:** `crates/foreman-claude/src/forget.rs` (`forget_transcript`, `confined`: a UUID id, no link or junction, inside `projects/`); its one caller is `crates/foreman-app/src/view/delete.rs` `delete_blocking`.
- **Mint and name rules:** `crates/foreman-app/src/hand.rs` `session_name`, `next_name`; `crates/foreman-supervisor/src/desired.rs` `workspace_session_name`, `relay_session_name`, `repo_session_name`; `FM_PREFIX` in `crates/foreman-core/src/slug.rs`; `mcp_config_path` and `name_taken` in `crates/foreman-claude/src/launch.rs`; `MintRequest` in `crates/foreman-supervisor/src/mint.rs`. Contract: Skills `docs/workstate/14-agent-launch.md`.
- **Contrast floors:** `crates/foreman-ui/src/themes.rs` (`NAME_FLOOR`, `SECONDARY_FLOOR`, `fit_floors`), test `the_contrast_floors_hold_under_each_bundled_theme`; notice tints in `crates/foreman-ui/src/banner.rs`.
- **The busy budget:** `crates/foreman-supervisor/src/budget.rs` (`Budget`, `mid_turn`, `tick_all`), the relay's exemption (`WantedSession::exempt_from_budget`), `max_working` in `crates/foreman-core/src/settings.rs`.

## Testing

- **cargo:** `cargo test --workspace`. Unit tests sit beside the code; integration tests in each crate's `tests/`. Fakes: `FakeWorkState` (`crates/foreman-workstate/src/fake.rs`), a fake claude (`crates/foreman-claude/tests/fake_claude.rs`), `MemoryStore` for secrets.
- **Selftests** drive the real app on a home under `target/` (`crates/foreman-app/src/main.rs` `parse_args`, `crates/foreman-app/src/selftest.rs` `start`):
  - `foreman.exe --selftest` — the gate's UI run (`selftest.rs` `gate`, steps in `selftest_*.rs`), clicking through `HitTargets`.
  - `--selftest-supervise` — the supervisor against real `claude --bg` sessions (`crates/foreman-app/src/selftest_supervise.rs`).
  - `--selftest-detach-shutdown` — attach tabs detach on quit (`detach_on_shutdown`); `--selftest-detach <n>` and `--selftest-perf` are measurement runs.
- **The clipboard mutex:** `crates/foreman-app/src/selftest_clipboard.rs` holds the named mutex `Local\Foreman-selftest-clipboard` while a selftest uses the clipboard, so parallel selftests on one machine do not clobber each other.
- **The gate**, run before every merge: `cargo build --release --workspace`, `cargo test --workspace`, `--selftest`, `--selftest-supervise`, `--selftest-detach-shutdown`, and 0 leftover `fm-selftest` sessions afterwards (`claude agents --json --all`).
