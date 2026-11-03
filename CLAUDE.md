# Foreman (AutoWork)

Native Rust/gpui app that supervises workspaces of Claude Code sessions. Work runs as a controller queue in Workman project `autowork`; `docs/LEARNINGS.md` holds what each unit found.

## Use shared components, never hidden hand-rolled ones

**This binds every agent and subagent working in this repo — brief subagents on it when you delegate.**

Before you build something generic yourself (a text input, a list, a dialog, an HTTP or OAuth client, a file watcher, a process helper, a parser, …), search for a shared one and use it. If none fits, extract a shared one into the crate it belongs to and list it below. Do not grow a private copy inside a feature. A unit that keeps a local implementation says why in its report.

Reusable components (crates, boundaries, flows and when to use each: `docs/ARCHITECTURE.md`):

- `supraman_ui::text_input`: the one text input (EntityInputHandler: selection, IME, clipboard); every text field uses it.
- `supraman_ui::button::{bordered, outline, pill, icon, revealed, toolbar, toolbar_turning, ghost, small, choice}`: every clickable button; the key is its id, hover group and hit key; `toolbar_turning` turns its icon while work it started runs; `revealed` is a row's hover icon, always laid out and painted (no shift, hit key kept), shown while its row's group is hovered or the row is selected.
- `supraman_ui::banner::{tinted, plain, dismissable, tint}`: every notice box; a status colour on its 0.12 tint (m5/26 contrast).
- `supraman_ui::hit::{HitTargets, hit_target}`: every clickable element records its painted bounds under its key; the selftests click through these.
- `supraman_ui::fonts::{UI, MONO, icons}`: the UI's font families; never a family name spelt out in a view.
- `supraman_ui::menu::{ContextMenu, MenuItem, item}`: every right-click menu, generic over the owner's action type; items paint under `menu-<key>`; `MenuItem::under(caption)` starts a section (a rule and a caption).
- `supraman_ui::confirm::Confirm`: every question before an action that cannot be taken back (`ask`), and every report with only its OK (`report`); the owner places it.
- `supraman_ui::toast::ToastSlot`: every short feedback that floats over a panel and goes by itself (`show` for done, `show_why` for why nothing was done).
- `supraman_ui::notices::{NoticeKind, render, keep_dismissed, remove_one}`: every list of plain notices with a × each; the owner keeps the kinds and their sources.
- `supraman_ui::tabs::{tab_strip, TabHeader, DraggedTab, DraggedSession, reorder, cycled, sanitize}`: the tab strip with drag-to-reorder (`on_move(from, to)`), the dragged tab's and session row's payloads for any other drop target, tab cycling, and a program's OSC title as a tab label; tab state, order and detach stay with the owner.
- `supraman_ui::hover_card::{HoverCard, tooltip}`: every hover card of a title and `label  value` rows.
- `supraman_ui::split::SplitPane`: every resizable side-by-side split; the owner keeps and persists the width.
- `supraman_ui::matcher::{find_all, prefix, highlighted, count}`: case-insensitive matching and a results list's highlighted label and count badge.
- `supraman_ui::details::{Page, Section, Line, render, text, index_of, OnLink}`: every details page in a tab: a title with a status pill, sections of `label  value` rows, links (`Section::link`, followed by the page's `on_link`), paragraphs and long text with Copy, and an actions bar; the owner builds the sections and keeps the `ScrollHandle`; `text` is what a test reads.
- `supraman_ui::settings_page::{SettingsPages, SettingsPage, row}`: a settings window's pages, registered by id with a title, an icon and a render fn, its page list, and one setting's row.
- `supraman_ui::nudge::nudge`: a loop on a gpui executor that sleeps until nudged or timed out, never a timer polling a flag.
- `supraman_ui::keyed_list::KeyedList`: every long list that lays out only its rows in view (`gpui::list`), a key per row so the row on top stays on top as rows come and go, and `reveal(hit)` to scroll to the row a hit key belongs to; the sidebar uses it.
- `supraman_ui::stall::{section, watch, take}`: every UI-thread section worth timing (`section(label)`, logged over 100 ms), the foreground heartbeat that logs a held UI thread with the sections that ran (`watch`, started once in `main`), and a selftest's read of both.
- `supraman_ui::themes` (`contrast`, `fit_floors`) and `supraman_ui::term_element::terminal_grid`: the themes with their contrast floors, and the terminal painter.
- `supraman_core::fs::write_atomic`: every write of a file another process or thread may read or write (settings, registry, supervisor and relay files); never a fixed `*.tmp` and rename by hand.
- `supraman_core::fs::lock_file`: an exclusive cross-process lock held while the returned file lives, around a read-modify-write (`refresh.lock`, `DesiredState::update`); never a check-then-write retry loop. `try_lock_file` is the same without the wait (`None` while held): a schedule's overlap lock.
- `supraman_core::fs::heavy_job_lock`: the machine-wide lock of one heavy job; every selftest mode holds it for its run, and `scripts/heavy-lock.ps1` runs a cargo build or test under the same file, so builds and selftests never overlap.
- `supraman_core::fs::load_json_or_notice`: a JSON state file whose loss is shown to the user as a notice, not an error (stars, dismissals).
- `supraman_core::fs::append_capped`: an append-only log that rotates to `<file>.1` at a size cap (event logs, `foreman.log`).
- `supraman_core::prune::prune`: the one retention rule for event logs and relay files left in the home; never a cleanup of your own.
- `supraman_core::Settings::update`: every change to `settings.toml` (load, edit, save under `<home>/settings.lock`); `Settings::load_with_notices` when a bad key should be shown.
- `supraman_core::Settings` sections (`sessions`, `workman`, `appearance`, `terminal`, `window`, each a `Section`): a key belongs in its table; another owner's table (a plugin's `[gitman]`) goes through `Settings::section`/`set_section` and survives every save.
- `supraman_core::secret::{SecretStore, FileStore, MemoryStore, default_store}`: every secret at rest (DPAPI file per key under `<home>/oauth/secrets`); `MemoryStore` in tests; `supraman_workman::secret` re-exports it.
- `supraman_core::Registry::load_all`: every list of workspaces that must survive one corrupt file (the supervisor); `list` only where one failure should stop the caller.
- `supraman_core::{slugify, validate_name, FM_PREFIX}`: a name as a session-name part; every Foreman session name starts with `fm-`.
- `supraman_core::{discover, rescan, scan}`, the relay files (`send`, `tell`, `inbox_dir`, `outbox_dir`) and the hook events (`append_event`, `EventReader`): shared by `sm` and the app.
- `supraman_core::home::{home_from_env, env_home, home_env, migrate_default_home, take_migration_notice}` (`HOME_VAR` `SUPRAMAN_HOME`, `LEGACY_HOME_VAR` `FOREMAN_HOME`): the Supraman home, `%APPDATA%\Supraman` by default; `env_home` for a hook's or session's home; `home_env` for a child's env (both names); the one-time copy of `%APPDATA%\Foreman` (the app's start; `Registry::from_env` refuses until it ran). Never read either var yourself.
- `supraman_core::paths::{user_home, claude_dir}`: the user's home and claude's config dir (`CLAUDE_CONFIG_DIR`-aware); never read `USERPROFILE`/`HOME` yourself.
- `supraman_core::paths::{normalize, path_key, within, main_checkout, linked_git_dir}`: path comparison (`within` for "this folder or below") and the `.git` file/dir walk without running git.
- `supraman_core::paths::pwsh_command`: the file pwsh runs for a bare command name (per `PATH` folder, `.ps1` first, then `PATHEXT`), what a [+] tab's `claude` resolves to.
- `supraman_core::{Registry::edit, Edit, Edited}`: every change to a registered workspace's folders, home and scans (`sm workspace add-folder`/`remove-folder`/`enable`/`disable`/`set-home`/`add-scan`/`remove-scan`, the app's Edit folders…), under the file's lock; a removed folder's session goes through `DesiredState::release`.
- `supraman_core::{Workspace, Folder, owners}` and `Registry::claim_folders`: a workspace's home, folders (git or plain) and scans; `home()` for where its own sessions run, `contains` for "is this cwd ours", `other_dirs` for the `--add-dir`s, `owners` for which workspace supervises a folder; never a root of your own.
- `supraman_core::schedule`: a workspace's schedules at `<home>/schedules/<ws>.toml` (`load`, `edit` under its lock; a covered edit sets `pending`, never the approval), the covered `fingerprint` with `approve` (the app's Approve only, C100) and `verify_approval`/`effective_status`, `next_fire`/`previous_fire` in a given zone across DST, the run records (`append_run`, `runs`), a run's files (`live_dir` with the `Live` file, `state_dir`, `fire_log`), a schedule secret's `secret_key` and `masked_tail` for every recorded stderr, `SCHED_SLUG_PREFIX`, and a run's `session_name`/`run_id`/`run_worktree`/`run_branch`, `preamble`, `base_prompt` (slash command via `command_file`) and profile flags (`Profile::permission_mode`, `Permissions::allowed`/`denied`, `DEFAULT_DENY`); `prune::prune` covers its files.
- `supraman_core::schedule::{covered_diff, covered_fields, schedule_of_session}` and `Display for Trigger`: what Approve shows (the covered fields, or their change since `approval.covered`), a run session's schedule id from its name, and a trigger as `sm schedule add --every` takes it.
- `supraman_core::desktop::{HiddenDesktop, spawn_on, quote_arg, input_desktop_name, thread_desktop_name}`: a private desktop that is never switched to and a child started on it with its pipes back (the selftests' default, `supraman-app` `selftest_desktop`); never `SwitchDesktop`, never `SendInput`.
- `supraman_core::process::spawn_detached`: a child that outlives the call (Run now's `smw`), no pipes, no wait.
- `sm::fire::recover_if_dead`: a live file whose sm is gone, recovered as `interrupted` (stop, never rm; revoke) while no fire holds the lock; the app calls it each poll.
- `crate::row_details` and `view/details.rs` (supraman-app): a session's and a workspace's Details tab (`TabKind::Details`, `open_details`, one tab per target): `row_details::{session, workspace}` say the facts as `Section`s, the poll refreshes them (`refresh_details`, which also reads `OnDisk`: branch, worktree, transcript), and the actions are the row's menu items run through `run_action`, so the page allows nothing the menu does not.
- `crate::schedules` and `view/schedules.rs` (supraman-app): the Schedules group, a schedule's Details tab (`TabKind::Schedule`, `open_schedule`, the page's words in `schedules::sections`) with Approve (`approve_shown` → `approve_schedule`, the only `schedule::approve` caller), Run now, the Edit dialog and the schedule Inbox entries (failed runs, and `schedules::waiting` for a pending schedule); the window's `registrar` is `FakeRegistrar` in every selftest.
- `supraman_core::schedule::{sync, Registrar, WindowsRegistrar, FakeRegistrar, task_xml, task_root}`: every Windows task of a schedule; `sync` makes `\Supraman\<ws>` (another home `\Supraman-<hex8>`) hold the active, approved schedules and nothing else, and empties this home's `legacy_task_root` folder (`\Foreman\<ws>`), never outside them; only the app calls it, after Approve, Pause and Remove (C100); tests and selftests use `FakeRegistrar` (C101). `supraman_claude::task_run_level` gives its run level.
- `supraman_core::sm_allowed_tools`: the `--allowedTools` rules that keep `sm` (and, for one release, `fm`) from parking, for every Foreman-launched session and scheduled run.
- `supraman_core::install_hook_exe` (`HOOK_EXE`, `COMPAT_HOOK_EXE`): the plugin's `bin/sm.exe` and its `bin/fm.exe` copy (docs/RELEASE.md, "The fm name").
- `supraman_core::time::{now_ms, epoch_ms, modified_ms, hex6}`: epoch milliseconds and a scratch-name suffix.
- `supraman_core::update::{Manifest, sign, verify, release_keys, sha256_file, signature_path}`: a release's `supraman-update.json` (signed as its bytes on disk), its ed25519 `.sig` and the release keys compiled in from `update.pub`; `Manifest::check_zip` for a downloaded zip; `sm release` (manifest, sign, verify, keygen) and the app's updater use it, never a hash or signature check of their own.
- `supraman_workman::updates` (`check_if_on`, `check`, `Source`, `Outcome`, `Available`, `last`, `dismiss`/`dismissed`/`shows`, `running_version`): the one look for a newer release (`GET <api>/repos/<repo>/releases/latest` with the kept ETag, manifest and `.sig` verified with `supraman_core::update::verify`), kept in `<home>/updates/`; `[updates] check` off sends nothing; tests serve it from `FakeWorkman`, the selftest pins `API_VAR` to a closed port. `crate::updates` (supraman-app) runs it at start and daily with `nudge`, shows the banner and Settings → About's line, and `UpdateAction` is what Update does.
- `supraman_core::install::{unpack, apply, cleanup, install_dir, file_list}` (`FILES`, `LEFTOVERS`) and `supraman_workman::updates::{download, prune_downloads, failed_apply}`: a release zip downloaded and checked against the signed manifest, unpacked (an entry escaping `Supraman\` refuses it), swapped over an install with a locked exe renamed aside and every move journalled (`apply`, only through `sm update apply`, run from a copy of the new sm.exe), and the `.old-<stamp>` leftovers removed at every start of Supraman and sm (`cleanup`); docs/RELEASE.md "How Supraman updates itself". Never a recursive delete of an install.
- `supraman_workman::http_agent_following`: the shared agent that follows redirects (a release asset's download URL); `http_agent` follows none.
- `supraman_core::process::{command, run, run_watched, git, shell_execute, alive, started}`: every child process outside the PTY: no console window, drained pipes, a timeout that kills the tree (`Kill::Child` for `claude`, whose tree holds the background service), and ShellExecute and pid liveness; never `Command::new` and `.output()` by hand. `run_watched` also ends the child when its `stop` says so and keeps what a killed child wrote (a schedule's script).
- `supraman_core::url::origin` (`supraman_workman::api_base`): the one origin rule; origins are map keys in `servers.json` and the minted records.
- `supraman_workman::{http_agent, read_reply, pct_encode, server_reason}`: the HTTP agent, an answer read whole, path/query encoding and a refusal's reason for every Workman call; tests serve them from `FakeWorkman` (`queue` for a route's answers in turn, `Answer::silent` for a server that never answers, `seen_when` to wait for a request, `fake::closed_base` for a port nothing listens on), never a hand-rolled server or `TcpListener`.
- `supraman_workman::{Client, Stream, Auth}`: the read API v1 (GET only, C21), `/api/stream`, and the OAuth sign-in whose `bearer_for(origin)` every read and mint carries; `FakeWorkman` (feature `fake`) in tests.
- `supraman_claude::mcp_config_path`: the path of a session's Workman config file, with the session-name check; write, read and delete it through this.
- `supraman_claude::{Claude, LaunchSpec, resume_argv, list_sessions}`: every `claude` launch, resume, stop, rm and listing outside a terminal tab; a tab runs `claude` in its PTY.
- `supraman_claude::transcript::Usage`: a session's tokens from its transcript (input, output, cache creation and cache reads, each message once), read on incrementally; a scheduled run's token cap.
- `supraman_supervisor::{AppTokens, bearer, workstate_role}`: the one `TokenProvider` of Foreman's sign-in, for the app's supervisor and for `sm`'s scheduled runs.
- `supraman_supervisor::LaunchConfig::launch_spec`: every supervised or scheduled `claude --bg` spec (name, cwd, prompt, mode, strict MCP, plugin dirs, add-dirs, `--settings` env, `--allowedTools`); the caller adds its token and its own flags. `session_env` is that env as pairs and `interactive_args` the plugin and `--allowedTools` flags, for a [+] session (`hand::extras`); `supraman-app` `supervise::launch_config` builds the app's `LaunchConfig` for both.
- `supraman_supervisor::DesiredState::update`: every change to a workspace's wanted sessions (the app and `sm want`); `DesiredState::release` stops the sessions of folders taken away (stop, never rm).

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
