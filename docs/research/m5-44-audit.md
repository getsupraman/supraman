# m5/44 audit: findings and proposed refactor units

Unit m5/44.1, 2026-10-28, worktree `m5-44-1` at `98df085`. This was read-only: nothing was built, run or changed under `crates/`, `plugins/`, `scripts/` or `Cargo.*`.

**Method.** Five parallel read-only reviewers each took one crate group and were briefed on C56, C57, the protected list and the categories:
- foreman-app UI, plus the c8 inventory;
- foreman-app core and the selftests, plus F-06, the process inventory and the SecretStore;
- foreman-term and foreman-ui;
- foreman-claude, foreman-supervisor and foreman-core;
- fm, foreman-workstate and plugins, plus F-05.

I ran the cross-crate duplicate pass myself, merged the overlapping reports, and re-checked the highest-risk claims against the code: A-26, A-27, A-28, A-29 and A-30.

**Columns.**
- **Risk** is the risk of the fix: what could break.
- **Prot.** flags protected business logic from m5/44's intent:
  - WS: the Workman contract
  - OAuth: m5/43
  - C15/17/21
  - CLI: peerProtocol, `--bg`, rm/resume, the daemon
  - Del: m5/38 deletion confinement
  - Mint: m5/35, m5/40, m5/48
  - Contrast: m5/26
- **Sev.** is the severity recorded in Workman. *blocker* is used only for a security hole, a UI freeze or data loss that is reachable in normal use. Everything else is *minor*.

## Summary

| Category | Findings | Highest risk |
|---|---|---|
| 1 Duplicates | A-01 … A-19 (19) | med: the claude runner and the origin parsers touch CLI/Mint |
| 2 Hidden hand-rolled components | A-20 … A-25 (6) | low–med (c8) |
| 3 Weak or fragile code | A-26 … A-52 (27) | **3 blockers**: A-26 OSC 8 click opens any URI; A-27 a UI freeze during an OAuth refresh; A-28 a damaged `servers.json` wiped on write |
| 4 Dead code | A-53 … A-57 (5) | low; A-53 needs a decision |
| 5 Test gaps on risky paths | A-58 … A-62 (5) | — |
| 6 Regressions / contradictions | A-63 … A-66 (4) | low; A-66 needs a decision |
| 7 Comments (C57) | A-67 … A-76 (10: 1 cross-cutting, 9 per crate) | none; about 670 comments to delete |
| 8 Known flakes | A-77 F-05, A-78 F-06a, A-79 F-06b | — |
| Structure | A-80 terminal.rs god-file, A-81 FleetView hub | med |

There are **81 findings** and **16 proposed units** (U1–U15, with U6 split into U6a/U6b), in 10 waves.

The findings are recorded in Workman against `m5/44`, source `audit m5/44.1`. A-26, A-27 and A-28 are F-07, F-08 and F-09 (blockers). A-01 … A-25 are F-10 … F-34. A-29 … A-81 are F-35 … F-87.

**Decisions the controller or René must take before the units start:**
1. **A-53:** is re-syncing the Zed terminal copy still a goal (LEARNINGS:415)? If not, about 1,000 lines of product code and about 1,000 lines of tests go.
2. **A-66:** should every tab, `claude` included, keep `TERM_PROGRAM=zed` / `ZED_TERM=true`?
3. **A-77 step 3:** should a production OAuth change be allowed (re-read the store once before declaring `invalid_grant`)? It is only needed if the F-05 diagnostics show a stale credential read.
4. **A-55:** keep `foreman-app/diag/*.ps1` as LEARNINGS evidence, or retire them?
5. **A-26:** which URI schemes may a Ctrl+click open (proposal: http, https, mailto; ask first for `file:`)?

---

## Findings

### 1. Duplicates

**A-01 — Four `claude` process runners**
- Where:
  - `crates/foreman-claude/src/cli.rs:100-152` (`run` and `drain`)
  - `crates/foreman-claude/src/launch.rs:330-372` and `:448-456` (`Claude::exec` and `drain`)
  - `crates/foreman-claude/src/daemon.rs:65-80`
  - `crates/foreman-app/src/selftest.rs:147`
- What: the spawn, pipe-drain, 20 ms `try_wait` poll and kill-on-timeout code is the same, and `drain` is copied word for word. The same failure maps to `ClaudeError::Failed` (`error.rs:17`) in one copy and `CliFailed` (`error.rs:45`) in the other. `claude agents --json --all` exists twice, with a 10 s timeout at `cli.rs:48` and 60 s at `launch.rs:304`. Both runners kill only the direct child, not its tree.
- Fix: one `run(cmd, timeout) -> Output`, kept in the shared process helper (A-23). The claude functions become thin wrappers with one failure variant.
- Risk: med (error text matched by the app and tests).
- Prot.: CLI. The argv must stay identical.

**A-02 — About 12 hand-rolled atomic writes; the fixed temp names race**
- Where:
  - `crates/foreman-core/src/settings.rs:232-242`, `registry.rs:92-94`, `relay.rs:82-89, 129-132, 309-312`, `migrate.rs:65-67`
  - `crates/foreman-supervisor/src/desired.rs:249-264`, `mint.rs:278-288`
  - `crates/foreman-claude/src/human.rs:101-108`
  - `crates/foreman-workstate/src/auth.rs:182-190`
  - `crates/foreman-app/src/stars.rs:64-71`, `dismissed_messages.rs:80-88`, `work_view/inbox.rs:220-233`
- What: only `desired.rs` uses a per-process temp name and retries the rename on `PermissionDenied`, which matters because the app and `fm` write the same files. Every other site uses a fixed `*.tmp`, so two writers clobber each other's temp file. `human.rs:106` ignores the rename result. `auth.rs:186` writes `""` when serialisation fails. `inbox.rs:229` writes non-atomically with `let _ =`. `stars.rs` and `dismissed_messages.rs` are the same load-with-notice + save code twice.
- Fix: `foreman_core::fs::{write_atomic, load_json_or_notice}` with a pid-unique temp name and the rename retry; every site uses it.
- Risk: low.
- Prot.: no. Keep `desired.rs`'s semantics.

**A-03 — `Settings` has no `update()`; about 8 load-modify-save copies**
- Where: `foreman-app/src/view.rs:1392-1405, 2020-2023, 2646-2650`; `view/term.rs:146-160, 174-188`; `settings_window.rs:277-295`; `supervise.rs:171`.
- What: each copy repeats load → mutate → save with its own error text, so two that race lose one update. `Settings::load(..).unwrap_or_default().workstate_origin()` appears 3× (`view.rs:880, 940, 1017`) and duplicates `Auth::configured_origin` (`auth.rs:208`).
- Fix: `Settings::update(home, |s| …)` modelled on `DesiredState::update`, and use `auth.configured_origin()`.
- Risk: low.
- Prot.: no.

**A-04 — Three origin parsers that disagree**
- Where: `foreman-workstate/src/client.rs:39` (`api_base`), `auth.rs:592` (`split_origin`), `foreman-core/src/settings.rs:115` (`url_origin`).
- What: `url_origin` cuts at `?` and `#`; `api_base` does not. `servers.json` and the minted origins are keyed by `api_base`, while the settings origin comes from `url_origin`, so the two can disagree on a URL with a query.
- Fix: a single origin function in foreman-core; `api_base` and `split_origin` delegate to it. Pin it with a table test.
- Risk: low–med (origin strings are map keys).
- Prot.: Mint (m5/48 origin comparison), OAuth.

**A-05 — Workman HTTP helpers are copied**
- Where:
  - percent-encoding 3×: `foreman-supervisor/src/mint.rs:163-169`, `foreman-workstate/src/client.rs:251`, `auth.rs:606-610` (byte-identical);
  - `server_reason` 2×: `mint.rs:148`, `client.rs:243`;
  - status + body read 3×: `mint.rs:136`, `auth.rs:577`, `client.rs:207-225`;
  - a hand-rolled test HTTP server in `mint.rs:357` that duplicates `foreman-workstate/src/fake.rs`.
- Fix: put these next to the one agent in `foreman-workstate/src/http.rs`, export them, and have mint's tests use `FakeWorkman`.
- Risk: low.
- Prot.: WS (request bytes must be unchanged).

**A-06 — The home directory is looked up 4×, and the peer registry ignores `CLAUDE_CONFIG_DIR`**
- Where: `foreman-claude/src/peer.rs:66-71` against `transcript.rs:299-307`; also `foreman-app/src/view.rs:1802` and `fm/src/probe.rs:227`.
- What: `sessions_dir()` builds `USERPROFILE/.claude/sessions` itself, while `claude_dir()` honours `CLAUDE_CONFIG_DIR`. With that variable set, Delete and transcripts look in one directory while direct send, ★ and `proc_tree` look in another.
- Fix: `foreman_core::paths::{user_home, claude_dir}`, and `sessions_dir = claude_dir()?.join("sessions")`. Add a test with the variable set.
- Risk: low–med (only changes anything when the variable is set).
- Prot.: CLI (peerProtocol registry).

**A-07 — The MCP config path is built in 3 places**
- Where: `foreman-claude/src/launch.rs:183, 192`; `foreman-supervisor/src/executor.rs:525`.
- What: revoke deletes a path it rebuilds, without the name check that `write_mcp_config` applies.
- Fix: `pub fn mcp_config_path(dir, agent) -> io::Result<PathBuf>`.
- Risk: low.
- Prot.: Mint (token file route).

