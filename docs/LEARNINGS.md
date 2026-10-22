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
