# Learnings

Measured facts that shape the design. Newest last; each with its date and how it was observed.

## 2026-10-22 — setup

- **WorkState was running but unregistered.** `workstate-api-1` had been up 47h and healthy, with
  `/health` 200, but `claude mcp get workstate` said not found. The setup skill's state table covered
  this ("server up, not registered"). → The app's tear-up must check registration as well as health.
- **Controller token gives 19 tools including `work_constrain`**, so the running image is new enough
  for `/rlarch:controller`. It is not yet proven to equal `feat/workstate` HEAD (M0.1).
- **A local-scope MCP entry shadows the user-scope one.** Registered `--scope user` from
  `Skills/mcp`; `claude mcp get` run *there* reports `Scope: Local config`. An older per-directory
  entry exists for that folder and takes precedence. From any other folder it resolves to user scope.
  → The app should inspect `claude mcp get workstate` **from each repo's cwd** before launching a
  session there, not once globally.
- **MCP tools only appear in a new session.** Registration in the current session never adds the
  tools to it. → The app registers MCP/plugins *before* launching sessions, never mid-session.
- **`feat/workstate` is ahead of `master`** in the Skills repo (master merged an earlier tip).
- **`claude agents --json`** lists interactive *and* background sessions; background rows carry
  `state` (seen: `blocked`), interactive rows carry `status` (`idle`/`busy`) and a `pid`.
- **Session peer registry**: `~/.claude/sessions/<pid>.json` holds `sessionId`, `cwd`, `name`,
  `status`, `peerProtocol: 1`, `peerFeatures: ["notify_idle","artifact_yield"]`,
  `messagingSocketPath: \\.\pipe\LOCAL\cc-msg-<hex>`, with a sibling `.key` file. `SendMessage`
  rides on this. `~/.claude/daemon/` holds `roster.json`, `dispatch`, `attach-journal`, `pty-pids`
  for `--bg` sessions; the daemon exits after 5s idle with no clients.
- **Zed licensing**: `gpui` Apache-2.0; `terminal`, `terminal_view`, `ui`, `theme`, `workspace`
  GPL-3.0. GPL accepted for this project.

## 2026-10-22 — m0/2 session state model

Observed with `probes/m0.2-states/run.ps1` (claude CLI 2.1.308, Windows 11): one `claude --bg
--permission-mode default` session per run, prompt "count 1..40 in words, then run `New-Item`
via the Bash tool". Polled `claude agents --json --all` and `claude agents --json` back to back;
one poll pair takes ~1.6–1.8 s, so transition times below are ±1 poll. Four runs, all PASS; runs
3–4 used the final script (runs 1–2 had logging/flag bugs, row data identical). Raw rows per run in
`probes/m0.2-states/.scratch/<ts>/rows.jsonl` (gitignored).

Background row lifecycle, in order (times from launch, runs 3 / 4):

| # | Trigger | `kind` | `state` | `status` | `waitingFor` | `pid` | in `--json` | in `--json --all` | at |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `claude --bg` returned (0.8 s, prints `backgrounded · <id8> · <name>`) | background | working | *(absent)* | *(absent)* | *(absent)* | yes | yes | 1.8 s / 1.8 s |
| 2 | session process up | background | working | busy | *(absent)* | set | yes | yes | 3.6 s / 3.6 s |
| 3 | Bash tool call parks on permission prompt | background | blocked | waiting | `permission prompt` | set | yes | yes | 12.6 s / 14.4 s |
| 4 | `claude stop <id>` (returns `stopped <id>` in ~0.8 s, exit 0) | background | stopped | *(absent)* | *(absent)* | *(absent)* | **no** | yes | first poll after stop |
| 5 | `claude rm <id>` (returns `removed <id>` in ~0.8 s, exit 0) | — | — | — | — | — | no | **no** | first poll after rm |

Fields present on every row: `id` (8 hex, = first 8 chars of `sessionId`), `sessionId` (UUID),
`name`, `cwd`, `kind`, `startedAt` (epoch ms), `state`.

- **`state` and `status` are two vocabularies on the same row.** `state` (working/blocked/stopped)
  is present from the first poll to rm; `status` (busy/waiting) and `pid` appear only while the
  process is alive and vanish on stop.
- **`waitingFor` appears only in the blocked state**, value `permission prompt`, together with
  `status=waiting`. It is absent (not null) otherwise.
- **The first row exists before the process does**: ~1 s after `--bg` returns, `state=working`
  with no `pid`/`status`; `pid` + `status=busy` follow ~1.8 s later.
- **`startedAt` changes**: the pre-process row and the stopped row carry the launch time (run 4:
  `…858311`); while the process is alive it carries the process start (`…859967`, +1.66 s).
- **`stop` keeps the row; `rm` deletes it.** After `stop`, the row leaves plain `--json`
  immediately but stays in `--json --all` as `state=stopped` (held ≥ 20 s, unchanged). After `rm`
  it is gone from both by the next poll. A supervisor polling plain `--json` cannot tell
  "stopped" from "removed" — it must use `--all`.
- **Parked session stays parked**: the blocked row was unchanged for 6–7 s until stop; no timeout
  or auto-deny was seen in that window.
- No other row fields changed during these runs.

## 2026-10-22 — running M0 as a controller (controller's own observations)

- **Nested `--bg` sessions inherit the user-scope MCP registration — here the controller token.**
  A worker can therefore write evidence that the store attributes to the controller; only the brief
  stops it. → Foreman launches every worker with `--mcp-config` carrying the **worker** token.
- **The installed rlarch plugin (1.5.0) lags `feat/workstate`'s skills.** A session loads whatever
  is installed. → Foreman launches with `--plugin-dir` pinned to the skill version it ships.
- **`Start-NestedAgent.ps1 -PromptFile` resolves against the caller's cwd**, not `-RepoRoot`;
  pass an absolute path.
- **`work_verify(command:)` must equal the item's `verify_cmd` literally** or the close guard
  refuses; omit `command` unless a different command ran.
- **Refs are per-run** (`m0/1` and `ws/1` coexist); qualify across runs. Claim leases are 30 min.
- **WorkState worker brief truncates BINDING rules and intent with `…`** and tells the worker to
  `work_status` its own item (findings F-02/F-03 → fixes ws/2, ws/3). Until fixed, the controller
  writes the full rule text into the brief file.
- **An empty WorkState server 503s `/api/stream`** until the first write creates the workspace
  (F-01 → ws/1); the dashboard reads it as "stream down".
- **`git worktree remove --force` can fail with "Permission denied"** right after `claude stop`:
  the stopped session still holds a handle for a moment. Retrying after ~3 s succeeded.
  → Foreman's teardown retries the delete with a short backoff.
- **Never take a session id from free text.** The m0/2 probe regexed it out of `--bg` output (F-04);
  resolve it from the `agents --json --all` row whose name you chose.

## 2026-10-22 — m0/3 hooks in --bg sessions

Observed with `probes/m0.3-hooks/run.ps1` (claude CLI 2.1.308, Windows 11), two runs, both PASS
(`.scratch/20261022-164329`, `.scratch/20261022-164425`; raw `hooks.jsonl` + `run.log` there, gitignored).
Sequence per run: `claude --bg --name aw-m0-hooks-<hex> --permission-mode default` with a prompt that
calls Bash → wait for `state=blocked` → `claude stop` → `claude --resume <sessionId> -p /compact` →
`claude --resume <sessionId> -p "<list every FOREMAN-NONCE…>"` → `claude rm`. All three processes ran
from the scratch dir. Hooks were configured in exec form (`command` = pwsh.exe, `args` = `-File
hook.ps1 -Event <E> -Out <abs path>`), so no shell selection was involved.

- **Project-local `.claude/settings.json` is honoured by a `--bg` session** started in that dir, with
  no trust prompt: its SessionStart/UserPromptSubmit/Notification hooks fired from the bg process.
  Writing the file from `run.ps1` was not refused, so `--settings` was not needed.
- **Hook events, identical in both runs (in order):**

  | # | Event | Fired by | Discriminator | Payload fields |
  |---|---|---|---|---|
  | 1 | SessionStart | `--bg` launch | `source=startup` | cwd, hook_event_name, model, scratchpad_dir, session_id, session_title, source, transcript_path |
  | 2 | UserPromptSubmit | `--bg` launch | — | cwd, hook_event_name, permission_mode, prompt, prompt_id, scratchpad_dir, session_id, session_title, transcript_path |
  | 3 | Notification | `--bg`, Bash parks on permission (~12 s in, before agents row shows `blocked`) | `notification_type=permission_prompt`, `message="Claude needs your permission"` | cwd, hook_event_name, message, notification_type, prompt_id, scratchpad_dir, session_id, transcript_path |
  | 4 | SessionStart | `--resume -p /compact` | `source=resume` | context_tokens, cwd, estimated_cache_write_usd, hook_event_name, prompt_cache_likely_expired, seconds_since_last_response, session_id, session_title, source, transcript_path |
  | 5 | PreCompact | same | `trigger=manual`, `custom_instructions=null` | custom_instructions, cwd, hook_event_name, prompt_id, session_id, transcript_path, trigger |
  | 6 | SessionStart | same, after compaction (~11 s later) | `source=compact` | cwd, hook_event_name, model, prompt_id, session_id, session_title, source, transcript_path |
  | 7 | SessionStart | `--resume -p <question>` | `source=resume` (no token/cost fields this time) | cwd, hook_event_name, session_id, session_title, source, transcript_path |
  | 8 | UserPromptSubmit | same | — | as #2 minus scratchpad_dir |
  | 9 | Stop | same, turn end | `stop_hook_active=false`, `effort={"level":"medium"}` | background_tasks, cwd, effort, hook_event_name, last_assistant_message, permission_mode, prompt_id, session_crons, session_id, stop_hook_active, transcript_path |

- **`session_id` is the same UUID in all nine events**, across the bg process and both `--resume -p`
  processes; `session_title` is the `--name` we gave. → Foreman can key hook events by `session_id`
  and match them to the `agents --json` row's `sessionId`.
- **Notification has no `title` field** here (the docs show one). It fired ~1–3 s before the agents
  row reached `state=blocked`, so it is a usable push signal for "parked on a permission prompt".
- **Not fired:** no Stop from the bg session (it was stopped while parked, mid-turn); no Stop and no
  UserPromptSubmit for the `-p /compact` run (a slash command is not a prompt or a turn); no
  SessionEnd configured, so not observed. Stop was seen only from the `-p` question run.
- **Compact works non-interactively**: `claude --resume <sessionId> -p /compact` exits 0 with empty
  stdout in ~20 s and fires PreCompact(manual) then SessionStart(source=compact). Auto-compact was not
  triggered (would need a full context window).
- **Nonce re-injection works.** The hook printed `FOREMAN-NONCE-<12 hex> (…source=<s>)` as plain
  stdout. In the next process the model listed four nonces: the startup one only *via the compact
  summary* (its original hook message was gone), and the resume, compact, and second-resume ones as
  hook messages in context. The compact-time nonce was in the reply both runs
  (`…5d848682d812`, `…58e26e7328ed`). → A SessionStart(compact) hook's stdout survives into the next
  turn, so Foreman can re-inject the role skill there. Caveat: every `--resume` also fires
  SessionStart (source=resume) and adds its own injection, and the model saw them all; a re-injected
  role text should be idempotent and tagged with its source.

## 2026-10-22 — m0/4 relay via native SendMessage

Observed with `probes/m0.4-relay/run.ps1` (claude CLI 2.1.308, Windows 11). Per run: `claude --bg` a
**target** (`aw-m0-relay-target-<hex>`, told to answer any peer message with the nonce) and a **relay**
(`aw-m0-relay-<hex>`, `--permission-mode auto`, prompt file in the scratch dir), both with cwd = the scratch
dir. The relay arms `Monitor` on `pwsh -File probes/m0.4-relay/inbox.ps1 -Dir <inbox>`, writes
`outbox/ready.json`, and then relays: inbox file `{to, body}` → `SendMessage(to, body)`; incoming
cross-session message → `outbox/msg-NNNN.json` `{from, from_name, body}`. The app (run.ps1) drops
`inbox/<ts>.json` (write `.tmp`, then rename) and waits for an outbox body carrying the nonce. Raw
`run.log` + `rows.jsonl` per run in `probes/m0.4-relay/.scratch/<ts>/` (gitignored).

| Run (`.scratch/…`) | Target mode | app→relay reads file | app→target (target transcript enqueue) | round trip (drop → outbox file seen) | Result |
|---|---|---|---|---|---|
| `20261022-165907` | auto | — | 6.93 s (read by hand) | ≈17.4 s (log ticks) | round trip OK; script exit 1 on a timestamp-parse bug of mine, fixed |
| `20261022-170022` | auto | 3.95 s | 6.09 s | 18.00 s | PASS, exit 0 |
| `20261022-170124` | auto | 3.59 s | 6.46 s | 16.79 s | PASS, exit 0 |
| `20261022-170233` | **default** | 3.37 s | 5.45 s | 14.00 s | PASS, exit 0 |

app→X = first line containing the nonce in X's transcript minus the drop time. In the relay that is the
`Read` tool result; in the target it is a `queue-operation`/`enqueue` line.

- **The round trip works with only supported mechanisms**: a file watcher under `Monitor`, `Read`,
  `SendMessage`, `Write`. Nothing was held, refused, or needed a permission prompt in auto mode (the
  `Monitor` pwsh command and both `Write`s went through the auto classifier without parking).
- **No hold with a `default`-mode target.** Relay (auto) → target (default) and the reply back were both
  delivered straight away, no `[Cross-session delivery notice]` in either transcript. The incoming
  message is tagged `from-mode="prompting"` in both the auto and the default runs, so whatever the hold
  rule compares, `auto` vs `default` on two `--bg` sessions did not trigger it. The SendMessage result
  only says "in that session's inbox, not yet read … a delivery notice follows if that session holds it".
  → Foreman must still treat a delivery notice as a possible event (the relay prompt writes it to the
  outbox as `from: "relay-notice"`), but cannot rely on mode mismatch to predict it.