**A-08 — `.git` parsing and path normalisation exist 3×**
- Where:
  - `gitdir:` at `foreman-claude/src/assign.rs:79-84` and `foreman-core/src/scan.rs:48-52` (`mint.rs:56-76` runs `git` instead);
  - path folding at `assign.rs:94-115`, `foreman-core/src/discover.rs:42-45` (no `\\?\` strip, no `..` fold) and `foreman-app/src/tab_card.rs:143`.
- Fix: `foreman_core::paths::{normalize, main_checkout}`.
- Risk: low.
- Prot.: no.

**A-09 — Time and id helpers are copied**
- Where:
  - `now_ms` at `foreman-claude/src/session.rs:132` and `foreman-core/src/relay.rs:65`, inline at `fm/src/hook.rs:180`, `epoch_ms` at `transcript.rs:289`, `modified_ms` at `relay.rs:167`;
  - `hex6` at `fm/src/probe.rs:545` and `foreman-app/src/selftest.rs:126`;
  - test prefixes at `fm/tests/work.rs:42`.
- Fix: `foreman_core::time` and one `hex6`.
- Risk: low.
- Prot.: no.

**A-10 — Discard-token parsing 3×, and `claude rm` spawned by hand**
- Where: `foreman-app/src/selftest_supervise.rs:266` and `fm/src/probe.rs:403`, against `foreman-claude/src/launch.rs:402` (`RmRefusal::parse`). The callers spawn `claude rm … --discard-unpushed` themselves (`selftest_supervise.rs:243`, `probe.rs:316`).
- Fix: use `RmRefusal::parse(..).discard` and `Claude::remove` (`launch.rs:289`).
- Risk: low.
- Prot.: CLI, Del. The behaviour stays identical.

**A-11 — `git -C` 2×, shell-open 3×**
- Where:
  - `git -C`: `foreman-supervisor/src/mint.rs:66`, and `fm/src/probe.rs:377` (no `no_window`);
  - `ShellExecuteW`: `foreman-app/src/signin.rs:61` and `elevate.rs:69`;
  - `explorer.exe` at `view/manage.rs:500`, while `cx.open_with_system` is already used at `settings_window.rs:273, 312`.
- Fix: `git(dir, args)` and `shell_execute(verb, target, args)` in the process helper (A-23); use the gpui opener where it fits.
- Risk: low.
- Prot.: no.

**A-12 — Two Win32 process-liveness checks**
- Where: `foreman-app/src/view.rs:2521-2541` (`process_alive`) against `foreman-claude/src/peer.rs:137-150` (`process_start`).
- What: off Windows the copy returns `false` while peer checks `/proc`. `elevate.rs:99`, a non-UI module, reaches into `view` for it.
- Fix: `foreman_claude::peer::alive(pid)`.
- Risk: low.
- Prot.: CLI (liveness).

**A-13 — Clipboard paste written twice, in two crates**
- Where: `foreman-app/src/view/term.rs:109-114` (Ctrl+V) and `foreman-ui/src/term_element.rs:728-733` (right-click).
- What: each decides on its own what an image-only clipboard does; only Ctrl+V falls back to `^V`.
- Fix: `Terminal::paste_clipboard(cx) -> bool` in foreman-term.
- Risk: low.
- Prot.: no (keep the m5/46 Alt+V behaviour).

**A-14 — Two copies of "rotate at cap, then append"**
- Where: `foreman-ui/src/lib.rs:80-89` (`append_capped`, which swallows errors at `:84, :87`) and `foreman-core/src/events.rs:79-89`.
- Fix: one helper in `foreman_core::fs` that returns `io::Result`.
- Risk: low.
- Prot.: no.

**A-15 — The 530 ms caret blink exists twice**
- Where: `foreman-app/src/text_input.rs:26-27, 665-673` against `foreman-term/src/terminal_settings.rs:25` and `terminal.rs:3479-3485` (`blink_phase`).
- Fix: `text_input` uses `CURSOR_BLINK_INTERVAL` and `blink_phase`.
- Risk: none.
- Prot.: no.

**A-16 — A test-only copy of the contrast maths, with weaker floors**
- Where: `foreman-app/src/view/left.rs:1734-1757`, a copy of `foreman-ui/src/themes.rs:350-364`.
- What: it asserts 4.5 and 3 against One Dark. `themes.rs:731-760` already asserts the real floors (7 and 4.5) for both bundled themes.
- Fix: delete it, or call `themes::contrast`.
- Risk: low.
- Prot.: Contrast. Keep the `themes.rs` assertions unchanged.

**A-17 — Small app-level copies**
- Where:
  - Esc closes the menu, then the confirm: 3× (`view.rs:2107-2111, 2135-2140`; `view/manage.rs:198-202`);
  - the pwsh `Shell::WithArguments`: 5× (`view.rs:1323, 1427, 1531, 1780, 1789`);
  - the 17-field `Tab {…}` literal: 2× (`view.rs:1710-1728, 1824-1842`);
  - "name, else short id": 6× (`view.rs:1416, 1668`; `manage.rs:304`; `delete.rs:59-62`; `item.rs:496`; `left.rs:1354`);
  - debounce-then-read: 4× (`work_view.rs:367-394`, `item.rs:160-173`, `journal.rs:144-158`, `inbox.rs:261-267`);
  - `cycled` (`view.rs:75-81`) against `find_step` (`term.rs:92-98`);
  - the font literals "Segoe UI" 4× and "Cascadia Mono" 3×;
  - the chevron drawn both as an `Icon` and as "▾▸" text (`work_view.rs:598`, `item.rs:636`).
- Fix: `dismiss_popup()`, `pwsh(args)`, `Tab::new`, `Session::display_name()`, a `Debounced` helper, and foreman-ui font constants.
- Risk: low.
- Prot.: no.

**A-18 — Selftest scaffolding copied**
- Where:
  - `async fn until(view, cx, what, f)` is declared 10×, each with its own `WITHIN`: `selftest_closed.rs:77`, `selftest_delete.rs:47`, `selftest_first_run.rs:28`, `selftest_left.rs:22`, `selftest_manage.rs:40`, `selftest_mark.rs:22`, `selftest_messages.rs:27`, `selftest_notices.rs:24`, `selftest_refresh.rs:33`, `selftest_tabs.rs:149`;
  - 3 variants: `selftest_title.rs:131`, `selftest_term.rs:62`, `selftest_supervise.rs:280`;
  - the clipboard read lambdas 3×: `selftest.rs:980`, `selftest_mark.rs:160`, `selftest_term.rs:34-40`.
- Fix: one `selftest::until(view, cx, within, what, f)` and one clipboard helper.
- Risk: low.
- Prot.: no.

**A-19 — Settings help text copies contract numbers by hand**
- Where: `foreman-app/src/settings_window.rs:476` ("every 30 s" = `workstate_setup::RECHECK_EVERY`) and `:708` ("6 to 72, default 14" = `foreman-core/src/settings.rs:13-16`).
- Fix: `format!` from the constants.
- Risk: none.
- Prot.: no.

*Not a finding:* the font bounds in `foreman-ui/src/term_element.rs:23-25` deliberately duplicate core's; `the_settings_bounds_are_the_painters` pins them (m5/41).

### 2. Hidden hand-rolled components (C56)

**A-20 — At least 7 private button builders**
- Where:
  - `foreman-app/src/view.rs:2212` (`button`), `:2278-2302` (tab close), `:2314-2333` (new shell);
  - `view/left.rs:131, 136, 186, 208, 230` (`icon_button`, `tinted_button`, `glyph_button`, `star_button`, `pill`);
  - `settings_window.rs:805` (`small_button`), `:822` (`choice`);
  - `titlebar.rs:44`;
  - `work_view.rs:734-754`, `work_view/item.rs:207-221, 501-520`, `work_view/journal.rs:270-288`.
- What: all are `div().id().border().hover().active()` variations on `cx.theme()`.
- Fix: a `foreman_ui::button` module (bordered, ghost-icon, pill, toggle/choice) with a test-hook key.
- Risk: low. The selftest hit keys must stay the same.
- Prot.: no.

**A-21 — The banner/notice box is built about 7×**
- Where: `view/left.rs:425-436, 470-485, 957-976, 1497-1511`; `settings_window.rs:544-554, 910-921`; `work_view/item.rs:281-293`.
- Fix: `foreman_ui::banner(severity, body, dismiss, actions)`.
- Risk: low.
- Prot.: Contrast. Keep the 0.12 tints.

**A-22 — The c8 generic UI pieces live in the binary crate**
- What: `text_input`, the context menu, the confirm dialog, toast and notices, the tab strip and hover card, the split pane, and the matcher with its results list.
- Fix: see the c8 inventory below.
- Risk: med (wide).
- Prot.: Del (the confirm dialog carries the deletion questions).

**A-23 — There is no generic process helper**
- What: `foreman_claude::no_window` (`cli.rs:90`) is the only flags helper, so foreman-supervisor and the app depend on the claude crate to spawn `git` or `ping`. foreman-term gets the same flag from Zed's `util::command::new_std_command` (`terminal.rs:3240-3247`). `fm/src/probe.rs:316, 378` and `probe_hooks.rs:253` spawn without `no_window`.
- Fix: a `foreman_core::process` module with `command`, `run(cmd, timeout)` (kill on timeout; job/tree optional), `git`, and `shell_execute`.
- Risk: med.
- Prot.: CLI (see the c8 inventory).

**A-24 — `signin.rs` parses `Auth`'s `servers.json` itself**
- Where: `foreman-app/src/signin.rs:46-51` (the file belongs to `auth.rs:100, 175`).
- What: stringly-typed JSON; a format change silently breaks the selftest's credential cleanup.
- Fix: `Auth::issuer_of(origin)`.
- Risk: low.
- Prot.: OAuth (selftest cleanup only).

**A-25 — Settings is one flat struct that drops unknown keys; there is no page registration**
- Where: `foreman-core/src/settings.rs:21-63, 222-243`; `foreman-app/src/settings_window.rs:23-46` (`Page` enum).
- What: `save` re-serialises the whole struct, so any table written by another page or a newer version is erased. Core owns UI constants such as fonts and themes.
- Fix: see the c8 inventory (namespaced sections).
- Risk: med (file format).
- Prot.: partly. `max_working` feeds the m5/45 budget and `workstate_url` feeds Mint.

*Shared already, not findings:* one HTTP stack (`foreman_workstate::http::agent`, ureq and native-tls; the SSE raw socket is justified at LEARNINGS:1358) and one secret store (`secret.rs`).

### 3. Weak or fragile code

**A-26 — A plain click on an OSC 8 link opens any URI** *(blocker, verified)*
- Where: `crates/foreman-term/src/terminal.rs:2855-2865` (`cx.open_url(link.uri())` at `:2864`).
- What: any program, or a `cat` of a hostile file, can print `ESC]8;;file:///…/x.exe` over ordinary text. One left click, with no modifier and no scheme check, then hands that URI to the shell's open action. Windows Terminal needs Ctrl+click.
- Fix: require `e.modifiers.secondary()` and an allowed scheme (decision 5).
- Risk: med (link clicks change; no selftest clicks links).
- Prot.: no.

**A-27 — A UI freeze of up to 15 s plus a lock wait during any OAuth refresh** *(blocker, verified)*
- Where: `foreman-workstate/src/auth.rs:235-288`. The `inner` guard is taken at `:235` and is still alive across `refresh_lock()` (`:251`) and `token_request` (`:256`). `status()` takes the same mutex (`:215`).
- What: the UI thread calls `status()` at `settings_window.rs:196` (`reload`, triggered by the fleet observer at `:119`), `view.rs:968` and `work_view.rs:208`. Any background refresh blocks the window: `workstate_setup::detect` every 30 s, a supervisor tick, a hand mint, or `fm` holding `refresh.lock`.
- Fix:
  - Release `inner` before `refresh_lock()` and the request, then take it again to set `held` and `broken`. Keep the file lock around the store read, the refresh and the store write.
  - Also compute `AuthStatus` on the background executor in the app.
