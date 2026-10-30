# Foreman (AutoWork)

Native Rust/gpui app that supervises workspaces of Claude Code sessions. Work runs as a controller queue in WorkState project `autowork`; `docs/LEARNINGS.md` holds what each unit found.

## Use shared components, never hidden hand-rolled ones

**This binds every agent and subagent working in this repo — brief subagents on it when you delegate.**

Before you build something generic yourself (a text input, a list, a dialog, an HTTP or OAuth client, a file watcher, a process helper, a parser, …), search for a shared one and use it. If none fits, extract a shared one into the crate it belongs to and list it below. Do not grow a private copy inside a feature. A unit that keeps a local implementation says why in its report.

Reusable components (crates, boundaries, flows and when to use each: `docs/ARCHITECTURE.md`):

- `foreman_ui::text_input`: the one text input (EntityInputHandler: selection, IME, clipboard); every text field uses it.
- `foreman_ui::button::{bordered, outline, pill, icon, revealed, toolbar, toolbar_turning, ghost, small, choice}`: every clickable button; the key is its id, hover group and hit key; `toolbar_turning` turns its icon while work it started runs; `revealed` is a row's hover icon, always laid out and painted (no shift, hit key kept), shown while its row's group is hovered or the row is selected.
- `foreman_ui::banner::{tinted, plain, dismissable, tint}`: every notice box; a status colour on its 0.12 tint (m5/26 contrast).
- `foreman_ui::hit::{HitTargets, hit_target}`: every clickable element records its painted bounds under its key; the selftests click through these.
- `foreman_ui::fonts::{UI, MONO, icons}`: the UI's font families; never a family name spelt out in a view.
- `foreman_ui::menu::{ContextMenu, MenuItem, item}`: every right-click menu, generic over the owner's action type; items paint under `menu-<key>`; `MenuItem::under(caption)` starts a section (a rule and a caption).
- `foreman_ui::confirm::Confirm`: every question before an action that cannot be taken back (`ask`), and every report with only its OK (`report`); the owner places it.
- `foreman_ui::toast::ToastSlot`: every short feedback that floats over a panel and goes by itself (`show` for done, `show_why` for why nothing was done).
- `foreman_ui::notices::{NoticeKind, render, keep_dismissed, remove_one}`: every list of plain notices with a × each; the owner keeps the kinds and their sources.
- `foreman_ui::tabs::{tab_strip, TabHeader, DraggedTab, DraggedSession, reorder, cycled, sanitize}`: the tab strip with drag-to-reorder (`on_move(from, to)`), the dragged tab's and session row's payloads for any other drop target, tab cycling, and a program's OSC title as a tab label; tab state, order and detach stay with the owner.
- `foreman_ui::hover_card::{HoverCard, tooltip}`: every hover card of a title and `label  value` rows.
- `foreman_ui::split::SplitPane`: every resizable side-by-side split; the owner keeps and persists the width.
- `foreman_ui::matcher::{find_all, prefix, highlighted, count}`: case-insensitive matching and a results list's highlighted label and count badge.
- `foreman_ui::settings_page::{SettingsPages, SettingsPage, row}`: a settings window's pages, registered by id with a title, an icon and a render fn, its page list, and one setting's row.
- `foreman_ui::nudge::nudge`: a loop on a gpui executor that sleeps until nudged or timed out, never a timer polling a flag.
- `foreman_ui::themes` (`contrast`, `fit_floors`) and `foreman_ui::term_element::terminal_grid`: the themes with their contrast floors, and the terminal painter.
- `foreman_core::fs::write_atomic`: every write of a file another process or thread may read or write (settings, registry, supervisor and relay files); never a fixed `*.tmp` and rename by hand.
- `foreman_core::fs::lock_file`: an exclusive cross-process lock held while the returned file lives, around a read-modify-write (`refresh.lock`, `DesiredState::update`); never a check-then-write retry loop.
- `foreman_core::fs::load_json_or_notice`: a JSON state file whose loss is shown to the user as a notice, not an error (stars, dismissals).
- `foreman_core::fs::append_capped`: an append-only log that rotates to `<file>.1` at a size cap (event logs, `foreman.log`).
- `foreman_core::prune::prune`: the one retention rule for event logs and relay files left in the home; never a cleanup of your own.
- `foreman_core::Settings::update`: every change to `settings.toml` (load, edit, save, retried against other writers); `Settings::load_with_notices` when a bad key should be shown.
- `foreman_core::Settings` sections (`sessions`, `workstate`, `appearance`, `terminal`, `window`, each a `Section`): a key belongs in its table; another owner's table (a plugin's `[gitman]`) goes through `Settings::section`/`set_section` and survives every save.
- `foreman_core::secret::{SecretStore, FileStore, MemoryStore, default_store}`: every secret at rest (DPAPI file per key under `<home>/oauth/secrets`); `MemoryStore` in tests; `foreman_workstate::secret` re-exports it.
- `foreman_core::Registry::load_all`: every list of workspaces that must survive one corrupt file (the supervisor); `list` only where one failure should stop the caller.
- `foreman_core::{slugify, validate_name, FM_PREFIX}`: a name as a session-name part; every Foreman session name starts with `fm-`.
- `foreman_core::{discover, rescan, scan}`, the relay files (`send`, `tell`, `inbox_dir`, `outbox_dir`) and the hook events (`append_event`, `EventReader`): shared by `fm` and the app.
- `foreman_core::paths::{user_home, claude_dir}`: the user's home and claude's config dir (`CLAUDE_CONFIG_DIR`-aware); never read `USERPROFILE`/`HOME` yourself.
- `foreman_core::paths::{normalize, path_key, within, main_checkout, linked_git_dir}`: path comparison (`within` for "this folder or below") and the `.git` file/dir walk without running git.
- `foreman_core::{Registry::edit, Edit, Edited}`: every change to a registered workspace's folders, home and scans (`fm workspace add-folder`/`remove-folder`/`enable`/`disable`/`set-home`/`add-scan`/`remove-scan`, the app's Edit folders…), under the file's lock; a removed folder's session goes through `DesiredState::release`.
- `foreman_core::{Workspace, Folder, owners}` and `Registry::claim_folders`: a workspace's home, folders (git or plain) and scans; `home()` for where its own sessions run, `contains` for "is this cwd ours", `other_dirs` for the `--add-dir`s, `owners` for which workspace supervises a folder; never a root of your own.
- `foreman_core::schedule`: a workspace's schedules at `<home>/schedules/<ws>.toml` (`load`, `edit` under its lock; a covered edit sets `pending`, never the approval), the covered `fingerprint` with `approve` (the app's Approve only, C100) and `verify_approval`/`effective_status`, `next_fire`/`previous_fire` in a given zone across DST, the run records (`append_run`, `runs`), `SCHED_SLUG_PREFIX`, and a run's `session_name`/`run_id`/`run_worktree`, `preamble`, `base_prompt` (slash command via `command_file`) and profile flags (`Profile::permission_mode`, `Permissions::allowed`/`denied`, `DEFAULT_DENY`); `prune::prune` covers its files.
- `foreman_core::fm_allowed_tools`: the `--allowedTools` rules that keep `fm` from parking, for every Foreman-launched session and scheduled run.
- `foreman_core::time::{now_ms, epoch_ms, modified_ms, hex6}`: epoch milliseconds and a scratch-name suffix.
- `foreman_core::process::{command, run, git, shell_execute, alive, started}`: every child process outside the PTY: no console window, drained pipes, a timeout that kills the tree (`Kill::Child` for `claude`, whose tree holds the background service), and ShellExecute and pid liveness; never `Command::new` and `.output()` by hand.
- `foreman_core::url::origin` (`foreman_workstate::api_base`): the one origin rule; origins are map keys in `servers.json` and the minted records.
- `foreman_workstate::{http_agent, read_reply, pct_encode, server_reason}`: the HTTP agent, an answer read whole, path/query encoding and a refusal's reason for every WorkState call; tests serve them from `FakeWorkState`, never a hand-rolled server.
- `foreman_workstate::{Client, Stream, Auth}`: the read API v1 (GET only, C21), `/api/stream`, and the OAuth sign-in whose `bearer_for(origin)` every read and mint carries; `FakeWorkState` (feature `fake`) in tests.
- `foreman_claude::mcp_config_path`: the path of a session's WorkState config file, with the session-name check; write, read and delete it through this.
- `foreman_claude::{Claude, LaunchSpec, resume_argv, list_sessions}`: every `claude` launch, resume, stop, rm and listing outside a terminal tab; a tab runs `claude` in its PTY.
- `foreman_supervisor::DesiredState::update`: every change to a workspace's wanted sessions (the app and `fm want`); `DesiredState::release` stops the sessions of folders taken away (stop, never rm).

## Comments state the present, not the past

**This binds every agent and subagent working in this repo — brief subagents on it explicitly when you delegate, because they default to narrating their own fix.** The full procedure is the `rlarch:code-comments` skill.

Comments are written subtractively. The default verdict on any comment — one you are about to write, one you find, one a subagent hands you — is **delete**. If you catch yourself weighing whether a comment earns its place, that deliberation is the answer.

A comment survives only when it says something the code cannot say by itself, in the **present tense**:

- No "used to", "previously", no naming a retired construct, no narrating what a change did.
- No teaching the reader what a language or framework feature does — the reader already knows Rust and gpui.
- No paraphrasing a rule decided in another file. Point at where the contract lives (`// replace covers a live token; see doc 14 §2.`) instead of restating it — the restatement goes stale and then misleads.
- Survivors get **one line**; more only when genuinely irreducible.
- Never *add* `///` docs during a cleanup pass. Doc comments on the shared components listed above stay.

Treat existing comments as untrusted: a comment that contradicts its code is wrong and gets deleted, not corrected — unless the contradiction suggests the *code* is the broken half, in which case report it and leave the code alone.