- **Addresses**: the receiver sees `<cross-session-message from="uds:\.\pipe\LOCAL\cc-msg-<hex>"
  from-name="<session name>" from-mode="prompting">`; the reply also carries `hop-chain="<hex>"`. The
  target replied to the pipe address (as SendMessage's docs say: copy `from`), not the name. → Foreman
  routes by `from_name` (we chose the names) and treats `from` as opaque.
- **Where the 14–18 s go** (run `170233`, relay transcript, seconds after drop): Monitor event enqueued
  0.6 → `Read` 1.7 → `SendMessage` 4.0 → turn ends 5.9 → target's reply enqueued in relay 10.5 → `Write`
  outbox 13.2 → app sees it 14.0. Each direction costs model turns: outbound 3 API calls (Read,
  SendMessage, end turn), inbound 2 (Write, end turn); the target's own turn is ~5 s of the middle.
- **Idle cost: zero turns while waiting.** The relay transcript has no lines between arming/ready and the
  first Monitor event (10 s+ gap per run); nothing polls the model.
- **Relay row while waiting on Monitor:** `status=busy` the whole time (pid set), `state=done` in three
  runs and `state=working` throughout in one (`170124`). An idle target shows `status=idle` with
  `state=done` (three runs) or `state=working` (`170022`). → A relay with an armed Monitor never reads
  as idle in `agents --json`; `state` alone is not a reliable idle signal. Foreman judges the relay by
  `ready.json` + `pid`, not by `status`.
- **`Monitor` is not persistent**: the tool reported "expires in 30m unless the source ends first". The
  relay prompt tells it to re-arm on expiry, which costs one wake-up turn per 30 min; inbox.ps1 records
  emitted paths in `<inbox>/.seen` and emits every unseen file on start, so files dropped during a re-arm
  gap are not lost or replayed. Re-arm itself was not observed (runs were < 1 min).
- **Teardown is clean**: `claude stop` + `rm` on both ids (from the rows we named) left no `inbox.ps1`
  pwsh process running in any run.
- **Failure modes seen**: only my parse bug (ConvertFrom-Json turns the transcript's ISO `timestamp` into
  a culture-formatted DateTime; regex the raw line instead). Transcript lines without a `timestamp`
  (`api-request-blob`) also carry the nonce; skip them when timing.

What Foreman's relay should look like:
- One relay session per Foreman instance, `--bg --permission-mode auto`, cwd = a Foreman-owned dir, name
  `foreman-relay-<hex>`; prompt from a file, as here, forbidding any recipient not named in an inbox file.
- Have the watcher print the **message itself** (one JSON line `{to, body}`) instead of a path: that drops
  the `Read` turn (~2 s and one API call per message). Keep `.seen` (or delete-after-emit) for re-arm.
- Atomic drops (write `.tmp`, rename); outbox files named by a counter the relay keeps, the app timestamps
  on arrival. Expect ~5 s app→target and ~15 s round trip with a trivially answering target.
- Health = `ready.json` present + row has a `pid`; re-arm expiry every 30 min is part of the steady state.

## 2026-10-22 — m0/5 gpui + terminal spike

`spikes/term-spike` (Variant A): git deps on Zed `gpui`, `gpui_platform`, `terminal`, `settings`,
`theme`, `theme_settings`, `release_channel`, `task`, `util`, `collections` at rev `03efd492…`; one
window, one terminal, painted by a ~300-line `canvas` adapted from `terminal_view/src/terminal_element.rs`
(text runs + background rects + cursor; no selection, hyperlinks, IME, mouse). Machine: Windows 11,
24 logical cores, NVIDIA RTX 3080, MSVC 14.44, Windows SDK 10.0.26100.

**Build facts**
- **Toolchain**: rustc 1.98.1 via repo-root `rust-toolchain.toml` (profile minimal). Machine default
  (`stable`) unchanged (`rustup default`).
- **CMake is a hard build requirement**, and not because of anything terminal-related: `terminal` →
  `settings` → `migrator` → `tree-sitter` with `features = ["wasm"]` (`crates/migrator/Cargo.toml:22`,
  unconditional) → `wasmtime-c-api-impl` 48.0.7. Seen: `cargo tree -i wasmtime`. That crate's
  `build.rs:40` does `Command::new("cmake")`, so it **needs `cmake` on PATH and ignores `$CMAKE`**
  (a `.cargo/config.toml` `[env] CMAKE=` was tried and failed identically). CMake ships with VS 2022
  but is not on PATH. → `scripts/cargo.ps1` prepends VS 2022's CMake 3.31.6 bin dir to its own
  process PATH; every build runs as `pwsh -File scripts/cargo.ps1 build --release -p term-spike`.
  The link then emits ~50 benign `LNK4217` warnings (tree-sitter importing wasmtime symbols).
- **Git deps do not use Zed's `Cargo.lock`.** A fresh resolve picked rust-embed 8.13.0; Zed pins
  8.11.0, and `settings` failed to compile (`EmbeddedCompressedFile` not found in
  `util::__rust_embed`). Fix: seed our `Cargo.lock` from Zed's at the same rev and let cargo prune it
  (`cargo fetch` then locked 50 packages: the Zed git crates plus `calloop` and `khronos-egl`). → Foreman must re-copy Zed's lockfile on every
  rev bump, or expect crates.io drift breakage.
- `[patch.crates-io]` copied from Zed: `tree-sitter-language`, `async-process`, `async-task`,
  `notify`, `notify-types`. The other Zed patches (cpal, livekit, webrtc, khronos-egl, calloop,
  windows-capture, scratch) are not needed by this tree.
- **Clean release build** (`target/` removed by `cargo clean`; registry and git checkouts warm):
  **3m 31s** (cargo's `Finished` line; an earlier `-q` run was ~3m 18s wall clock), 487 `Compiling`
  lines in the log.
  Default release profile (opt-level 3, no LTO; Zed's own release uses thin LTO + cgu=1).
- **Incremental rebuild** after touching `main.rs`: **5.23 s**.
- **Binary size**: `target/release/term-spike.exe` **17,244,160 B (16.4 MiB)**, default release
  profile, no LTO. wasmtime is linked in (the `LNK4217` lines resolve its symbols into the exe), and
  cranelift-codegen, prost/protox and Zed `proto` are compiled as part of the build. Their share of
  the 16.4 MiB was not measured.

**Runtime facts** (`term-spike --selftest`; load = 24 busy threads in-process, one per logical core,
xorshift spin for 10 s, at normal priority competing with the UI thread)
- **GPU backend**: gpui_windows picked **Direct3D 11, feature level 11.1** on the RTX 3080 (its own
  `log::info!` lines, printed by the spike's stderr logger). DirectWrite text; UI font fallback Segoe UI.
- **Frame time** = interval between successive `SpikeView::render` calls while the view calls
  `window.request_animation_frame()` each frame. `render` only runs inside `Window::draw`, which gpui
  immediately follows with `Window::present` (`gpui/src/window.rs:1910-1911`), so each interval is one
  drawn + presented frame (full terminal prepaint: `set_size` + `sync` + grid layout + paint). It is
  CPU-side cadence, not GPU scan-out time. The window was active in every run, so gpui's 30 fps
  inactive-window throttle was not in play.
- **The three runs, verbatim:**

  ```
  run 1: pwsh prompt after 970 ms (from process start)
         idle private bytes: 97.4 MiB (102129664 B) (working set 62.8 MiB (65896448 B))
         private bytes under load: peak 99.5 MiB (104284160 B) / at end 98.8 MiB (103616512 B) (peak working set 64.1 MiB (67203072 B))
         frame time under load: p50 16.55 ms, p95 19.60 ms, max 25.78 ms over 600 frames
         nonce seen after 137 ms (FOREMAN-TERM-b77cd210)
  run 2: pwsh prompt after 1037 ms (from process start)
         idle private bytes: 97.6 MiB (102334464 B) (working set 62.6 MiB (65634304 B))
         private bytes under load: peak 99.7 MiB (104505344 B) / at end 99.1 MiB (103874560 B) (peak working set 64.0 MiB (67072000 B))
         frame time under load: p50 16.51 ms, p95 19.63 ms, max 25.65 ms over 601 frames
         nonce seen after 124 ms (FOREMAN-TERM-e566ef44)
  run 3: pwsh prompt after 977 ms (from process start)
         idle private bytes: 97.4 MiB (102170624 B) (working set 63.1 MiB (66113536 B))
         private bytes under load: peak 99.3 MiB (104169472 B) / at end 98.6 MiB (103428096 B) (peak working set 64.2 MiB (67362816 B))
         frame time under load: p50 16.64 ms, p95 19.08 ms, max 24.74 ms over 600 frames
         nonce seen after 124 ms (FOREMAN-TERM-c8bc1d0c)
  ```
  All three exited 0. An earlier run during development: p50 16.67 / p95 18.12 / max 20.42 ms.
- **Reading**: under a fully saturated CPU, the window held vsync (60 Hz, p50 ≈ 16.6 ms), with p95
  < 20 ms and worst frame < 26 ms. **~97 MiB private / ~63 MiB working set** idle for one terminal;
  load adds ~2 MiB. Private bytes are our process only (the pwsh child, under ConPTY, is not counted).
- **Nonce**: `echo FOREMAN-TERM-<hex>\r` written to the PTY 0 ms after the first `PS ` prompt appears
  in the grid; it counts as seen when the nonce occurs ≥ 2 times in `Terminal::get_content()` (typed
  line + output), polled every 5 ms. 124–137 ms. The pwsh prompt (`-NoLogo -NoProfile`) appears ~1 s
  after process start.
- Not verified: visual correctness of the painted grid (no screenshot taken; the nonce check reads
  the emulator grid, not pixels); keyboard input in interactive mode; `--attach` (left to the
  controller).

**Variant B — vendoring `terminal` (assessed, not built)**

What the non-test code of `crates/terminal` (≈6.6k lines: `terminal.rs` 1–3661, `alacritty.rs` 1234,
`alacritty/hyperlinks.rs` 1–483, `mappings/` 773, `pty_info.rs` 292, `terminal_settings.rs` 1–168)
actually takes from each Zed sibling, and what a vendored copy would do instead:

| Dep | Used for (file:line) | Vendored replacement |
|---|---|---|
| `settings` (drags `fs`, `migrator`→tree-sitter[wasm]→**wasmtime + CMake**, `settings_json`, `settings_content`, `paths`) | `TerminalSettings` + `impl Settings` (`terminal_settings.rs:21-142`); `AlternateScroll` re-export (`:6`); `TerminalSettings::get_global` at `terminal.rs:1830` (keep_selection_on_copy), `:2747` (open_links_in_mouse_mode), `:2821` (mouse_up) | plain `TerminalConfig` struct held by `Terminal`; local `AlternateScroll` enum. **Cut.** |
| `theme` | `cx.theme()` for OSC colour queries (`terminal.rs:1720`); `get_color_at_index(&Theme)` (`:3575`) | pass a 16-colour + fg/bg palette struct. **Stub.** |
| `theme_settings` | only `FontFamilyName` (`terminal_settings.rs:14`) | drop with `TerminalSettings`. **Cut.** |
| `task` (drags `proto` → prost/protox codegen, `zed_actions`) | `Shell` (`terminal.rs:31`), `SpawnInTerminal`/`HideStrategy`/`TaskState` task mode (`:965`, `:1596`, `:3188-3304`) | copy the `Shell` enum (~20 lines); delete task mode (Foreman only runs a shell or `claude attach`). **Cut.** |
| `release_channel` | `AppVersion::global` for `TERM_PROGRAM_VERSION` (`terminal.rs:1111`) | a version string arg. **Cut.** |
| `util` | `PathStyle`, `ResultExt`, `truncate_and_trailoff`, `shell::{get_windows_system_shell, ShellKind}` (`util/src/shell.rs`), `command::new_std_command`, `process::Child` (headless path `terminal.rs:3365-3400`) | keep `util` as a git dep (no settings/wasmtime in its tree), or copy ~5 functions + `ShellKind`. **Keep.** |
| `collections`, `gpui` | `HashMap`/`VecDeque`; entities/executor | keep (gpui is the point). |
| `alacritty_terminal` (Zed fork rev `4c129667`), `vte`, `sysinfo`, `windows` | emulator, PTY, process info | keep as-is. |

Effort: ~3–4 h to get a vendored crate compiling (mechanical edits concentrated in
`terminal_settings.rs`, the three `get_global` sites, the task-mode branches, colour lookup), plus
porting or dropping the ~2.8k lines of tests, which mostly build a `SettingsStore::test` (budget a
further ~half day for that). Over the brief's ~2 h bar, so not built.

**A vs B recommendation**: **ship A now, plan B before Foreman's first real build.** A works on this
machine with real numbers (60 fps under full load, ~97 MiB private, 124–137 ms keystroke-to-grid
round trip), so the terminal approach is sound. But A's costs are almost all incidental to the
terminal: CMake on PATH, a lockfile that must be re-copied from Zed per rev, wasmtime + cranelift +
protobuf codegen in a terminal binary, and ~490 crates / 3.5 min cold builds. B removes `settings`,
`task`, `theme_settings` and `release_channel` (and with them `migrator`/tree-sitter/wasmtime/CMake,
`proto`, `fs`) for ~1 day of work. It keeps `gpui` + `util` + `collections` as git deps and
`alacritty_terminal` as-is, so it still tracks Zed for the hard parts. Binary size and memory savings
from B are expected but **not measured**: this spike only measured A.

## 2026-10-22 — controller observations, second half of M0 and the ws run

- **A tool's schema is fixed at session start, like its registration.** After ws/8 added `tree:` to
  `work_status`, this session's `work_status` still had no such parameter; the live server accepted it
  over raw JSON-RPC. → Foreman restarts (or respawns) a session after upgrading WorkState if it needs a
  new parameter, and its own client must not cache tool schemas across a server upgrade.
- **Parallel units need per-worktree claims** (ws/8, now live): pass `tree:"<worktree path>"` to
  `work_status(in_progress)`. Paths are case/slash-normalised on Windows and lock server-wide; branch
  names lock per workspace. Without `tree:` a run allows one in-progress unit.
- **Back up the store before a migration reaches it.** `docker exec workstate-db-1 pg_dump -Fc` (126 KB
  for this whole record). In Git Bash set `MSYS_NO_PATHCONV=1`, or `/tmp/...` inside the container is
  rewritten to a Windows path.
- **A deferral to `"backlog"` lands inside the same run** and blocks the run's close (WorkState F-06);
  use a project-level `backlog` run. A run is addressed by `<run>/<run>` when its ref is ambiguous (F-07).
- **The read side merged same-ref items across runs** (`m0/2` showed `ws/2`'s criteria, evidence, commit;
  DB rows were right) — fix in flight as ws/10. Until it lands, do not trust `work_get`/`work_tree`
  detail for refs that repeat across runs; ask the database or keep refs unique per project.
- **Integrated gate hitch:** one 896 ms frame in the term-spike selftest while a .NET build ran
  concurrently (earlier max 21–26 ms). Cause unproven; M1's terminal unit measures hitches under external load.
- **Agents found real bugs outside their brief and stopped to ask** (notes consumed unseen, both-runs
  assumption resolve, deferral ref collision, a test-signature conflict under C4). Every one was right.
  The routes in the brief ("STOP and report", "send BLOCKED with the numbers") are what made that safe.
- **Steering an idle agent into the next adjacent unit** (ws/2 → rule-count cap, ws/3 → ws/4, ws/5 → 6, 7)
  cost far less than a fresh launch and kept one commit per unit.

## 2026-10-22 — m1/1 terminal Variant B vs A

`crates/foreman-term` (Variant B) is Zed's `crates/terminal` @ `03efd492` copied and cut:
- `TerminalSettings` + `impl Settings` are replaced by a plain `TerminalConfig` gpui global (`terminal_settings.rs`;
  defaults = Zed's `default.json`), and `AlternateScroll` is a local enum.
- Task mode (`TerminalMode::task`, `TaskState`, `task_summary`, `kill_active_task`, `wait_for_completed_task`)
  is deleted. `Shell`/`ShellKind` come from `util::shell` (`task` only re-exported them).
- `AppVersion` is replaced by `TerminalConfig::program_version`.
- OSC colour replies and `get_color_at_index` take a `TerminalPalette` (16 ANSI + 8 dim + fg/bg/bright fg/cursor)
  instead of `&Theme`.
- The test modules of `terminal.rs` and `alacritty/hyperlinks.rs` are dropped (they build a `SettingsStore`).

It keeps `gpui`, `util` and `collections` as git deps, plus the alacritty fork, `vte` and `sysinfo`.
`cargo tree -p foreman-term` contains none of `settings`, `theme_settings`, `task`, `release_channel`,
`migrator`, `tree-sitter*`, `wasmtime*` or `proto` (`cargo tree -p foreman-term -i settings` → "did not match
any packages"). Both foreman-term and term-spike B build with plain `cargo`, no CMake on PATH.

**Variant switch needs `--no-default-features`.** Cargo features only add, so `--features vendored` on its
own still builds A's crates and CMake. term-spike has `default = ["zed"]` (A) and `vendored` (B), with
`compile_error!` if both or neither are on:
- A: `pwsh -File scripts/cargo.ps1 build --release -p term-spike`
- B: `cargo build --release -p term-spike --no-default-features --features vendored`

B's term-spike still depends on Zed `theme` (no settings in its tree) and calls `theme::init(JustBase)`, so
`term_element.rs` paints both variants from the same theme. Only the indexed-colour lookup is cfg'd.

**Selftest additions:** frames > 50 ms / > 100 ms plus the three worst frames with their offset into the window.
`--selftest-external` uses an external load instead of the 24 in-process spinner threads: it wipes
`target-load/release`, starts a release build of the *other* variant with `CARGO_TARGET_DIR=target-load`
immediately before the 10 s window, and kills it with `taskkill /T` afterwards. A's exe builds B with plain
cargo; B's exe builds A through `scripts/cargo.ps1`. The build compiled 153–166 crates in the window and was
still running at its end in every run.

Same machine as m0/5. Cold build = `cargo clean`, then `Measure-Command` wall clock with registry and git
checkouts warm. Incremental = `touch main.rs` and rebuild. Runtime = the copies in `target-bin/`, 3 runs each.
Values are listed run 1 / run 2 / run 3.

| | A (Zed `terminal` git dep) | B (`foreman-term` vendored) |
|---|---|---|
| Needs CMake / `cargo.ps1` | yes (wasmtime-c-api build.rs) | **no** |
| Cold release build | 186.9 s | **102.8 s** (−45 %) |
| Incremental after touching main.rs | 6.03 s | 4.09 s |
| Crates (`cargo tree --prefix none`, unique) | 487 | term-spike B **350**; foreman-term alone 326 (term-spike's own deps — Zed `theme`, `gpui_platform`, … — add 24) |
| Binary size (release, no LTO) | 17,284,096 B (16.5 MiB) | **12,005,888 B (11.4 MiB)** (−31 %) |
| Idle private bytes, in-process runs | 97.7 / 97.2 / 97.6 MiB | 98.4 / 97.5 / 98.0 MiB |
| Peak private bytes under in-process load | 99.8 / 99.8 / 99.8 MiB | 98.9 / 99.4 / 99.1 MiB |
| Peak private bytes under external load | 98.3 / 98.2 / 98.4 MiB | 98.0 / 98.1 / 97.8 MiB |
| In-process load: p50 | 16.67 / 16.67 / 16.67 ms | 16.67 / 16.66 / 16.66 ms |
| In-process load: p95 | 18.05 / 17.95 / 17.98 ms | 18.04 / 18.18 / 17.89 ms |
| In-process load: max | 20.24 / 19.59 / 19.24 ms | 19.03 / 20.06 / 18.70 ms |
| In-process load: frames > 50 / > 100 ms | 0 / 0 in all 3 | 0 / 0 in all 3 |
| External load: p50 | 16.67 / 16.66 / 16.67 ms | 16.67 / 16.66 / 16.66 ms |
| External load: p95 | 18.32 / 18.26 / 18.07 ms | 18.33 / 18.37 / 18.38 ms |
| External load: max | 21.06 / 20.50 / 20.47 ms | 30.82 / 20.91 / 33.35 ms |
| External load: frames > 50 / > 100 ms | 0 / 0 in all 3 | 0 / 0 in all 3 |
| Keystroke-to-grid (nonce), in-process runs | 123 / 108 / 124 ms | 120 / 109 / 124 ms |
| Keystroke-to-grid (nonce), external runs | 123 / 124 / 121 ms | 137 / 109 / 137 ms |

**Reading:**
- Runtime is the same. Both variants hold vsync (p50 16.67 ms) with p95 < 18.5 ms under either load.
  Memory is within ±1 MiB: the cut code (settings store, wasmtime, protobuf) is linked into A but never
  touched at runtime. ~97 MiB private is gpui + D3D11 + DirectWrite + alacritty.
- B's two 31–33 ms frames came while it was building **A** (wasmtime/cranelift crates, heavier per job). A
  never ran under that load, so this is not a terminal-crate difference.
- The cost of A is all build-side: CMake, +40 % crates, +84 s cold build, +5.0 MiB binary.

**The M0 896 ms hitch did not reproduce, for A or B.** No frame over 50 ms in 12 runs, 6 of them while a
concurrent `cargo build` saturated the machine. So the cause is not the terminal crate. Differences from the
M0 gate, any of which could be it: that load was a **.NET build**, not cargo. That run's window state was not
recorded; here the window was active from start to end of every window (`window active: true / at end true`).
A window that is not in the foreground would get gpui's 30 fps throttle and lower scheduler priority, which
could produce one long frame; this is unverified. And the window here only covers the first 10 s of a cold
build (proc-macros and small crates); a link step or LTO phase was never inside it. Not tested: an inactive
window, a dotnet load, the link phase.

**Recommendation: switch Foreman to B.** Same frame times, memory and latency. It builds with plain cargo,
45 % faster cold, 31 % smaller binary, 137 fewer crates. Maintenance is a ~4k-line copy to re-sync per Zed rev
bump; the hard parts (alacritty fork, gpui) are still deps.

**`scripts/cargo.ps1` can be dropped** once Variant A is retired. Nothing in B's tree needs CMake. Until
then only A builds use it, including the `--selftest-external` load of a B exe. B does **not** remove the
Zed-lockfile caveat: `gpui`/`util` are still git deps and resolve against the Cargo.lock seeded from Zed.
The foreman-term build added 38 packages to the lock without breakage, but rev bumps should still re-seed.

## 2026-10-22 — m1/2 Variant A retired, foreman-term tests ported

**A is gone.** `term-spike` depends on `foreman-term` only (no `zed`/`vendored` features); `scripts/cargo.ps1`
is deleted. `--selftest-external`'s load is now a second cold `cargo build --release -p term-spike` into
`target-load/`. `Cargo.lock`, regenerated by `cargo build` (no `cargo update`): **824 → 678 packages**, 146
removed, none added, no surviving package changed version. Gone: `wasmtime*`, `migrator`, `tree-sitter*`,
`settings*`, `theme_settings`, `task`, `release_channel`, `proto`/`prost*`, Zed's `terminal`, `notify`/`notify-types`.
- Removing a dep can strand a `[patch.crates-io]` entry: cargo then writes `[[patch.unused]]` into
  `Cargo.lock` and warns. `tree-sitter-language` and `notify`/`notify-types` had to leave the root
  `Cargo.toml` with A; `async-process`/`async-task` are still used by gpui.
- Selftests with plain cargo, no CMake on PATH (one run each): in-process load p50 16.59 / p95 19.55 / max
  24.25 ms, external load p50 16.62 / p95 20.41 / max 31.82 ms, 0 frames > 50 ms, ~98 MiB private, nonce
  108–123 ms. Same as m1/1's B.

**Test port: 58 of Zed's 60 `terminal.rs` tests and all 37 `hyperlinks.rs` tests.** Assertions are Zed's;
only setup changed:
- `SettingsStore::test` + `theme_settings::init` → `cx.set_global(TerminalConfig::default())`; the one
  settings mutation (`open_links_in_mouse_mode = false`) is `cx.update_global::<TerminalConfig>`.
- The hyperlink tests read Zed's default `path_hyperlink_regexes` / `path_hyperlink_timeout_ms` from
  `default.json`; those are copied into a test-only `default_terminal_settings()` (the multi-line regex is
  joined with `
` as `TerminalSettings::from_settings` does).
- Task-mode builders (`TerminalMode::task` + `wait_for_completed_task`) are now `interactive_with_completion`
  plus a test helper awaiting that channel; the exit-status assertions are unchanged and pass.
- `#[perf]` (util_macros) is a no-op unless built with `--cfg perf_enabled` beyond adding `#[test]`, so it
  is dropped/replaced by `#[test]` instead of adding `util_macros` + `perf` to the lock.
- Dev-deps, both from Zed's terminal crate: `gpui` with `test-support`, `rand 0.9` (already in the lock).
- Skipped: `test_kill_active_task_completes_and_captures_output`,
  `test_kill_active_task_on_completed_task_is_noop` — `kill_active_task` is task mode, not vendored.
- On Windows 110 tests run, all pass: 53 of the 58 (5 are `cfg`'d to unix/linux/macos upstream), 40 from
  hyperlinks (35 of the 37 + 5 `#[perf]` benchmarks that are tests too), 17 pre-existing.

## 2026-10-22 — m1/3 foreman-core registry + scanner, `fm platform`

- `fm platform add P:/Projects/Magic/Nomos --dry-run` → 13 repos, exactly the PLAN list; the four
  `*.worktrees`, `y_legacy`, `.local-nuget`, `.claude` are not listed. `%APPDATA%\Foreman` did not exist before
  or after. Branches read from `.git/HEAD`: main ×7, master ×4, `stage` (Nomos), `production` (Nomos.Extract).
  CLAUDE.md in 8 of 13, AGENTS.md in 3.
- Deps all already in `Cargo.lock`: `serde 1.0.229`, `toml 0.9.8`, `thiserror 2.0.17`, `chrono 0.4.42`
  (`now`+`serde`, no default features), dev `tempfile 3.27.0`. No `clap` in the lock → `fm` parses args by
  hand. Lock diff: +2 packages (`foreman-core`, `fm`), no version changed.
- On Windows a directory's mtime read through `read_dir` metadata can change between two reads with nothing
  written (NTFS refreshes the parent's cached copy lazily); a "scan writes nothing" test must compare file
  contents/mtimes and the directory set, not directory mtimes.

## 2026-10-22 — m1/4 `claude agents --json --all` on a busy machine, `fm sessions`

One read of `claude agents --json --all` (CLI 2.1.308, Windows 11), 236 rows; counts only, no names/paths kept.

| `kind` | `state` | `status` | `pid` | `waitingFor` | rows |
|---|---|---|---|---|---|
| background | stopped | — | — | — | 115 |
| background | **done** | — | — | — | 107 |
| background | working | busy | set | — | 1 |
| background | blocked | — | — | — | 1 |
| interactive | — | idle | set | — | 10 |
| interactive | — | busy | set | — | 2 |

- **Interactive rows have no `id` and no `state`** (all 12). `sessionId`, `name`, `cwd`, `kind`, `startedAt`
  are on all 236. → `foreman-claude` requires only `sessionId`, `cwd`, `kind`; `id` = `sessionId[..8]` when
  absent (m0/2: they are equal on background rows); a missing `name` becomes `""`.
- **`state: done` exists** and is the commonest terminal state after `stopped` (107 rows, no pid/status);
  m0/2 never produced it. → `SessionState::Done`, health `Stopped`.
- **A row can stay `state: blocked` after its process is gone** (1 row: no pid, status or waitingFor, started
  hours earlier). Attaching to it would target a dead process. → `Health::Orphaned { last_state }` for a
  background `working`/`blocked` row with no pid older than `ORPHAN_AFTER` = 60 s; younger stays `Running`,
  because m0/2's legitimate pre-process row (working, no pid) lasts ~1.8 s.
- `fm sessions --platform nomos` (temp `FOREMAN_HOME`, platform from `P:/Projects/Magic/Nomos`, 13 repos):
  8 non-stopped sessions in 2 repos, 7 interactive idle + the orphaned one, exit 0. The CLI's `where claude`
  resolves to `claude.exe`, so `Command::new("claude")` works without a shell.
- Deps: `serde_json 1.0.151` and `thiserror 2.0.17`, both already in `Cargo.lock`; lock diff +1 package
  (`foreman-claude`), no version changed. Timeout is a `try_wait` poll loop, so no `wait-timeout` crate.

## 2026-10-22 — m1/5 `foreman` fleet viewer, attach/detach gate

Measured with `target/release/foreman.exe --selftest` / `--selftest-perf` (CLI 2.1.308, same machine as m0/5).

- **`claude --bg` refuses an untrusted workspace**: in a freshly `git init`ed dir under this (trusted) worktree it
  exits 1 with `Workspace not trusted. Run \`claude\` in <dir> once and accept the trust prompt, then retry.` and
  creates no row. Trust follows the git root: a nested repo is a new workspace; a plain subfolder of a trusted
  repo/worktree is trusted. → Foreman cannot launch into a repo the user never opened Claude in; M2 must
  surface this as a state ("trust needed"), not edit `~/.claude.json`. The gate therefore registers a read-only
  scan of `P:/Projects/LarchSys` (13 repos) and launches from `<worktree>/target/selftest-cwd`.
- **Worktree mapping works live**: a session whose cwd is inside `AutoWork.worktrees/m1-5` (nearest `.git` is a
  file) is assigned to repo `AutoWork` by `assign()`'s gitdir path, 3 of 3 runs.
- **Launch → listed in the panel with a pid**: 3.29–3.42 s from window open (`--bg` itself returns in 0.79–0.87 s).
  Health was `running` (status busy) every time, never the pre-process row by then.
- **`claude attach <id>` in a foreman-term tab**: first output painted 324–381 ms after the click (the Claude Code
  banner). **Closing the tab detaches**: `release_pty_resources` (terminate, kill after 100 ms) → the attach
  process exited 20–36 ms later, and the session row stayed `running` with its own pid 2 s later. So the
  attach process is a client; the session process is separate (different pid).
- **Synthetic clicks are real**: `Window::dispatch_event` with MouseMove/Down/Up at an element's painted centre
  fires its `on_click` through gpui's normal hit test (hit test uses the event position, no frame needed).
  Bounds come from an absolutely positioned zero-paint `canvas` child that records its bounds in prepaint.
- **Idle**: 120.3–120.7 MiB private (59 MiB working set) with the panel only (term-spike, one terminal: ~98 MiB).
  Frames over 6 s while the panel refreshed twice (each refresh = registry read + `claude agents --json --all`
  on a background thread): p50 16.65–16.67 ms, p95 18.46–18.98 ms, max 19.98–31.91 ms, 0 frames > 50 ms.
- **`--selftest-perf`** (pwsh -NoProfile tabs, only the active tab is painted), two runs:
  0 tabs 119.3–119.7 MiB; **1 tab 127.4–130.2 MiB**, p95 18.33–18.44 ms, max 21.04–22.75 ms;
  **10 tabs 210.0–211.5 MiB**, p95 18.25–18.53 ms, max 20.13–22.04 ms; 0 frames > 50 ms. ≈ 9 MiB private per
  extra terminal (alacritty grid + scrollback in our process; the pwsh children are not counted).
- Painter moved to `crates/foreman-ui` (needs Zed `theme`, which foreman-term deliberately does not). Lock diff
  +2 packages (`foreman-ui`, `foreman-app`), no version changed; no new external dependency.

## 2026-10-22 — m1/6 selftest repo discovery; the cold-start frame hitch

- **`foreman --selftest` failed from the main checkout** (controller's M1 integrated gate, master c9ce10a):
  `FAIL setup: scan of P:\Projects found no AutoWork repo`, exit 1. The platform root was `workspace_root()`'s
  grandparent, which is only right for `<root>/AutoWork.worktrees/<unit>`; from `<root>/AutoWork` it is
  `P:\Projects`. It had passed only in the m1-5 worktree. → Fix: walk up from `workspace_root()` to the nearest
  `.git` with `foreman_claude::main_repo` (was the private `worktree_main` in `assign.rs`, now public and
  case-preserving): a `.git` directory is the main repo, a `.git` file's `gitdir: <repo>/.git/worktrees/<n>`
  (relative or absolute) points at it. Platform root = its parent, expected repo = its folder name. No `.git`
  at or above → a setup error naming the path. The launch cwd is still `<workspace>/target/selftest-cwd`.
  From this worktree: `platform larchsys = P:/Projects/LarchSys (13 repos)`, session listed under
  `./AutoWork`, PASS. The main-checkout run is the controller's post-merge gate.
- **Cold-start hitch**: two frames of ~880–900 ms were seen, each on the **first** run of a freshly built exe
  (M0 integrated gate: term-spike, 896 ms; M1 integrated gate on master c9ce10a). Over ~30 warm runs:
  0 frames over 50 ms. Suspected: Defender scanning the new exe (or a DLL it maps) on first
  launch, or GPU shader-cache warm-up for a new binary; **neither is proven**. Not seen in m1/6's own first runs
  of fresh builds (foreman max 27.89 ms, term-spike max 19.75 ms), so it is not every first run.
  → Both selftests now print the worst frame's time in ms since process start (`foreman_ui::process_start()`,
  taken first thing in `main`): foreman `max 27.89 ms (at +8975 ms since process start)`; term-spike
  `worst: 19.7 ms at +7496 ms (+9536 ms since process start)`. A hitch at a few seconds after start on a first
  run reads as cold start; one late in the window, or on a warm run, as load. Note foreman's frame window opens
  ~9 s after start (after attach/detach), so a hitch in the first seconds is outside it.

## 2026-10-22 — m1/7 closing an Attach tab: the "stop" was a misread `done`; safe detach

Measured with `crates/foreman-app/diag/states.ps1` and `target/release/foreman.exe --selftest-detach <n>` /
`--selftest-detach-shutdown` / `--selftest` (CLI 2.1.308, Windows 11, worktree m1-7).

- **The defect.** The M1 gate's `--selftest` failed 2 of 4 from the main checkout with `fm-selftest-… is listed but
  stopped — the close stopped it instead of detaching`. **Nothing stopped it.** A control session that is never
  attached (`diag/states.ps1`, raw `claude agents --json --all` row every 0.5 s) goes `state=done` on its own with
  its process alive, as soon as its turn ("Reply with the word ready…") ends:

  | Run | no-pid row | pid up | `status=busy` | **`state=done status=idle`, same pid** |
  |---|---|---|---|---|
  | 1 | +3.2 s `working` | +6.3 s pid 27304 (`working`, `idle`) | +12.6 s | **+18.3 s, pid 27304** |
  | 2 | +1.3 s `working` | +2.3 s pid 20192 (`working`, no status) | +5.6 s | **+8.8 s, pid 20192** |

  m1/4 mapped `done` → `Health::Stopped` from 107 `done` rows that all had no pid/status. The selftest checked
  liveness ~7–9 s after launch, so whether it read `working` or `done` was the turn's timing: the ~50 %.
  → **`state` describes the turn, not the process.** Health is now derived from `pid` first for background rows
  (`done`/`stopped` with a pid → `Idle` or whatever `status` says; without one → `Stopped`); table in
  `foreman-claude/src/lib.rs`.
- **Process chain** (Win32_Process ParentProcessId, both control runs): session `claude.exe --session-id <id>` ←
  `claude.exe --bg-pty-host \\.\pipe\cc-daemon-…-pty-<id8> 200 50 -- …` ← `claude.exe daemon run --origin transient`
  (the supervisor in `~/.claude/daemon/roster.json`, one for all bg sessions) ← WmiPrvSE. `claude attach` is never
  an ancestor of the session. foreman-term spawns it with plain `CreateProcessW` + `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`,
  no job object; `release_pty_resources` on Windows = `ClosePseudoConsole` then `TerminateProcess` of that one pid
  100 ms later (no tree kill, `terminate_child_process` is a no-op off unix).
- **Detach input.** Typing `0x1A` (Ctrl+Z, `claude attach`'s "drop back to your shell") into the attach tab ends the
  attach process by itself in **404–555 ms** (40 closes below). `ESC [ D` (←, "returns to agent view") does **not**:
  the attach process was still running 5 s later (1 run) and the tab went to "detach failed" as designed.
- **Safe close** (`view.rs` `close_tab`): an Attach tab types Ctrl+Z, waits ≤ 5 s (`DETACH_WITHIN`) for the attach
  process to exit, and only then releases the PTY. On timeout it kills nothing: the tab stays, titled
  `(detach failed)`, with a notice; closing it again retries. A window close (WM_CLOSE) with Attach tabs open
  detaches them first and closes the window when the last is gone, or stays open if one failed. Shell tabs close as
  before.
- **Results**, every close asserting the session row's pid is the same live process before and after:
  `--selftest-detach 10` ×3: **30/30 PASS**, detach 404–555 ms, rows seen `working/busy`, `done/idle`
  (most cycles), `blocked/idle`. `--selftest-detach-shutdown` ×2: PASS, window closed 417 / 436 ms after WM_CLOSE.
  `--selftest` ×3 (worktree): PASS, step 5 detach 458–531 ms. `--selftest-perf`: unchanged (1 tab 129.4 MiB,
  10 tabs 211.2 MiB, max frame 22.47 ms).
- Also seen: `state=blocked, status=idle` on a live pid with no `waitingFor` (2 `--selftest`, 1 shutdown,
  1 `--selftest-detach 2` run; why the turn reads blocked is not known). Health reads it as `Idle`.

## 2026-10-22 — m2/1 launcher probes

Measured with `cargo run -p fm -- probe launch` (runs 1–4; runs 3–4 are the final code, identical answers) and
`crates/fm/probe/resume.ps1 -Variant bare|prompt|name` (CLI 2.1.308, Windows 11). Sessions `aw-m2-probe-<hex>-*`,
cwd `<worktree>/target/aw-m2-probe-<hex>/<probe>` (plain subfolder, trusted). Every row was stopped + removed.

- **(a) Env on the `claude --bg` process does NOT reach the session.** With `FM_PROBE=<nonce>` set on `--bg`, the
  session's shell tool printed nothing (`ENV=EMPTY`), and a dummy http server in `--mcp-config`
  (`url …/mcp/${FM_PROBE}`, header `X-Fm-Probe: ${FM_PROBE}`, recorded by a local TCP listener in `fm`) received
  `POST /mcp/$%7BFM_PROBE%7D` with header `${FM_PROBE}`: an unset variable is left **literal**, no error. The daemon
  spawns the session (m1/7 chain), so the client's env is gone. 4/4 runs.
- **(a2) `--settings '{"env":{"FM_PROBE":"<nonce>"}}'` does reach it**: the tool printed the nonce, and the url and
  header in `--mcp-config` expanded to it. **But the value is written to `~/.claude/daemon/roster.json`** (the
  daemon's saved launch options; found by a byte search, file name only) for as long as the row exists; after `rm`
  it was gone (`grep -c` = 0). It is also in every tool's environment, so a stray `env` dump puts it in the
  transcript. 2/2 runs.
- **(b) `--mcp-config` beats the user-scope registration of the same name, even without `--strict-mcp-config`.**
  A `workstate` entry (`http://localhost:5290/mcp`, `Bearer fm-probe-invalid`) in `--mcp-config`: the session had no
  `mcp__workstate__*` tool at all and reported `workstate (AUTH_HEADER_REJECTED): Server rejected the configured
  Authorization header (HTTP 401). … OAuth fallback is disabled when headers.Authorization is set.` The valid
  user-scope server was not loaded beside it. Without strict: 4/4 runs (run 2 reported only `TOOL=absent`, no
  text). With `--strict-mcp-config` and the per-role file: runs 3–4, same 401 (runs 1–2's strict probe used an
  unexpanded `${WORKSTATE_TOKEN}` header and failed the same way). Evidence is the session's own report (written to a file by the Write tool); no
  `--debug` log was taken, because it could log headers.
- **(c)** Trust refusal not re-measured; m1/5's text is the `TrustNeeded` parser's fixture.
- **(d) Restart keeping the transcript = `claude --bg --resume <sessionId> [-- <prompt>]` with no other flag**, run
  from the session's cwd. It prints `note: woke session <id8> with its saved options (--name, --permission-mode).`
  then `backgrounded · <id8> · <name>` (`(idle — send a prompt to start)` without a prompt). Row before/after:
  same `id`, same `sessionId`, same `name`, **new `pid`** (e.g. `64acf621… pid 56040` → stopped `pid -` →
  `pid 65512`). The model answered from memory with the exact line it wrote before the stop, and the saved
  `--mcp-config` was re-applied (the dummy server got new requests after the resume).
  **Any extra flag starts a copy**: `--bg --name <n> --resume <sid>` printed `note: background session <id8> keeps
  its own saved options, so the flags you passed started a copy as <new id8>. Without flags, the same command
  continues <id8> itself.` → a second row with the same name and a new `sessionId`; the old row stays
  (`stopped`/`done`, no pid). The copy also carries the transcript. `claude respawn <id>` only restarts a live
  session on the current binary; it is not a resume.
- **Names are not unique in the CLI.** `--bg --name <n>` while a (stopped) row named `<n>` exists is accepted
  (`backgrounded · <new id8> · <n>`), leaving two rows. After `rm`, the name is free and a new launch gets a new
  `sessionId`. → `Claude::launch` refuses with `NameTaken` when any `--all` row has the name;
  `launch_unchecked` is the bare CLI.
- After `claude stop`, the row's `state` was `done` (turn had ended) or `stopped` (turn still running), always
  without `pid`: `stop` does not normalise `state`; pid is the signal (m1/7).
- **Decided role-token route**: the preferred one (token as env on `--bg` + `Bearer ${WORKSTATE_TOKEN}`) cannot
  work — (a). `--settings` env works but persists the token in `roster.json`, outside Foreman's control, and hands
  it to every tool. → **Fallback: `<FOREMAN_HOME>/mcp/workstate-<role>.json`** holding the literal header, created
  empty, ACL'd owner-only (`icacls /inheritance:r /grant:r <USERNAME>:F`: one ACE, the user, full control; 0600 on
  unix), then filled; launched with `--mcp-config <file> --strict-mcp-config`. Only the path is in argv and in
  `roster.json`. `LaunchSpec::with_role_token(dir, role, url, token)`; the caller reads the token. Verified by
  probe b2 with the invalid token: the 401 above.
- `--mcp-config <configs...>` is variadic, so the prompt is passed after `--`; `claude --bg … -- <prompt>` works.
- `foreman --selftest` after this unit: PASS ×2; but frame p95 was 231 ms (first run of the fresh exe) and 61 ms
  (second), against m1/7's ~18 ms. `foreman-app` is not changed by this unit; the cause was not investigated.
- Deps: `fm` gains `serde_json` (already in the lock, same version); `foreman-claude` gains an example
  `fake_claude` for its integration test. No new external dependency.

## 2026-10-22 — m2/3 foreman-supervisor: desired state, pure reconcile, backoff

Design decisions only; this unit launched no session and measured nothing live. Rule table (rule → test) in
`crates/foreman-supervisor/src/lib.rs`.

- **Liveness is `is_live`**: a pid, or the fresh pid-less `working`/`blocked` row right after `--bg` (B11/B12,
  before `ORPHAN_AFTER`). `state` is never read for liveness, so `done` + live pid (m1/7) is left alone. A live
  wanted session is never launched or resumed; a `Blocked` one becomes an Inbox row (Attach), never a restart.
- **Absent → Launch, present but not live → Resume** (`--bg --resume <sessionId>` with no flag, from the
  **row's** cwd, m2/1). Not started → `Stop` every live wanted session; never `rm`. Two rows with one wanted name →
  `Duplicate`, none touched. Rows whose name is not wanted are never touched.
- **Settle window (20 s)** after a successful launch/resume: the row trails `--bg` by ~1 s and the pid by up to
  ~6 s (m0/2, m1/7), so an absent row then means "starting", not "launch again" (`Wait(Settling)`). Without this a
  1 s tick would double-launch: `Claude::launch`'s `NameTaken` check races the row.
- **Failures**: a launch/resume error, or a start that is dead after `settle` but within `stable_after` (60 s).
  Backoff after failure 1–4: 5 s, 30 s, 2 min, 10 min; the 5th gives up (`GaveUp`, Retry clears). 60 s alive
  clears the count. `TrustNeeded` is not counted and never retried: Inbox row with `RunInTerminal{cwd, "claude"}`
  until Retry. Memory is in-process: an app restart forgets failures (and retries once).
- **Cap**: live `fm-` rows with `state=working` take slots of `Policy::max_working` (default 3); Resumes before
  Launches; the rest `Wait(Cap)`. Counted from one platform's tick: two platforms ticking in the same snapshot can
  overshoot by their own launches — the app should tick platforms one after another on fresh snapshots, or share a
  budget (M2/4's call).
- **cwd is a field the app chooses** (`WantedSession::cwd`): the platform root is usually not a git repo, so not a
  trusted workspace (m1/5). The prompt comes from `plugins/foreman/roles/<role>.md` with `{platform}` filled
  (`role_prompt`); `platform.md` is a placeholder until M3.
- Tokens come through `TokenProvider::token(role) -> Option<RoleToken>` and go to `LaunchSpec::with_role_token`
  (owner-only file under `LaunchConfig::mcp_dir`); `None` launches without WorkState. Tests use `fake-token`.
- Deps: `foreman-supervisor` uses foreman-core, foreman-claude, serde, thiserror, toml (all already in the lock);
  dev tempfile. No new external dependency. `cargo tree` shows no gpui/foreman-ui/foreman-term.
## 2026-10-22 — m2/2 hook ingest

Measured with `cargo run -p fm -- probe hooks` (runs 1–8; runs 7–8 are the final code) and throwaway `claude -p`
sessions `aw-m2-probe-expand*` from `<worktree>/target/aw-m2-probe-expand/cwd` (Windows 11; CLI 2.1.308 at the
start, **auto-updated to 2.1.309 during the unit** — runs 5–8 ran on 2.1.309 per their debug logs). Every
`aw-m2-probe-*` row was stopped + removed by the probe's cleanup (`0 … left` every run).

- **How a plugin hook finds `fm.exe`: `${CLAUDE_PLUGIN_ROOT}/bin/fm.exe`, exec form.** In an exec-form hook
  (`command` + `args`) Claude Code expands only its own placeholders: `"command": "${FOREMAN_FM}"` failed with
  `Hook command failed to spawn (SessionStart:startup): Executable not found in $PATH: "${FOREMAN_FM}"` (debug log),
  although `FOREMAN_FM` was in the session env via `--settings`. Shell form `"$FOREMAN_FM" hook X` worked (bash
  expands it) but depends on Git Bash being present (else the hook runs in PowerShell, where that line does nothing).
  → `plugins/foreman/hooks/hooks.json` uses `${CLAUDE_PLUGIN_ROOT}/bin/fm.exe`; `fm plugin install <dir>` copies the
  running `fm.exe` there (gitignored `bin/`; written as `fm.exe.new` and swapped in by rename, since Windows will
  not overwrite an exe a hook is running but will rename it).
- **`--plugin-dir` must not be a `\?\` verbatim path.** Run 1 passed `canonicalize()`'s `\?\P:\…\plugins\foreman`;
  Claude Code appends `/bin/fm.exe`, which is invalid in a verbatim path: not one event was logged in either
  session, the park session never parked in 180 s and the compact session never ended its turn in 480 s. With the
  plain path (prefix stripped) everything fired.
- **`--settings '{"env":{…}}'` reaches hook processes** (FOREMAN_HOME / FOREMAN_ROLE were seen by `fm hook`), as it
  reaches tools (m2/1).
- **Events observed through the plugin** (all `--bg` unless noted; `fm hook` keeps `ts, event, session_id, name
  (=session_title), source?, notification_type?, trigger?, cwd, transcript_path?` — never `prompt` or
  `last_assistant_message`; a reply is read back from `transcript_path` instead):

  | Event | When | Kept discriminator | Note |
  |---|---|---|---|
  | SessionStart | launch, ~0.4 s before UserPromptSubmit | `source=startup` | stdout = role injection |
  | UserPromptSubmit | launch prompt; each `--resume -- <prompt>` | — | |
  | Notification | Bash parked on permission, 8.9–10.5 s after start | `notification_type=permission_prompt` | |
  | Notification | ~60 s after a turn ended in an auto-compaction loop (runs 2–4) | `notification_type=idle_prompt` | no Stop that turn |
  | PreCompact | auto-compaction mid-turn | `trigger=auto` | one bailed: "Reactive compact: no assistant messages in summarize set" |
  | SessionStart | after each compaction, ~10 s after PreCompact | `source=compact` | stdout = role injection |
  | SessionStart | `--bg --resume <sid> -- <prompt>` | `source=resume` | the plugin + `--settings` env are part of the saved options: hooks fired, role injected |
  | Stop | turn end | — | **no `session_title`** in the Stop payload → `name` is empty on Stop lines |

- **Auto-compaction can be triggered cheaply (closes 6-d)**: `--autocompact 100k` + `--settings` env
  `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=80`, `CLAUDE_CODE_AUTO_COMPACT_WINDOW=100000`, `CLAUDE_CODE_MAX_OUTPUT_TOKENS=8000`
  + `--strict-mcp-config`, then **one** ~80 KB word file `Read` per turn (`--bg --resume <sid> -- <prompt>` between
  turns; the first one already crossed). Debug log: `autocompact: … level=ok effectiveWindow=92000` → `level=compact`
  → `routing through reactive (thresholdSource=env)`; run 7: PreCompact(auto) +9.9 s (bailed: "no assistant
  messages in summarize set"), PreCompact(auto) +15.3 s, SessionStart(compact) +27.8 s, Stop +33.2 s. Pitfalls
  measured on the way:
  - the effective window is the window **minus the output reserve**: `--autocompact 100k` alone gave
    `effectiveWindow=80000` (−20k); with the model's `maxOutputTokens` 128000 not capped, and this user's MCP servers
    loaded, `--bg` sessions compacted on the first call and kept re-compacting every ~13 s (runs 2–4, PCT 50 and 80
    alike), then ended the turn without a Stop. A fresh `-p` session's baseline here is ~50.5k tokens
    (`cache_creation 30840 + cache_read 19628`); `--strict-mcp-config` lowers it enough that the loop stops.
  - **several big reads in one turn loop**: after a compaction the next `Read` refills past the threshold, it
    compacts again, … until `autocompact: rapid-refill breaker tripped — 3 consecutive refills within <3 turns each`
    and the turn ends with `Notification(idle_prompt)` and **no Stop** (run 6; run 5, four reads, passed by luck).
  - every compaction re-fires SessionStart(compact); a compaction during the *question* turn (runs 2–4) replaces
    the earlier injection, so the proof needs a turn that does not compact.
- **Role re-injection after auto-compact works** (runs 5, 7, 8). The role file was rewritten v1 → v2 right after SessionStart
  (startup) and v2 → v3 before `--bg --resume`; asked to list every `FOREMAN-ROLE` line, the session wrote the v2 tag
  (only the compact-time SessionStart could have injected it) and the v3 tag (resume), not v1 (dropped by the
  compaction). Debug log shows the injection verbatim: `Hook SessionStart:compact (SessionStart) success:\nFOREMAN-ROLE
  probe v3e80ac1f\nYou are the Foreman m2/2 probe role, version 2 …`.
- **`fm hook` cost** (release, `crates/fm/probe/hook-timing.ps1`, SessionStart with a 1 KB role, 50 runs, spawn to
  exit): min 9.1 ms, median 10.8 ms, p95 30.0 ms. Claude Code itself reported 30–107 ms per foreman hook in the debug
  log (its own spawn overhead included; hooks of other plugins took 340–500 ms).
- Deps: `foreman-core` gains `serde_json` and `sha2` 0.10.9 (both already in the lock, same versions). No new
  external dependency.
- **Controller's gate from the main checkout (2026-10-23, CLI 2.1.309): `--bg` sessions isolate themselves.**
  The gate's question turn, run from `P:/Projects/LarchSys/AutoWork/target/…`, did not write `role-tags.txt` in its
  cwd: it made `.claude/worktrees/role-tags` (branch `worktree-role-tags`, locked), wrote and *committed* the file
  there, and `claude rm` then kept the row. My runs had passed only because their cwd was in a linked worktree.
  → The probe now asks for the answer as the reply text and reads it from the Stop/SessionStart payload's
  `transcript_path` (last assistant text in the `.jsonl`); `transcript_path` joins the kept event fields.
- **Measured with `fm probe isolation`** (two runs, from cwd = the main checkout; three `--bg` sessions each told to
  `Write` `iso.txt` into its cwd under `target/` and reply with the path):

  | Launch cwd | Settings | Where it wrote | Row `cwd` while live | `claude rm` |
  |---|---|---|---|---|
  | main checkout `target/…/iso-main` | default | `.claude/worktrees/iso-probe/iso.txt` — a worktree it created (name chosen by the model, not `--name`), branch `worktree-iso-probe`, **locked**, 1 commit "probe: add iso.txt" made by the session itself | the worktree | **refused, row + worktree + branch kept**: `kept <id> — its worktree is still at “…\.claude\worktrees\iso-probe” / 39 unpushed commits on “worktree-iso-probe” … discard the worktree and its commits: claude rm <id> --discard-unpushed <sha>@<token>` |
  | same | `--settings {"worktree":{"bgIsolation":"none"}}` | in its cwd | unchanged | `removed <id>`, nothing left |
  | linked worktree `AutoWork.worktrees/m2-2/target/…` | default | in its cwd | unchanged | `removed <id>`, nothing left |

  - "39 unpushed" = the session's 1 commit + the 38 commits local `master` has that no remote has: rm counts what
    exists on no remote, not what is not on the main checkout's HEAD.
  - The row's `cwd` follows the session into the worktree while it lives and **reverts to the launch cwd once
    stopped**, so a cleanup cannot find the worktree from a stopped row; rm's refusal names it.
  - `claude rm <id> --discard-unpushed <sha>@<token>` (copied from the refusal) removed the row, the worktree and the
    `worktree-*` branch: `removed <id> / worktree: “…\iso-probe”`. It left an empty `.claude/worktrees/` (and
    `.claude/`) in the main checkout; the probe removes those when empty.
  - Control: the documented setting `worktree.bgIsolation: "none"` (settings file or `--settings` JSON), no CLI flag.
    `--worktree [name]` is the opposite (start in a new worktree). Docs (agent-view, "Automatic isolation"): isolation
    is skipped inside any linked worktree, outside a git repo, and for writes outside the cwd.
  → For M3 repo-controllers: launch with an explicit choice. Either a Foreman-made linked worktree as cwd (writes
    stay there; rm is clean), or `bgIsolation:"none"` when the session must edit the checkout itself. Default
    isolation in a main checkout leaves model-named worktrees with commits that a plain `claude rm` will not delete.
- `fm probe hooks --root P:/Projects/LarchSys/AutoWork` (binary from the m2-2 worktree, cwd = main checkout, scratch
  under the main checkout's `target/`): PASS twice, compact turn: PreCompact(auto) +3.5/+6.5 s, SessionStart(compact)
  +15.4 s, Stop +19.5 s (run 2: +5.9/+11.9/+21.9/+26.4 s); reply v1=no v2=yes v3=yes; no worktree, branch or row left.
- Probe cleanup (`Fleet::remove`): `stop` → `rm`; on a refusal, rerun with the `--discard-unpushed` argument it names;
  else remove the worktree it names with `git worktree unlock` + `remove --force --force` + `branch -D worktree-*`,
  then `rm` again. Only worktrees named by the refusal for the probe's own row are touched.

## 2026-10-23 — m2/4 supervisor in the app

Measured with `crates/foreman-app/diag/kill.ps1`, `diag/resume.ps1` and `target/release/foreman.exe
--selftest-supervise` (CLI 2.1.309, Windows 11, worktree m2-4). Sessions `fm-selftest-…`, cwd under the
worktree's `target/`, `--strict-mcp-config`, `--settings {"worktree":{"bgIsolation":"none"}}`; every row removed.

- **A killed session's pid leaves its row at once**: after `taskkill /PID <row pid> /F` (the
  `claude.exe --session-id <id>` process only) the row had no pid 0.67–0.8 s later, in all 3 diag runs and
  all `--selftest-supervise` runs (0 polls ever showed the dead pid). → `is_live` can keep trusting `pid`; no
  process-alive filter before `tick` (`process_alive` stays in `view.rs`).
- **The daemon respawns a session killed mid-turn by itself**: killed while `state=working` (a `sleep 150`
  Bash turn, 70 s old) or `state=blocked status=idle` (m1/7's odd state) → ~12 s later the same row had a new
  pid (`working`/`idle`), no resume issued. Killed while `state=done status=idle` → stays `done`, no pid, for
  60 s+ (not respawned).
- **A resume too soon after the death of an idle session starts a copy.** `claude --bg --resume <sessionId>`
  1 s or 5 s after the kill: `note: session <id8> is already running in the background, so this started a copy
  as <new id8>` — a second row **with the same name** (= `Duplicate` in reconcile). 10, 20, 30 s after: `note:
  woke session <id8> with its saved options (--name, --permission-mode, --strict-mcp-config, --settings)`, same
  row. After `claude stop` (not a kill) it wakes at once (m2/1). → Reconcile rule **R13**: a wanted row that
  was seen alive less than `respawn_grace` (30 s) ago is not resumed yet (`Wait(Respawn)`), whatever its
  `state`. And a resume whose stdout says `backgrounded · <other id8>` is recorded as a failure ("started a
  copy"), never as a start.
- **Right after `--bg --resume` the row shows a transient `sessionId`** with the new pid (e.g.
  `327a1c87-…` for session `b91b6265-…`), back to the session's own ~5 s later; `id` stays. Every run of step c
  saw it. Match a resumed row by `id`, or wait 5 s before reading its `sessionId`.
- `--bg --resume` with no prompt fires no hook until a prompt arrives (no SessionStart(resume) within 30 s).
- **Shared busy budget** (René, 2026-10-22: "a setting in the app, shared across platforms"):
  `settings.toml` `max_working` (default 3), the `busy ≤ − N +` stepper in the left header, re-read every tick.
  `foreman_supervisor::tick_all(supervisors, snapshot, max_working, now)` ticks every platform on one snapshot
  in turn with one `Budget`: `max` − live `fm-` rows with `state=working` − wanted sessions still settling after
  a start (`Supervisor::starting`); each launch/resume that succeeds takes a slot. `reconcile` gets the free
  slots as a parameter (`Policy::max_working` is gone). Step f, 3 runs: limit 1, two platforms started → max 1
  working; the second went live 1.9–2.7 s after the first one's turn ended (first idle 9.6–13.1 s).
- **Trust**: Start on a `git init`ed dir → `TrustNeeded` Inbox row 6.7–8.4 s after the click, no row created.
  Run in terminal opens pwsh there and types `claude` without Enter once the shell's output has been still for
  500 ms — this profile's prompt is `<dir> (master) >`, no `PS `, and PSReadLine draws a history prediction
  after the typed text (`claude attach 6c08e610`), which is display only.
- **Park**: a wanted session with `--permission-mode default` told to run one Bash command → Inbox `blocked`
  (`waiting for permission prompt`) with Attach; its hook log's last event was Notification(permission_prompt)
  35–45 s before the row was read. The platform session itself is `auto` (`WantedSession::permission_mode`,
  default `auto`), so the selftest's park is a separate wanted session of the testbed platform.
- **Role re-injection per platform**: one `<FOREMAN_HOME>/roles/platform.md` (the template from
  `plugins/foreman/roles`, written at app start) serves every platform: `fm hook` fills `{platform}` from
  `FOREMAN_PLATFORM`, passed with `FOREMAN_HOME`/`FOREMAN_ROLE` in `--settings {"env":…}`. Step b found the
  rendered tag (`FOREMAN-ROLE platform v<sha8>`) in the session's transcript after SessionStart(startup), 3/3.
- **Start → live**: 4.9–5.3 s from the click to a row with pid (one 2 s refresh, `Claude::launch`'s name check
  + `--bg`, then the pid). Stop → both testbed rows without pid 6.7–6.8 s after the click, rows kept.
- **fm.exe at start**: the app finds `plugins/foreman` by walking up from `foreman.exe` and copies the `fm.exe`
  built beside it into `plugins/foreman/bin/` (`foreman_core::install_hook_exe`, moved from `fm`); a missing
  `fm.exe` is a notice, sessions then launch without hooks.
- **Tokens**: `settings.toml` `workstate_env` (+ `workstate_url`, default `http://127.0.0.1:5290/mcp`);
  `AppTokens` reads `WORKSTATE_<ROLE>_TOKEN` from that file at launch time (unit-tested on a fake file). Unset or
  unreadable → `--strict-mcp-config` and no WorkState server; every supervised launch is strict. The selftest
  ran with no `workstate_env`.
- Gate from this worktree: `--selftest-supervise` 3/3 PASS, `--selftest` 2/2 PASS (frame p95 19.83–19.86 ms,
  max 23.92–26.22 ms, 0 > 50 ms; idle 120.4–120.8 MiB), `--selftest-detach 10` 10/10.
- Deps: `foreman-supervisor` gains `serde_json` (in the lock, same version); `foreman-app` gains
  `foreman-supervisor` and dev `tempfile`. No new external dependency.

## Fleet UI after M2 (René's first hand check)

- **Resume from the tree**: a background session with no process resumes in a tab that runs
  `claude --bg --resume <sessionId>; claude attach <id>` (pwsh `-Command`). The session wakes under the same
  8-hex id with its transcript, so the tab is an ordinary Attach tab: closing it (× or middle click) types
  Ctrl+Z and the session keeps running. Measured on a throwaway `--bg` session: 10–14 s from the click to the
  attached screen, row `idle` again, same pid afterwards. An interactive session with no process resumes
  in the tab itself (`claude --resume <sessionId>`). Not offered for `fm-` rows (Start resumes them) or for a
  session this app saw alive in the last 30 s (the daemon's own respawn, R13). A live interactive session
  belongs to the terminal that runs it: no attach, no resume.
- **`CLAUDE_CODE_CHILD_SESSION`**: started from a shell inside a Claude Code session, Foreman inherited the
  marker and the `claude` a tab started said "Transcript saving is off — inherited CLAUDE_CODE_CHILD_SESSION
  marker". `main` removes it before anything starts.
- **Divider drag in gpui**: `on_drag(DraggedDivider, …)` on a 7 px grab area plus `on_drag_move::<DraggedDivider>`
  on the row holding both panels (it fires in the capture phase wherever the cursor is). Mouse input sent with
  `SetCursorPos` does not start a gpui drag; relative `mouse_event` moves do. The width is saved to
  `settings.toml` `left_width` on mouse up; double click resets it.
- **Role → WorkState token**: WorkState has controller, orchestrator and worker tokens only, so
  `WORKSTATE_PLATFORM_TOKEN` never existed in `Skills/mcp/.env`. A role's own line still wins; else platform →
  `WORKSTATE_CONTROLLER_TOKEN`, repo → `WORKSTATE_ORCHESTRATOR_TOKEN` (`tokens::workstate_role`). The settings
  window shows which line each role would use, never the value.

## 2026-10-23 — m2x/2 "platform" is now "workspace"

René, 2026-10-23: Foreman's unit of supervision (a folder of git repos) is a **workspace**, "yes, do the full
rename", on-disk data included. No CLI aliases (`fm workspace …`, `fm sessions --workspace`); the entries above
keep the old word because they record what was measured under it.

- **Names**: `Workspace`, `<FOREMAN_HOME>/workspaces/`, role `workspace` (`roles/workspace.md`, placeholder
  `{workspace}`), session `fm-<w>-workspace`, env `FOREMAN_WORKSPACE`, token line `WORKSTATE_WORKSPACE_TOKEN`
  (else `WORKSTATE_CONTROLLER_TOKEN`). The helper that finds this build's cargo workspace is `cargo_root()` now,
  so the two meanings never share a name. Claude's own trust message still says "workspace … is not trusted".
- **Migration** (`foreman_core::migrate_home`, app start + every `fm` registry command): `platforms/` →
  `workspaces/` by rename (both present: neither touched, a notice every run until the human resolves it);
  `supervisor/<w>.toml` field/role/session renamed and written back only when changed. A desired state that was
  started is saved **stopped**, with a notice: `start_workspace` reuses the saved sessions, so without the session
  rename Start would add `fm-<w>-workspace` beside a still-wanted `fm-<w>-platform` and both would be kept alive;
  and the live `fm-<w>-platform` row is no longer wanted by anything, so the human stops it. The migration never
  stops a session. The kept `prompt` still says "platform controller" until the next Start rewrites it.
- **Old sessions on resume** keep their launch env: `fm hook` falls back to `FOREMAN_PLATFORM` and maps
  `FOREMAN_ROLE=platform` to `roles/workspace.md` (tag `FOREMAN-ROLE workspace v<sha8>`).
- `toml::Table` round trip of a desired-state file loads back as the same `DesiredState`
  (`a_file_from_before_the_rename_loads_and_migrates_to_the_current_shape`).
- Gate from this worktree: `cargo test --workspace` 258 passed (252 + 6 new), `--selftest` PASS (frame p95
  22.01 ms, max 26.51 ms, 0 > 50 ms; idle 121.2 MiB), `--selftest-supervise` PASS (step b: transcript has
  `FOREMAN-ROLE workspace v690ab65d`), no `fm-selftest*` row left.

## 2026-10-23 — m3/1 the relay

Measured with `crates/foreman-app/diag/relay.ps1` (the relay exactly as Foreman launches it: rendered
`roles/relay.md`, `--plugin-dir`, `--add-dir <home>/relay/<w>`, `--strict-mcp-config`, auto, `bgIsolation:"none"`;
messages dropped with `fm send`, answers read from the outbox), `diag/timeline.py` on the transcripts, and
`foreman.exe --selftest-supervise` step r (CLI 2.1.309, Windows 11, worktree m3-1). Sessions `fm-selftest-<hex>-*`,
cwd and `FOREMAN_HOME` under the worktree's `target/`; every row stopped and removed.

- **Round trip app → relay → target → relay → app: 11.2–16.8 s** with a target told to answer with the nonce
  (diag: 14.75 / 15.19 / 14.44 s; 12.74 s and, at minute 34, 11.16 s; `acceptEdits` target 11.79 / 15.85 / 16.76 s;
  selftest step r through `foreman_core::send` 12.7 and 13.7 s, in the app's Messages 2 s later = one refresh).
  M0 had 14.0–18.0 s. The gain is the outbound leg only: the `fm inbox` line is the Monitor event itself
  (in the relay transcript 0.2–0.6 s after the drop), and `SendMessage` follows 1.6–2.1 s later in the same API
  call; app → target 2.9–3.1 s when the relay is idle (M0: 5.5–6.9 s). Outbound 2 API calls (SendMessage, end
  turn), inbound 2 (Write, end turn). The rest is the target's own turn (~7–8 s) and the relay's `Write`
  (~2.5 s from the enqueue to the file).
- **A turn holds the next message ~3 s after its last text**: the transcript's `stop_hook_summary` comes ~3.1–3.5 s
  after the turn's last assistant text, and a Monitor event or reply enqueued meanwhile waits for it (rounds 2–3:
  app → target 6.3–8.1 s when the drop landed while the relay was finishing the previous turn). Hook time is not
  it: `fm hook Stop` 29–52 ms, the user-scope GitKraken Stop hook (`gk.exe ai hook run`) 298–347 ms — user-scope
  plugin hooks run in Foreman's sessions too.
- **Write access to the outbox: `--add-dir <FOREMAN_HOME>/relay/<w>`; nothing parked.** The relay's cwd is the
  workspace root; `ready.json` and every `msg-NNNN.json` are outside it. In auto mode every `Write` there went
  through without a prompt (no `blocked` row, no Notification) in all 4 diag relays and in the selftest runs
  (3 relays each: testbed, c1, c2). The `fm.exe inbox` Monitor command
  (absolute path, forward slashes, quoted) was accepted the same way. Here `FOREMAN_HOME` was the worktree's
  `target/…-home`, a sibling of the cwd, not `%APPDATA%\Foreman`; the real path was not tried (C17).
- **Monitor re-arm works** (diag `-HoldMinutes 33`, relay up 34 min): armed 14:23:43; the expiry notice
  (`[Monitor expired after …]`) arrived 14:53:43, 30 min 0 s later; the relay re-armed with the same command and
  rewrote `ready.json` in one turn — Monitor + Write in one API call, 2 calls in all, `ready.json` 3.9 s after the
  notice. Nothing in the transcript between 14:24:18 and the notice: an armed relay costs zero turns idle. A message
  dropped at minute 33.7 went through in 11.16 s. → `ready.json`'s mtime is the last arm; the app calls a relay
  whose `ready.json` is older than 31 min "watcher expired, not re-armed".
- **Held message: not observed.** A `plan`-mode target (relay in auto): the message was *enqueued* in the target 3 s
  after each of three drops, no `[Cross-session delivery notice]` and no SendMessage failure reached the relay,
  but the target never answered: its first turn had written a plan (to `~/.claude/plans/<name>.md` — outside the
  worktree, left there) and parked on `ExitPlanMode` (`state=blocked`, `waitingFor=permission prompt`) 16 s after
  launch, so the queued messages wait behind a human. Foreman sees that as the target's Inbox `blocked` row, not
  as a relay event. `acceptEdits` target: delivered and answered like auto. With M0's `default`: no hold in any
  mode tried so far.
- **An idle session can keep `state=working`** (m0/4) and the busy budget counts `fm-` rows with `state=working`:
  the first two selftest runs failed step f (limit 1) because an idle relay-test target held the only slot —
  once the selftest's own target, once the diag's target running beside it. Relays themselves are exempt (their
  rows never count, their launches take no slot); the selftest now stops its target after step r. A non-relay
  session that idles with `state=working` still blocks the budget — open, for M3's controllers.
- **Relay launch order**: Start writes the relay first in the desired state; the supervisor launches it before the
  workspace session in the same tick (selftest: relay `startedAt` 1.6–2.6 s earlier), and relays first on every tick
  because they skip the cap. `ready.json` 14.5–17.8 s after the Start click. Stop stops the relay after the other
  wanted sessions of the tick.
- **Teardown**: `claude stop` ends the relay's `fm.exe inbox` child: 0 watcher processes left after every diag run.
- Outbox numbering is the relay's: it continues from the highest `msg-*.json` (Glob) after a restart or
  compaction; `read_outbox` treats a file that does not parse for 5 s as unreadable (shown, with its raw text).
- Gate from this worktree: `cargo test --workspace` 273 passed (258 + 15 new), `--selftest-supervise` PASS
  (step b relay 1651 ms before the workspace session; step r `ready.json` 15.3 s after Start, round trip 10.3 s,
  in Messages with Attach after 11.8 s; step g relay stopped with the rest; 8 rows removed, 0 left),
  `--selftest` PASS (frame p95 22.36 ms, max 30.36 ms, 0 > 50 ms; idle 121.1 MiB).
- Deps: `fm` gains `chrono` (in the lock, same version, `alloc` only). No new external dependency.

## 2026-10-23 — m3/2 repo sessions on demand, named WorkState sessions, fm inside sessions, busy = busy

Measured with `crates/foreman-app/diag/insession.ps1` (sessions exactly as Foreman launches them: `--plugin-dir
plugins/foreman` with this build's `fm.exe` installed, `--strict-mcp-config`, `--settings` env + `bgIsolation:"none"`),
`diag/busy.ps1` and `foreman.exe --selftest-supervise` step w (CLI 2.1.309, Windows 11, worktree m3-2). Sessions
`fm-selftest-<hex>-*`, cwd and `FOREMAN_HOME` under the worktree's `target/`; every row stopped and removed.

- **The plugin's `bin/` is on the session's PATH.** An auto session asked to run `where.exe fm` and `fm --version`
  got `…\m3-2\plugins\foreman\bin\fm.exe` and `fm 0.1.0`, unprompted with its **PowerShell** tool. → Roles call
  `fm` by name; `{fm}` (the absolute path) still renders for roles that want it.
- **Allowed-tools form: `Bash(fm:*)` and `Bash(<abs fm, forward slashes>:*)` match; PowerShell rules do not.** In
  `--permission-mode default` (where an unmatched command parks): `--allowedTools "Bash(<abs fm>:*)"` + `<abs fm>
  --version` and `"Bash(fm:*)"` + `fm --version` ran without a prompt; the control (no rule, Bash `fm --version`)
  parked (`state=blocked`, `waitingFor=permission prompt`, Notification 6 s later); `"PowerShell(fm:*)"` and
  `"PowerShell(fm *)"` with the PowerShell tool parked the same way. → Every launch passes
  `--allowedTools "Bash(fm:*)" "Bash(<abs fm>:*)"` (`LaunchConfig::allowed_tools`, `supervise::fm_allowed_tools`), and
  the roles say "with the Bash tool, never PowerShell". Auto mode ran `fm` through PowerShell without a park too, but
  that is the classifier's call, not a rule.
- **`claude --bg --session-id <uuid>` does not work**: `warning: --bg manages the session id; ignoring --session-id
  (use --resume <id> to continue an existing session)`, and the row got a uuid of its own. → `X-WorkState-Session`
  is left out of the launch config; WorkState keys a session by (workspace, name) anyway (doc 11), and
  `fm roster --json` has the `sessionId` for whoever needs it.
- **Busy signal: `status`, not `state`.** `diag/busy.ps1` (the relay selftest's target prompt: answer messages, then
  wait): launch → `state=working status=idle` with a pid (+2.4 s, before its first prompt) → UserPromptSubmit and
  `status=busy` in the same 500 ms poll → Stop and **`state=working status=idle`** (the m3/1 case: idle, waiting for
  messages, `state` still `working`) → a SendMessage from another session: UserPromptSubmit + `status=busy` in the
  same poll → Stop + `state=done status=idle`. In every insession run `status` flipped busy/idle in the same poll as
  the UserPromptSubmit/Stop hooks, and was `waiting` while parked. → `foreman_supervisor::mid_turn`: live and
  `status=busy`; only a live row with no `status` yet (the pid-less row right after `--bg`) falls back to
  `state=working`. Hooks agree and are not consulted, so `reconcile`/`Budget` still take one snapshot.
  `Supervisor::starting` now holds a slot through `settle` until the row is mid-turn (a fresh `status=idle`
  second before the first prompt would otherwise free it). Parked (`status=waiting`) takes no slot.
- **Named WorkState sessions**: one owner-only `<home>/mcp/workstate-<session name>.json` per session (was per
  role) with `Authorization` + `X-WorkState-Agent/-Launched-By/-Origin/-Cwd/-Foreman-Role`
  (`SessionIdentity`, `write_session_mcp_config`; unit-tested shape and ACL with `fake-token`). A resume re-applies
  the file its launch saved. **Checked against the real server by the controller** (autowork-7c, 2026-10-23
  13:32 UTC; this unit has no real token): a hand-launched `fm-check-named` (`--strict-mcp-config --mcp-config`
  with exactly this shape, real CONTROLLER token, Agent=fm-check-named, Launched-By/Origin=foreman/check,
  Foreman-Role=workspace, no Session header) called `work_brief` once → one `agent_session` row: name
  fm-check-named, role controller (from the token), external_role workspace, origin and launched_by foreman/check,
  cwd = the launch cwd, client_name claude-code, claude_session_id empty. `work_brief` printed no "session …" hint
  line, which it shows only to unnamed callers.
- **`fm want <w> <repo> [--prompt-file]` / `unwant` / `roster [--json]`.** `want` adds `fm-<w>-<repo slug>` (role
  `repo`, cwd = the repo, auto, `bgIsolation:"none"`, prompt = `roles/repo.md` with `{repo}`/`{workspace}` + an
  optional "First task:"); a repo named `workspace`/`relay` is refused, an unknown one lists the known ones. `unwant`
  moves the name to the desired state's `retired`: reconcile rule R14 stops a live retired row (before the relay),
  never launches or resumes it. `roster` = `agents --json --all` rows named `fm-<w>-…` (not those of a registered
  workspace whose name extends `<w>-`) + wanted sessions without a row, tab-separated or JSON.
- **The desired-state file has two writers now** (the app's Start/Stop, `fm want/unwant`): `DesiredState::update`
  re-reads the file before saving and starts over when it changed (≤ 10 times); `save` writes a per-process temp
  file and retries the rename while the other side has the file open. **Remaining race**: a write landing between
  the re-read and the rename is lost (milliseconds; the loser's change is not applied, nothing is corrupted).
- **Selftest step w**: the testbed's repo is a plain dir `<root>/Repo` registered by hand, **not** a `git init`ed
  one: a `git init`ed dir under `target/` is untrusted (step d is built on exactly that). `fm want` → the supervisor
  launched `fm-selftest-<hex>-repo` live 5.0 s later; its first task ran `Bash: fm roster selftest-<hex>` itself
  without a park and replied with its own line; `fm roster` listed `…-repo  repo  idle  <id>  <cwd>`; `fm unwant` →
  stopped 2.2 s later, not resumed 5 s after.
- `extra_plugin_dirs` (`settings.toml`) go to every launch as more `--plugin-dir` after Foreman's own, re-read every
  tick; the settings window's new "Plugins" page lists them read-only (warns when a dir has no
  `.claude-plugin/plugin.json`). The selftests run without any.
- Gate from this worktree: `cargo test --workspace` 288 passed (274 + 14), `--selftest-supervise` PASS (step b relay
  1550 ms before the workspace session; step w as above; step f max 1 working over 26 polls, the second live only
  after the first idled; 9 rows removed, 0 left), `--selftest` PASS (frame p95 21.38 ms, max 24.36 ms, 0 > 50 ms;
  idle 121.2 MiB).
- Deps: `fm` gains `foreman-supervisor` (path); `foreman-app` gains `serde_json` (in the lock, same version). No new
  external dependency.

## 2026-10-23 — m3/3 the two skills: workspace-controller, repo-controller

Measured with `crates/foreman-app/diag/skills.ps1` (a relay, a workspace session and a repo session exactly as
Foreman launches them: rendered `roles/*.md` as the prompt, `--plugin-dir plugins/foreman` with this build's
`fm.exe`, `--settings` env FOREMAN_HOME/ROLE/WORKSPACE(/REPO) + `bgIsolation:"none"`, `--allowedTools "Bash(fm:*)"
"Bash(<abs fm>:*)"`, auto, `--strict-mcp-config` and **no WorkState config**; FOREMAN_HOME and cwd under the
worktree's `target/`), `diag/endstate.ps1`, and `foreman.exe --selftest-supervise` (CLI 2.1.309, Windows 11, worktree
m3-3). Sessions `fm-selftest-<hex>-*`; every row stopped and removed. No session had a `work_*` tool, so nothing
touched the real store.

- **Skills from `--plugin-dir` are listed as `foreman:<name>`.** The transcript's first `attachment` of type
  `skill_listing` has `names` (61 here) and `content` (`- <name>: <description>` lines); both sessions listed
  `foreman:repo-controller` and `foreman:workspace-controller`. The SessionStart injection is in the transcript too:
  `FOREMAN-ROLE workspace vb2ba9581` / `FOREMAN-ROLE repo v247eda8d`, each followed by the role text naming
  `/foreman:workspace-controller` resp. `/foreman:repo-controller`. → `--selftest-supervise` step b now fails unless
  the workspace role names its skill and the session's `skill_listing` names both (`skills_listed`).
- **Both sessions invoked their skill as the first act.** Workspace: `Skill(foreman:workspace-controller)` →
  `ToolSearch` (loads the deferred `SendMessage`) → `SendMessage(fm-…-relay, "fm-…-workspace ready, but WorkState is
  not visible here — I can talk, not plan. Foreman's settings need workstate_env.")` — the skill's no-WorkState
  line verbatim; the relay wrote it to `outbox/msg-0001.json` with `from_name` = the workspace session. Turn ended
  18 s after launch. Repo (first task a direct request: list the foreman:/rlarch: skills): `Skill(foreman:repo-
  controller)`, then the list as its reply, turn ended 13 s after launch, nothing sent.
- **The user-scope rlarch plugin loads in Foreman's sessions and has no `controller` and no `nested-claude`.** The repo
  session listed `rlarch:code-comments, dotnet-reflect, dotnet-source, orchestrate, ts-source` — the installed
  version, not the `feat/workstate` branch. → `/rlarch:controller` needs `settings.toml` `extra_plugin_dirs` =
  `P:/Projects/LarchSys/Skills/plugins/rlarch` (m3/2); the repo-controller skill sends `BLOCKED` naming exactly that
  when the skill is missing. Which of two same-named plugins wins when both are loaded was not measured.
- **Claude Code reads the end of a turn into `state`, and the daemon acts on it.** `diag/endstate.ps1`, one turn
  each, bare sessions: a last text "Should I continue?" → `blocked/idle` for 85 s (m1/7's unexplained state: "needs
  input"); "ready", "waiting", "I'm now waiting for the next message …", SendMessage + "ready", ToolSearch + "ready",
  Skill + "ready" → `done/idle`. But the **workspace session** (role + skill telling it to wait for messages) ended
  `blocked/idle` with a long closing paragraph (diag run 1, selftest run 1) and `working/idle` with the one line
  `waiting` (diag run 2; selftest run 2) — the context, not the last word alone, decides. `done: ready sent` ended
  `done/idle` (diag run 3, selftest runs 3–4). The repo session's direct-request turn (a list as the reply) ended
  `working/idle` every time. It matters: m2/4's daemon respawns a killed `working` or `blocked` session by itself, so
  `--selftest-supervise` step c failed twice ("came back with pid …, but not by the supervisor") until the workspace
  role and skill ended the turn with `done: <what it did>`. → Both skills end every turn with one line `done: …` that
  asks nothing; questions go by `SendMessage`. A model that ignores it still reads `Idle` in Foreman (no
  `waitingFor`), and a killed one comes back by the daemon instead of the supervisor — it comes back either way.
- **Decided in the skills** (no code): project ref = the workspace name; one run per repo `<task-slug>-<repo-slug>`;
  an integration run `<task-slug>` with `<task-slug>-verify` for the workspace-controller's own cross-repo check,
  blocked by every repo run; prompt files at `<workspace root>/.foreman/prompts/<run ref>.md`, kept; a repo whose
  run is all blocked is wanted only when its blocker lands; messages `APPROVE?`/`DECIDE:`/`DONE:` to the relay,
  `BLOCKED <unit>:`/`DONE <run>:` repo → workspace, `ANSWER`/`DECISION … (from the human, verbatim)` back.
- **Two gaps the skills route around, not fixed:** (1) `fm want` for a session that already has a row resumes it
  with **no prompt** (`resume_prompt` is `None` for repo sessions; m2/4: no hook fires until a prompt), so the
  workspace-controller must `SendMessage` the task as well; likewise a repo session the supervisor resumes after a
  crash sits idle until a message. (2) `fm send` from inside a session is delivered *from `fm-<w>-relay`*, i.e. as
  the human's voice, and `Bash(fm:*)` allows it; both skills forbid it in words only.
- **The relay's send was refused by the auto-mode classifier twice** (selftest runs 11 and 13 of 14, step r):
  `SendMessage to fm-selftest-…-target failed: Permission for this action was denied by the Claude Code auto mode
  classifier. Reason: [External System Writes].`, relay row `state=blocked status=busy`. New in this unit: before the
  drop the relay has already received the workspace session's `ready`. Not reproduced on purpose: `diag/relay.ps1`
  (no workspace session) 3/3 rounds delivered, `diag/skills.ps1 -Drop` (a drop right after the workspace's
  `ready`) 3/3 delivered. So: intermittent, cause unknown, not routed around. If it happens to a real relay the
  human's message is not delivered and a `relay-notice` lands in Messages; m3/4 should watch for it.
- Gate from this worktree (final code, run 14): `cargo test --workspace` 291 passed (288 + 3: roles/skills, skill
  listing, `Hit::fully_visible`), 0 failed; `--selftest-supervise` PASS (step b `FOREMAN-ROLE workspace v85771130`,
  skills listed both, first turn Skill → ToolSearch ×2 → SendMessage; step c resumed by the supervisor after 32 s;
  step w the repo session ran `fm roster` with Bash, no park; steps s/f/g as in the finding below; 9 rows removed,
  0 left); `--selftest` PASS (frame p95 20.79 ms, max 24.10 ms, 0 > 50 ms; idle 120.8 MiB). The 13 runs before:
  c ×2 (runs 1–2), f ×3 (3–5), the new click helper ×5 (6–10), r ×2 (11, 13), PASS once (12, before step s
  existed) — each cause named above or below; only r's is open.

## 2026-10-23 — m3/3 finding: the left panel could not scroll; the last workspaces were out of reach

Found by `--selftest-supervise` step f once the workspace session posts `ready` (one more Messages row). Separate
from the skills; fix in its own commit.

- **Symptom**: step f failed 3 runs in a row with `not both launched within 180 s`; `-c2` got no row at all, not
  even its relay (which skips the busy cap). A diagnostic bail (step f now fails if a desired state is not started
  3 s after its Start click) showed `start-<c2>` painted at y 846 in an 820 px window: the click landed outside the
  window. A wheel event over the tree did not move it (four runs, bounds unchanged ×20).
- **Cause**: the left column is header, Inbox, Messages, notices, then the tree (`id("fleet")`, `flex_1`,
  `overflow_y_scroll`). The tree had no `min_h_0`, so it grew to its content and never scrolled; and the Inbox and
  Messages sections, plain flex children with `min-height: auto`, could not shrink, so with `min_h_0` alone the tree
  is what gets squeezed. Dump of run 10: Inbox y 110–330, Messages 330–560, tree from 560 with rows to 888+. A human
  could not reach those workspaces either.
- **Fix** (`view.rs`): the tree gets `min_h_0`; Inbox and Messages are capped at 25 % / 30 % of the panel
  (`max_h(relative(…))`, `flex_shrink_0`) and scroll inside themselves (`id("inbox")`/`id("messages")` +
  `overflow_y_scroll`). A hit target now records its clip as well as its bounds (`Hit { bounds, visible }`,
  `visible` = `window.content_mask()` at prepaint = the visible area of the scroll container around it).
- **Harness** (`selftest.rs`): `click` first scrolls its target **wholly** into its visible area — mouse moved
  into that area, then a wheel event of the missing distance (gpui gives a scroll to the hitbox under the pointer:
  the earlier attempts sent it at mid-window, over the Inbox) — and fails with every bounds it saw if it never
  gets there. New step **s**: window resized to 480 px, the tree's lowest row (y 787, tree visible 315–480) scrolled
  to y 457, wholly visible; size restored. Step g's Stop then scrolled back up (329 → 506 in 502–820), and step d's
  Inbox button scrolled inside the capped Inbox (277 → 243 in 70–267).

## 2026-10-23 — m3/6 `fm send` refuses inside a Foreman session

- **The guard keys on `FOREMAN_ROLE` being set and non-empty** in `fm`'s own environment. Every session Foreman
  launches gets it through `--settings {"env":…}` (`foreman-supervisor` `session_settings`), and so does every `fm`
  it runs with Bash; the human's shell does not. `fm send` then exits 1 with one stderr line
  `fm send speaks as the human through the relay and is refused inside a Foreman session (FOREMAN_ROLE=<role>); use
  SendMessage to reach a session`, checked in `main.rs` before `registry()` — nothing under `FOREMAN_HOME` is read or
  written (`crates/fm/tests/relay.rs`: the home dir stays empty). Absent or empty: unchanged.
- **The guard is the enforcement; the skills' "never run `fm send`" (m3/3 finding) is now the explanation.** A model
  that ignores the words gets the refusal, not the human's voice. Not covered: a session that unsets the variable
  itself, or writes into `relay/<w>/inbox` by hand — the guard closes the pre-approved `Bash(fm:*)` path only.
- `FOREMAN_ROLE` leaking from a Foreman session into a test run would now fail `fm send` tests, so
  `tests/relay.rs`'s helper removes it (as `tests/hook.rs` already did).

## 2026-10-24 — m3/4 the first end-to-end task (test workspace `e2e`)

Two throwaway git repos under `target/e2e` (`textkit`: `greet`; `textcli`: `python -m textcli greet <name>`,
importing the sibling textkit), test home `target/e2e-home` (`max_working = 3`, `extra_plugin_dirs` = rlarch).
Task thrown with `fm send`: add `--shout` to `textcli greet`, built on a new `textkit.shout`.

- **Timeline** (local): Start 17:08 → workspace-controller `ready` 17:09:26 → task sent 17:09:34 → `APPROVE?` with
  project `e2e`, runs `shout-textkit`, `shout-textcli` (blocked by textkit's task), integration run `shout` /
  `shout-verify` 17:11:34 (2 min) → approved 17:11:49 → `fm want e2e textkit`. Then **trust** (below) until the
  next day. textkit session 00:01:16 → nested unit 00:02:18 → merged `e126cae` 00:03:40. textcli session 00:04:17
  → `BLOCKED` 00:04:41 (below) → decision 00:05:45 → nested unit 00:08:06 → merged `2ae66b4` 00:09:28 →
  workspace-controller ran `shout-verify` itself and sent `DONE` 00:11:31, then retired both repo sessions
  (`fm unwant`). Stop: all six sessions stopped in 3.3 s. Work time without the human waits: ~12 min.
- **Verified by me** from the repos: textkit 3 tests OK, textcli 8 tests OK, `greet --shout Ada` and
  `greet Ada --shout` → `HELLO, ADA!`, `greet Ada` → `Hello, Ada!`; both repos clean on main, one worktree each.
  WorkState project `e2e` 6/6 done, each run with its merge sha.
- **Trust is a human step per repo.** A git-init'ed folder is untrusted even inside a trusted checkout, so the
  first `fm want` stalls with Inbox "trust needed". It is the human's to accept ("Run in terminal" / the repo `+`,
  accept, `/exit`, Retry). The workspace-controller correctly said nothing about it: the Inbox row is Foreman's.
- **Defect — the skill listing is truncated.** Both repo sessions' listings had six rlarch skills but not
  `rlarch:controller` / `rlarch:nested-claude` (the two longest descriptions, 953/960 chars). textkit called the
  Skill tool anyway and it loaded; textcli trusted the listing and escalated `BLOCKED`, which reached the human as
  a `DECIDE` with two wrong options (add the plugin + restart / waive nested agents). Answered: "call the Skill
  tool directly". Fix queued as m3/8 (skills say: a missing listing entry is not a blocker, a failed Skill call is).
  **Correction 2026-10-26**: not truncation — the two skills' SKILL.md YAML frontmatter did not parse, so they
  loaded without a description and were left out of the listing (fixed in rlarch 2.2.0, Skills `e00f76b`; tf/7).
- **The escalation path worked as designed**: repo-controller → workspace-controller (`BLOCKED`, verbatim) →
  relay → app Messages (`DECIDE:`) → human `fm send` → workspace-controller → repo-controller (`DECISION` forwarded).
- **Seen by René in the UI** (queued in backlog): tabs monochrome — `NO_COLOR=1` inherited because I launched the
  test Foreman from a Claude Code shell (backlog 5); block glyphs (the Claude logo) with gaps between rows (4);
  no mouse-wheel scroll in a terminal tab (6).

## 2026-10-24 — m3/8 skills call Skill for unlisted rlarch skills; `done:` lines that don't read as blocked

- **Skills/roles** (`plugins/foreman/skills/{repo,workspace}-controller/SKILL.md`, `plugins/foreman/roles/{repo,workspace}.md`):
  a session calls the Skill tool for `rlarch:controller`, `rlarch:nested-claude` and any skill its role names
  even when the skill listing does not show it — the listing is truncated (m3/4); only a failed Skill call is a
  blocker. The repo-controller's old "Not in your skill list → BLOCKED" is gone. The workspace-controller answers
  such a `BLOCKED` itself ("call the Skill tool anyway"), never as a `DECIDE` to the human. A turn that waits on
  someone ends `done: waiting on <who> for <what>`, never with the word BLOCKED in the `done:` line (the
  `BLOCKED` message via `SendMessage` is unchanged). Pinned by `crates/fm/tests/skills.rs`.
  **Correction 2026-10-26**: the listing was not truncated — rlarch's `controller` / `nested-claude` frontmatter
  did not parse (fixed in rlarch 2.2.0); the skills now say a skill can be missing from the listing (tf/7).
- **Rule chosen** — `Session::supervised_health` (`crates/foreman-claude/src/session.rs`): an `fm-…` row on a live
  pid that `health()` reads `Blocked` with **no `waitingFor`** (B1 `status=waiting` / B3 `state=blocked`, no status)
  is `Idle`. Used by the Inbox (`reconcile` R4), the fleet panel (`fleet::build`, before `with_hook`) and
  `fm roster`. Why: a Foreman session talks to Claudes; a turn that merely *reads* as a question waits on a peer's
  message, and only Claude's own UI waiting on something (`waitingFor`, measured only as `permission prompt`)
  needs the human's Attach. `waitingFor` *present*, not = "permission prompt", so a future value (a plan or a
  question dialog) still reaches the Inbox. The hook's permission-prompt upgrade (`with_hook`) still applies after
  it. Non-`fm-` rows and `health()` itself are unchanged (budget uses `status` only, unaffected).
- **What it was before**: m3/3 measured the question-ending turn as `blocked/idle`, which `health()` already read
  `Idle` (B5) — so the m3/4 textcli row most likely never made an Inbox row (not re-measured). The open paths were
  B1/B3 without `waitingFor` on a live pid, which did reach the Inbox as "waiting for an answer"; now they don't.
- Tests: `fleet::foreman_session_waiting_on_a_peer_is_idle_a_parked_one_blocked`,
  `reconcile::blocked_without_waiting_for_is_not_reported`, `skills_call_the_skill_tool_even_when_the_listing_omits_it`,
  `skills_end_a_waiting_turn_without_the_word_blocked`. Red: rule disabled → fleet 1 failed, reconcile 1 failed;
  skills reverted to master → 2 failed.
- Gate: `cargo build --release --workspace` ok; `cargo test --workspace` 297 passed (293 + 4), 0 failed;
  `--selftest` PASS (p95 23.29 ms, max 30.36 ms, 0 > 50 ms; idle 121.1 MiB); `--selftest-supervise` PASS (step e:
  `state=blocked status=waiting waitingFor=permission prompt` → Inbox `blocked` with Attach); `cargo test -p fm skills` 2 passed.

## 2026-10-26 — tf/4 block-element glyphs

- **Cause**: `term_element.rs` painted U+2580–U+259F as Consolas glyphs; at `LINE_HEIGHT_FACTOR = 1.3` the glyph
  box is shorter than the cell, so stacked rows of `█▛▜▐▌` (the Claude logo) showed gaps. Windows Terminal draws
  these itself, as Zed does.
- **Fix**: ported Zed's block-element rects (`collect_block_element_regions`, `push_block_element_region`,
  `block_element_regions_to_rects`, the 8×24 subcell grid, shades as foreground at 25/50/75 % opacity). Those cells
  leave the text run and become filled quads covering the full cell; adjacent same-colour regions merge, also
  across lines. Painted after text runs, as in Zed. Sextants (U+1FB00–1FB3B) not ported (not in the brief).
- `foreman_term::Cell::with_char(c, fg)` exists so foreman-ui tests can build cells; the fallback theme for tests
  comes from `theme::ThemeRegistry::default()` (no `App` needed).
- Tests: `block_element_*` (5) in `crates/foreman-ui/src/term_element.rs`. Red: block-element hook disabled in
  `layout_grid` → 3 failed. Screenshot: `target/tf-4/foreman-block-glyphs.png` (logo solid, no row gaps).
- Gate (built with `CARGO_TARGET_DIR` on C: because P: was full): release build ok; `cargo test --workspace
  --no-fail-fast` 301 passed, 1 failed (302 = 297 + 5). The failure is `fm/tests/want.rs:41`: `fm::repos::plugin_of`
  walks up from the exe, and a target dir outside the repo has no `plugins/foreman` above it. `--selftest` PASS
  (p95 23.39 ms, max 38.73 ms, 0 > 50 ms; idle 121.9 MiB); `--selftest-supervise` PASS when run from exe copies in
  the worktree's `target/release` (it FAILs at step b from the C: path for the same reason).

## 2026-10-26 — tf/5 NO_COLOR from a parent Claude Code shell

- **Symptom** (René, 2026-10-24, "why is it white?"): every terminal tab monochrome. Cause: foreman.exe launched
  from a Claude Code tool shell inherits `NO_COLOR=1` and `CLAUDECODE=1` from it, and so does every tab's pwsh,
  git, `claude`. PSReadLine, `$PSStyle`, git and Claude Code all honour `NO_COLOR`.
- **Rule** — `env_scrub` (`crates/foreman-app/src/main.rs`), applied with `remove_var` at the top of `main`
  before any thread: `CLAUDE_CODE_CHILD_SESSION` always (as before); with `CLAUDECODE=1` also `NO_COLOR` and
  `CLAUDECODE`. A `NO_COLOR` without `CLAUDECODE` is the user's own and stays.
- **Why drop `CLAUDECODE` too**: in the claude 2.1.310 binary it is the "inside a Claude Code session" marker —
  read with `CLAUDE_CODE_CHILD_SESSION` as the nested-session test (`Boolean(CHILD_SESSION||CLAUDECODE)`), it
  turns off prompt history when stdin/stdout are not TTYs, and Claude Code itself deletes `CLAUDECODE` /
  `CLAUDE_CODE_SESSION_ID` / `CLAUDE_CODE_CHILD_SESSION` from env it hands to sessions it starts fresh. A `claude`
  in a Foreman tab is top-level, not nested.
- **Not scrubbed (open)**: the tool shell also exports `CLAUDE_CODE_SESSION_ID`, `CLAUDE_PID`,
  `CLAUDE_CODE_SESSION_ATTENDED`, `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_MESSAGING_SOCKET`/`_TOKEN`,
  `CLAUDE_CODE_EXECPATH`, `CLAUDE_EFFORT`, `CLAUDE_JOB_DIR`; tabs still inherit them when Foreman is started that way.
- **Proof**: foreman.exe started from a Claude Code tool shell (`CLAUDECODE=1 NO_COLOR=1`), scratch `FOREMAN_HOME`,
  `+` tab: `$env:NO_COLOR`/`$env:CLAUDECODE` empty, `Write-Host -ForegroundColor Red` red, git log coloured,
  PSReadLine syntax colours — `target/tf-5/tf-5-colour.png` (PrintWindow).
- **Gotcha, gate off the repo**: with `CARGO_TARGET_DIR` outside the repo (P: was full), `fm` and
  `--selftest-supervise` can't find `plugins/foreman`: both walk up from the exe (`fm::repos::plugin_of`,
  `supervise::find_plugin_dir`). `crates/fm/tests/want.rs` fails 1 test and selftest-supervise fails step c. Fixes used:
  a junction `<target dir>/../plugins` → the worktree's `plugins` for `cargo test`; copies of the exes under the
  worktree's `target/release` for the selftest. PrintWindow from pwsh needs `SetProcessDPIAware` first, or the
  rect and capture are scaled (150%: a 1415 px crop of a 2122 px window).
- Tests: `tests::env_scrub_drops_no_color_from_a_claude_code_shell`, `tests::env_scrub_keeps_a_users_no_color`,
  `tests::env_scrub_always_drops_child_session` (`cargo test -p foreman-app env_scrub` 3 passed). Red: NO_COLOR rule
  removed → 1 failed.
- Gate: `cargo build --release --workspace` ok; `cargo test --workspace` 300 passed (297 + 3), 0 failed;
  `--selftest` PASS (p95 21.88 ms, max 35.03 ms, 0 > 50 ms; idle 121.3 MiB); `--selftest-supervise` PASS.

## 2026-10-26 — tf/8 the rest of a Claude Code tool shell's variables

- **Rule** — `env_scrub` (`crates/foreman-app/src/main.rs`): with `CLAUDECODE=1` it now also drops `TOOL_SHELL`:
  `CLAUDE_CODE_SESSION_ID`, `CLAUDE_PID`, `CLAUDE_CODE_SESSION_ATTENDED`, `CLAUDE_CODE_ENTRYPOINT`,
  `CLAUDE_CODE_MESSAGING_SOCKET`/`_TOKEN`, `CLAUDE_CODE_EXECPATH`, `CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT`,
  `CLAUDE_EFFORT`, `CLAUDE_JOB_DIR`. They are the parent session's identity and settings; the messaging socket and
  token would let a tab's process speak as that session. The list is the names a 2.1 tool shell had on 2026-10-26
  (`CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT` was not in tf/5's list; it is set neither in `~/.claude/settings.json` nor
  in the user environment, so Claude Code sets it). Explicit names, not a `CLAUDE_CODE_*` prefix: a user's own
  `CLAUDE_CODE_*` setting (e.g. a provider switch) passes through a tool shell too and must reach the tabs.
- **Without `CLAUDECODE`** nothing but `CLAUDE_CODE_CHILD_SESSION` is touched: a `CLAUDE_EFFORT` (or any other of
  these) set by the user is deliberate and stays.
- **Not scrubbed**: `AI_AGENT` and `COREPACK_ENABLE_AUTO_PIN`, also exported by the tool shell, are not Claude
  Code's own namespace and not in the brief; a tab inherits them when Foreman is started from a tool shell.
- Tests: `tests::env_scrub_drops_the_parent_sessions_identity_from_a_claude_code_shell` (socket + token + every
  `TOOL_SHELL` name), `tests::env_scrub_keeps_claude_vars_without_claudecode`. Red: `scrub.extend(TOOL_SHELL)`
  removed → 1 failed.
- Gate (P: had 4.2 GB free after the release build, so `CARGO_TARGET_DIR` on C: with the `plugins` junction):
  `cargo build --release --workspace` ok; `cargo test --workspace` 307 passed (305 + 2), 0 failed; `cargo test -p
  foreman-app env_scrub` 5 passed; `--selftest` PASS (p95 21.38 ms, max 30.41 ms, 0 > 50 ms; idle 121.6 MiB);
  `--selftest-supervise` PASS (from exe copies in the worktree's `target/release`, deleted afterwards).

## 2026-10-26 — tf/6 wheel scroll

- **Cause** (René, 2026-10-24, "I can't scroll in a terminal"): the Zed port kept `Terminal::scroll_wheel` and its
  mode logic, but nothing called it — `term_element.rs` dropped Zed's mouse handling and the tab wired only keys.
- **Fix**: the div around a tab's grid (`render_right`, `crates/foreman-app/src/view.rs`) has `on_scroll` →
  `Terminal::scroll_wheel(e, SCROLL_MULTIPLIER = 1.0 (Zed's default), MouseInputMode::ReportToTerminal)`, then
  `cx.notify()` on the view; the queued `Scroll` is applied by the grid's prepaint `sync`. The Terminal decides, as
  in Zed: mouse tracking on → an SGR/normal mouse report to the PTY; alt screen + alternate scroll → `ESC O A/B`
  arrow keys; else the scrollback (`display_offset`). Already in the port, unchanged: output arriving while scrolled
  up keeps the view (alacritty), and every `input()` queues `Scroll::Bottom`. No focus check (Zed scrolls only a
  focused or standalone view; Foreman's keys go to the active tab anyway).
- **Windows wheel**: gpui_windows sends `ScrollDelta::Lines`, one notch = `WHEEL_DELTA` × the system's 3 lines,
  positive = up; **shift+wheel becomes horizontal** (x), so Zed's "shift forces a local scroll" never sees a y delta
  on Windows — shift+wheel does nothing in a tab, as in Zed on Windows.
- Tests: `test_report_mode_wheel_without_mouse_mode_scrolls_scrollback` (offset 3, no PTY write, output does not yank,
  a keystroke returns to 0) and `test_report_mode_wheel_on_alt_screen_sends_arrow_keys` (incl. shift → nothing) in
  `crates/foreman-term/src/terminal.rs`; they pin the port's behaviour for Foreman's mode. Selftest step 6
  (`wheel_scroll`): a `-NoProfile` `+` tab, `lines*2+20` lines, wheel events dispatched over the grid through the
  window: `display_offset 0 → 6 → 3 → 0`. Red: handler body disabled → step 6 FAIL "left display_offset at 0".
  The user's pwsh profile prompt has no `PS ` (the first run of step 6 timed out on it), hence `-NoProfile`.
- Hand check (`target/tf-6/`, PrintWindow; real `mouse_event` relative moves + `MOUSEEVENTF_WHEEL`): 3 notches up =
  9 lines, 3 down = bottom, 5 up then `x` typed = bottom, scrolled up while `tick` lines arrive = view held.
  `claude` TUI case not tried (an untrusted scratch dir would need the trust prompt); the alt-screen path is unit-tested.
- **Gotchas, synthetic input**: `Process.MainWindowHandle` of foreman.exe is its console, find the gpui window by
  `EnumWindows` + pid; relative `mouse_event` moves are accelerated (2×20 px landed +79 px), use ±1 steps after
  `SetCursorPos`; check `WindowFromPoint` is Foreman (and make it topmost) before clicking, and that it is the
  foreground window before `SendKeys`.
- Gate (P: had 2.4 GB free → `CARGO_TARGET_DIR` on C:, `plugins` junction beside it, exe copies in the worktree's
  `target/release` for the supervise selftest): `cargo build --release --workspace` ok; `cargo test --workspace`
  307 passed (305 + 2), 0 failed; `--selftest` PASS (p95 21.72 ms, max 24.30 ms, 0 > 50 ms; idle 131.8 MiB — up
  from ~121 MiB, the step-6 shell tab ran and closed before the measurement); `--selftest-supervise` FAIL at step c
  on the first run (the daemon resumed the killed workspace session, not the supervisor — the known model-turn case
  above), PASS on the rerun.

## 2026-10-26 — mint/7 per-session tokens

- **What**: a session whose role has a static WorkState token no longer gets that token in its
  `<FOREMAN_HOME>/mcp/workstate-<name>.json`. At each launch **and each resume** the supervisor POSTs
  `{base}/api/agent-sessions` (`base` = the MCP URL's origin) with the static token as bearer and
  `{name, role (controller/orchestrator), launchedBy, origin, cwd, externalRole (workspace/repo), replace: true}`
  (Skills `docs/workstate/14-agent-launch.md` §2) and writes only the returned `wss_` token, owner-only as before
  (`LaunchSpec::with_minted_token`, `foreman_claude::write_mcp_config`). Code: `crates/foreman-supervisor/src/mint.rs`
  (`Minter` trait, `HttpMinter` on `ureq` 3 without default features — plain HTTP, no proxy, no redirects, 15 s —
  and `MintedSessions`), wired in `executor.rs` (`mint`, `remint`, `bind_claude_sessions`, `revoke_unwanted`).
- **Resume re-mints**: `--bg --resume` takes no flag (m2/1) and re-reads the saved `--mcp-config` path, so the
  supervisor rewrites that same file with a fresh token before the resume. `replace: true` always: the row and
  `sessionId` stay, the old token is revoked server side.
- **Headers dropped**: the file carries `Authorization` only. A `wss_` principal carries name and role and the
  server ignores `X-WorkState-Agent` for it; launcher, origin, cwd and Foreman role are on the row from the mint
  body. Two sources (headers vs row) could only disagree.
- **sessionId** per name in `<FOREMAN_HOME>/supervisor/<w>.sessions.json` (`.json`, not `.toml`: `migrate_home`
  rewrites every `supervisor/*.toml`). No token in it. It survives an app restart, so a Stop after a restart still
  revokes.
- **PATCH** `{claudeSessionId}` once per mint, when the name's single live row shows a `sessionId` (the tick already
  has the `agents --json --all` snapshot). Failed or not, it is not repeated until the next mint; a failure is a notice.
- **Revoke**: every tick, each minted name that is not (workspace started ∧ still wanted ∧ not given up) is
  DELETEd and its config file deleted — Stop, `fm unwant` (R14), give-up, removal from the desired state. A 404 is
  revoked. A failed revoke is a notice (`Supervision` keeps the last 5 among the panel's warnings); the file goes
  anyway, and the unused token lapses after its idle window.
- **Failure**: a refused or unreachable mint fails the launch/resume into the existing `launch failed` row with
  Retry and backoff, detail `cannot mint a WorkState token for <name>: WorkState answered HTTP <status>: <reason>`;
  nothing is launched and **no fallback** to the static token. `RoleToken`'s and `Minted`'s `Debug` redact the token;
  `MintError` never carries the bearer.
- **Expiry**: 72 h idle window, sliding on use (rlarch 2.2.0). A session idle longer gets HTTP 401 on `/mcp`;
  nothing polls for it — its next resume (or relaunch) re-mints with `replace`. A session that is live but idle past
  the window and then prompted keeps failing its WorkState calls until it is restarted.
- **Not covered**: sessions already running when this lands keep their old file (with the static token) until their
  next resume rewrites it. A workspace removed from the registry drops its supervisor without revoking.
- Tests (`cargo test -p foreman-supervisor mint`, 15): fake minter — mint body and bearer, only `wss_` in the file
  and neither token in argv, identity per session, resume replaces on the same `sessionId`, PATCH once per mint,
  DELETE on Stop / unwant / give-up, failed revoke = notice, persistence across a restart, mint failure = launch
  failed with Retry and no file; real `HttpMinter` against a `TcpListener` thread (POST/PATCH/DELETE shapes, 409
  reason, unreachable). `cargo test -p foreman-app mint`: `AppTokens` from a fake `.env` → `HttpMinter` → fake
  server → config holds the fake `wss_`, DELETE on Stop. Red: returning the static token from `mint` fails 4.
- **Selftest**: no new `--selftest-supervise` step — it launches real sessions, which would then load an MCP config
  pointing at a fake server, and the selftests run without `workstate_env`; the app-level test above covers the
  same path in-process. With no `workstate_env` there is no token, no mint, nothing changes (selftest PASS).
- **Live check (controller's, needs the real token)**: with `settings.toml` `workstate_env` set, Start a workspace;
  `<home>/supervisor/<w>.sessions.json` gets a `sessionId`; `<home>/mcp/workstate-fm-<w>-workspace.json` holds a
  `Bearer wss_…` (check with `Select-String wss_ -Quiet`, do not print it) and not the static token; WorkState's
  `agent_session` row has name, role, external_role and, after the first poll, the claude session uuid; Stop → the
  row is revoked and the file is gone.
- Gate (P: had 12 GB free, built in place): `cargo build --release --workspace` ok; `cargo test --workspace` 323
  passed (309 + 14), 0 failed; `--selftest` PASS (p95 20.45 ms, max 28.20 ms, 0 > 50 ms; idle 131.2 MiB);
  `--selftest-supervise` PASS on the first run.