- Risk: med (refresh races).
- Prot.: OAuth. The token semantics must not change.

**A-28 — An unreadable `servers.json` is treated as empty and then written back** *(blocker, verified)*
- Where: `foreman-workstate/src/auth.rs:178-180` (`load_known`: `.ok()…unwrap_or_default()`). It is saved back at `:278-280` (`invalid_grant`) and `:324-326` (`sign_in`).
- What: a read or parse error erases every other origin's metadata and client_id. `status()` reports a damaged file as "Not signed in".
- Fix: return the default only on NotFound, otherwise `AuthError::Store`, and never save over a file that could not be read.
- Risk: low.
- Prot.: OAuth.

**A-29 — Quit blocks the UI thread on HTTP revokes** *(verified)*
- Where: `foreman-app/src/view.rs:818-821`. `on_app_quit` calls `Hands::release_all()` synchronously, with a 15 s agent per call (`mint.rs:131`).
- What: N hand tabs and an unreachable server mean a quit that hangs N×15 s.
- Fix: return a background task with a total deadline, or use the 4 s revoke agent.
- Risk: low.
- Prot.: Mint.

**A-30 — A failed save of the minted-token record is swallowed** *(verified)*
- Where: `foreman-app/src/hand.rs:256, 296` (`let _ = self.minted.save()`).
- What: a lost `hands.json` record means the token is never revoked at the next start, and the user is not told.
- Fix: surface the error in `Launch::Minted{notice}` and the release notice.
- Risk: low.
- Prot.: Mint.

**A-31 — `daemon_status` leaks a thread and a child on timeout**
- Where: `foreman-claude/src/daemon.rs:65-80`, polled every 15 s while unelevated (`supervise.rs:207`).
- What: a hung CLI piles up processes.
- Fix: the shared runner (A-01).
- Risk: low.
- Prot.: CLI (daemon pipe, m5/25).

**A-32 — The selftest `claude()` has no timeout and runs in cleanup and the watchdog**
- Where: `foreman-app/src/selftest.rs:147`, used at `:361`, at `selftest_supervise.rs:225, 236, 243`, from the watchdog at `:281`, and from the panic hook.
- What: a hung `claude stop` or `rm` keeps the SCRATCH lock forever, and `exit(1)` never runs.
- Fix: `Claude::stop` / `Claude::remove`.
- Risk: low.
- Prot.: CLI.

**A-33 — A failed Stop is swallowed every tick**
- Where: `foreman-supervisor/src/executor.rs:324-327` (`let _ = self.launcher.stop(&id)`).
- What: the workspace looks stopped while its sessions run.
- Fix: a NeedsYou row or notice after N consecutive failures.
- Risk: low.
- Prot.: CLI (R5/R14; Stop never becomes rm).

**A-34 — A Re-mint press is lost when its stop fails**
- Where: `foreman-supervisor/src/executor.rs:539-550` (with `:228-231`).
- What: `mem::take(remint_due)` drops those names, and `server_changed` will not schedule them again because `checked` is set.
- Fix: put a name back unless its stop succeeded or no row exists.
- Risk: low.
- Prot.: Mint (m5/48).

**A-35 — Resume copy detection parses CLI text and fails open**
- Where: `foreman-supervisor/src/executor.rs:672-682` (`woke_in_place` splits stdout on `"backgrounded · "`).
- What: if claude changes that text, a forked copy counts as a successful start. `launch.rs:259-260` says never to resolve a session from this text.
- Fix: treat unrecognised stdout as unknown (a notice), or confirm from the next snapshot's row id. Add a test.
- Risk: med.
- Prot.: CLI (resume, m2/4).

**A-36 — `DesiredState::update` is check-then-write and is called from three writers**
- Where: `foreman-supervisor/src/desired.rs:287-290`.
- What: the writers are the UI thread (`view.rs:1135, 1153`), the poll thread (`supervise.rs:157`, `sync_relay`) and `fm want`. There is a window between the re-read and the rename, and after 10 tries it overwrites anyway, so a lost `started=true` or relay change is possible.
- Fix: a lock file around read-edit-save, and Start/Stop off the UI thread.
- Risk: med.
- Prot.: no. Keep the supervisor semantics.

**A-37 — Flags and conditions are polled in timer loops on the foreground executor**
- Where:
  - `foreman-app/src/view.rs:586-592` (50 ms, for the life of the app), `:735` and `:864` (250 ms);
  - `view/left.rs:373-387` (200 ms);
  - `view.rs:1330-1353` (`type_in_shell_when`: `get_content()` of the whole terminal every 50 ms, for up to 15 s);
  - `view.rs:1496-1516` (500 ms per hand tab, for the tab's life);
  - `view.rs:1960-1982` (20 ms).
- What: an event already announces each of these: `TerminalEvent::Wakeup`/`CloseTerminal`, or a deadline.
- Fix: channels instead of `AtomicBool`s; sleep until the deadline; drive the shell-prompt wait and the hand release from the terminal subscription (`view.rs:1871`). Do **not** adopt Zed's marker handshake (`terminal.rs:2290-2306`): it types a visible command.
- Risk: med (refresh, detach and hand-release timing).
- Prot.: CLI (attach detach), Mint (hand release).

**A-38 — The WorkHub stream blocks a background executor thread forever**
- Where: `foreman-app/src/work_view.rs:160-177` (a std `mpsc::recv_timeout` loop).
- Fix: `Stream::subscribe` returns an async receiver.
- Risk: med.
- Prot.: WS (delivery semantics unchanged).

**A-39 — Opening the context menu does registry IO and opens every pid on the UI thread**
- Where: `foreman-app/src/view/manage.rs:306` (`send_name_of`, `:92-96`, calls `peer::live_entries`).
- Fix: resolve it asynchronously, or reuse the `tab_sessions` snapshot.
- Risk: low.
- Prot.: no.

**A-40 — The selection text is rebuilt every frame under the terminal lock, and only selftests read it**
- Where: `foreman-term/src/alacritty.rs:937-941` (via `sync`, `terminal.rs:2500`). It is read at `selftest.rs:957` and `selftest_term.rs:96, 239`.
- Fix: an on-demand `Terminal::selection_text()`.
- Risk: low.
- Prot.: no.

**A-41 — Every output wakeup starts a sysinfo refresh of the shell**
- Where: `foreman-term/src/terminal.rs:1673-1675` → `pty_info.rs:190-229`.
- What: on Windows the pid is always the shell, so the result rarely changes, yet the work runs back to back while output streams.
- Fix: throttle (at most 1/s, plus on Enter). `BreadcrumbsChanged` already resolves tabs (`view.rs:1883`).
- Risk: low–med (tab→claude resolution must still fire).
- Prot.: no.

**A-42 — Theme files are read from disk on the UI thread at every Appearance change**
- Where: `foreman-ui/src/themes.rs:560-582` (from `:547, 594, 622`; `settings_window.rs:81, 195`).
- Fix: cache the catalog, loaded off the UI thread.
- Risk: low.
- Prot.: no (`fit_floors` unchanged).

**A-43 — Event logs and relay files are never pruned, and are re-read on every poll**
- Where: `foreman-core/src/events.rs:141, 153-164`; `relay.rs` (inbox/sent, rejected, `outbox/*.json`, `answered.json`); `read_outbox` (`relay.rs:177`, every 2 s from `supervise.rs:418`).
- What: `EventReader` re-opens every session log on every poll, re-reads from 0 after a start, and keeps cursors for every session id ever seen.
- Fix: age-based pruning in one place; drop cursors for missing files.
- Risk: low.
- Prot.: no.

**A-44 — In-memory structures that only grow**
- Where:
  - `foreman-app/src/clipped.rs:11-14, 51-58` (labels that vanish while cut stay forever, and a re-shown label gets a stale tooltip);
  - `work_view/journal.rs:57-64` (`prepend` is uncapped while `append` caps at 2000, and the whole list renders every frame);
  - `text_input.rs:326-334, 377-393` (undo snapshots);
  - `foreman-ui/src/term_element.rs:540-547` (`WIDTH_PER_PX` per FontId);
  - `foreman-term/src/terminal.rs:3001-3004` (`cwd_history`);
  - `foreman-claude/src/transcript.rs:330, 431` (`listed`) and `foreman-supervisor/src/desired.rs:21` (`retired`).
- Fix: clear per frame (as `HitTargets` does, `view.rs:2421`), cap `prepend` and the undo stack (about 100), clear the glyph map when the font changes, prune by scrollback.
- Risk: low.
- Prot.: no.

**A-45 — The SSE line buffer is unbounded, and the stream skips the API header check**
- Where: `foreman-workstate/src/stream.rs:427` (`self.line.push(b)`; the head is capped at 64 KiB at `:309`, a line is not).
- What: a 200 without `X-WorkState-Api: 1` is accepted, which `client.rs:229-232` rejects.
- Fix: cap a line at 64 KiB, then `Lost`; check the header in `HttpReader::feed`.
- Risk: low.
- Prot.: WS (read API v1).

**A-46 — One corrupt workspace file stops every supervisor**
- Where: `foreman-core/src/registry.rs:78` (`list()` fails on the first bad `.toml`) → `foreman-app/src/view.rs:451-453`.
- Fix: return the good workspaces plus per-file errors as notices.
- Risk: low.
- Prot.: no.

**A-47 — One bad older settings key fails the whole file, and the supervisor silently uses defaults**
- Where: `foreman-core/src/settings.rs:22-63` (only `terminal_*` is lenient; the test at `:282-283` asserts this) → `supervise.rs:318` uses defaults and `view.rs:1397` skips saves.
- Fix: lenient per field or per section, and report the bad key.
- Risk: low.
- Prot.: partly (`max_working` → budget).

**A-48 — Small stringly-typed protocol checks**
- Where:
  - `foreman-claude/src/launch.rs:395-398`: `RmRefusal::parse` matches any text containing "kept";
  - `launch.rs:423-428`: the stderr+stdout join is built twice;
  - `transcript.rs:112-113`: byte matching on `"type":"user"` assumes compact JSON;
  - `foreman-supervisor/src/budget.rs:116`: the relay is found by a `-relay` suffix, while reconcile uses `role == relay` (`desired.rs:53`);
  - the `"fm-"` prefix is hardcoded at `session.rs:152`, `budget.rs:116`, `desired.rs:72, 76, 82, 105` and `migrate.rs:84`;
  - `foreman-workstate/src/health.rs:17-27` classifies errors by "refused", "10061" and "10060".
- Fix: anchor "kept"; join once; `is_relay()`; an `FM_PREFIX` constant; match on `io::ErrorKind`.
- Risk: low.
- Prot.: CLI (rm text), the budget.

**A-49 — Ctrl+click on a plain-text URL swallows the click and does nothing**
- Where: `foreman-term/src/terminal.rs:2737-2746` (press suppressed) and `:1902-1904` (`Event::Open` emitted); `foreman-app/src/view.rs:1893` (`_ => {}`, so `Open` and `NewNavigationTarget` are never handled).
- What: the regex search runs on every mouse move while Ctrl is held (`:2603-2650`), and its result is never used.
- Fix: handle `Event::Open` in `view.rs` with the same scheme check as A-26; otherwise remove the regex path.
- Risk: low.
- Prot.: no.

**A-50 — Ctrl+wheel zoom steps once per event, whatever the delta**
- Where: `foreman-app/src/view.rs:2362-2380`.
- What: a precision touchpad zooms in large jumps. Wheel handling lives in the app, while every other terminal mouse handler is in `term_element.rs:717-772`.
- Fix: accumulate the delta, and move the handler next to the others.
- Risk: low.
- Prot.: no.

**A-51 — The Settings description renders a run of 14 spaces**
- Where: `foreman-app/src/settings_window.rs:756` (missing `\` continuations; its content is also stale, see A-63).
- Fix: add the continuations and rewrite the text.
- Risk: none.
- Prot.: no.

**A-52 — A slow `--bg` launch stalls the poll thread**
- What: the tick runs on the poll thread, and each launch can wait up to `WRITE_TIMEOUT` (60 s) plus an extra `claude agents` call before every `--bg`. The session list and event polling stall, and later launches act on an old snapshot. R6 and R13 cover the duplicates.
- Fix: no change now; record it in ARCHITECTURE and consider launching off the poll thread later.
- Risk: med.
- Prot.: CLI.

### 4. Dead code

**A-53 — About 1,000 lines of the Zed terminal port are dead in the product (plus about 1,000 lines of tests)**
- Where, all in `foreman-term/src/terminal.rs` (each checked with a repo-wide grep; tests-only, or unreachable given `view.rs:1849-1864`):
  - Headless mode: 86-101, 1109-1112, 1218-1254, 3210-3329.
  - Display-only terminals: 978-1089, 1966-2037, 3198-3208.
  - ANSI text parsing: 192-326.
  - The `actions!` block: 622-658.
  - The init-command marker: 907-944, 2217-2315.
  - Vi mode, which cannot be entered: 113-131, 2354-2450.
  - Foreground command: 2963-2973, 3342-3411.
  - Single-use helpers: 1961, 2049, 2074, 2115, 2517, 2528, 2533-2543, 2652, 3086, 3171.
  - Remote branches: 2319, 2953, 2993, 3023.
  - Activation-script clearing: 1378-1392.
  - `copy_on_select`, always false: 2850.
  - Events nobody reads: 680-691.
- Fix: delete them with their tests (**decision 1**). The copy already diverges a lot from Zed.
- Risk: low.
- Prot.: no.

**A-54 — Unused functions**
- Where (each grep-verified):
  - `foreman-claude/src/transcript.rs:351` (`Transcripts::projects`);
  - `foreman-core/src/discover.rs:61` (`Registry::update`, own test only);
  - `foreman-claude/src/assign.rs:36` (`repo_of`, tests only);
  - `launch.rs:49, 132, 170` (`with_session_token`, `SessionIdentity::headers`, `write_session_mcp_config`: the retired static-token route, called only from `fm/src/probe.rs:89`);
  - `foreman-app/src/stars.rs:41` (`Stars::contains`, tests only);
  - `tab_card.rs:131-133` (`branch_at`, a pass-through);
  - the public `API_HEADER` re-export in `foreman-workstate/src/lib.rs`.
- Fix: delete them; for the token route, delete it together with `probe launch` (A-55).
- Risk: low.
- Prot.: no.

**A-55 — Stale probes and diagnostics**
- Where:
  - `fm/src/probe.rs:15` (`WORKSTATE_URL = http://127.0.0.1:5290/mcp`, the local stack, removed 2026-10-28) and `:433-445` (`probe launch`, which assumes the pre-OAuth static token);
  - `crates/fm/probe/resume.ps1` (`fm probe session-id` replaces it);
  - `foreman-app/diag/*.ps1` (8 m1/7–m3/3 probes, each with its own launch/poll/stop/rm; referenced only from LEARNINGS 552, 776, 882, 940, 1007).
- What: LEARNINGS:884 cites `diag/timeline.py`, which does not exist. The other `fm probe` subcommands (`hooks`, `isolation`, `rm-transcript`, `session-id`) are reachable and are the CLI drift checks, so they stay.
- Fix: delete `probe launch` and `resume.ps1`; decide on `diag/` (**decision 4**); optionally put the probes behind a cargo feature, because `fm.exe` runs on every hook.
- Risk: low.
- Prot.: partly (CLI probes).

**A-56 — Compatibility branches for sessions from before m2x/2 and m5/4**
- Where: `fm/src/hook.rs:27-28` (`FOREMAN_PLATFORM`, role `platform`) and `fm/src/relay.rs:103-115` (no `FOREMAN_SESSION`). The executor passes `FOREMAN_SESSION` (`executor.rs:64`).
- Fix: delete both branches.
- Risk: low.
- Prot.: no.

**A-57 — foreman-term tests that never run, and three URL crates**
- What: many `terminal.rs` tests are `#[cfg(not(target_os = "windows"))]` (3629, 3695, 3757, 4386 …) and never run on the only target. `foreman-term/Cargo.toml:29, 34, 35` pulls in `percent-encoding`, `url` and `urlencoding`; `terminal.rs:1885` and `hyperlinks.rs:228` each decode with a different one.
- Fix: prune the tests in the A-80 move; keep one crate.
- Risk: low.
- Prot.: no.

### 5. Test gaps on risky paths

**A-58 — Deletion**
- What: nothing covers an I/O failure partway through (the transcript is removed, then `remove_dir_all` fails), and `find_transcript` has no unit test.
- Where: `foreman-claude/src/forget.rs`.
- Fix: add both tests.
- Prot.: Del.

**A-59 — Minting and hands**
- What: no test covers:
  - Re-mint when the stop fails (A-34);
  - `server_changed` when the stop fails;
  - the mint succeeding but the `hands.json` save failing (A-30);
  - the quit-time `release_all` (A-29);
  - the tab-restart selection after a server change (`view.rs:983-1009`).
- Fix: add the tests; extract a pure `stale_tabs(tabs, elsewhere)` for the last one.
- Prot.: Mint.

**A-60 — Supervisor budget and resume**
- What: no test covers:
  - `starting()` on a stopped workspace, or with `held` set;
  - `woke_in_place` with unrecognised stdout (A-35);
  - a trust refusal on resume;
  - a row whose cwd is gone;
  - `sessions_dir` with `CLAUDE_CONFIG_DIR` set (A-06);
  - the `human.rs` ledger with two writers.
- Prot.: CLI, budget.

**A-61 — OAuth**
- What: no test covers a damaged `servers.json` (A-28), or `status()` while a refresh is in flight (A-27). The fake's ids are guessable counters (`foreman-workstate/src/fake.rs:307-311, 364`), and its refusals do not say whether a token was "redeemed" or "unknown" (`:420-427`).
- Fix: add the tests; randomise the fake ids per fake; add an `error_description` (fake only, so C21 allows it).
- Prot.: OAuth (test-only).

**A-62 — Clipboard**
- What: there are no unit tests for paste (`terminal.rs:2482-2490`), the `Copy` event (`:1804-1814`) or the image-only `^V` fallback (`view/term.rs:110-113`, m5/46). Only the real-clipboard selftests 6b, 6c, 7l and 7r cover them, and those are slow and share the user's clipboard. A `TestAppContext` clipboard pattern already exists (the test at `terminal.rs:5209`).
- Fix: add `#[gpui::test]`s:
  - bracketed paste, with inner ESC removed;
  - `a\r\nb\n` → `a\rb\r`;
  - copy trims trailing blanks;
  - an image-only clipboard makes `terminal_key` return false.
- Prot.: no.

### 6. Regressions and contradictions with LEARNINGS

**A-63 — The docs say rlarch must be in `extra_plugin_dirs`; LEARNINGS:1330-1335 decided it need not be**
- Where: `plugins/foreman/README.md:19-20`, `plugins/foreman/skills/repo-controller/SKILL.md:41-42` (the BLOCKED text), and `foreman-app/src/settings_window.rs:756`.
- What: rlarch is installed user-scope, so sessions load it with no `extra_plugin_dirs`.
- Fix: rewrite those lines.
- Risk: low.
- Prot.: no.

**A-64 — Plugin text out of date**
- Where: the repo-controller's "Known gap" (`SKILL.md:64-70`, and step 3 at `:49-56`, which uses `Start-NestedAgent.ps1`); `plugins/foreman/README.md:10`.
- What: the gap predates rlarch's `agent_signin`/`agent_launch` (LEARNINGS:1337 queued the switch). The README's environment list leaves out `FOREMAN_SESSION`.
- Fix: check the gap against the current `rlarch:nested-claude` and update it; add the variable.
- Risk: low.
- Prot.: Mint (text only).

**A-65 — Comments that contradict their code (C57: delete them; none points at broken code)**
- Where:
  - `foreman-supervisor/src/mint.rs:1-4, 111-112, 233` ("static role token"; the bearer has been the OAuth access token since m5/43);
  - `executor.rs:61` (points at `with_session_token`, but the code calls `with_minted_token` at `:404`);
  - `foreman-app/src/view.rs:1873` (says claude's console title arrives as `TitleChanged`, but the OSC title arrives as `BreadcrumbsChanged`);
  - `view.rs:2085` (lists 2 inputs; the code checks 4);
  - `view.rs:2205-2207` (a doc on `held_line` that describes `button`);
  - `work_view/inbox.rs:356` (gives the wrong example string);
  - `foreman-term/src/terminal.rs:2933-2934` ("reset to 0", but the code does `%= height`);
  - `foreman-ui/src/themes.rs:201-202` (leaves out `fit_placeholder`);
  - `foreman-app/src/tray.rs:70` ("placeholder; m5/14 replaces this", but m5/14 shipped the icon);
  - `selftest.rs:1-4` (lists four modes, then says "Both run…").
- Fix: delete them in the comment passes.
- Risk: none.
- Prot.: no.

**A-66 — Every tab claims to be Zed**
- Where: `foreman-term/src/terminal.rs:667-676` (`insert_zed_terminal_env`: `TERM_PROGRAM=zed`, `ZED_TERM=true`, and `TERM_PROGRAM_VERSION` set to Foreman's version).
- What: LEARNINGS m5/46 lists these variables but never decides on them. `claude` may detect an IDE from them.
- Fix: **decision 2**. For example `TERM_PROGRAM=foreman`, then watch claude's IDE detection for drift.
- Risk: med.
- Prot.: next to CLI.

*Checked and not regressions:* `foreman_dark()` skips `fit_floors` (deliberate, m5/31); `TerminalBounds::default()` reports 6 lines (m4/2c); `line_edit.rs` named in LEARNINGS:2221 was replaced by `text_input.rs` (m5/30, m5/42). That last one is a stale LEARNINGS pointer for U15, not code.

### 7. Comments (C57)

These counts are estimates of comments to delete: unit-id narration, retired constructs, rules restated from other files, multi-line essays and stale text. The per-file detail is in the appendix.

| # | Crate | ≈ to delete | Worst examples |
|---|---|---|---|
| A-67 | foreman-app (UI files) | 180 | `view.rs` ~50; `view/left.rs:424` "unreadable in m1"; `settings_window.rs:30` "m5/13: no longer in the left panel"; `text_input.rs:1-8` essay, `:26` paraphrases the terminal constant; `view/delete.rs:1-12` essay |
| A-68 | foreman-app (core and selftests) | 150 | `hand.rs:1-28` (28-line essay through m5/20…m5/43), `:49` "as before m5/20"; `selftest.rs:487` "(m3/3: the tree once could not scroll)"; `supervise.rs:641` "written before m5/32"; `fleet.rs:149` |
| A-69 | foreman-term | 87 | `hyperlinks.rs:340-343` "This used to be: … I couldn't find it."; `mappings/keys.rs:270-273` "m5/9: …", `:109` commented-out code; `terminal.rs:716`, `:2057-2059` "//To test:", `:665-666` SSH remote; `pty_info.rs:128-132` Linux essay |
| A-70 | foreman-ui | 24 | `term_element.rs:1-6` "Kept: … Dropped: …"; `themes.rs:194, 305` "before m5/31"; `lib.rs:141-148` |
| A-71 | foreman-claude | 57 | `launch.rs:45-48` (the retired token route); `daemon.rs:1-6` essay; `forget.rs:1` "(m5/38, René 2026-10-28)"; `human.rs:1-12`; `transcript.rs:1-10` |
| A-72 | foreman-supervisor | 67 | `executor.rs` ~25; `lib.rs:54-62` history (the rule table is spec, so it stays); `desired.rs:191` "M2's desired state" |
| A-73 | foreman-core | 25 | `relay.rs:5-6` "Since m5/4 … Since m5/32 …"; `settings.rs:14` restates foreman-ui's bounds; `events.rs:1-3` |
| A-74 | foreman-workstate | 37 | `auth.rs:1-19` (19-line essay restating the contract, "(m5/43)"); `stream.rs:1-9` WHATWG essay; `model.rs:502` "m5/19:" |
| A-75 | fm | 33 | `work.rs:1-4` (restates `auth.rs`); `relay.rs:56-58` (restates the supervisor), `:103-104` "before m5/4"; `hook.rs:27`; the probes' "(m2/2)" tags |
| A-76 | plugins/foreman | 12 | `README.md:7, 11, 13, 15, 18` (m-unit tags); `workspace-controller/SKILL.md:174-175` (says the rule twice) |

In total, about 670 comments to delete.

### 8. Known flakes

**A-77 — F-05: `fm work_errors_are_one_line_without_the_token` intermittently fails with invalid_grant**
- Where: `crates/fm/tests/work.rs:167-179`, `foreman-workstate/src/auth.rs:251-281`, `fake.rs:307-311, 420-427`, and `secret.rs` (CredReadW/CredWriteW).
- The test is the only one that rotates one grant **twice, across two separate `fm` processes**.
  - It signs in once in-process. The refresh token goes to Credential Manager under a random prefix (`work.rs:42-59`).
  - Then three `fm` children run one after another against the same home.
  - Each child takes `refresh.lock`, reads the stored refresh token, refreshes and stores the rotated one.
  - The fake refuses any reuse with a bare `invalid_grant`.
  - The failure is always the **second** `fm` (LEARNINGS:2973-2975, 3029-3030).
- Ruled out by reading the code:
  - a real server (the settings name the fake's port);
  - a shared home or `%APPDATA%` (each `Home` uses a tempdir, and `APPDATA` is removed);
  - a shared credential (random prefix plus a per-port issuer);
  - `tests/oauth.rs` (it uses `MemoryStore` and is a separate binary);
  - a double refresh inside one process (only a 401 re-refreshes, and the fake sends none);
  - ureq retries (ureq 3.5 has none);
  - leftover data resetting the connection (the fake reads the whole request).
- Remaining hypotheses:
  - **(a) Stale credential read.** fm#2's `CredReadW` does not see fm#1's `CredWriteW`. This is the only fm→fm handover in the suite.
  - **(b) Someone else redeemed the token.** The fake's ids are guessable counters (`fake-rt-N`, `fake-client-1`). This is weaker; no such client was found in the repo.
- Fix, in order:
  1. Diagnostics, test-only. After each `fm` call, assert the store equals `oauth.issued().last()`, and on failure print the fake's `/connect/token` bodies. Have the fake answer `error_description: redeemed|unknown`.
  2. Random ids per fake. This rules out (b).
  3. Only if it is (a): re-read the store once before declaring `invalid_grant`, and retry if it now holds a different token. That is a production OAuth change (**decision 3**).
  - Do not hide the flake with a sign-in per call.
- Risk: low (1–2), med (3).
- Prot.: OAuth.

**A-78 — F-06a: selftest step 7l, "the row moves 48 px after Copy"**
- Where: `foreman-app/src/selftest_mark.rs:159-187`.
- Cause (hypothesis): the check compares the **absolute y** of `session-row-<sid>` in a tree that re-sorts itself live.
  - `fleet::sort` orders by health rank, then by `started_at` descending (`fleet.rs:189-207`).
  - The gate repo also shows the user's real AutoWork sessions (`selftest.rs:11-12`).
  - The tree is rebuilt every `REFRESH_EVERY` = 2 s (`view.rs:83`).
  - `before` is read once, ahead of up to 5 `CLIPBOARD_TRIES` rounds, and `after` comes after the toast plus 100 ms. That window spans polls.
  - A rank change of one row above, or of the mark row itself (Unknown → Idle, LEARNINGS m5/34), moves it by a multiple of `SESSION_LINE` = 24 px (`view/left.rs:173`); 48 px is two rows.
- Second contributor: the menu `.occlude()`s (`view/menu.rs:194`) and is drawn outside `left-panel` (`view.rs:2500`). Moving onto the menu item drops the panel's hover, so `release_left` runs (`left.rs:777-781`, `:390`). Inbox and Messages rows held by the m5/18 hold (`left.rs:28-33`) then appear above the tree.
- The check also cannot catch the regression it was written for: an in-flow toast shrinks `tree-area`; it does not move row tops.
- Fix: measure `hits["tree-area"]` (`left.rs:517`) before the click and after the toast has painted, and assert its bottom and height are unchanged. Drop the absolute row-y check, or retake unless the row key order and the inbox, message and notice keys are identical.
- Risk: low.
- Prot.: no.

**A-79 — F-06b: selftest step 7c, "window not found" on close_settings**
- Where: `foreman-app/src/selftest_left.rs:93-106, 125-153`; `tray.rs:175-191`; `selftest.rs:306-312`.
- **Hypothesis 1, timing:**
  - `until(v.settings_window)` returns once the deferred `open` has stored the handle (`view.rs:1380-1383`), before the window is shown, activated or painted.
  - `open_settings_at(About)` queues `activate_window` (`view.rs:1366-1373`), and the close follows at once.
  - `tray::close` hides the window, waits a fixed **100 ms** (an empirical value, LEARNINGS m5/22), then calls `remove_window`.
  - Under load, the new window's show, activate or `WM_MOUSELEAVE` messages arrive after `remove_window`, and gpui logs "window not found".
  - The WM_CLOSE path has the same race.
- **Hypothesis 2, attribution:** `teardown_errors()` scans the **whole** `foreman.log` since the run began, so any earlier "window not found" (steps 1–7b, or the main window) is blamed on 7c. Only "Invalid window handle" is filtered (`foreman-ui/src/lib.rs:74`).
- Fix:
  - Wait for `tray::is_shown` (`tray.rs:165`) and a painted frame before closing.
  - Record the log's length before each close and scan only what follows.
  - Replace the fixed 100 ms with "wait until `!IsWindowVisible`, then yield", or a capped `PeekMessage(hwnd, PM_NOREMOVE)` drain.
  - Keep m5/22's close discipline: hide outside the update, and `on_window_should_close` returns false.
- Risk: low.
- Prot.: no.

### Structure

**A-80 — `foreman-term/src/terminal.rs` is 6,480 lines**
- Fix: after A-53, split it by responsibility:
  - the root: mod, re-exports and `Event` (1-102, 678-711);
  - `content.rs` (103-191, 327-621);
  - `bounds.rs` (779-845, with tests 6194-6246);
  - `builder.rs` (847-1489);
  - `events.rs` (712-777, 1593-1956);
  - `io.rs` (2107-2113, 2188-2215, 2317-2333, 2453-2521);
  - `scroll.rs` (2119-2186, 2877-2942);
  - `mouse.rs` (2545-2876, 3413-3419);
  - `process.rs` (2952-3166, 3331-3338);
  - the colours into `mappings/colors.rs` (3424-3477, 3492-3500);
  - `windows_terminal.rs` (`blink_phase`, `trim_line_ends`, 3479-3490, with tests 6396-6480);
  - the tests into `terminal/tests/{selection, process, hyperlinks, perf, cwd}.rs`.
- Risk: med (a large diff, but a pure move).
- Prot.: no.

**A-81 — `FleetView` is the hub every component is tied to**
- What: `foreman-app/src/view.rs:200-357` holds about 100 fields in 2,650 lines. The `hits: HitTargets` test hook (`view.rs:112-129, 2190-2203`) is threaded into every widget.
- Fix: as the c8 units extract components, move `HitTargets` into foreman-ui as a generic test-hook type, and move each feature's state onto its own entity (settings pages, toasts, tabs).
- Risk: med.
- Prot.: no.

---

## c8 inventory (GitMan and FileMan readiness)

The cross-cutting blocker is the selftest hook. Every widget takes `&self.hits` and calls `hit_target(..)` (`view.rs:112-129, 2190-2203`). An extraction must first move `HitTargets` (or a `Fn(String) -> AnyElement` hook) into foreman-ui, or the selftests lose their click targets.

**text_input**
- Where today: `foreman-app/src/text_input.rs:1-1340` (tests 1342-1564):
  - `LineText` 177-417, pure, with no gpui;
  - rows and layout 150-557;
  - `TextInput` 563-1005;
  - `EntityInputHandler` 1024-1099;
  - `TextElement` 1101-1276;
  - `bind_keys` 74-117, called from `main.rs:265`.
- Ties to foreman-app: only `ActiveTheme` (987, 1292-1293), the selftest field `caret_painted` (582), and the `unicode-segmentation` dependency.
- Target: **foreman-ui**.
- What the extraction needs:
  - Move it as is, and add the dependency to foreman-ui.
  - Use `CURSOR_BLINK_INTERVAL`/`blink_phase` (A-15).
  - Update the callers (`view.rs:609, 1187`, `manage.rs:55`, `term.rs:193`, `settings_window.rs:128`) and the CLAUDE.md path.

**Context menu**
- Where today: `view/menu.rs:48-69` (`MenuItem`, `MenuTarget`, `ContextMenu`) and `:177-229` (`render_menu`). State at `view.rs:292`, mounted at `:2500`, opened at `manage.rs:286-313`, Esc at A-17's three sites.
- Ties to foreman-app:
  - `MenuItem.action: MenuAction` (`menu.rs:15-46`) mixes menu commands, the confirm-only `*Now` variants and `Dismiss`;
  - a click calls `FleetView::run_action` (`menu.rs:221-225`);
  - the hit keys `menu-<key>`;
  - the theme.
  - The builders `workspace_items`, `repo_items` and `session_items` (83-169) are app logic and stay.
- Target: **foreman-ui**.
- What the extraction needs:
  - `ContextMenu<A>` and `MenuItem<A>{key, label, action: A}`.
  - A render function with `on_pick(A)`, `on_dismiss` and a test hook, or an entity that emits `Picked(A)` and `DismissEvent`.
  - Split `MenuAction` into menu commands and a separate confirm payload.
  - A shared `dismiss_popup`.

**Confirm dialog**
- Where today: `menu.rs:71-76` (`Confirm{text, yes, action: MenuAction}`). It renders inline in the notices column, not modal (`left.rs:1496-1537`). Answered at `manage.rs:399-406`; created at `manage.rs:346, 371` and `delete.rs:183, 230, 267, 318`.
- Ties to foreman-app: the `MenuAction` payload; `MenuAction::Dismiss` as an OK-only sentinel (`left.rs:1526`, `delete.rs:268, 318`); `FleetView.confirm`; `view::button` with the hit keys `confirm-yes` and `confirm-cancel`.
- Target: **foreman-ui**.
- What the extraction needs:
  - `Confirm<A>{text, yes, cancel: Option<…>, action: Option<A>}`, where `None` means OK only and replaces the sentinel.
  - An `on_answer(bool)` callback; the owner chooses the placement.
  - **Protected (Del):** the m5/38 deletion flow passes through it, so keep its texts and the order of the second question.

**Toast and notices**
- Where today:
  - Toast: `view.rs:359-365` (`Toast`, `TOAST_FOR`), `manage.rs:267-283` (`show_toast`), render at `left.rs:737-765`.
  - Notices: `view/notices.rs:8-67` (kinds Setup, Supervise, Hand, Action; `dismiss_key`, `keep_dismissed`), render at `left.rs:423-509`, the Workman notice at `left.rs:915-976`.
  - Banner copies: A-21.
- Ties to foreman-app: the FleetView fields (`view.rs:228-240, 350`); the app's `Notice` enum; status colours; the hit keys `toast` and `notice-dismiss-<kind>-<n>`.
- Target: **foreman-ui**.
- What the extraction needs:
  - `ToastSlot` with `show(text, cx: &mut Context<V>)`, generic over the owning view because the timer runs through `cx.spawn`, plus `render(hook)`.
  - `banner(...)`.
  - `NoticeList<K: Copy + Ord>` with dismiss-by-text and `keep_dismissed`.
  - The app keeps only the kinds and their sources. Keep the 0.12 tints (Contrast).

**Tab strip and hover card**
- Where today:
  - Strip: `view.rs:2238-2333`.
  - Model: `view.rs:137-198` (`TabKind`, `Detach`, `Tab`, `label`).
  - Behaviour: `cycled` 75-81, `activate_tab`/`cycle_tab` 1572-1653, close/detach/remove 1926-2008, keys 58-72.
  - Card: `tab_card.rs:76-200` (`Card`, `CardView`, `LAST_CARD`); `sanitize` 18-53; the card is built at `view.rs:1745-1776`.
- Ties to foreman-app:
  - `Tab` holds `Entity<Terminal>`, `TabKind::Work(Entity<WorkView>)` and the `Detach` state machine (**protected CLI:** the Ctrl+Z detach of `claude attach`).
  - The close button goes to `FleetView::close_tab`.
  - The hit keys `tab-{id}` and `close-tab-{id}`; `Icon::Close`; `clipped_with`.
- Target: **foreman-ui**.
- What the extraction needs:
  - `TabHeader{id, label, active}` and `tab_strip<V>(tabs, tooltip, on_activate, on_close, on_new, hook, cx)`, with `cycled` moving alongside.
  - `HoverCard{title, rows}`, whose view lives in foreman-ui while the field builder (`FleetView::tab_card`) stays in the app.
  - `sanitize` moves with the card, or into foreman-term as an OSC-title helper.
  - Tab state and detach stay in the app.

**Split panel**
- Where today:
  - The only resizable split is the left panel / terminal: constants `view.rs:94-96`, `DraggedDivider` `:376-383`, the drag and 7 px grab area `:2455-2498`, double-click reset `:2489-2495`, `save_left_width` `:1392-1405`, `Settings.left_width`.
  - A fixed split is the Work tab's board/item pane (`work_view.rs:723-729`, 42 %, `min_w` 380).
- Ties to foreman-app: the FleetView fields; a `Settings` write on mouse-up, on the UI thread.
- Target: **foreman-ui**.
- What the extraction needs:
  - `SplitPane{id, first, min_first, min_second, default}`.
  - A per-id drag payload, so that two splits can coexist.
  - `on_resize` and `on_resize_end` callbacks; the app persists through `Settings::update` (A-03).
  - An optional relative mode.

**Matcher and results list**
- Where today:
  - Matcher: `search.rs:85-123` (`find_all`, `id_prefix`, `session_marks`) and `:127-152` (`search` over `fleet::Tree`). It is case-insensitive substring and id-prefix matching, **not fuzzy**.
  - Results: `view/results.rs:20-47` (`marked` highlight, `count` badge) and `:105-186` (`render_results`, which reuses `FleetView::render_session`).
  - State: `view.rs:341, 343, 609-610, 2099-2122`.
- Ties to foreman-app: `Tree`, `SessionRow` and `WorkspaceNode`; `FleetView::search_tree`; the `search_marks` side channel into `render_session` (`left.rs:1355, 1376-1394`); `reveal` and `open_result`.
- Target: `matches(hay, needle) -> Vec<Range>` and `HighlightedLabel` (= `marked`) go to **foreman-ui**. The grouped tree results stay in the app.
- What the extraction needs:
  - A generic list needs `trait ResultNode{key, label, marks, children}` plus a row callback.
  - There is no `uniform_list` today.
  - For real fuzzy matching, evaluate zed's `fuzzy` crate at the pinned rev (unchecked).

**Generic process helper**
- Where today:
  - `foreman_claude::no_window` (`cli.rs:90`).
  - The runners listed in A-01.
  - `icacls` at `launch.rs:200`; `git` at `mint.rs:67` and `fm/src/probe.rs:378`.
  - `ShellExecuteW` at `signin.rs:69` and `elevate.rs:76`; `explorer.exe` at `manage.rs:500`.
  - The probe spawns at `probe.rs:316` and `probe_hooks.rs:253` (no `no_window`), `probe_rm.rs:125` and `probe_session_id.rs:109`.
  - Selftest spawns at `selftest.rs:148`, `selftest_manage.rs:307`, `selftest_refresh.rs:67`, `selftest_supervise.rs:148, 910`.
  - ConPTY shells go through foreman-term (`alacritty.rs:174, 191`; the environment is scrubbed at `main.rs:129`) and stay there.
- Ties: foreman-supervisor, fm and the app depend on foreman-claude just for `no_window`.
- Target: **foreman-core** (`process` module).
- What the extraction needs:
  - A `windows` dependency under cfg(windows) in foreman-core.
  - `command(program)` with no_window; `run(cmd, timeout) -> Output{code, stdout, stderr}` with drain threads and a kill on timeout (optionally a job object for the tree); `git(dir, args)`; `shell_execute(verb, target, args, cwd)`.
  - foreman-claude keeps the claude argv and error mapping on top of it.
  - **Protected (CLI):** the argv and the timeouts of every claude call stay the same.

**SecretStore**
- Where today: `foreman-workstate/src/secret.rs`:
  - the trait (get/set/delete; errors never carry the secret);
  - `MemoryStore`;
  - `CredentialStore` (`CredReadW` :78, `CredWriteW` :110 with `CRED_PERSIST_LOCAL_MACHINE` and user "Foreman", `CredDeleteW` :119);
  - the target is `<prefix>:<issuer>`, where the prefix is `Foreman/Workman` or `FOREMAN_CREDENTIAL_PREFIX` (`PREFIX_VAR`).
  - The only production user is `Auth` (`auth.rs:37, 125, 149`).
  - Other users: `signin.rs:55`, `selftest.rs:175`, `fm/tests/work.rs:11`, and the `MemoryStore` tests (`hand.rs:399`, `tokens.rs:52, 74`, `workstate_setup.rs:125`, `tests/oauth.rs`).
  - Minted `wss_` tokens are plaintext in `<home>/mcp/workstate-<name>.json`, made owner-only with icacls (`launch.rs:171-198`), which is correct for claude's `--mcp-config`. They stay out of scope.
- Ties to foreman-app: none beyond those callers.
- Target: **foreman-core** (`secret` module).
- What the extraction needs:
  - Move the trait, both stores and `default_store`.
  - Add a cfg(windows) `windows` dependency (`Win32_Foundation`, `Win32_Security_Credentials`) and a `getrandom` dev-dependency.
  - Make the namespace a parameter, keeping the `Foreman/Workman` default and the `<prefix>:<key>` format **byte-identical** so existing credentials stay readable. Keep `PREFIX_VAR`.
  - Re-export from foreman-workstate so `Auth::with_store` compiles unchanged.
  - Optionally zeroize the blob copies (`secret.rs:86, 98`).
  - **Protected (OAuth).**

**Settings page registration with namespaced tables**
- Where today:
  - `foreman-core/src/settings.rs`: one flat struct with 11 keys from 5 features:
    - sessions: `max_working`, `extra_plugin_dirs`, `relay_idle_secs`, `run_as_admin`;
    - Workman: `workstate_url`;
    - appearance: `theme`, `terminal_colors`;
    - terminal: `terminal_font_size`, `terminal_font_family`, `terminal_ligatures`;
    - window: `left_width`.
  - `load` is whole-file serde, lenient for 3 fields only (A-47). `save` drops unknown keys (A-25). There is no `update` (A-03).
  - The window (`settings_window.rs`): the `Page` enum and `ALL` (23-46); `SettingsView` holds `WeakEntity<FleetView>` plus its own status (48-72); `reload` re-reads everything and calls `auth.status` on the UI thread (186-197); the page renderers (321-761); the widgets `row`, `choice` and `small_button` (764-839).
- Ties to foreman-app:
  - The pages read FleetView state directly: `cli`, `last_refresh_took`, `term_font_*`, `installed_fonts`, `signing_in`, `sign_in_error` and `workstate`.
  - The pages call FleetView methods: `set_term_font_size`, `set_term_font`, `set_max_working`, `set_workstate_url`, `sign_in`/`sign_out`/`cancel_sign_in` and `recheck_workstate`.
  - `Page` is referenced from `left.rs:19, 320, 949`, `view.rs:1365` and 5 selftests.
- Target: the store goes to **foreman-core**; the window shell, the page trait and the widgets go to **foreman-ui**; each page stays with its owner.
- What the extraction needs:
  1. **Core:** a `SettingsFile` backed by a `toml::Table` that keeps unknown tables, and `trait SettingsSection: Serialize + DeserializeOwned + Default { const KEY: &str; }` with `get::<S>()` and an atomic `update::<S>(home, f)`.
     - The sections are `[sessions]`, `[workstate]`, `[appearance]`, `[terminal]` and `[window]`.
     - Each section is lenient on its own and reports its errors.
     - The flat keys move through a `migrate.rs` step (the existing precedent).
     - `max_working` is read every tick (`supervise.rs:318`), so that read must stay cheap.
  2. **App:** a `SettingsStore` global entity that emits `Changed(section)`. It replaces each page's re-read and FleetView's mirror fields.
  3. **foreman-ui:** `trait SettingsPage { id(); title(); render(store, window, cx) }`, a registry filled at start, and `open_settings(page_id)` in place of the `Page` enum. The selftests address pages by id.
  4. Each page gets its own model instead of `WeakEntity<FleetView>`:
     - the Workman page gets an auth/sign-in model that owns `signing_in` and `sign_in_error` and computes `AuthStatus` off the UI thread (A-27);
     - Appearance gets a terminal-font model;
     - About gets read-only status.
  5. `row`, `choice` and `small_button` move into foreman-ui with the buttons (A-20).
  - `Registry`, `DesiredState` and `MintedSessions` are three more "named record file under home" stores. They can share the storage layer later, but that is not part of c8.

---

## Proposed refactor units

**Rules for every unit:**
- The full gate (cargo test, `--selftest` 5/5, `--selftest-supervise`, `--selftest-detach-shutdown`, leftover 0).
- C57 applies to the code the unit touches.
- The report names each behaviour change and why it is an improvement.
- Protected logic keeps its behaviour unless a decision above says otherwise.

**About "may run alongside":** two units may run in parallel only if their file lists do not intersect. New files count.

### U1 — Terminal link safety (A-26, A-49)
- Files:
  - `crates/foreman-term/src/terminal.rs` (the mouse-up and Ctrl+click paths only);
  - `crates/foreman-app/src/view.rs` (the `Event::Open` arm only).
- Behaviour changes:
  - An OSC 8 link opens only on Ctrl+click and only for allowed schemes. This closes the hostile-output hole.
  - A Ctrl+click on a plain URL opens it, as in Windows Terminal, where today it swallows the click.
- Risk: low–med. Prot.: no.
- Alongside: U2.

### U2 — OAuth and Workman client robustness (A-27 auth side, A-28, A-45, A-48 `health.rs`, A-61, A-77 F-05 steps 1–2)
- Files:
  - `crates/foreman-workstate/src/{auth.rs, fake.rs, stream.rs, health.rs}`;
  - `crates/foreman-workstate/tests/{oauth.rs, stream.rs}`;
  - `crates/fm/tests/work.rs`.
- Behaviour changes:
  - A refresh no longer holds the mutex the UI reads, so the freeze goes away.
  - A damaged `servers.json` is reported, never overwritten, so data is no longer lost.
  - An SSE line is capped and the API header is checked.
  - `health` classifies errors by error kind, not by text.
  - The fake's ids are random and its refusals say why.
  - F-05 becomes self-diagnosing.
- Risk: med. Prot.: **OAuth/WS**.
- Alongside: U1.

### U3 — Shared foundations in foreman-core (A-02 non-app, A-03 core, A-04, A-05, A-06, A-07, A-08, A-09, A-14, A-46, A-47)
- Files:
  - new `crates/foreman-core/src/{fs.rs, paths.rs, time.rs, url.rs}`;
  - `crates/foreman-core/src/{lib.rs, settings.rs, registry.rs, relay.rs, migrate.rs, events.rs, discover.rs, scan.rs}`;
  - `crates/foreman-claude/src/{peer.rs, transcript.rs, human.rs, assign.rs, session.rs, launch.rs (mcp_config_path only)}`;
  - `crates/foreman-supervisor/src/{desired.rs, mint.rs, executor.rs (mcp path only)}`;
  - `crates/foreman-workstate/src/{client.rs, http.rs, auth.rs (pct, split_origin)}`;
  - `crates/foreman-ui/src/lib.rs` (`append_capped`);
  - `crates/fm/src/hook.rs`.
- Behaviour changes:
  - Concurrent writers no longer clobber each other's temp file (a pid-unique temp name plus the rename retry everywhere).
  - The peer registry honours `CLAUDE_CONFIG_DIR`, as the transcripts already do.
  - One origin rule.
  - A bad workspace file or settings key degrades to a notice instead of stopping the supervisor.
  - `Settings::update` exists.
- Risk: med. Prot.: **CLI, Mint**.
- Runs after U2 (`auth.rs`).

### U4 — Process helper into foreman-core (A-01, A-10, A-11, A-12, A-23, A-31, A-32; c8 process)
- Files:
  - new `crates/foreman-core/src/process.rs`, `crates/foreman-core/Cargo.toml`;
  - `crates/foreman-claude/src/{cli.rs, launch.rs, daemon.rs, error.rs, lib.rs}`;
  - `crates/foreman-supervisor/src/mint.rs` (`git`);
  - `crates/fm/src/{probe.rs, probe_hooks.rs, probe_rm.rs, probe_session_id.rs}`;
  - `crates/foreman-app/src/{signin.rs, elevate.rs, selftest.rs, selftest_supervise.rs, selftest_refresh.rs, view/manage.rs}`;
  - `crates/foreman-app/src/view.rs` (`process_alive` removal only).
- Behaviour changes:
  - Every spawn has no window and a timeout.
  - `daemon status` and selftest cleanup can no longer hang or leak processes.
  - Probes no longer flash consoles.
  - One error variant for a CLI failure.
- Risk: med. Prot.: **CLI, Del** (argv and timeouts identical).
- Alongside: U5.

### U5 — Supervisor correctness (A-33, A-34, A-35, A-36, A-48 budget and prefix, A-52 notes, A-58, A-60)
- Files:
  - `crates/foreman-supervisor/src/{executor.rs, desired.rs, budget.rs, memory.rs}` and its tests;
  - `crates/foreman-claude/src/forget.rs` (tests only).
- Behaviour changes:
  - A failing Stop surfaces as a NeedsYou row instead of silence.
  - A Re-mint is retried, not lost.
  - An unrecognised resume output is "unknown", not success.
  - `DesiredState::update` is serialised by a lock file, so updates are no longer lost.
- Risk: med. Prot.: **CLI, Mint, budget, Del (tests)**.
- Alongside: U4. U5 avoids `mint.rs`; U3 must have landed first, because of `desired.rs` and `executor.rs`.

### U6a — App: UI-thread work and hands (A-03 app callers, A-17 Esc/pwsh/Tab/display-name, A-27 app side in `view.rs`, A-29, A-30, A-37, A-39, A-46 notice, A-50, A-59)
- Files:
  - `crates/foreman-app/src/{view.rs, view/manage.rs, view/term.rs, view/left.rs, hand.rs, supervise.rs, fleet.rs}`.
- Behaviour changes:
  - No timer-polling loops; event-driven waits.
  - Quit no longer hangs on revokes.
  - A failed `hands.json` save is shown.
  - The context menu opens without IO.
  - Ctrl+wheel zoom follows the delta.
- Risk: med. Prot.: **CLI (detach), Mint (hand release)**.
- Alongside: U6b, U12, U13.

### U6b — App: Work tab, settings window, persistence (A-02 app files, A-19, A-24, A-27 settings side, A-38, A-44 app parts, A-51, A-63 settings hint, A-17 debounce)
- Files:
  - `crates/foreman-app/src/{work_view.rs, work_view/inbox.rs, work_view/item.rs, work_view/journal.rs, settings_window.rs, clipped.rs, stars.rs, dismissed_messages.rs, text_input.rs (undo cap), signin.rs}`;
  - `crates/foreman-workstate/src/{stream.rs (async receiver), auth.rs (issuer_of)}`.
- Behaviour changes:
  - The stream no longer pins a thread.
  - Settings reads `AuthStatus` off the UI thread.
  - Dismissed state is written atomically, and errors are shown.
  - The caches are bounded.
- Risk: med. Prot.: **OAuth, WS**.
- Alongside: U6a and U13. **Not** U4 (`signin.rs`), so U4 must land first.

### U7 — Terminal performance and clipboard (A-13, A-40, A-41, A-42, A-44 term and ui parts, A-62)
- Files:
  - `crates/foreman-term/src/{terminal.rs, alacritty.rs, pty_info.rs}`;
  - `crates/foreman-ui/src/{term_element.rs, themes.rs}`;
  - `crates/foreman-app/src/{view/term.rs, selftest.rs, selftest_term.rs}` (the `selection_text` readers).
- Behaviour changes:
  - No per-frame selection string.
  - The sysinfo refresh is throttled.
  - The theme catalog is cached.
  - One paste path: right-click paste of an image-only clipboard behaves like Ctrl+V.
- Risk: low–med. Prot.: no (keep the m5/46 Alt+V behaviour).
- Runs after U6a (`view/term.rs`) and after U12 (the selftests).

### U8 — Terminal dead code and split (A-53, A-57, A-80)
- Files: `crates/foreman-term/src/**`, `crates/foreman-term/Cargo.toml`, `crates/foreman-term/tests/**`.
- Behaviour changes: none; the change is deletion plus a move. One URL crate.
- Risk: med (size). Prot.: no.
- Needs **decision 1**. Runs after U1 and U7.
- Alongside: U9.

### U9 — c8 part 1: foreman-ui foundations (A-15, A-20, A-21, A-22 text_input, A-81 HitTargets)
- Files:
  - new `crates/foreman-ui/src/{text_input.rs, button.rs, banner.rs, hit.rs, fonts.rs}`, `crates/foreman-ui/Cargo.toml`, `crates/foreman-ui/src/lib.rs`;
  - delete `crates/foreman-app/src/text_input.rs`;
  - `crates/foreman-app/src/{main.rs, view.rs, view/left.rs, settings_window.rs, titlebar.rs, work_view.rs, work_view/item.rs, work_view/journal.rs}`;
  - `CLAUDE.md` (the component list).
- Behaviour changes: none visible. The buttons and banners are pixel-identical; the hit keys are unchanged.
- Risk: med. Prot.: **Contrast**.
- Alongside: U8.

### U10 — c8 part 2: the menu, confirm, toast/notices, tab strip, split pane, matcher (A-16, A-17 Esc, A-22)
- Files:
  - new `crates/foreman-ui/src/{menu.rs, confirm.rs, toast.rs, notices.rs, tabs.rs, hover_card.rs, split.rs, matcher.rs}`;
  - `crates/foreman-app/src/{view.rs, view/menu.rs, view/notices.rs, view/delete.rs, view/manage.rs, view/results.rs, view/left.rs, search.rs, tab_card.rs}` and the selftests that name these keys.
- Behaviour changes:
  - `MenuAction` is split, and the confirm dialog no longer needs the `Dismiss` sentinel.
  - The UI is otherwise identical.
- Risk: med–high (wide). Prot.: **Del** (the confirm texts and order), **CLI** (the tab detach stays in the app).
- Runs after U9. Two sub-units are possible: (menu, confirm, toast, notices) and (tabs, card, split, matcher).

### U11 — c8 part 3: namespaced settings and page registration, SecretStore into core (A-25, A-47 sections, c8 SecretStore)
- Files:
  - `crates/foreman-core/src/{settings.rs, migrate.rs, secret.rs (new), Cargo.toml}`;
  - `crates/foreman-workstate/src/{secret.rs, lib.rs}`;
  - new `crates/foreman-ui/src/settings_page.rs`;
  - `crates/foreman-app/src/{settings_window.rs, view.rs, view/left.rs, view/term.rs, supervise.rs}` and the 5 selftests that address `Page`.
- Behaviour changes:
  - `settings.toml` gets sections, migrated from the flat keys.
  - Unknown tables survive a save.
  - One bad section falls back alone.
  - Pages register by id.
- Risk: **high** (file format, sign-in persistence). Prot.: **OAuth** (credential targets byte-identical), budget, Mint.
- Runs after U10.

### U12 — Selftest hygiene and F-06 (A-18, A-78, A-79)
- Files:
  - `crates/foreman-app/src/{selftest_closed.rs, selftest_delete.rs, selftest_first_run.rs, selftest_left.rs, selftest_manage.rs, selftest_mark.rs, selftest_messages.rs, selftest_notices.rs, selftest_tabs.rs, selftest_title.rs, selftest_work.rs, selftest_zoom.rs, selftest/frames.rs, tray.rs}`;
  - `selftest.rs` (`until`, the per-step log offset).
- Behaviour changes:
  - Step 7l measures the tree area instead of row y.
  - Step 7c waits for the window to be shown and scans only its own log lines.
  - `tray::close` drains by state, not by a fixed 100 ms.
- Risk: low–med (the close discipline is from m5/22). Prot.: no.
- Runs after U4 (`selftest.rs`, `selftest_supervise.rs`, `selftest_refresh.rs`).
- Alongside: U6a, U6b, U13.

### U13 — fm and plugin cleanup (A-54, A-55, A-56, A-63 plugin text, A-64)
- Files:
  - `crates/fm/src/{probe.rs, hook.rs, relay.rs, main.rs}`, `crates/fm/probe/resume.ps1`;
  - `crates/foreman-claude/src/{transcript.rs (projects), assign.rs (repo_of)}` and `crates/foreman-core/src/discover.rs` (`Registry::update`);
  - `crates/foreman-app/src/{stars.rs (contains), tab_card.rs (branch_at)}`;
  - `crates/foreman-app/diag/**` (per decision 4);
  - `plugins/foreman/**`.
- Behaviour changes:
  - `fm probe launch` is gone.
  - The pre-m2x/2 and pre-m5/4 fallbacks are gone.
  - The docs match LEARNINGS.
- Risk: low. Prot.: partly (CLI probes).
- **Conflicts:** `stars.rs` with U6b, `tab_card.rs` with U10, `launch.rs` (`with_session_token`) with U4. Either drop those parts into the owning unit, or run U13 after U6b. The recommended way is to give A-54's app items to U6b and the `launch.rs` items to U4; U13 then keeps fm, the plugins and the two claude/core deletions, and runs after U3 and U4.

### U14a–d — Comment passes per crate group (A-65 … A-76)
- **U14a:** `foreman-term` and `foreman-ui`.
- **U14b:** `foreman-claude`, `foreman-supervisor` and `foreman-core`.
- **U14c:** `foreman-workstate`, `fm` and `plugins/foreman`.
- **U14d:** `foreman-app`.
- Behaviour changes: none. Comments only, following `rlarch:code-comments`; never add `///`.
- Risk: none.
- U14a, U14b and U14c are disjoint and may run together after U11. U14d runs last because it touches the most-edited files.

### U15 — Docs
- Writes `docs/ARCHITECTURE.md`: the crates and their boundaries, the shared components and helpers from U3, U4 and U9–U11, and what is shared versus app-only.
- Refreshes CLAUDE.md's component list.
- Fixes the stale LEARNINGS pointers: `diag/timeline.py` (LEARNINGS:884), `line_edit.rs` (LEARNINGS:2221), the local-stack URL in the probe notes, and the static-token mentions after m5/43.
- Files: `docs/ARCHITECTURE.md`, `CLAUDE.md`, `docs/LEARNINGS.md`.
- Risk: none.
- Runs last.

### Order

| Wave | Units | Why |
|---|---|---|
| 1 | U1 ‖ U2 | security and the blocker fixes first; disjoint |
| 2 | U3 | foundations that later units call |
| 3 | U4 ‖ U5 | the process helper, and supervisor correctness |
| 4 | U6a ‖ U6b ‖ U12 ‖ U13 | app fixes, selftests, cleanup; disjoint once U13 has handed off its shared items |
| 5 | U7 | terminal performance (needs U6a and U12) |
| 6 | U8 ‖ U9 | terminal split ‖ the start of the foreman-ui extraction |
| 7 | U10 | c8 part 2 |
| 8 | U11 | c8 part 3 (highest risk, last of the code) |
| 9 | U14a ‖ U14b ‖ U14c, then U14d | comment passes |
| 10 | U15 | docs |

---

## Appendix: per-file comment counts (≈ to delete)

- **foreman-app UI:**
  - `view.rs` 50, `view/left.rs` 22, `text_input.rs` 15, `work_view.rs` 10
  - `view/manage.rs` 8, `settings_window.rs` 8, `work_view/inbox.rs` 8
  - `view/delete.rs` 6, `view/term.rs` 6, `work_view/item.rs` 6, `tab_card.rs` 6
  - `view/menu.rs` 5, `work_view/journal.rs` 5
  - `view/results.rs` 4, `search.rs` 4
  - `compose.rs` 3, `clipped.rs` 3, `dismissed_messages.rs` 3, `signin.rs` 3
  - `view/notices.rs` 2, `stars.rs` 2
  - `view/icon.rs` 1, `titlebar.rs` 1
- **foreman-app core:**
  - `selftest_supervise.rs` 25, `selftest.rs` 22, `hand.rs` 20
  - `supervise.rs` 10, `selftest_left.rs` 10, `selftest_work.rs` 9
  - `fleet.rs` 8, `selftest_manage.rs` 8, `selftest_term.rs` 7, `main.rs` 6
  - `tray.rs` 3, `workstate_setup.rs` 3
  - the other selftests 1–4 each; `tokens.rs`, `instance.rs` and `elevate.rs` about 1 each
- **foreman-term:**
  - `terminal.rs` 60 (about 150 lines, mostly going with A-53)
  - `alacritty/hyperlinks.rs` 8, `mappings/keys.rs` 7, `pty_info.rs` 6, `alacritty.rs` 3
  - `mappings/mouse.rs` 1, `terminal_settings.rs` 1, `examples/pty_dump.rs` 1
- **foreman-ui:** `themes.rs` 15, `term_element.rs` 6, `lib.rs` 3.
- **foreman-claude:**
  - `launch.rs` 12, `peer.rs` 8, `transcript.rs` 8
  - `daemon.rs` 6, `human.rs` 5, `lib.rs` 4
  - `session.rs`, `cli.rs`, `assign.rs`, `proc_tree.rs` 3 each; `forget.rs` 2
- **foreman-supervisor:**
  - `executor.rs` 25, `lib.rs` 10, `mint.rs` 8, `desired.rs` 8
  - `memory.rs`, `reconcile.rs`, `needs.rs`, `budget.rs` 4 each
- **foreman-core:** `relay.rs` 8, `settings.rs` 8, `events.rs` 3, `migrate.rs` 3, `scan.rs` 3.
- **foreman-workstate:** `auth.rs` 20, `stream.rs` 8, `fake.rs` 4, `lib.rs` 2, `model.rs` 2, `tests/oauth.rs` 1.
- **fm:**
  - `probe*.rs` 17 in total, `work.rs` 4, `relay.rs` 4, `hook.rs` 2
  - `tests/work.rs` 3; `tests/skills.rs` and `tests/want.rs` 3 together
- **plugins/foreman:** `README.md` 5, `workspace-controller/SKILL.md` 4, `repo-controller/SKILL.md` 3.
