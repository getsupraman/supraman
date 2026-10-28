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

## 2026-10-26 — Foreman sessions use the published rlarch (2.2.1)

- rlarch 2.2.1 is published and installed user-scope (`~/.claude/plugins/cache/rlarch/rlarch/2.2.1`, git fbbaf42).
  Measured: `claude -p` with Foreman's session flags (`--plugin-dir plugins/foreman --strict-mcp-config
  --mcp-config <file>`) loads `rlarch:code-comments` from that cache path. Foreman's sessions therefore get the
  published plugin with **no** `extra_plugin_dirs`; René's real `settings.toml` never set it. Only the test homes of
  m3/4 pointed it at the Skills checkout, which ran unreleased skills. Decision (René): use the published plugin and
  keep `extra_plugin_dirs` for plugins that aren't installed.
- Not yet measured: whether `--strict-mcp-config` also hides a plugin's own MCP servers (rlarch's `agents` server,
  `agent_launch`). Queued after M4 (backlog 6, then 7: repo-controllers launching units via `agent_launch`).

## 2026-10-26 — m4/1 read API client

- **Crate `foreman-workstate`**: serde models for every read API v1 shape (Skills `docs/workstate/10-read-api.md`),
  a blocking `Client` (ureq 3 like the minter: no TLS, no proxy, no redirects, 15 s), and `Stream`, the
  `/api/stream` subscriber. Read only (C21): `GET` and nothing else. Models are lenient by construction: no
  `deny_unknown_fields`, every struct `#[serde(default)]`, `| null` → `Option`, and `ItemStatus`/`ItemResolution`
  carry `#[serde(other)] Unknown` so a value added after v1 decodes instead of failing the whole response. `flags`,
  inbox `kind` and item `kind` stay strings for the same reason. Instants stay ISO strings.
- **Every 2xx must carry `X-WorkState-Api: 1`**, else `Error::Api` (missing, or another version). Non-2xx maps to
  `Error::Http{status, reason}` before the header check (a 404 from something that is not WorkState has none);
  `reason` is the JSON `error/title/message/detail` cut to 200 chars. The token is scrubbed from every reason
  (a server that echoes it back, as the fake does, prints `<redacted>`); `Client`'s `Debug` redacts it.
- **URLs take ids**: `board(run_id)`, `item(id)`, `brief(id, for)`; refs only go to `resolve(ref)`, ids and
  query values percent-encoded (`kinds=transition%2Cnote` — the comma is encoded too, which a server decodes).
- **Token**: `client_from_settings` reads `WORKSTATE_READER_TOKEN` from `workstate_env`, else
  `WORKSTATE_CONTROLLER_TOKEN`; base = origin of `workstate_url` (default `DEFAULT_WORKSTATE_URL`, now in
  foreman-core). `read_env_value` moved from foreman-app's `tokens.rs` to `foreman_core::read_env_value` (tokens.rs
  re-exports it); `api_base` moved from `foreman_supervisor::mint` to `foreman_workstate::api_base` (mint re-exports
  it), so the supervisor depends on foreman-workstate.
- **Stream**: plain HTTP/1.1 on a `TcpStream` with a 200 ms read timeout, not ureq — ureq's timeouts are per phase
  (a whole body), so a silent server would make the thread unstoppable. Handles chunked, `Content-Length` and
  close-delimited bodies; SSE per WHATWG (`data:` lines join with `\n`, `:` comments, `\r\n`/`\r`/`\n`, `event:`
  ignored, `id:` sent back as `Last-Event-ID`, `retry:` ignored). Signals: `Connected` (after every 200 — the app
  must re-read everything then, frames between a loss and a reconnect are gone), `Event(StreamEvent)`, `Lost(reason)`.
  Backoff 0.5 s doubling to 30 s, reset by a 200. `stop()`/drop returns within one poll. No `X-WorkState-Api` check
  on the stream (doc: "unchanged transport"). A frame whose data is not a `StreamEvent` is skipped.
- **Gotcha**: on Windows a connect to a closed 127.0.0.1 port takes ~2 s to be refused (SYN retries), so an
  unreachable-server test costs 2 s per attempt unless the connect timeout is short.
- **fm**: `fm work overview <workspace>` resolves `<workspace>` (kind `project`, exactly one), prints the project
  rollup, its runs (rollup, status, branch, claims, inbox, last verify), its claims from `workingNow`, inbox and
  honesty counts. `fm work board <workspace> <run-ref>` resolves the run ref; since a ref can name runs of other
  projects, it reads each `kind=run` candidate's board and keeps the one whose `project.id` is the workspace's.
  Rows: indented seq, status, criteria n/m, claim holder (session name, else agent; `(expired)`), pass/FAIL/-,
  title, container rollup, open blockers, flags. "n/m done" = done / (total − cancelled − superseded). Errors are
  one line `error: …`, exit 1, never the token.
- **Fake**: `foreman_workstate::fake::FakeWorkState` (feature `fake`, a dev-dependency only) answers exact targets
  from a table and records method, target and `Authorization`; fixtures in `crates/foreman-workstate/tests/fixtures/`
  are doc-10-shaped with an `addedLater` field in most objects and future enum values.
- Tests: `cargo test -p foreman-workstate` 16 (8 unit: SSE split at every byte, chunked split at every byte, non-200,
  query encoding, Debug redaction, token choice; 6 client: every endpoint's target, bearer, GET only, decode of each
  fixture, 401/404/500/no header/v2/bad JSON/unreachable; 2 stream: chunked frames + drop + 401 + reconnect with
  `Last-Event-ID` + stop while silent, backoff growth while unreachable). `cargo test -p fm --test work` 4 (overview,
  board picking the workspace's m4 among two, one-line errors, `--help`). Red: header check and token scrub disabled
  → 2 failed (client errors test, fm errors test).
- Gate (P: had 11 GB free at the start, built in place; 4.6 GB after): `cargo build --release --workspace` ok;
  `cargo test --workspace` 342 passed (323 + 20 new − 1 `api_base` test moved), 0 failed; `--selftest` PASS (p95
  25.01 ms, max 29.29 ms, 0 > 50 ms; idle 133.2 MiB); `--selftest-supervise` FAIL at step c on the first run (the
  daemon resumed the killed workspace session, not the supervisor — the known case), PASS on the rerun.

## 2026-10-26 — m4/5 task box

- **Why**: in the m3/4 E2E the human threw the task and answered `APPROVE?`/`DECIDE:` with `fm send` in a terminal.
- **Task box** (`crates/foreman-app/src/compose.rs`, `view.rs` `render_compose`): a started workspace whose row
  shows `relay ready` has **Task…**; it opens a box under the row. Not shown otherwise: the relay label beside it
  says why, and a long label (`relay up, watcher not armed yet`) plus the button pushed Stop past the panel's right
  edge — step s failed on it ("stop-…-u is not fully inside its visible area": inside vertically, out on the right). gpui has no text area in our deps (Zed's `editor` is not vendored), so
  the box is a minimal input of its own: while it has the focus the window's `KeyDown`s go to it, not to the active
  tab (`key_down`) — characters by `key_char` (AltGr = Ctrl+Alt passes), Enter a new line, Ctrl+Enter sends,
  Escape cancels, Backspace/Delete/arrows/Home/End, Ctrl+V pastes. No IME composition (a dead key or IME text
  arrives as `WM_CHAR` → the platform input handler, which nothing registers — same as the terminal tabs), no
  selection. A left click in the terminal gives the keys back; a click on the box takes them.
- **Send** = `foreman_core::send(home, w, "fm-<w>-workspace", text.trim())` — what `fm send` does. Refused with the
  reason shown in the box unless the row shows `relay ready` (`FleetView::relay_refusal`: stopped / relay not
  running / watcher not armed / expired; the box shows "Cannot send now: …" while that holds). Success clears the
  box and shows `sent: <first line>`; it stays open for the next one.
- **Answers** (Messages, only rows from `fm-<w>-workspace`): `APPROVE?` → **Approve** (sends `approve`) and
  **Reply…**; `DECIDE:` → **Reply…** (the same box, under the row, empty). After a send the row shows
  `answered: <first line>`: in memory, and in `<home>/relay/<w>/answered.json` (`{outbox file: first line}`,
  `foreman_core::mark_answered`, tmp + rename), read back by every tick, so a restart does not offer Approve twice.
- **Decision — supersede**: an unanswered `APPROVE?` with a later `APPROVE?` from the same sender shows
  "superseded by a later APPROVE?" and no buttons (`foreman_core::ask_states`): the skill re-sends `APPROVE?` after
  every change request, so only the newest plan is approvable. `DECIDE:` questions are per unit and never
  supersede each other.
- **Selftest step t** (`--selftest-supervise`, run after c): Task… clicked, a two-line task typed with synthetic
  `KeyDown`s through `window.dispatch_event`, Ctrl+Enter → a drop `{to: fm-selftest-<hex>-workspace, body}` (in
  `inbox/` or `inbox/sent/`); an `APPROVE?` injected into the outbox as from the workspace session → Approve +
  Reply… painted, Approve clicked → a drop `approve`, `msg-answered-…` painted, `answered.json` has it; text typed +
  Escape → box closed, nothing dropped; then both drops in `sent/` and the workspace session idle 15 s without a
  break (step f's budget of 1 must not meet it mid-turn). `FOREMAN_SELFTEST_HOLD_SECS` holds the screen for a
  screenshot (`target/m4-5/task-box-answered.png`, PrintWindow).
- **Gotcha — step t before step c fails c**: the drops reach the real workspace session as the human's prompts (a
  non-task, then an `approve` with no plan); it answers, and ends those turns `working`/`blocked` rather than with
  `done: …`. Step c then kills an idle-looking session whose `state` the daemon respawns by itself (m2/4), and c
  fails "came back … but not by the supervisor". Run twice right after r: FAIL at c both times, the second even
  after waiting for both drops delivered + 15 s idle. Step t now runs after c, as the brief's "after the relay step"
  still allows.
- Tests: `relay::tests::a_question_is_known_by_its_first_word`,
  `…a_later_approve_supersedes_earlier_unanswered_ones_and_decides_stay_open`,
  `…answered_questions_persist_per_workspace` (foreman-core); `compose::tests::*` (3, foreman-app). Red: supersede
  rule disabled → 1 failed.
- Gate (P: had 4–9 GB free; built in place, `target/debug` deleted after the first `cargo test` to make room):
  `cargo build --release --workspace` ok; `cargo test --workspace` 329 passed (323 + 6), 0 failed; `--selftest`
  PASS (p95 20.95 ms, max 24.73 ms, 0 > 50 ms; idle 132.1 MiB); `--selftest-supervise` PASS on run 5. Runs 1–2 (t
  right after r) FAIL at c, see the gotcha; run 3 (t after c) FAIL at c with t not yet run — the m2/4 model-turn
  case, independent of this change (c's message now names the killed row's state); run 4 passed c and t, FAIL at
  s — the Task… overflow above, fixed.

## 2026-10-26 — m4/2 Work tab

- **What**: a **Work** button on each workspace row (right after the name) opens or focuses the right-pane tab
  `Work · <workspace>` (`TabKind::Work(Entity<WorkView>)`, no terminal; × and middle click close it like any tab).
  It shows the WorkState project whose ref is the workspace's name: the overview's runs **in the overview's own
  order** (stable while you click; newest-activity order would reshuffle on every event) with rollup bar,
  `n/m done`, status, claims, inbox, last verification; a click expands a run into its board — rows by
  `qualifiedRef`, indented by `depth`, status, criteria `checked/total`, claim (holder @ tree), verification
  (pass/fail by whom), commit, `blocked by` refs, flags as badges. Open runs start expanded, done/cancelled/
  superseded collapsed. Code: `crates/foreman-app/src/work_view.rs`; view.rs only wires the button, the tab kind,
  `open_work` and the render arm.
- **The run's own row**: the board lists the run first (numeric `ref`, e.g. `13`); `item_rows` drops the row whose
  item id is the board's run id — the run is the section header.
- **Live, one stream per app**: `WorkHub` is a gpui `Global` entity created by the first Work tab: it reads
  `client_from_settings` on the background executor, starts one `Stream`, and forwards its signals as entity events
  (a background task blocks on `recv_timeout(1 s)`, so a pool thread is parked on it; it ends when the hub is
  gone). Each `WorkView` subscribes: any event → overview re-read, an event whose `runId` is an expanded run → that
  board, both debounced 300 ms (a pending flag, so a burst costs one read); `Connected` after `Lost` → re-read the
  overview and every expanded board. No timer polling at all. Answers carry `seq`: an older overview or board
  arriving after a newer one is dropped. A failed board re-read keeps the shown board.
- **Messages, nothing else**: no `workstate_env`/token → "WorkState is not configured: … Choose its .env in
  Settings (Ctrl+,)."; resolve finds no project of that ref → "WorkState has no project with ref '<w>' …"; two →
  says so; transport/HTTP errors → "Cannot read WorkState: …" (the client's errors are already token-free). An
  unconfigured hub re-reads the settings when the next Work tab opens.
- **Seam for m4/3**: a row click selects it (`element_selected` background) and calls `WorkView::on_select_item`,
  which emits `WorkEvent::ItemSelected(item_id)`; the selftest subscribes to it.
- **Header width** (m4/5's step s): the Work button adds fixed width to the row, so the relay label now truncates
  (`min_w_0` + `overflow_hidden`) instead of pushing Task…/Stop past the panel edge; step s passed.
- **Fake**: `FakeWorkState` routes are now mutable (`set`), and `/api/stream` (when not routed) is held open as SSE
  — `push(data)` writes a frame to every open stream, `drop_streams` closes them. foreman-app depends on
  foreman-workstate with `fake` on, since `--selftest` runs it. Test: `crates/foreman-workstate/tests/fake.rs` (2).
- **Selftest step 7** (`--selftest`, between the wheel step and the idle measurement, now step 8): workspace
  `selftest-work` (empty scratch dir `target/selftest-work`, not `selftest-cwd`, whose session belongs to
  `larchsys`), a fake with project `selftest-work`, runs sw2 (in progress) and sw1 (done), and the scratch
  home's settings → a fake `.env` + the fake's URL. Clicks Work, asserts both runs and sw2's 3 rows painted and the
  run row/sw1 rows not, frames during the load (0 > 50 ms), expands sw1, selects sw2/2, then changes sw2/1 on the
  fake and pushes 3 events: `work-status-sw2/1=done` painted within 2 s, exactly 1 board + 1 overview re-read; every
  request a GET with the fake token; middle click closes the tab. Result: painted 52 ms after the click, live update
  ~360 ms after the push. `FOREMAN_SELFTEST_HOLD_SECS` holds the tab for a screenshot
  (`target/m4-2/work-tab-fake.png`, PrintWindow).
- Tests: `work_view::tests::*` (5: project pick, run row as header, event → board only when shown, closed runs
  collapsed, formatters), `tests/fake.rs` (2). Red: `board_to_reload` forced to `None` → step 7 FAIL "sw2/1 done
  after the stream events: not painted within 2000 ms".
- **Gotcha**: pwsh 7 `Add-Type` C# can't use `System.Drawing.Bitmap` (forwarded to assemblies Add-Type won't
  reference); keep the C# to `EnumWindows`/`PrintWindow` on an HDC and build the `Bitmap` in PowerShell.
- Gate (after merging master ccc7b4a; `CARGO_TARGET_DIR` on C: with the `plugins` junction, exe copies in the
  worktree's `target/release` for supervise, deleted after): `cargo build --release --workspace` ok; `cargo test
  --workspace` 355 passed, 0 failed; `--selftest` PASS (p95 39.82 ms, max 41.99 ms, 0 > 50 ms; idle 122.7 MiB —
  frame times on this machine were ~35 ms p50 the whole session, also before the Work step); `--selftest-supervise`
  PASS on the first run.

## 2026-10-26 — m4/2b frame-time regression

- **Cause**: `FleetView::render` rebuilds the tree every frame (`fleet::build` → `assign` → `repo_of`), and
  `repo_of` falls back to `main_repo(cwd)` — a filesystem walk up to the nearest `.git` — for every session its
  repos don't contain by prefix. m4/2's selftest registers workspace `selftest-work` with **no repos**, so every
  session on the machine (271) missed the prefix and was walked on every frame, on the UI thread. Measured with a
  walk counter over the 6 s idle window: ~85 000 walks over ~170 frames (≈500/frame). The pre-existing `larchsys`
  workspace already walked ~237/frame (sessions in `*.worktrees` and outside its repos), which is why ccc7b4a sat
  at ~19 ms rather than at vsync. The Work tab, `WorkHub`, the stream and the relay truncation were cleared by
  measurement: idle frames were already ~35 ms *before* step 7 ran, and skipping only the registry entry in
  `selftest_work::prepare` brought them back to 19.6 ms. In the real app any repo-less workspace, or any session
  outside the registered repos, costs this per frame.
- **Fix**: the poll (background thread) computes `fleet::main_repos(sessions)` once per distinct cwd; `FleetView`
  keeps it beside `sessions` and passes it to `fleet::build` through `Supervised::main_repos`, which uses
  `foreman_claude::assign_with` instead of walking. `repo_of` also returns `None` at once for a workspace without
  repos. `assign`/`repo_of` keep their signature (fm uses them).
- **Guard**: `foreman_claude::main_repo_walks()` counts walks; `--selftest` step 8 fails when the idle window sees
  more than (refreshes + 1) × sessions walks, i.e. any rendering walk. Red: `main_repos: None` in `FleetView::tree`
  → `FAIL step 8: 75387 main_repo walks over 318 idle frames, at most 1084 expected` — with p50 still 18.7 ms
  (the empty-repos early return hid the frame cost), so the walk count is the check, not a frame budget. Unit
  test: `fleet::tests::a_polls_main_repos_table_places_worktree_sessions_without_touching_the_disk`.
- **Numbers** (same machine, alternating, release, `--selftest` idle line): ccc7b4a p50 19.0/19.6/19.3/20.0/19.3
  ms, ~300-315 frames; b999d77 p50 34.9/35.4/37.5/35.7/36.1/35.0/35.9 ms, ~160-172 frames; fixed p50
  16.66/16.63/16.64/16.64 ms, ~360 frames (vsync), p95 ≤ 20.5 ms, 660 walks (3 polls × 220 distinct cwds).
- **Gotcha, comparing builds**: extracting another commit (`git archive | tar`) and building it with the *same*
  `CARGO_TARGET_DIR` silently links the other tree's path crates — the extracted files carry old mtimes, so cargo
  treats the existing `foreman-claude` rlib as fresh. A "clean b999d77" measured 20 ms that way because it had
  linked the fix. Give each tree its own target dir (or `touch` its sources).
- **Flake**: `--selftest` step 6 (`wheel down: display_offset 1 → 0 → 0`: two notches up moved only 1 line)
  failed in a cluster of consecutive runs on all three builds, ccc7b4a included; unrelated to this change.

## 2026-10-26 — m5/4 direct messages

- **Why**: the relay's `SendMessage` was refused by the auto-mode classifier in 3 of 18 selftest runs (7-d2).
  René chose the hybrid: human → session goes **straight into the session's pipe**, session → human is
  **`fm tell`**, and the **relay stays as the fallback**. Protocol facts: `docs/peer-messaging.md`
  (autowork-36's measurements, claude 2.1.304–2.1.310).
- **Down**: `foreman_claude::peer` (`find_session`, `deliver`) + `human::Sender::send`, used by the task box,
  Approve/Reply and `fm send`. Read only on `~/.claude/sessions`: exact name, pid alive **with the same
  `procStart`** (= the process creation FILETIME, equal to `GetProcessTimes` for all 20 live sessions),
  newest wins, `peerProtocol == 1`, the `.key`'s token read, written and dropped (`Token`'s `Debug` is
  redacted; key-file errors never carry its text). One connection: auth line + user line, no `from`, no
  `from-mode`, 5 s bound. Content: `<cross-session-message from-name="foreman">` + `From the human (via Foreman,
  workspace <w>) [m-<8hex>]:` + body.
- **Direct only to a session whose launch mode Foreman knows is prompting** (`DesiredState::launch_mode`:
  `auto`/`default`/`acceptEdits`): a bypass or plan receiver holds an unmoded message silently and Foreman would
  never hear of it. Unknown or other mode → the relay, with the reason.
- **"sent direct" means written, not confirmed.** Foreman has no inbox, so no receipt comes back; a rate,
  duplicate or queue-full drop would be silent. Mitigations: a fresh `[m-<8hex>]` per message (never the same
  text twice), `relay/<w>/direct.json` keeps ≥ 10 of the receiver's 30-token burst (refill 0.5/s; one send a
  second spends half a token each), else the relay. Proof of delivery is only the receiver acting on it.
- **Which path** shows everywhere: the box `sent direct [m-…]: <line>`, the answered row `answered: approve ·
  sent direct [m-…]` (or `sent via relay (<why>)`), `fm send` prints it.
- **Up**: `fm tell "<body>"` / `fm tell --file <path>` writes `relay/<w>/outbox/<ms>-tell-<pid>-<seq>.json`
  `{from, from_name, body}` as the relay does (never `msg-NNNN`, the relay's numbering); the sender is
  `FOREMAN_SESSION` (new in the session env), else derived from `FOREMAN_ROLE`/`FOREMAN_REPO` for sessions
  launched before. Allowed inside Foreman sessions (unlike `fm send`); `Bash(fm:*)` already covers it.
- **Skills**: the workspace controller reaches the human only with `fm tell` (`--file` for anything multi-line)
  and reads both shapes as the human: Foreman's frame (`from-name="foreman"` + that first line) and the relay
  fallback (`from-name="fm-<w>-relay"`). The repo controller never runs `fm tell`. Pinned in
  `crates/fm/tests/skills.rs`, including the exact first line `human::framed` writes. In the selftest the
  workspace session's first turn announced ready with `fm tell` on its own.
- **Incident, and the guard**: the first `peer.rs` test used a live session's pipe path copied from the
  registry as its fixture; two `cargo test` runs connected to it (fake token, "hi"). Now under `cfg(test)`
  `deliver` refuses every pipe outside `\.\pipe\LOCAL\cc-msg-foremantest-*` (test:
  `under_test_a_real_looking_pipe_is_refused…`; red: guard off → 1 failed), and the wire test serves its own
  `cc-msg-foremantest-<pid>-wire` pipe. The selftest writes only to `fm-selftest-<hex>-*` names it chose,
  after checking the registry `cwd` (`own_peer`).
- **Dropping the relay later** would need: a reply path for sessions not launched by Foreman (none: they answer
  with `fm tell` only if they run Foreman's plugin), a fallback for bypass-mode sessions (stream-json stdin for
  sessions Foreman launches, or `crossSessionInbound: accept`), and confidence that `peerProtocol 1` holds
  across claude updates (watch `peerProtocol`/`peerFeatures`). Until then it costs one idle session per
  workspace.
- Tests: `peer::tests` (12: key-file name with a fixed fixture, stale/reused pid, newest wins, unknown
  protocol, missing/broken key never echoed, wire lines, unwrappable body, liveness, test guard, served-pipe
  wire), `human::tests` (6: bucket, ids, launch mode, fallback reasons, pacing), `relay::tests::tell_…`,
  `desired::tests::the_launch_mode_…`, `fm/tests/relay.rs` (`tell`, fallback wording), `fm/tests/skills.rs` (2 new).
- Gate (P: had 8.8 → 4.6 GB free: `CARGO_TARGET_DIR` on C: with the `plugins` junction, exe copies in the
  worktree's `target/release`, deleted after): `cargo build --release --workspace` ok; `cargo test --workspace`
  377 passed, 0 failed; `--selftest-supervise` PASS on the first run (step r: 30/30 direct nonces answered in
  42.3 s, fallback via relay 11.5 s round trip; step t: task box `sent direct`, in the transcript after 0.3 s;
  Approve direct; 32 direct deliveries, 0 held, 0 dropped); `--selftest` FAIL twice at step 7 (Work tab frame
  times, p95 54.5 / 146.5 ms; steps 1–6 pass) — the frame-time fix aw-m4-2b is doing in `work_view.rs`, code
  m5/4 does not touch.

## 2026-10-26 — m4/2c wheel step flake

- **Cause (a test bug, not an app bug)**: step 6 sized its output (`lines * 2 + 20`) from
  `last_content().terminal_bounds` read right after the tab's terminal appeared. `TerminalBuilder` builds it with
  placeholder bounds (`TerminalBounds::default()`: origin 0,0, 6 lines); the grid's first prepaint
  (`term_element::prepaint` → `set_size`) gives it the real 41. Read in that window, the step typed `1..32`, which fit
  on the 41-line screen and left 1 line of scrollback, so the 2-notch `Scroll::Delta(6)` was clamped to 1.
- **Measured** (temporary `eprintln!` in `scroll_wheel`, the `Scroll` apply, `sync`, `Resize`, and the step's wait loop):
  every run delivered `Lines(0, 6)` → `Delta(6)` with `scroll_px` 0 → 108 px. Passing runs: `history 64`,
  `0 -> 6`. Failing runs: `history 1`, `0 -> 1`, wait loop `total_lines=6` 12 ms before the first `resize` to
  754 px. 3 of 15 runs failed before the fix (runs 1, 10, 14). Wheel, notch size and the prepaint-applied `Scroll`
  were all correct.
- **Not user-visible**: the PTY gets the real size at the first prepaint, before the shell prints its prompt, and a
  user cannot see or scroll the grid before it is painted. Only code that reads `terminal_bounds` before the first
  paint sees 6 lines.
- **Fix**: `selftest.rs` `wheel_scroll` waits until the terminal's bounds have a non-zero origin (laid out by the grid)
  before sizing the output, and fails with `… left N lines of scrollback; two notches up need 6` if the scrollback
  is too short, so a sizing bug cannot pass itself off as a wheel bug again.
- **Proof**: 30 of 30 consecutive `--selftest` passes (`0 → 6 → 3 → 0`). Red: wheel handler call removed in `view.rs`
  → `FAIL step 6: two wheel notches up … left display_offset at 0` (2/2). Race forced (`lines` = 6) →
  `FAIL step 6: 32 lines in a 6-line grid left 1 lines of scrollback; two notches up need 6` (2/2), the exact
  numbers of the original flake.
- **Pattern**: before a selftest reads anything sized by layout (`terminal_bounds`, `num_lines`), wait for the
  first prepaint. A fresh `Terminal` reports 6 lines until then.

## 2026-10-26 — m4/3 item page

- **Layout**: a pane to the **right of the board** in the same Work tab, not a stacked view: the board keeps 42%
  (min 380 px) of a 1300–2000 px tab, so the next row is one click away and stays highlighted. Code:
  `crates/foreman-app/src/work_view/item.rs` (child module of `work_view`). A board row opens the item with an
  empty history; path elements, children, findings and links navigate inside the pane and push onto **Back**
  (`← Back`, then `← Board`); **Escape** closes the pane. Escape goes through `FleetView::key_down` (the root
  holds focus and a Work tab has no terminal), which hands it to the active Work tab's `WorkView::escape`, so
  WorkView needs no focus handle of its own.
- **Sections**, each hidden when empty: header (breadcrumbs from `path`, clickable; ref, kind, status,
  resolution, severity, risk tags, title, opened/started/closed), intent, verify command, claim, criteria (☑/☐,
  evidence, checkedAt), reports, verifications, artifacts, children, findings, notes (kind, from → to, by,
  consumed or **not consumed**), links out ("this blocks it") and in ("it relates this"), assumptions, recent
  events (collapsed by default; "older events are not shown" when `eventsHasMore`).
- **Said vs proven**: reports are muted, italic, dashed border, badge `claimed`; verifications have a solid
  border and tint in their result colour and a bold `✓ proven` / `✗ failed`. A passed verification with
  `runBy == "worker"` is `✓ passed (worker's own run)` in warning colour and does not count as proof.
  `done_unverified` = status `done` and no verification that passed with `runBy != "worker"` → a warning banner
  "Done, but no orchestrator verification proves it". The fixtures use `runBy: "controller"` for the proven
  case, so "orchestrator verification" here means any non-worker role.
- **Claim → Attach**: `FleetView` keeps `work_live: Rc<RefCell<Vec<LiveSession>>>`, rewritten on each poll in
  `apply` from the sessions `fleet::attachable` accepts (Claude uuid, 8-hex id, name), and passes it, plus an
  attach callback that calls `FleetView::attach_by_id` through a weak handle, to `WorkView::open` as a
  `FleetLink`. The page joins `claim.session.claudeSessionId` to `LiveSession::claude_session_id` (the name is
  never a join): a match shows "worked on by <name>" + **Attach**; no match shows "claimed by <holder>" with
  tree and expiry and no button. The attach tab's title reads `<workspace> · supervised · <name>` (the Inbox's
  `attach_by_id` label).
- **Live**: a stream event whose `itemId` is the open item, or whose `runId` is the item's run (from `path`, or
  the item itself when it is a run), schedules one debounced re-read (`DEBOUNCE`, as the boards); a resync
  re-reads it. Reads run on the background executor; an answer for an item the pane has left is dropped, an
  older `seq` is dropped, a failed re-read keeps the page on screen. No new polling.
- **Selftest step 7** (`--selftest`, after the m4/2 checks): uses the **session the gate launched** (step 2), not
  an injected fleet row — `selftest_work::run` now takes that `Session`, and sw2/1's item route is set once its
  `session_id` is known. Clicks sw2/1 → 19 keys painted (header, both path elements, intent, verify command,
  claim, criteria c1/c2 checked and c3 unchecked, `item-report-1`, `item-verification-0=pass`,
  `item-attach-<id>`, artifact, note, both links, assumption, events toggle) and no warning, events or empty
  sections; pushes one event after checking c3 on the fake → `item-criterion-c3=checked` within 2 s and exactly
  1 item re-read; holds here for `FOREMAN_SELFTEST_HOLD_SECS`; expands the events; follows the in-link to sw1/1
  (done, only a worker verification; its board row now says so too) → `item-done-unverified`; Back → sw2/1
  (no warning), Back → the board; reopens, Escape → closed; frames over all of that must be ≤ 50 ms; then
  Attach from the claim, through `selftest::open_attach` / `close_attach` (now `pub(crate)`, `open_attach`
  takes the click key): the tab paints the session and detaches with the session alive as the same pid.
  Result: page 35 ms after the click, c3 ~330 ms after the push, item-page frames max 22.7–24.1 ms (one earlier
  run 49.4 ms, the first time the pane laid out — near the budget, watch it), attach detached in ~500 ms.
  Screenshot: `target/m4-3/item-page-fake.png` (PrintWindow during the hold).
- Tests: `work_view::item::tests::*` (5: done-unverified rule, event → item re-read on item or run, run of an
  item, claim join only on `claudeSessionId`, keys/labels). Red: the item re-read in `on_hub` forced off →
  `FAIL step 7: item page: c3 checked after the stream event: not painted within 2000 ms`.
- Gate: `CARGO_TARGET_DIR=C:\fmbuild\m4-3\target` with the `plugins` junction at `C:\fmbuild\m4-3\plugins` (one
  level deeper than the brief's `C:\fmbuild\plugins`, so it cannot clash with another worker's junction; the
  exes walk up and find it the same way); `cargo build --release --workspace` ok; `cargo test --workspace` 383
  passed, 0 failed; `--selftest` PASS ×3 on the final build (and on the two screenshot runs); `--selftest-supervise` PASS on the
  first run, from exe copies in the worktree's `target/release`.

## 2026-10-26 — m5/2 packaging

- **Release layout** (docs/RELEASE.md): `Foreman\{foreman.exe, fm.exe, VERSION, plugins\foreman\{hooks,roles,skills,
  .claude-plugin,bin\fm.exe}}`, built by `scripts\release.ps1` (honours `CARGO_TARGET_DIR`; default out
  `<target dir>\dist\Foreman`, zip `Foreman-<crate ver>-<yyyyMMdd>-<sha>.zip` beside it, ~8.6 MB). No signing, no
  installer for v1 — a zip extracted anywhere is enough; SmartScreen may ask once for an unsigned exe.
- **One lookup**: `foreman_core::find_plugin_dir` replaces `supervise::find_plugin_dir` and `fm`'s `plugin_of`:
  `<exe dir>\plugins\foreman`, then the plugin the exe sits in (`<plugin>\bin\fm.exe`), then the walk up. A plugin
  needs `hooks\hooks.json` and `roles\` (the app's old check only wanted the hooks). The error names both places.
- **claude version policy**: `MIN_VERSION` 2.1.308 (the oldest CLI every fact — `--bg`, `agents --json --all`
  fields, `--plugin-dir`, `peerProtocol 1` — was measured on) refuses; up to `TESTED_VERSION` 2.1.310 runs;
  newer, including a new major, runs with a warning (app notice, `fm: warning:` on stderr, `WARNING` line in both
  selftests). The old check refused any major but 2 — a claude 3.0 would have blocked every user.
- **The selftest-supervise target used the repo's plugin** (`cargo_root()/plugins/foreman`), whose `bin\fm.exe` only
  exists if a dev build ran there. From the release folder the app installs fm into its own plugin, so the target
  had no `fm` and step r failed `answered 0 of 30 direct nonces within 360 s` (a natural red). It now uses
  `find_plugin_dir(current_exe)`, the plugin the app under test found. `fm probe hooks` still uses the repo's — a
  dev probe.
- **Still repo-bound in a release exe**: both selftests put their scratch (`FOREMAN_HOME`, cwd, testbed) under
  `<cargo root>\target` baked in at compile time (`selftest::cargo_root`), on purpose: an already-trusted tree. A
  release folder copied to another machine cannot run the selftests; the app itself has no compile-time path.
- **cargo test with `CARGO_TARGET_DIR` outside the repo** still needs a `plugins` junction above the target dir:
  `fm/tests/want.rs` runs the debug `fm.exe` without a role file, so it walks up for the repo plugin.
- Gate (CARGO_TARGET_DIR=C:\fmbuild\m5-2, CLI 2.1.310): `cargo build --release --workspace` ok, no warnings;
  `cargo test --workspace` 382 passed, 0 failed (378 + plugin lookup 2, version cases 3 − 1 replaced); release zip
  extracted to `C:\fmbuild\dist-test\Foreman`, no junction, nothing named `plugins` above it: `--selftest` PASS and
  `--selftest-supervise` PASS (step r 30/30 direct), both on the first run after the fix. Copy deleted after.

## 2026-10-26 — m5/9 terminal input

- **What was missing**: only wiring. `foreman-term` still had Zed's whole mouse model (`Terminal::mouse_down/drag/move/up`,
  `mappings/mouse.rs` SGR/normal/UTF-8 reports, alacritty selection, `copy`, `paste` with bracketed paste); the
  port's `term_element.rs` never called it and never painted `content.selection`. Now `term_element::register_mouse_listeners`
  (window-level listeners + a hitbox, as Zed's `TerminalElement`) feeds them, `selection_rects` paints the highlight, and
  `view.rs` `key_down` maps Ctrl+Shift+C (copy), Ctrl+Shift+V / Ctrl+V (paste; Ctrl+V with no text on the clipboard,
  e.g. an image, still goes to the program as ^V). Right-click copies a selection, else pastes. Ctrl+C stays the program's.
- **What `claude` negotiates** (2.1.310, measured with `crates/foreman-term/examples/pty_dump.rs`, a raw ConPTY dump):
  conhost first asks for win32-input-mode and focus (`CSI ?9001h CSI ?1004h`, we ignore 9001); claude then enables
  bracketed paste `?2004h`, `?2031h`, focus `?1004h`, mouse `?1000h ?1002h ?1003h ?1006h` (any-motion, SGR), queries
  XTVERSION `CSI >0q`, kitty keyboard `CSI ?u`, OSC 7501, and resets modifyOtherKeys `CSI >4m`. `claude attach` sends the
  same mouse/paste set. We answer neither query (alacritty's `kitty_keyboard` is off), so claude stays on legacy keys.
- **Shift+Enter**: claude inserts a newline for LF (what `keys.rs` already sent for Shift+Enter), `ESC CR`, kitty
  `CSI 13;2u` and `CSI 27;2;13~`, directly and through `claude attach`; a win32-input-mode Shift+Enter record submits.
  With real OS keystrokes (SendInput) into a Foreman tab, Shift+Enter inserted a newline both in a `claude` tab and in a
  `claude attach` tab (this change does not touch that key path). René's report did not reproduce on this machine with claude 2.1.310;
  no key mapping was changed. If it recurs, capture the claude version and whether the task box had focus.
- **Mouse in claude**: a real click inside the prompt text moves claude's cursor (`target/m5-9/s3-click-moves-cursor.png`);
  Shift+drag selects locally while claude has mouse mode on (`s4-shift-drag-selection.png`).
- **Tests**: `terminal.rs` `test_claude_mouse_modes_report_sgr_click_drag_release` (claude's modes → `CSI <0;6;3M`,
  `CSI <32;8;3M`, `CSI <0;8;3m`), `keys.rs` `test_enter_variants_ignore_terminal_modes`, `term_element.rs` selection
  span tests; selftest step 6b (drag over a `wheel line` row → selection text → Ctrl+Shift+C → clipboard → Ctrl+V →
  echoed at the pwsh prompt; the user's clipboard is restored). Red: listeners not registered → `FAIL step 6: step 6b:
  … selection is None`.
- **Pattern**: before porting more of Zed's terminal, check `foreman-term` — the model side is usually already there and
  only the element/view wiring is missing.

## 2026-10-26 — m4/4 inbox and journal

- **WorkState rows in the Inbox**: `work_view::WorkInbox` (`crates/foreman-app/src/work_view/inbox.rs`), one per app
  (a gpui global), created in `FleetView::new`, so the `WorkHub` (and its one `/api/stream`) now starts with the app,
  not with the first Work tab; unconfigured → no rows, no error line (the Work tab explains). The refresh loop hands it
  the registered workspace names after each poll (`set_workspaces`: only new names cause reads — resolve the ref,
  `pick_project`, then `GET /api/v1/inbox?project=`); no project → nothing, quietly; a failed read keeps the last rows.
  `render_inbox` paints them under the supervisor's needs: `WorkState · <kind>` (dashed left border), workspace ·
  `qualifiedRef`, `since …`, `what`, and the `call` in small mono (shown, never run: C21). A click is
  `FleetView::open_work_item` → `open_work` (focus or open the tab) + `WorkView::open_from_inbox` (the item page).
  Keys: `inbox-ws-<row key>`, × `inbox-ws-dismiss-<row key>` (its click stops propagation).
- **When it re-reads**: never on a timer. A stream event maps `runId` → project through the overview's runs (read at
  start, on Configured/Resync, and again when an event names a run it does not know — a new run may be this
  project's); a known run re-reads only its project's workspaces, another project's run nothing, no `runId` every
  inbox. Events are debounced (`DEBOUNCE`, 300 ms) into one background read; Resync (Connected after Lost) re-reads
  all; an older inbox `seq` is dropped. `WorkView` now also re-reads its open item on Configured (it could stay
  "Reading the item…" when opened before the hub was ready).
- **Dismissal** (`Dismissed`): `<home>/workstate-dismissed.json` = `{ workspace: { row key: since } }`, written off
  the UI thread (one writer at a time, newest state wins). A row is hidden while key **and** `since` match. Shows
  again when: its kind changes (another key), its `since` changes (the condition started over — also when it happened
  while the app was closed), or a successful read saw it gone (pruned then, so it is new when it comes back). Foreman
  writes nothing to WorkState.
- **Journal** (`work_view/journal.rs`): a `Journal`/`Runs` toggle in the Work tab header (`work-journal=closed|open`)
  replaces the runs with the project's events, newest last, auto-scrolled to the bottom on growth: time, item ref
  (else the run's), session name (else actor), kind, summary; a row with an item opens its page. Opening reads the
  newest page (`scope=<project>&limit=50`, no `since`); a stream event (debounced, one read in flight) or a resync
  reads `since=<cursor>&limit=500` forward, following `hasMore`. **v1 pages only forward**: in the server
  (`ApiV1.Lists.cs`) no `since` = newest page, `since` = the page after it (exclusive), `since=0` = no since. So
  "load older" reads forward from a window `(oldest − 200, oldest)`, widened ×4 until it holds ≥ 50 events or reaches
  `since=1`; more is offered while the window started above 1. Cost: `seq` 1 (the workspace's first event) is never
  shown. A `before=` on `/api/v1/journal` would make this one call — a request for skills-5d via the controller.
- **Selftest step 7** (`selftest_work.rs`, `--selftest`): the fake also answers the inbox (2 rows), `resolve?ref=larchsys`
  (no project), sw2/2's page and the journal pages. At the start of step 7, before any Work tab: both rows painted.
  After the m4/3 checks (`inbox_and_journal`): row click → the Work tab, sw2/2's page; × → row gone, item page
  unchanged, the file holds the key; an inbox with a third row + an event without a run → the new row painted, the
  dismissed one not; the row with a new `since` + an event → painted again; Journal → 150..152 + `journal-older`;
  `seq` 153 pushed → painted ~340 ms later; load older → 20, 40, no more offered, order 20, 40, 150..153; event 40 →
  sw1/1's page; frames ≤ 50 ms over all of it. `FOREMAN_SELFTEST_JOURNAL_HOLD_SECS` holds the journal for a screenshot.
- Tests: `work_view::inbox::tests::*` (5: dismissal holds / starts over / pruned when gone / file round trip / event →
  which inboxes, keys) and `work_view::journal::tests::*` (3: append/prepend, the older window on a `FakeWorkState`,
  forward reads follow the cursor). Reds: the journal's `schedule_journal` in `on_hub` off → `FAIL step 7: inbox and
  journal: the live journal event: not painted within 2000 ms`; `Dismissed::hides` forced false → `FAIL step 7: inbox
  and journal: × left inbox-ws-done_unverified:… painted`.
- **Journal rows are two lines** (time · ref · kind · who, then the summary): one line of fixed columns left the
  summary ~0 px wide beside the item page at 150 % scaling (the first screenshot showed it wrapping per character).
  Screenshot: `target/m4-4/inbox-journal-fake.png` (PrintWindow during `FOREMAN_SELFTEST_JOURNAL_HOLD_SECS`).
- **Frame budgets under a parallel selftest**: with aw-m5-11's `--selftest-supervise` running at the same time,
  `--selftest` failed its 50 ms frame checks in any of the three measured phases (m4/2 board load, m4/3 item page,
  m4/4 inbox/journal: p50 21–48 ms, max 58–268 ms). On a quiet machine every phase measured p50 ~16.6 ms, max
  20–39 ms. Run the gate's selftests when no other worker's selftest is open.
- Gate (CARGO_TARGET_DIR=C:\fmbuild\m4-4\target, `plugins` junction at C:\fmbuild\m4-4\plugins, CLI 2.1.310):
  `cargo build --release --workspace` ok, no warnings; `cargo test --workspace` 395 passed, 0 failed (387 + 8);
  `--selftest` PASS 4 in a row on the final build (runs 2–5 of 6 there; 1 and 6 failed frame budgets with m5-11's
  supervise running), and 3 in a row (7–9) on the build before the row layout change; `--selftest-supervise` PASS on
  the build before the row layout change (that change is the Work tab journal's render only, which supervise does
  not open).

## 2026-10-26 — m5/11 deterministic supervise selftest

- **Step c's flake was the kill, not the supervisor.** Whether the workspace session's turn ends `done`, `working` or
  `blocked` is the model's (the context decides, see the skills entry above); the daemon respawns a session killed
  while `working`/`blocked` by itself (m2/4), and
  Foreman's R13 deliberately leaves it to the daemon for 30 s — so "came back, but not by the supervisor" was Foreman
  behaving as designed. Measured on two probe sessions mid-turn (`sleep 400`, `state=working status=busy`):
  `taskkill /PID` → new pid 15 s later (`working/busy`, then `blocked/idle`), no resume issued; `claude stop` → row
  `stopped`, no pid, for 100 s+. → Step c now ends the process with `claude stop <id>` (the daemon never respawns a
  stopped session) and asserts the supervisor's resume (R3) as before: last attempt after the stop, same id and
  sessionId, one row. On an idle-`done` row the stop leaves `state=done` (not `stopped`), no pid, in 1.4–2.2 s;
  resumed by the supervisor after 31–37 s (the 30 s respawn grace + a tick). Daemon-respawns-a-crash stays R13's unit
  test; a selftest cannot make the model's turn end deterministically, so it no longer depends on it.
- **Step e's flake was the time the prompt stayed open.** In all ~30 park transcripts under `~/.claude/projects` the
  park called `Bash: echo parked > parked.txt` and parked (no result) — default mode always asks for it. In the two
  failing ones the call got a result 50 s and 2 min after it was made: someone approved the prompt (another Foreman
  window shows every `fm-` session's Blocked row; m5/10). Step e looked only ~10 min after Start. → Step e runs right
  after b and asserts Foreman's own observations: the Notification hook logged `permission_prompt` for the park's
  session (10–18 s after launch) **and** the Inbox shows it Blocked with Attach to its row. A park whose command already
  has a result fails at once with "answered before this step saw it parked" instead of timing out.
- **Step w's flake was the reply text.** The repo session's `fm roster` call had the right result; the step read the
  last assistant text on the first `Stop` event, and that text was the model's own ("Direct request: run the command
  and answer."). → `tool_result(transcript, matches)` finds the first matching tool call's result (string or text
  parts) and the step asserts a `fm-…-repo\trepo\t` line in it; the reply is not read. Test
  `selftest_supervise::tests::finds_the_result_of_the_matching_tool_call`.
- Red checks (each a temporary patch, built, run, reverted): no `NeedsYou::Blocked` in reconcile → FAIL step e "no
  permission_prompt + Blocked Inbox row … (hook events: …, Notification(permission_prompt))"; no Resume for a pid-less
  wanted row → FAIL step c "not resumed within 120 s of `claude stop`" (nothing else brought it back either); `fm
  roster` without `-repo` rows → FAIL step w "its `fm roster` result does not list it as repo".
- Gate (CARGO_TARGET_DIR=C:\fmbuild\m5-11 + `plugins` junction, CLI 2.1.310): `cargo build --release --workspace` ok,
  no warnings; `cargo test --workspace` 388 passed (387 + 1), 0 failed; `--selftest` PASS (p95 18.71 ms, max
  21.13 ms, idle 131.9 MiB); `--selftest-supervise` PASS 5 of 5 consecutive (plus 1 PASS before the reds).

## 2026-10-26 — m5/7 Inbox row for a token idle past its window

- **Signal**: no WorkState polling. Each minted session's `lastActive` (epoch ms, in `<w>.sessions.json`, no token)
  is set at every mint and slid by a turn the tick sees on its single live row (`mid_turn`: `status=busy`), at most
  once a minute — the cadence the server renews `expiresAt` at. A turn after the window has passed does **not** slide
  it: use does not renew a lapsed token. A file from before m5/7 has no `lastActive`; the first tick sets it to now.
  `expiresAt` is not used: it is right at the mint only, and the 72 h is `Policy::token_window`.
- **Row**: `NeedsKind::TokenExpired` ("token expired", error colour) for a wanted, minted session of a started
  workspace whose single live row had no turn for longer than the window; `since` = last activity + window. Remedy
  `Remedy::Remint { id }`, button **Re-mint**, which goes through the existing Retry queue (`Supervisor::retry`): no
  new wiring in `supervise.rs`.
- **Re-mint = stop, then the normal resume**: a `--bg --resume` of a live row starts a copy (m2/4), so the next tick
  `claude stop`s the live row (no Inbox row that tick), R13 waits the 30 s respawn grace, R3 resumes, and the resume
  path (`remint`) mints with `replace` into the same config file, which the resumed process reloads. A Retry on any
  other row never stops a session.
- **Assumption**: a turn is taken as WorkState use. A turn that makes no WorkState call does not renew the token, so
  the row can come late (never early) for a session that works without WorkState for 72 h.
- `view.rs` got the two arms its exhaustive matches need (label, colour, key, button); nothing else in the app.
- Tests (`cargo test -p foreman-supervisor mint`, 16): fake clock in
  `mint_a_live_session_idle_past_its_token_window_asks_for_a_remint` (window slides on a turn, row at +window +1 s,
  a late turn keeps it, Re-mint stops, resume after the grace re-mints, fresh window) and
  `mint_no_token_row_for_a_stopped_workspace_or_an_unminted_session`. Reds: renewing a lapsed token on a turn → 1
  failed; not adding the rows → 1 failed.

## 2026-10-26 — m5/8 the workspace project binds its repos

- **Mint**: a repo session's `POST /api/agent-sessions` body carries `repo: {root, remote?}` (`MintRequest::repo`,
  `MintRepo`), so its bare `work_brief()` opens the project bound to that repo. `root` is the parent of
  `git -C <cwd> rev-parse --path-format=absolute --git-common-dir` (a worktree gives its main checkout), `remote`
  is `git remote get-url origin`, left out without one; both exactly as git prints them, since WorkState normalises
  (Skills `docs/workstate/02-data-model.md`, Identity; `RepoKeys` in `Repos.cs`). `mint::git_repo` runs git at each
  mint of a session with a `repo` (launch and every resume); the workspace session and the relay have none and send
  no `repo` (the workspace root is not a repo). A cwd git cannot read sends no `repo` rather than failing the launch.
  Seam: `Supervisor::with_repo_of`.
- **Skill**: `workspace-controller` §3 **Bind the repos** — on a fresh start with an existing project, right after
  creating it (§4.2), and when `fm workspace show` lists a repo the project lacks: `work_get(refs:["<w>"])` lists
  the bound repos, then per missing enabled repo the two git commands and `work_update(ref:"<w>", repo:{root,
  remote?})`. Refused because another project holds the key → `fm tell` the human, never work around it.
  `roles/workspace.md`'s fresh start names it. Runs stay unbound: repo-controllers keep `work_brief(scope: "<run
  ref>")` (pinned).
- **Not checked live** (the controller's, C21): that a bare `work_brief()` from a minted repo session lands in the
  workspace project, and that the server accepts git's `P:/…` form of the root for a key the skill bound.
- Tests: `cargo test -p foreman-supervisor mint` 18 — `mint_http_client_sends_the_repo_of_a_repo_session` (fake
  server: body `repo` with and without `remote`; the existing body test proves no `repo` for the workspace
  session), `mint_git_repo_of_a_worktree_is_its_main_checkout` (real git in a temp dir: main, worktree, origin, not
  a repo), `mint_each_session_gets_its_own_token_with_its_identity_in_the_mint` (repo session has it, workspace
  session none). `cargo test -p fm skills`: `skills_bind_every_repo_to_the_workspace_project`. Red: `repo: None`
  in the mint → 1 failed.
- Gate for m5/7 + m5/8 (CARGO_TARGET_DIR=C:\fmbuild\m5-78 + `plugins` junction, CLI 2.1.310): `cargo build --release
  --workspace` ok, no warnings; `cargo test --workspace` 405 passed (400 + 5), 0 failed; `--selftest` PASS (p95
  20.74 ms, max 23.22 ms, 0 > 50 ms; idle 132.2 MiB); `--selftest-supervise` PASS on the first run.

## 2026-10-26 — m5/12 workspace UI

- **Own context menu, not Zed's `ui::ContextMenu`.** `ui` is not a dependency and pulls in `settings`/`workspace`
  machinery; the menu we need is a list of labels. `view/menu.rs`: `deferred(anchored().position(click)
  .snap_to_window_with_margin(8))` over everything, `.occlude()`, closed by `on_mouse_down_out` (capture phase, so a
  right-click on another row closes the old menu first and the row's bubble-phase handler opens the new one), by Esc in
  `FleetView::key_down`, and by choosing an item. Items are data (`MenuAction`), built by pure functions
  (`workspace_items`/`repo_items`/`session_items`, unit-tested) and run by `FleetView::run_action`, so every item calls
  the same method its button does (c3).
- **Discovery is shared, not copied.** `foreman_core::discover(dir, name, depth)` is what `fm workspace add` and the
  app's Add both run; `Registry::check_new` refuses a taken name **or an already-registered root** (new
  `CoreError::RootTaken`; paths compared ignoring case, separators and a trailing `\`). `fm workspace add` now refuses
  an already-registered folder too unless `--force`. `rescan` keeps each repo's `enabled`; `Registry::update` is
  load-edit-save.
- **Disabled repos are now listed** (dimmed, "disabled", last, no sessions, no `+`): without that, Disable from the
  menu would hide the only place to Enable it again. Add's review registers unticked repos as `enabled = false` rather
  than leaving them out, for the same reason.
- **Remove waits for Stop to land.** Deleting the registry entry of a started workspace makes the supervisor drop it
  without stopping anything (`Supervision::tick` retains only registered workspaces). So Remove → `stop_workspace`
  (started = false) → the entry and `<home>/supervisor/<w>.toml` go only once a poll taken after the request shows none
  of its wanted sessions alive (`finish_removals`, after each `apply`). Nothing else under the home, and nothing in the
  workspace folder, is touched.
- **Selftest clicks need the frame after a change.** `hits` are the last painted frame's: a press right after an action
  that re-lays out the panel (a notice disappearing) lands on the old position — the first 7b run right-clicked the
  wrong row. Step 7b waits for something only the new frame paints (beta's `+`) before the next press.
- **Screenshots of our own window: `PrintWindow(hwnd, PW_RENDERFULLCONTENT)`, not `CopyFromScreen`.** The screen copy
  caught another Foreman window (and its tray menu) lying over ours. `scripts/shot.ps1 -ProcessId -Out`;
  `FOREMAN_SHOTS=<dir> foreman --selftest` writes add-review, menu-session, menu-workspace, menu-repo, confirm-remove.
- "Show in Work tab" is offered only when an open Work tab's loaded boards show the session holding an unexpired claim
  (no extra WorkState reads from the left panel). Rename is not offered: the name is in the desired-state file, every
  `fm-<w>-…` session name and the WorkState project binding — not cheap.

## 2026-10-26 — m5/1 tray

- **Design (b), tray-resident, not a daemon.** gpui on Windows runs a plain `GetMessageW`/`DispatchMessageW` loop on
  its main thread, so the `tray-icon` crate (0.26, `default-features = false`: no GTK/appindicator on Windows) works
  there unchanged: its hidden `tray_icon_app` window and the `muda` popup menu are pumped by gpui's loop. Its menu and
  click handlers run inside that window procedure, so they only push a `tray::Command` into a `futures` mpsc channel;
  a gpui foreground task applies it with the app in hand (`tray::apply`). No IPC, no state sync.
- **gpui has no "hide window".** `Window::on_window_should_close` returning `false` keeps it; the hide is
  `ShowWindow(hwnd, SW_HIDE)` on the HWND from gpui's `HasWindowHandle` impl. The window, its tabs (attached
  terminals stay attached) and the refresh/supervisor loop keep running: that loop is a foreground-executor task,
  driven by the message loop, not by frames. Show is `ShowWindow(SW_SHOW)` + `SetForegroundWindow` +
  `activate_window`. The caption Close button is `HTCLOSE` → `WM_CLOSE`, the same path Alt+F4 and a posted `WM_CLOSE`
  take, so the selftest's posted `WM_CLOSE` is the button's path. The settings window closes with the hide.
- **Quit** (tray) sets `quit_asked` and runs the old close path (detach Attach tabs, then remove the window; the last
  window closing quits the app). With a workspace started it asks first (`Window::prompt`, at most one: gpui panics on
  a re-entrant prompt). The tray icon is dropped after the event loop: `std::process::exit` skips drops and would
  leave a ghost icon until the next hover.
- **Single instance per `FOREMAN_HOME`**: named mutex `Local\Foreman-<FNV-1a of the lowercased canonical home>`
  (`\\?\` stripped, so a home hashes the same before and after it exists; `DefaultHasher` is not stable across
  builds). A second launch sets the auto-reset event `Local\Foreman-<key>-open` (after `AllowSetForegroundWindow`)
  and exits 0; the first waits on it on a thread and queues `Command::Open`. Selftests take the lock on their own
  home *before* `prepare` clears it, so a second selftest on the same build dir now fails at setup instead of
  deleting a running one's home; René's Foreman (other home) never collides. Tests `instance::tests::*` (3).
- **A selftest whose window closes under it used to exit 0 with no verdict** (the selftest task dies with the
  window). Found by the red check below; `selftest::finish` now FAILs "the app quit before the selftest gave a
  verdict", and `--selftest-detach-shutdown` FAILs a window that closed before its Quit.
- Screenshot of a tray menu from a script: post `6002` (tray-icon's `WM_USER_TRAYICON`) with `lparam = WM_RBUTTONUP`
  to the process's `tray_icon_app` window; the menu (`#32768`) opens at the cursor. Capture from a DPI-aware process
  (`SetProcessDPIAware`), else `CopyFromScreen` coordinates are scaled and miss it. `target/m5-1/tray-menu.png`.
- Autostart at login: not done (not trivial to test without writing this machine's Run key, a red line).
- Placeholder icon `crates/foreman-app/assets/tray.ico` (16–64 px), decoded with `image` (ico); m5/14 swaps the file.
- Red checks (temporary patch: `should_close` never hides, built, run, reverted): `--selftest-detach-shutdown` →
  FAIL "the window closed 488 ms after WM_CLOSE, before Quit: closing must only hide it"; `--selftest-supervise` →
  FAIL "the app quit before the selftest gave a verdict" (before the verdict guard both exited 0).
- Gate (CARGO_TARGET_DIR=C:\fmbuild\m5-1 + `plugins` junction, CLI 2.1.310): `cargo build --release --workspace`
  ok, no warnings; `cargo test --workspace` 403 passed (400 + 3), 0 failed; `--selftest` PASS 6 of 6 (p95
  19.6–22.6 ms); `--selftest-supervise` PASS 4 of 4 (step h: hidden 37–40 s, 12–14 ticks, the stopped workspace
  session resumed while hidden, Open → shown in ≤ 6 ms, repainted in ~110 ms; step q: Stop all in 3.9–6.0 s);
  `--selftest-detach-shutdown` PASS.

## 2026-10-26 — m5/15 selftest under load

- **Two things are measured per frame now, and only one depends on the machine.** `FrameProbe` keeps the frame
  interval (render → render) and adds the window's own work: root `render` → `FrameProbe::paint_marker`, a zero-size
  canvas painted as the root's last child, so render + layout + prepaint + paint of every view, without the wait for
  vsync. `FrameStats` gains `work_p10/p50/p95`. The interval says how the user saw it; the own work says what Foreman
  cost. A build stretches the first without Foreman doing more.
- **Own work is judged always, on its fastest tenth (`work_p10 ≤ 16 ms`).** A per-frame cost (m4/2's filesystem walk,
  the red check's sleep) lifts every frame; preemption lifts only some. Wall-clock own work was not enough on its own:
  beside `cargo build --release` at 100 % CPU, p10 went 5 → 19 ms with no regression (rustc threads at normal priority
  preempt the UI thread mid-frame; the cores' siblings are busy too). So the gate raises its UI thread to
  `THREAD_PRIORITY_HIGHEST` (`frames::raise_ui_thread`, thread only, the process class stays normal so the sessions and
  shells it starts are not raised). After that: p10 5–7 ms quiet, 8–10 ms at 97 % CPU — and frame intervals held vsync
  at 97 % too (p50 16.3–16.9 ms).
- **Interval checks run only on a quiet machine, decided in the same run.** Before each measured phase (step 7's board
  load, item page, inbox+journal; step 8's 6 s idle window) a 1 s idle baseline is recorded, with the share of all
  cores other processes used (`GetSystemTimes` − this process's `GetProcessTimes`). Busy = baseline p50 > 18.5 ms, a
  baseline frame > 50 ms, or other processes ≥ 75 % CPU (before or during the phase) → `interval checks SKIP (machine
  busy: …)` in the step's line. Quiet → no frame over 50 ms, as before; a failure is confirmed by a second baseline
  after the phase (load that started mid-phase reads as SKIP). Step 8's idle window was only printed before; it is
  judged now.
- **Red check:** `FOREMAN_SELFTEST_FRAME_SLEEP_MS=15` (every root render sleeps 15 ms) → `FAIL step 7: step 7 board
  load: own work per frame p10 20.2–23.8 ms, over the 16 ms budget`, quiet (4/4) and beside the build (4/4).
- **Step 6b and a second selftest's clipboard.** The copied line carries the run's tag (`wheel line N fm-<hex>`, step 6
  prints it), the clipboard is re-read until it holds this run's value (1 s), and Ctrl+Shift+C / Ctrl+V are pressed
  again (5 tries) when a foreign value replaced ours. Seen in a loaded run: the first Ctrl+V was not echoed (gpui read
  clipboard format 0 while another process held it), the second was.
- **Step 7b under load:** `press` once found its row missing from `hits` for a frame (`nothing painted under
  repo-row-…/beta`) at 100 % CPU; `scrolled_into_view` now waits up to 2 s for the key to be painted again and lists what
  it saw if it never is.
- Seen once, while P: was full (0 bytes free): `FAIL step 7b: add-save is not fully inside its visible area after
  scrolling: 808..827 in 0..820` (unchanged on every scroll), and the next run failed in setup with os error 112.
  Probably the full disk, not load. Not chased further.
- Gate (final binary, `CARGO_TARGET_DIR=C:\fmbuild\m5-15\target`): `cargo test --workspace` 416 passed (414 + 2 in
  `selftest::frames`). `--selftest` beside a looping `cargo build --release --workspace` into a second target dir: 3/3
  PASS (other processes 97 % CPU; own work p10 8.5–8.7 ms; interval p50 16.4–16.7 ms, 0 frames > 50 ms). Quiet (other
  processes 48–61 %, which is what the parallel units leave): 3/3 PASS, interval checks applied (`quiet`), own work
  p10 5.2–6.2 ms. Red check 2/2 FAIL loaded, 2/2 FAIL quiet. `--selftest-supervise` PASS (supervise step d not
  touched).

## 2026-10-26 — m5/13 the left panel's icon action bar

- **The status line is gone; an icon bar replaces it** (`view/left.rs` `render_action_bar`): Add workspace, Expand all,
  Collapse all, show/hide stopped (eye / eye-off, with the hidden count), then on the right a CLI warning when there is
  one, the busy stepper (− `busy ≤ n` +) and Settings. 26×26 px buttons, 16 px glyphs, muted → text colour on hover
  (`group_hover`, because an `svg` with its own colour does not inherit the parent's hover colour), a deeper fill
  pressed and while a toggle is on; every one has a tooltip (`view/icon.rs` `Tooltip`).
- **Icons: Zed's, compiled in.** `crates/foreman-app/assets/icons/*.svg` are copied from `.repos/zed/assets/icons`,
  whose `LICENSES` says they are Lucide's (ISC; Feather portions MIT) — copied alongside. `svg().data(bytes)`
  (`include_bytes!`) tints them with the text colour and needs no `AssetSource`, so `main.rs` (`Application::with_assets`)
  stays untouched.
- **Version and refresh moved to Settings → About** (`settings_window.rs` `Page::About`): Foreman's version, the claude
  CLI's (with the newer-than-tested or failed text in its colour), the refresh interval and the last refresh's time, the
  Foreman home. The settings window observes the fleet entity so About stays live. The newer-than-tested warning still
  shows in the main window: the notice above the tree as before, plus a warning icon in the bar (tooltip = the text,
  click = Settings → About). `FleetView::open_settings_at(Some(page))` turns an open settings window to that page.
- **A stepper's click reads the live value, not the render's.** The first 7c run captured `n` in the − closure; the
  click right after + landed before the next frame and set `n − 1` from the stale `n` (3 → 2 instead of 4 → 3).
- `--selftest` step 7c (`selftest_left.rs`): the bar's icons share one size and one centre line; Collapse all hides
  the gate workspace's 13 repo rows and Expand all brings them back; the stopped toggle and busy ±1 round-trip; a hover
  shows the tooltip (`m5-13-bar-tooltip.png`); Settings opens, `open_settings_at(About)` turns it to About
  (`m5-13-settings-about.png`, `scripts/shot.ps1 -Title` for a window other than the main one — PowerShell turns
  `$null` into `""` for a `string` P/Invoke parameter: `[NullString]::Value`).
- Screenshots: `target/m5-13/before.png` / `after.png` (scratch home, workspace `ws`).

## 2026-10-26 — m5/10 alerts only for this Foreman's sessions

- **Why it happened**: a unit's `--selftest-supervise` parks `fm-selftest-<hex>-park` with its cwd under the unit's
  worktree, i.e. inside René's registered workspace, so his Foreman listed it under ./AutoWork as **blocked** with the
  warning fill and "waiting for: permission prompt" — and he answered it. (His Inbox could not have had a row for it:
  `NeedsYou` only ever comes from reconciling this home's desired state.)
- **Rule** (`fleet::is_foreign`): an `fm-` session is another Foreman's when this home neither wants it (no desired
  state lists it) nor names it (`fm-<w>-…` for a registered workspace `w`, which keeps this home's retired sessions and
  the first frames before the first tick its own). Such a row is still listed, dimmed, with an "another Foreman" tag
  on its second line (tooltip says why), a muted dot and waiting-for line, no warning fill, ranked as running; Attach stays. Non-`fm`
  sessions and this home's own alert as before. `FleetView::apply` also drops any `NeedsYou` naming a foreign session,
  so the Inbox holds the rule whatever a tick reports.
- A selftest's sessions are told apart by that rule with no change to how they launch: their names are
  `fm-selftest-<hex>…` and only the selftest's own home registers `selftest-<hex>` (its park stays Blocked there, step
  e). A home that registered a workspace literally named `selftest-<hex>…` would claim them — not worth a reserved
  prefix.
- Not changed: the busy budget (`foreman_supervisor::budget::mid_turn`) still counts every mid-turn `fm-` row on the
  machine, another home's included. Arguably right (it is machine load), but a running selftest lowers the user's
  budget.
- Tests: `fleet::tests::another_homes_fm_rows_are_foreign_shown_and_never_ranked_blocked`,
  `reconcile::tests::another_homes_parked_session_raises_nothing` (a guard: reconcile already only looks at wanted
  names). Red: `is_foreign` → `false` fails the fleet test. `--selftest-supervise` step e2 injects a fake
  `fm-elsewhere-<hex>-park` (blocked on a permission prompt, cwd in the testbed's repo) into every snapshot
  (`FleetView::injected`, which the supervisor sees too): after two ticks it is listed as another Foreman's with the
  tag painted, no Inbox row, and the testbed's own park is still Blocked in the Inbox (`m5-10-another-foreman.png`).

## 2026-10-26 — m5/3 first run

- **Three states, one wording** (`workstate_setup::WsStatus`): (a) no `workstate_env`; (b) the `.env` cannot be read,
  or has no token line for workspace sessions (`WORKSTATE_WORKSPACE_TOKEN` else `_CONTROLLER_`) or repo sessions
  (`_REPO_` else `_ORCHESTRATOR_`) — the same fallback `tokens::token_from` launches with; (c) files fine but
  `GET <origin>/health` (WorkState's unauthenticated route, Skills `12-install-harness-identity.md`) is not 2xx, as
  `HTTP 503` / `connection refused` / `timed out`. `problem()` + `fix()` are the notice, the Settings → WorkState
  Status row and the Work tab's message (`load_client`; a read's `Error::Transport` maps to (c)'s words).
- Key presence only: `foreman_core::has_env_key` returns a bool and the value never leaves the parser; no message,
  `Debug` or test output can carry a token (a unit test asserts a fake value is absent from the message).
- Off the UI thread: `FleetView::watch_workstate` runs `detect` on the background executor at start, every 30 s and
  within 250 ms of `recheck_workstate()` (Retry, Check now, any Settings change, the one-click Use). `/health` has
  a 3 s timeout of its own, not the client's 15 s.
- The offer: `~/.workstate/.env` (rlarch 2.3.0 `workstate-setup`'s home install) when `workstate_env` is unset and
  the file exists → "Use ~/.workstate/.env" writes it into settings.toml and re-configures an unconfigured `WorkHub`
  (the Inbox picks it up without a Work tab). A clone install (`<clone>/mcp/.env`) is not searched for: there is no
  cheap way to know where a clone is; the fix text names it for Choose…. Test hook: `FOREMAN_WORKSTATE_INSTALL`
  replaces `~/.workstate`; both selftests set it to a folder under the scratch home before anything runs.
- Never blocks: Start goes ahead; `start_warning` says the sessions run without WorkState (a/b) or that the server is
  unreachable (c); cleared on Stop or when the status turns Ready. The notice's × hides that problem for the session;
  a different problem shows again.
- The gate's fake `.env` now carries fake controller/orchestrator keys and the fake answers `/health`, so the gate runs
  with no WorkState notice; `--selftest-supervise` runs with none configured, so it shows (a) and its Start warns.
- Tests: `workstate_setup::tests` (no env ± installed file, missing file, keyless/half env, fake server Ready, closed
  port), `health::tests` (2xx, 503, closed port, no Authorization sent), `dotenv` `has_env_key`. `--selftest` step 7d:
  no env → notice naming `rlarch:workstate-setup`, no offer; fake install `.env` → offered, one click writes
  settings.toml, fake answers, notice gone; closed port → URL + `connection refused` + Retry, × hides it; settings put back.
- **Notices could squeeze the tree to 0 px**: the left panel is header, Inbox (≤25 %), Messages (≤30 %), then the
  notices unbounded, then the tree (`min_h_0`). With the WorkState notice and a Start warning on top of the
  supervise testbed's notices, step s found `ws-row-…-u` in a 0 px tall visible area. The notices now sit in their
  own scroll container (`#notices`, shrinkable) and the tree keeps `TREE_MIN_SHARE` = 30 % of the panel; the Start
  warning is one short line.

## 2026-10-26 — m5/14 UI detailing

- **Text glyphs do not centre; icons in fixed boxes do.** `▸`/`▾`, `+` and `×` sat on the font's baseline, a few px
  off their hit area's centre, and an empty repo's `" "` placeholder was narrower than `▾`, so empty and foldable repo
  names did not line up. Now: Zed's `chevron_right`/`chevron_down`/`close`/`plus` SVGs (Lucide, ISC, beside m5/13's)
  in fixed boxes — disclosure 16 px (empty when the row does not fold), `glyph_button` 20 px ([+], Inbox ×), tab × 18 px,
  new-shell + 36 px wide. Hit keys unchanged (`new-claude-…`, `inbox-ws-dismiss-…`, `close-tab-…`, `new-shell`).
- **One height per row kind**: workspace 30 px, repo 26 px, a session's first line 24 px (`WORKSPACE_ROW`, `REPO_ROW`,
  `SESSION_LINE` in `view/left.rs`); the shared `button()` is 20 px with its label centred. Flex children that may
  overflow (path, branch, relay label, session name, waiting-for/hook lines, Work titles, tab titles) are
  `min_w_0().truncate()` — gpui's `truncate()` is overflow hidden + nowrap + ellipsis, but a flex child without
  `min_w_0` never shrinks below its text, so it pushes its siblings out instead of ellipsing.
- **States**: every clickable row and button has a hover fill and a pressed fill (`.active`, ghost/element `_active`);
  `button()` gains `border_selected` on hover; attach/resume are muted outlined pills whose border, fill and text come
  up on hover; the session row whose attach tab is active is selected (`ghost_element_selected` + a 2 px
  `border_focused` left edge); disabled repos keep 0.6 opacity and get a bordered "disabled" tag. Active tab: 2 px
  `border_focused` line on top; inactive tabs hover. Only theme colours (`cx.theme()`), no new palette.
- **Focus ring: not done.** The window has one focus handle and `FleetView::key_down` sends every key not taken by
  the task box / Add name to the terminal, Tab included; a ring needs per-element focus handles and a Tab traversal
  that would take Tab from the terminal. The two text fields (task box, Add name) already show `border_focused`.
- **App icon**: `scripts/icon-gen` (own `[workspace]`, not a member; resvg 0.46 + image 0.25 from the lock) renders an
  SVG to PNG previews, a preview sheet on dark/light taskbar colours, `<name>.ico` (16/24/32/48/256, PNG entries) and
  `<name>-tray.ico` (16–48: `image` decodes an .ico's largest entry, and 48 px scales down to the tray better).
  Proposals in `docs/icon-proposals/`. **gpui's Windows platform loads icon resource 1 of the exe as its window
  class icon** (`gpui_windows` `load_icon`: `LoadImageW(module, 1, IMAGE_ICON)`), so one resource gives both the window
  and the Explorer icon: `crates/foreman-app/build.rs` + `assets/foreman.rc` (`1 ICON "foreman.ico"`) with
  `embed-resource` 3.0 (already in the lock via gpui's `windows-manifest`; uses the SDK's rc.exe). Checked with
  `[System.Drawing.Icon]::ExtractAssociatedIcon(foreman.exe)`.
- **René picked B, the hard hat** (`docs/icon-proposals/b-hardhat.svg`): `icon-gen` → `assets/foreman.ico` (16–256,
  exe + window) and `assets/tray.ico` (`b-hardhat-tray.ico`, 16–48). To change it: edit the SVG, rerun
  `cargo run --release --manifest-path scripts/icon-gen/Cargo.toml -- <svg> <out>`, copy both, rebuild.
- `FOREMAN_SHOTS` now also writes `m5-14-terminal-tab`, `m5-14-inbox`, `m5-14-work-board`, `m5-14-work-item`,
  `m5-14-work-journal` (`--selftest`) and `m5-14-messages` (`--selftest-supervise`). Before/after: `target/m5-14/before/`,
  `target/m5-14/after/`.
- Seen, not fixed (also in the before shots): with the tree scrolled, its top row paints over the last ~15 px of a
  full Inbox (both half-lines visible where they meet). Not chased.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-14\target` + `plugins` junction): `cargo build --release --workspace` ok, no
  warnings; `cargo test --release --workspace` 422 passed, 0 failed; `--selftest` on the polish build 4 PASS of 5 —
  the fifth FAILed step 6b ("Ctrl+Shift+C … left the clipboard at Some(\"not copied\") (5 tries)", parallel units on
  the machine; m5/15's clipboard race, the terminal is untouched here) — and PASS again on the icon build;
  `--selftest-supervise` PASS. Step 8's interval checks SKIPped as "machine busy" (baseline p50 ~45 ms) in every run,
  before the change too; own work p10 5–7 ms.

## 2026-10-26 — m5/19 nulls from the real server
- The fake's fixtures were written from doc 10's prose and simply left out the fields they had no value for, and a
  missing field defaults under `#[serde(default)]` — but the real server *sends* `"intent": null`, and null into a
  `String` is an error. Absent ≠ null: the model's types come from `WorkState.Api/App/V1/V1Contract.cs` (every C# `?`
  is an `Option`), and step 7's sw1/1 page now sends every nullable as an explicit null.

## 2026-10-26 — m4/6 E2E: task box → Approve → Work tab → item page, all in the app
Test Foreman on `target/e2e4-home` (workspace e2e4: `textkit`, `textcli`; max_working 3; real WorkState; published
rlarch, no extra_plugin_dirs). Task: `--shout` for `textcli greet` on a new `textkit.shout`. Screenshots in
`target/e2e4-shots/` (01–25, live-*). No `fm` command was typed.
- **Timings.** Start 19:37:27 → workspace + relay up < 15 s. Task box Send 19:38:40 (`sent direct [m-a133a663]`,
  prompt submitted at once) → APPROVE? in Messages ~19:40:30 (≈ 2 min, plan of 3 runs incl. an integration verify
  blocked by both). Approve 19:44:01 (`sent direct [m-b7fc0378]`) → fm-e2e4-textkit launched < 15 s → textkit done
  19:47 (e14d7a4) → textcli launched 19:48, done 19:51 (65cf5d8) → integration verify done 19:52. Approve → all
  done ≈ 8 min. Workspace Stop 20:19: every e2e4 session stopped, none left in `claude agents`.
- **Verified by me** in the repos: textkit and textcli tests OK, `greet --shout Ada` → `HELLO, ADA!`, plain greet
  unchanged. The item page shows the agent's report ("claimed") beside the orchestrator's verification with its own
  red check and the commit (22-item-verification.png).
- **Found and fixed on the way:** the item page could not open on the real server (`"intent": null`) → m5/19.
  Quitting the app and starting the new build on the same home reattached to the running workspace.
- **Found, queued:** a Messages row arriving under the pointer moved the Task… button (missed click) and work_tree
  tables wrap unreadably in Messages; Work tab times are UTC without a zone → m5/18. A `+` hand session in a repo is
  not minted: its bare `work_brief` said "repo not given" and opened another project → m5/20 (m5/8 c4 stays open).
- **Driving the app from outside.** PrintWindow shoots a window even when another covers it, so a screenshot says
  nothing about what a click will hit: a selftest Foreman opened at the same position took the foreground and three
  clicks landed in it. Clicks/wheel are now PostMessage'd to the target HWND (no foreground needed); typing needs
  real keystrokes (posted WM_CHAR is ignored), so it is guarded: type only while GetForegroundWindow's pid is the
  target, re-checked per chunk.

## 2026-10-26 — m5/17 tab hotkeys

- gpui matches key bindings *before* any `on_key_down` listener and a handled action stops the event, so actions
  `NextTab`/`PreviousTab` bound in the `FleetView` key context beat `FleetView::key_down`, the only path from a key to
  a PTY; no per-chord filtering in the terminal code. Plain Tab / Shift+Tab are unbound and still go to the PTY.
- Unbound, Ctrl+Tab and Ctrl+Shift+Tab already sent no bytes (`to_esc_str` has no mapping); Ctrl+PgUp/PgDn did
  (`\x1b[5;5~`), and that is what selftest 7e's red check caught (handlers made to `cx.propagate()`: the pwsh
  `ReadKey` readers printed `PageUp 4`). `[Console]::ReadKey` modifiers: Alt = 1, Shift = 2, Control = 4.

## 2026-10-26 — m5/20 hand sessions minted

- A `+` session (`crates/foreman-app/src/hand.rs`) is minted `role: worker` (orchestrator since m5/35), `externalRole: "hand"`, `repo` =
  `git_repo(cwd)`, name `hand-<folder>-<n>`, `launchedBy: foreman/hand`. A worker token cannot mint (doc 14 §2), so the
  bearer is the repo role's static token, else the workspace's; a `.env` with only a worker line opens it plain.
- The tab types `try { claude --mcp-config '<file>' } finally { Remove-Item -LiteralPath '<file>' }`: the shell
  deleting the file is how the app sees claude exit (polled every 500 ms, then DELETE). Tab close, quit
  (`on_app_quit`, synchronous: gpui polls quit futures for only 200 ms) and the next start (`supervisor/hands.json`)
  revoke the rest. Not live-checked here (C21 is the controller's): interactive `claude` honouring a non-strict
  `--mcp-config` over the user-scope `workstate` is (b) above, measured on `--bg` sessions.

## 2026-10-26 — m5/16 the tree "painting over" a full Inbox

- **It did not.** The tree's `overflow_y_scroll` already clips (gpui's `overflow_mask` intersects with every
  ancestor's mask, and the Windows shaders clip quads and glyph sprites to it). In the m5-14 `m5-14-work-board`
  shot the Inbox's content stops at y 392 (its own clip, inside its 1 px bottom border at 393, colour 27,29,35 on a
  34,37,43 panel) and the tree's starts at 394. Both sections cut a row mid-text at the same, near-invisible line,
  so the two half-rows read as one row painted over another.
- Fix: below each capped, scrolling section (Inbox, Messages, notices) a border in `text_muted` at 35 % and 6 px of
  panel background before the next (`SEAM_GAP`, `left.rs`). Hit targets `inbox-area`, `messages-area`, `tree-area`
  record each section's clip.
- `--selftest` step 7f: clears the 7b info line so the tree sits right under the Inbox, wheels the tree through six
  offsets and, at each, checks every tree row's painted part (bounds ∩ clip) against the Inbox's area and its last
  row: no overlap, and at least 4 px apart. The overlap half passes on the old layout too (nothing overlapped); the
  gap half is what fails there.
- Not this: 7b's one-off "nothing painted under repo-row-selftest-added/beta". A `hit_target` canvas records its
  bounds at prepaint whatever the clip, so a key missing for 2 s means the row was not in the rendered tree.

## 2026-10-26 — m5/18 rows that hold still, tables in Messages, local times, a recalled hand line

- **Hold** (`view/left.rs` `Hold`): the first mouse move over the left panel records the Inbox (needs + WorkState
  rows) and Messages keys there are; while the pointer keeps moving there only those rows render, so an arrival
  above it moves nothing. A row that goes away still goes at once. The rest appear when the pointer leaves
  (`on_hover(false)`) or has rested `HOLD_STILL` (2 s, a 200 ms timer task); the section header says
  "· n new, shown when the pointer leaves" on the line it already had. Only the Inbox and Messages are held, the
  tree's own rows are not.
- **Tables** (`segments`): two or more consecutive lines that each hold a tab, a gap of two spaces after their first
  word, or a box-drawing rule become one block: Cascadia Mono, one `whitespace_nowrap` line per row, the block
  `overflow_hidden` (clipped at the panel's edge, not wrapped). Tabs expand to 8-column stops. A single gappy line or
  an indent-only line stays prose.
- **Times**: WorkState sends RFC 3339 in UTC; `work_view::short_time` now parses it and shows `chrono::Local`
  (`clock` feature; `iana-time-zone` was already in the lock via zed). Unparseable text is shown cut to the
  minute, as before. The Inbox's "since" uses it too.
- **Hand line** (m5/20 follow-up): the typed line stays in PSReadLine history, so it now starts with
  `if (Test-Path -LiteralPath '<config>')` and otherwise prints why and runs plain `claude`. Keeping it out of
  history was the other option, but whether a line is kept depends on the user's PSReadLine settings and
  `AddToHistoryHandler`; a line that is safe to rerun works on any profile.
- `--selftest-supervise` step t0, at the start of step r (Task… just shown, Messages still short — a capped
  section never grows, so later in the run the test would prove nothing): the pointer moves onto `task-<w>`, a five-row `work_tree` message is written to
  the outbox while it keeps nudging 1 px (a hand still aiming), and once the app has read the file the button's
  painted bounds must equal the aimed ones; the click is sent at the aimed spot. Then the pointer leaves, the row
  appears and must now have moved Task… (else the step fails as void), is expanded, and the table must paint 5
  `msg-table-…` lines of one height (< 24 px), back to back. Red checks: hold off → "moved task-… from y 516.7
  to y 598.0 under the pointer"; segments off → "the table's lines painted: not within 5 s"; `SEAM_GAP` 0 →
  7f "ws-row-larchsys paints from 264.0, 1.3 px under the last Inbox row".

## 2026-10-26 — m5/21 step 7b after the seam/hold

- **Not the seam or the hold:** `Poller::load` read the registry, then ran `claude agents` (~0.4 s); a Save inside that window was shown by `reload_workspaces`, then the poll's `apply` put the pre-Save list back, and the row was gone until the next poll (~2.4 s, over 7b's 2 s). F-01's `repo-row-…/beta` was the same. Fix: `registry_writes` (`view.rs`), bumped by `reload_workspaces`; a poll that read before a write keeps the shown list. 7b now holds one poll between its read and its apply across Save (`PollHold`); guard off → "a poll that read the registry before Save took selftest-added out of the tree".
- Step 7's "the reloaded inbox: not painted within 2000 ms" was the m5/18 hold (pointer resting on the panel after the ×, `HOLD_STILL` 2 s vs `LIVE_WITHIN` 2 s): the step now moves the pointer off the panel first.

## 2026-10-27 — m5/23 terminal like Windows Terminal

- Zed's vendored terminal already had copy, paste, word/line/Shift-click selection and search; what was missing was
  the wiring: Ctrl+C fell through to `try_keystroke` as ^C. The chord table is `view/term.rs::chord` (unit-tested);
  the full key and mouse list is `docs/KEYS.md`. Alt+drag is alacritty's `SelectionType::Block`.
- Blink: `Terminal::cursor_blink` (lit for 530 ms after input or a cursor move, then alternating); `FleetView`
  repaints at each edge only while a blinking terminal is shown, and `key_down` notifies so a key lights it at once.
  The default style is a blinking Bar; DECSCUSR from the program still wins (`Term::cursor_style`).
- PSReadLine's parameter colour is `\e[90m` (index 8); the theme's bright black was near the background. ANSI 0–15
  are now Campbell (`terminal_settings::CAMPBELL`). Selftest step 6c (`selftest_term.rs`) covers copy/^C/right-click/
  cursor/find/zoom; shot `m5-23-psreadline-parameter`.
- m5/24, 6c flaky (2 of 5): "cursor is Hidden" was a frame painted while PSReadLine had DECTCEM off mid-redraw; the
  next frame, 30–45 ms later, was a blinking Bar, so 6c waits up to 1 s for a shown cursor and still fails on any
  other shape. "selection one line off" was the drag reading row 0 from a frame one line behind the grid (frame
  `total_lines` 2402, live 2403: PSReadLine's echo wrapped and scrolled); `select_wheel_line` now waits until
  `last_content` matches `Terminal::grid_extent()` and drags again only if the grid moved during the drag.
- Product bug found on the way: `mouse_down` resolved its anchor with the last frame's `display_offset`, drags with the
  live one, so scrolled back with output arriving the anchor landed a line below (selection `""`). It now queues
  `BeginSelection(type, pixels)`, resolved on the live grid (`test_drag_anchors_on_the_shown_row_…`). A loop of 6b→6c
  with fresh output each pass: 5 of 30 FAIL before, 0 of 40 after.

## 2026-10-27 — m5/22 GUI subsystem, logs, version triangle, hand launch, shell cwd

- **No console, still a console app.** `windows_subsystem = "windows"` would cost `--selftest` in pwsh: pwsh neither waits for a GUI exe nor sets `$LASTEXITCODE` (measured: returns in 0.05 s, no output). foreman.exe stays console-subsystem with `consoleAllocationPolicy detached` in its manifest (Windows 11 24H2+): from Explorer/ShellExecute no console is made (measured: `GetConsoleWindow` 0, no conhost child), from a shell it shares the shell's. gpui embeds its own manifest (forced by `gpui_platform`, 0x0409); a second RT_MANIFEST #1 in another language links, and the LANG_NEUTRAL one wins at process start (checked both ways round), so `assets/foreman.manifest` repeats gpui's settings. Older Windows: `release_own_console` frees a console made for us (a flash). Console children (`claude`, `git`, `icacls`) get `CREATE_NO_WINDOW` (`foreman_claude::no_window`), else each would flash one.
- **"window not found" / "Invalid window handle" were not harmless noise of closed windows; they were our close paths.** gpui takes a window out of the app while it updates it, and `ShowWindow` delivers WM_ACTIVATE/focus/position messages synchronously, so hiding or showing from inside that window's update (tray hide on ✕, `remove_window` then gpui's own `ShowWindowAsync(SW_HIDE)` in Drop) made each callback miss it. A caption ✕ on Settings let `DefWindowProc` destroy the HWND before gpui's Drop touched it (0x80070578, 0x80040102 = DRAGDROP_E_INVALIDHWND). Fix: `tray::{hide, show, close}` run their Win32 calls in a task of their own (as gpui's `activate` does), `close` hides, lets posted messages drain (100 ms; without it one WM_MOUSELEAVE/paint still logged), then removes; every `on_window_should_close` returns false. Step 7c fails on any such line in the selftest's `logs/foreman.log`; red: plain `remove_window` → 8 lines, hide inside the update → 4, no drain → 1. One line stays out of reach: on WM_DESTROY gpui_windows only *posts* the message that takes the HWND off the list its VSync thread `RedrawWindow`s, so about 1 close in 10 logs `[ERROR gpui_windows::platform] Invalid window handle (0x80070578)` for a redraw of the dead window; harmless, so `foreman_ui::is_vsync_race` drops exactly that line.
- Logs: `<home>\logs\foreman.log`, rotated to `.1` at 1 MiB, opened per line so a selftest home can still be removed. Version: only a newer major.minor raises the ⚠ (the Inbox notice is gone; it was also what pushed the tree down in 7f on claude 2.1.311). Hand `+`: pwsh `-NoExit -EncodedCommand` (base64 UTF-16LE) runs the launch unseen; nothing is typed, so nothing to recall either. New shell cwd: `fleet::shell_cwd` (active tab's workspace, innermost root, case- and `/`-insensitive), step 7g reads the pwsh's own `Get-Location`.

## 2026-10-27 — m5/25 claude children and the daemon pipe

- **Integrity level, not the console.** claude's background service listens on `\\.\pipe\cc-daemon-<pipe.key>-*`; started by an elevated `claude`, it refuses every unelevated client (`ECONNREFUSED`), which then starts its own service, loses the lock race (`another daemon won the lock race`) and fails after 45 s. René's terminals elevate (`profiles.defaults.elevate`), so his service is High; Explorer starts Foreman at Medium, and its claude children *and* its tab pwsh inherit Medium. Measured with `claude daemon status`: from a High shell `reachable` with no flags, `CREATE_NO_WINDOW`, `DETACHED_PROCESS` and `CREATE_NEW_CONSOLE`+`SW_HIDE` alike; under `runas /trustlevel:0x20000` `unreachable (connect ECONNREFUSED …)`. m5/22's flags were innocent; it only made an Explorer start usable. 0c0d85a "worked" because the E2E started it from an elevated shell.
- Foreman cannot fix it from Medium without UAC. `foreman_claude::daemon_mismatch` (unelevated + live service + unreachable control.sock) holds the supervisor (`Supervisor::hold`, one `LaunchFailed` row with the remedy, no 5×45 s), checked every 15 s and on Retry (`supervise::DaemonWatch`); tabs print the line instead of a 45 s failure. Settings → Run Foreman as administrator re-launches elevated (`elevate.rs`, `ShellExecuteW runas`, `--elevated --home` since UAC drops the environment and the copy must never ask again) after taking the instance lock (a launch that only hands off never asks UAC), `--after <pid>` when restarted from the Inbox's Restart as administrator. The lock and its open event need no change: an unelevated second launch opens an elevated instance's (default security) and hands off; `tests/elevation_handoff.rs` checks it at Medium (passed before the change too).
- Test: `daemon::tests::a_medium_il_process_cannot_reach_an_elevated_service` runs the real status at Medium via `runas /trustlevel:0x20000` (skipped unless elevated with an elevated service).

## 2026-10-27 — m5/26 add-workspace input, contrast, selection, tooltips

- **The "hang" was not one.** René had picked a folder that is itself a repo (`P:\Projects\LarchSys\Gaia`). `scan` listed only the repos *below* the root, so the review said "0 of 0 repos" and Save refused, but the error line sat between the list and the buttons and he did not see it. The frozen caret was by design: a static `▏`, and clicks did not place it. The posted-message repro on 2ae06cd could not make keys fail: a fresh start, a pwsh tab, a full tree, a window re-shown from the tray, and a Medium-IL Foreman (`runas /trustlevel:0x20000`) all kept Win32 focus after the picker (`GetGUIThreadInfo`). SendInput does not work as a test tool in these sessions: injected keys and clicks are dropped for every window (a WinForms textbox too). PostMessage reaches gpui's wndproc. A posted WM_MOUSEMOVE does not hover, because the real cursor's position sends WM_MOUSELEAVE at once.
- Now `scan` lists a root with `.git` (a folder, or a linked worktree's file) as a repo: first, named after its folder (`<folder>-root` when a child has that name), path = root, shown as `<name> (root)`. Consumers checked by tests: the registry and `fm want` cwd (`tests/want.rs`), `assign::repo_of` (children keep theirs), the `git_repo` mint key (`mint.rs`), branch via the worktree's gitdir. The relay never uses repo paths. The name is a `LineEdit` (`line_edit.rs`): click/arrows/Home/End/Shift/Ctrl+A, a blinking caret, space→`-`, and a refused character shows a hint. Errors sit tinted next to Save, Save while scanning says so, and the window is re-activated after the picker. Step 7b drives it with posted messages and adds a one-repo workspace (shot `add-review-single-repo`).
- Contrast (WCAG, One Dark panel #22252b, `text` 10.9:1): `text_muted` was 3.2:1, and 1.8:1 in a 0.5-opacity row. The tree now uses `text` blends: quiet 0.78 (7.2:1), secondary 0.62 (5.1:1), dimmed rows 0.8 (5.2 / 3.8). `left.rs::the_tree_text_is_readable_against_the_panel` computes them from the theme (shots `cut-label-tooltip`, `m5-26-selection` show the tree).
- Selection as WT: `SELECTION_BACKGROUND` #FFFFFF, selected glyphs drawn in `terminal_background` (`term_element::Selected`). 6c reads `Terminal::selection_painted`. Cut labels: `clipped.rs` measures each label against its width every frame and attaches the full-text tooltip from the next frame on. 7b hovers the cut root path. At the 280-px minimum panel the row's buttons squeeze that path to 0 px.

## 2026-10-27 — m5/27 closed interactive sessions, stars

- **Why CI-CD-pipeline vanished.** `claude agents --json --all` lists background sessions, stopped ones too, but an interactive session only while it runs. A closed one leaves only `<claude dir>/projects/<slug>/<sessionId>.jsonl`. The slug turns every non-alphanumeric character into `-`, so it only narrows which dirs to scan (prefix of an enabled repo's slug, which includes its `.claude/worktrees` and `.worktrees` siblings). cwd, the custom title, the agent name, the ai-title and the first prompt come from the lines. In claude 2.1.306, meta lines are type-first (`{"type":"custom-title",…}`), they repeat, and the last one wins. A rename writes `custom-title` and `agent-name` together.
- Cost on René's machine: 979 project dirs, 737 of them prefixed `P--Projects-LarchSys-AutoWork`. The first scan took 6 s for AutoWork (1958 transcripts within 14 days, 1808 of them `fm-` selftests) and 1.3 s for Gaia. A rescan takes ~100 ms because `foreman_claude::Transcripts` caches the dir list and each dir's listing by mtime and reads an appended file from its last offset. It runs on its own background loop every 10 s (`watch_transcripts`), never on the poll thread. `fm-` transcripts are dropped unless starred: background transcripts carry the same `entrypoint:"cli"` and `mode:"normal"`, so nothing else tells them apart.
- Resume of a closed or interactive row goes through the `+` launch (`hand_claude`, `Launch::script_with`), with its own token when one can be minted. Stars live in `<FOREMAN_HOME>/starred.json`. The selftest reads transcripts from `FOREMAN_CLAUDE_DIR` = `<scratch home>/fake-claude` (step 7h; a fake `claude.cmd` through `FleetView::claude_command`), never `~/.claude`. Two fixture ids with the same first 8 hex digits collide in the tree's element ids and hit keys.

## 2026-10-27 — m5/29 tab title from the program, hover card

- **What claude sends.** claude 2.1.311 sets `process.title`, which ConPTY turns into OSC 0: `claude` at start, then `✳ <name>` while idle and `◐`/`◑` (alternating every 960 ms) while working, where the name is the session's custom title, then the ai title, then the agent type, then `Claude Code`. On exit it sets `""`. `CLAUDE_CODE_DISABLE_TERMINAL_TITLE` turns it off. The step 2 bg session was seen as `◑ fm-selftest-…`. The glyph stays in the label: `✳` renders as a small green emoji glyph and reads well.
- **ConPTY's own title is no title.** When nothing names the console, ConPTY sends the absolute path of the running executable: `C:\Program Files\WindowsApps\Microsoft.PowerShell_…\pwsh.exe` for a pwsh tab, `…\.local\bin\claude.exe` for an attach tab. Zed's guard compared the title with `shell_program`, but that is `Some("")` here, because `resolve_path` comes back empty for the App Execution Alias. Measured: the guard never fired. `terminal.rs::is_default_title` now also treats any absolute `*.exe` path as "no title". That empties `breadcrumb_text`, so the tab label falls back to its own.
- `Tab::program_title` = `tab_card::sanitize(breadcrumb_text)` on `BreadcrumbsChanged`. Sanitising drops controls (C0/C1), bidi and zero-width characters, folds whitespace runs to one space, and caps the title at 200 chars plus `…`. `Tab::label()` is what the tab shows. The tab's label uses `clipped_with(…, own_tooltip: false)`, because the card on the whole tab carries the full text. The card is gpui's `.tooltip()`, which waits 500 ms (`DEFAULT_TOOLTIP_SHOW_DELAY`). Its branch is read when it opens (`foreman_core::branch_at`, walking up to the nearest `.git`). Step 7i measured the card at about 555 ms. The fields come from the tab: workspace/repo from its cwd (`tab_card::place`), session from the attached row or the interactive resume, kind is hand/bg attach/shell, started is the session's `started_at` or the time the tab opened. A Work tab has no kind.
- **Elevated:** an elevated console prefixes the default title with `Administrator: ` (localised). The controller's elevated gate failed step 7i 5 of 5 until `is_default_title` also matched any `<prefix>: <absolute exe path>` (ecfc937). The agent's gate ran non-elevated, so it never saw this. René runs Foreman elevated.
- Step 7i (`selftest_title.rs`): a pwsh tab prints `ESC]2;✳  name<TAB>BEL`, and the label must read `✳ name`. The step then hovers the tab: the card's first line is the title, it has the workspace, folder, `kind  shell`, a start time, and the branch when the folder is in a repo, and it must not show before 400 ms. Then `ESC]2;BEL`, and the label must be the fallback again. Shot `m5-29-tab-card`.

## 2026-10-27 — m5/32 relay on demand

- **Why**: on René's machine `relay/gaia` and `relay/nomos` had 0 relay sends (`inbox/sent` empty), the outbox
  only `fm tell` files: since m5/4 the always-on relay was an idle session per workspace that never carried a
  message. René chose (2026-10-27) to keep the fallback but run it only when needed; dropping it was not chosen.
- **Shape**: the demand is read from files, not from a flag someone must set: `relay_demand` = a `*.json` drop waits
  in `relay/<w>/inbox`, or `relay/<w>/last-drop` (written by every `foreman_core::send`) is younger than the
  idle timeout. So `fm send` from a shell needs no access to the desired state, and a drop made while Foreman
  was closed starts the relay on the next tick after Start. `supervise::sync_relay` runs in every tick between
  loading the desired states and reconciling: wants the relay (first, `DesiredState::set_relay`) or retires it
  (`unwant` → `retired` → R14 stops it). A relay wanted again comes back by **resume** of its stopped row with the
  re-arm prompt, and a new watcher prints every drop still in the inbox, so nothing queued is lost.
- **Upgrade**: a desired state that still lists the relay (Start wrote it before m5/32) is retired on the first
  tick unless a drop waits; Start no longer touches the relay, except that a wanted one follows the root.
- **The app no longer refuses** a send because the relay is not `relay ready`: a started workspace takes the
  message (direct, or queued for the relay it starts); Task… shows on every started workspace.
- **Timeout**: `settings.toml` `relay_idle_secs` (default 600), counted from the last drop, not from delivery.
  A drop still waiting keeps the relay wanted however old it is. Not covered: a relay that takes longer than the
  timeout to arm and is retired between the watcher printing the drop and its `SendMessage` (seconds, at 10 min).
- **Selftest** (`--selftest-supervise`, `relay_idle_secs = 60`): the testbed's desired state is written with the
  relay listed, as before m5/32. Step b: no relay row, it is retired, row `direct`. Step r: 30 direct sends, still no
  relay row; the forced fallback → `relay starting` after 2.0 s, armed and `relay ready` after 17.8 s, nonce back in
  the outbox after 23.0 s, in Messages with Attach; retired, stopped and `direct` 61.5 s after the drop. Step g counts
  2 supervised rows (workspace, park): the retired relay is no longer the workspace's.
- Tests: `relay::tests::the_relay_is_in_demand_…`, `desired::tests::set_relay_…`,
  `supervise::tests::the_relay_is_wanted_on_a_fallback_and_retired_after_the_idle_timeout` (fallback, waiting drop,
  timeout, upgrade with and without a queued drop), `relay_status_reads_direct_starting_ready_and_expired`,
  settings round trip with `relay_idle_secs`.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-32\target`): `cargo build --release --workspace` ok; `cargo test --release
  --workspace` 491 passed, 0 failed; `--selftest-supervise` PASS (first run, 333 s); `--selftest` 4 of the first 5 (run 4
  FAIL step 7b, `selftest_manage.rs` `edit_name`: the name was read right after posting End, 3 Backspace and `added`
  and showed `selftest-` — m5/26's step, a read-before-dispatch race, not code m5/32 touches), then 5 of 5 PASS;
  `--selftest-detach-shutdown` PASS; no `fm-selftest` rows of this worktree left (4 `fm-selftest-c5886c-*` rows with
  cwd in the main checkout are another run's, not touched).

## 2026-10-27 — m5/28 closed sessions show up at once

- **Where the 30 s went.** `claude agents --json --all` drops a killed interactive session at once (probe: 920 ms
  after the kill, including the ~550 ms the list call itself takes). Foreman then waited three times: the 2 s poll,
  the 10 s transcript scan (the row was gone from the tree until the scan read its transcript), and `RESUME_GRACE_MS`,
  30 s after it was last seen alive. Step 7j measured close → resume **30.8 s** before (out of the snapshot 2.5 s,
  closed row 8.2 s), **2.5 s** with the grace for bg only and a scan nudged when an interactive row leaves the
  snapshot, and **1.2 s** with the pid watch. Why closing Foreman's own tab felt quicker was not measured.
- Rules now: `view::offers_resume` keeps the grace only for `Kind::Background` (the daemon respawns those; an
  interactive one is never respawned). Unit test `only_a_background_session_waits_out_the_grace_before_resume`.
  `apply` nudges the transcript scan when an interactive session leaves the snapshot. Every 250 ms of the poll wait,
  the pids of live interactive sessions are checked (`process_alive`), and one that was alive and has ended triggers
  a poll at once. A pid this app cannot open is never watched, so it costs nothing. Refresh (action bar ↻, and F5
  in `key_down` only while the active tab has no terminal, because programs in a terminal need F5 (mc, debuggers,
  TUIs) and every key goes to the active terminal tab) sets both nudges. Step 7j: the ending session is injected with a windowless `ping` as its pid,
  killed right after a poll, and must offer resume within 2.3 s (0.9–1.7 s measured). A plain poll takes `REFRESH_EVERY` plus
  `claude agents` (≥ 2.5 s), so it cannot pass (red: pid watch off → 2.9 s, FAIL). Refresh and F5 with a Work tab active must poll and scan within 1.5 s of being triggered
  right after both ran. F5 with a shell tab active must reach its pwsh key reader (`F5 0`) and leave
  `refresh_asks` unchanged.
- **Probing real claude.** `Start-Process claude` / `conhost claude` opened a visible console on René's desktop (do not do
  this). The probe also inherited `CLAUDE_CODE_CHILD_SESSION` from this session: claude then says transcript saving
  is off. Those sessions were never listed in `claude agents` (4 tries, 30–60 s each). Whether the marker or the
  console was the cause was not separated. A headless probe works:
  `ProcessStartInfo` with `UseShellExecute=false`, `CreateNoWindow=true`, and `CLAUDE_CODE_CHILD_SESSION` removed
  from its environment, in a trusted cwd (the worktree's untrusted folder prompt also blocks it), named `fm-selftest-…`,
  and killed by its pid.

## 2026-10-27 — m5/30 search box over the tree, one shared text input

- **How a key reaches an input in gpui on Windows.** The platform hands every WM_KEYDOWN to gpui before
  `TranslateMessage` (`translate_accelerator`). If anything takes the key down (a bound action, or a `key_down`
  listener that calls `stop_propagation`), it is never translated, so no WM_CHAR follows. Characters, dead keys and
  IME commits reach `EntityInputHandler::replace_text_in_range` only through WM_CHAR / WM_IME_COMPOSITION, and only
  for the focus handle whose element called `window.handle_input` in the last paint. So `FleetView::key_down`
  returns early, without stopping, while a text input has the focus (`input_focused`). Its edit and move keys are
  actions bound in the `TextInput` context (`text_input::bind_keys`), so they run before that listener. A dead key
  arrives with `prefer_character_input` and skips the bindings by itself.
- **`text_input.rs`** replaces `line_edit.rs`. `LineText` is the gpui-free model: grapheme steps, Ctrl+arrow
  words (`-`, `.` and space separate words), the double-click word, undo where typing runs join until a space or a
  caret move and an IME composition is one step, and UTF-16↔byte maps. `TextInput` is the entity: the element
  from `.repos/zed/crates/gpui/examples/input.rs` plus horizontal scroll, theme colours, a placeholder, its own
  blink task (530 ms, lit after each edit or move), click / drag / Shift+click / double / triple click, Ctrl+C/X/V,
  Ctrl+Z/Y, and an optional character filter (the Add name's `name_char`: a refused character emits `Refused`, and
  the field shows the hint). Zed's `editor` crate is not used. Focus: the input stops its own mouse-down, and a
  left mouse-down anywhere else focuses `FleetView` again. `on_focus_lost` does the same when the Add review's
  input is dropped, because keys with no focus go to the window root, not to `FleetView`'s `key_down`.
  Enter/Esc still save/cancel the review after a click on a repo took the keys from the name.
- **Search** (`search.rs`, `view/results.rs`): case-insensitive. A session name or a workspace/repo name matches
  anywhere; a session id matches from its start only (the short id or the full session id), so `5e1f` finds
  `5e1f7e57…` and not `01d05e1f…`. It searches the whole tree (`search_tree`: stopped shown, every closed one, none
  behind `more…`). Results are grouped workspace › repo with counts, a summary line, and `No results for "…"`. Session
  rows are the tree's own `render_session` (star, attach/resume, menu), with marks from `search_marks`. A click
  brings its attach tab forward, or attaches. A workspace/repo result row clicked clears the box and unfolds it.
  The results use their own scroll id (`search-results`), so they start at the top, not at the tree's offset.
  Key: **Ctrl+Shift+E** (KEYS.md). A Ctrl+letter would be taken from the program in a terminal tab. Esc clears the
  box; on an empty box, Esc hands the keys back.
- **F-04 fixed, and why 7b flaked.** The posted letters' WM_CHAR is queued behind the posted WM_KEYDOWN, so a read
  right after posting can run first. Step 7b now waits for each state it checks (`until_app`: the caret at 0 after
  the click, the edited name). A second cause showed while m5-3334's selftest ran in parallel: gpui takes modifiers
  from `GetKeyState`, which posted messages never set. Alt was held in the UI thread's key state, so End,
  Backspace and the letters arrived as `alt-end`, `alt-backspace`, `alt-a`… (trace from `observe_keystrokes`, run 9).
  The letters still typed (WM_CHAR); End matched no binding. `post_keys` now waits until no modifier is held in that
  state and fails naming the modifier if one stays held for 5 s. Not fixed: a modifier pressed mid-sequence.
  Parallel gates also share the clipboard: step 6b failed twice (runs 2 and 7, "not copied") while the other
  unit's selftest ran.
- Step 7k (`selftest_search.rs`): dispatched Ctrl+Shift+E (a posted Ctrl is not in the key state), a query typed
  as posted keys, and checks on the painted result rows: `5e1f` → the 7h closed session only, while stopped
  sessions are hidden in the tree; `closed session` → it by title; `larchsys` → the workspace with its name marked;
  `zzqx9` → No results. A posted Esc clears the box and the tree is back; a second Esc unfocuses it. Step 7f allows
  the search box between the Inbox and the tree. Shot `m5-30-search-id`.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-30\target`): build ok; `cargo test --release --workspace` 510 passed, 0
  failed; `--selftest` 5 of 5 PASS on the final binary (runs 10–14, after 2 failures caused by the parallel run and
  the 7b fix above); `--selftest-supervise` PASS (318 s); `--selftest-detach-shutdown` PASS; no `fm-selftest` rows
  from this worktree.


## 2026-10-27 — m5/33 the active tab's session is marked in the tree

- **Mapping.** An attach tab knows its 8-hex id. A shell tab (a `+` claude, a resume, a repo's New claude, or
  `claude` typed by hand) is resolved by `foreman_claude::proc_tree`: one Toolhelp snapshot, a breadth-first
  walk below the tab's pty pid (`Terminal::pid`, the pwsh ConPTY started), the nearest `claude.exe` that has a
  live `~/.claude/sessions/<pid>.json` (`peer::entry_of`: the file's `pid` is that pid and it runs with the
  registered `procStart`; read only). A claude whose `procStart` is older than the shell's start only
  inherited a reused parent pid and is skipped. The tab keeps the entry's `sessionId` (`Tab::claude`);
  `FleetView::tab_runs` matches it against a row's `session_id`, so `/clear` (new sessionId) moves the mark
  when the registry and `claude agents` have the new id.
- **When.** `FleetView::resolve_tabs` runs on the background executor, one run at a time (asked again while
  running → once more after), on `TitleChanged`/`BreadcrumbsChanged` (claude names the console at start, idle,
  work and exit, m5/29), on every poll, and a tab's `CloseTerminal` clears it. A session found that the last
  `claude agents` does not list yet sets the poll nudge, so its row comes with the next list, not after
  `REFRESH_EVERY`.
- **Latency, claude+Enter → mark painted** (step 7l, a real `fm-selftest-<hex>-mark` typed into a `-NoProfile`
  shell tab): 3.5–3.9 s without the poll nudge (registry ~1.5 s, then the 2 s poll + `claude agents`),
  **1.8–3.3 s with it** (10 runs; one slow claude start: registered 5.5 s, marked 6.6 s). `/exit` clears the
  mark in 0.17–0.73 s: claude removes its registry entry before its process ends, so the step waits for the
  process separately.
- **Mark**: `ghost_element_selected` background + a 2-px `border_focused` bar (as the attach mark before),
  hit `marked-<sessionId>`. A row that runs in another tab gets a `tab` pill (`tab-link-<id>`) that brings
  that tab forward; a search result click (m5/30's `open_result`) also brings a shell tab forward.
- Tests: `proc_tree::tests` (4: nearest first and cycles, nearest registered live claude, a file written by
  another pid, the real snapshot), `peer::tests::live_entries_and_entry_of_…`,
  `view::tests::a_tab_runs_its_attached_id_or_the_claude_found_under_its_shell`. Step 7l kills its claude by
  pid in cleanup (`Scratch::kill`, only while the pid has the recorded creation time).

## 2026-10-27 — m5/34 Copy SendMessage name, a toast instead of the info line

- **The ref, measured on claude 2.1.312**: ListAgents prints `name [ref]` where ref = the shortest prefix
  (at least 6 hex, one more than the longest prefix it shares with a neighbour in the sorted list) of
  `sha256("session:" + messagingSocketPath)`, the socket path exactly as registered. Found in the bundled JS
  (`R(kind,id) = String(hash(`${kind}:${id}`))`, `M=6`, `_e()` widens shared prefixes; `stableId ?? id`) and
  checked against 7 live sessions' ListAgents refs (all matched). With the remote flag
  `tengu_session_stable_address` on, claude hashes `session:sid:<sessionId>` instead; Foreman cannot see the
  flag. Names are compared trimmed and lowercased for ambiguity, entries without a pipe are not listed.
- `peer::send_name(live, session_id)` → the registered name, or `name [ref]` only when another live listed
  session shares it. The menu offers "Copy SendMessage name" when that lookup finds a live entry. The row's
  health is not used: a just-started interactive row is `Unknown` for a poll or two, which hid the item in 2
  of 2 runs once the poll nudge made the row appear earlier. "Copy name" and "Copy id" stay.
- **Quiet feedback**: `action_info` and its notice above the tree are gone. Added / Removed / Rescanned /
  Stopped / Copied go to `FleetView::show_toast`: an absolutely positioned box at the bottom of the left panel
  (`elevated_surface_background`, success-tinted border, `✓ text`), gone after `TOAST_FOR` (2.5 s); the newest
  replaces the last. Step 7l right-clicks its row, copies, checks the clipboard (own value, retried as 6b
  does), the toast, and that `session-row-<id>`'s top is the same before and after (it was).
- Step 7b (m5/26/m5/30's name field) flaked 2 of 7 runs before the merge with the name read right after the
  posted keys; m5/30's version on master waits for the edited name, which the merge kept.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-3334\target` + `plugins` junction, claude 2.1.312, after merging master
  b7fc76c): `cargo build --release --workspace` ok; `cargo test --release --workspace` 520 passed, 0 failed;
  `--selftest` 5 of 5 PASS; `--selftest-supervise` PASS; `--selftest-detach-shutdown` PASS; no `fm-selftest`
  rows or registry entries from this worktree left.

## 2026-10-27 — m5/31 themes: Zed theme JSON, VS Code Dark Modern as the default

- **One source.** Every colour the app paints comes from `cx.theme()` (one hard-coded colour: the titlebar ✕
  hover, Windows' red). `foreman_ui::themes::apply` builds a `theme::Theme`, sets it with
  `GlobalTheme::update_theme`, updates `TerminalConfig.palette` (the OSC colour answers) and refreshes every
  window. Call sites need no change. The terminal painter caches its palette per `Theme::id`, and every built theme gets a fresh id
  (the old `OnceLock` would have kept the first theme's palette forever).
- **Loader** (`themes.rs`, own serde types, not Zed's `theme_settings`, which pulls in the settings crate):
  `#RRGGBB` / `#RRGGBBAA` only, unknown top-level and style keys ignored, `null` or a bad value keeps One
  Dark's value for that key. A Zed key with `.`→`_` is the field name (`text.muted` → `text_muted`), via a
  field list for `ThemeColors` and `StatusColors`, plus `players[0]` and `accents`. Terminal keys a file leaves
  out fall back to its own terminal colours first: `terminal.ansi.background` → `terminal.background`,
  bright/dim foreground → foreground, `dim_*` → 70 % of the normal colour, as before m5/31. The Dark Modern file: 160 keys,
  97 set a colour, 14 are editor/minimap/debugger keys Foreman does not paint, `syntax` and
  `background.appearance` skipped.
- **Themes.** VS Code Dark Modern (bundled, `include_str!`, default), Foreman Dark (One Dark + Campbell ANSI,
  i.e. the colours before m5/31), and each theme in `<home>\themes\*.json`. A name that is not found falls back to the
  default with a note on the Appearance page. `settings.toml`: `theme = "<name>"`, `terminal_colors = "campbell"`
  (unset = the theme's). Campbell swaps ANSI 0–15 only; background, foreground and cursor stay the theme's, and the
  selection stays WT white (`SELECTION_BACKGROUND`, untouched). Settings → Appearance applies at once.
- **Contrast floors** (`fit_floors`, run on every loaded file). The m5/26 blends are measured against the panel, where they are painted:
  names (text × 0.78) ≥ 7, secondary (× 0.62) ≥ 4.5. `text.muted`, `text.accent`, error/warning/success/info/hint
  must reach ≥ 4.5 on panel, background and editor background. `fit_backgrounds` (run for Foreman Dark too) fades the marked row
  (`ghost_element.selected`, m5/33) and a search match (`search.match_background`, m5/30) toward the panel until
  the text on them holds. Keys adjusted: Dark Modern `info` #BFBFBF66 → #868686 (2.58 → 4.54),
  `ghost_element.selected` #04395E → #04395E80 (secondary 3.82 → 4.51), `search.match_background` #C6510666 →
  #C6510661 (name 6.94 → 7.15); Foreman Dark `ghost_element.selected` #3B3F4A → #3B3F4A8A (3.97 → 4.50).
  Measured, Dark Modern: names 7.16, secondary 4.98; Foreman Dark: 7.22, 5.11. Pinned in
  `the_contrast_floors_hold_under_each_bundled_theme` and `the_newer_elements_read_under_each_bundled_theme`.
  Not fitted, reported: Foreman Dark's `search.match_background` is the panel colour (#22252B), so a match
  shows only as bold. The placeholder is 3.07 (Dark Modern) and 2.77 (Foreman Dark), and Foreman Dark's `text.muted` is 3.2; both are
  as before. Dark Modern's `text.muted` equals `text` (#CCCCCC), kept as the file has it. Its bright black is
  #666666 (3.1:1 on #181818, PSReadLine's parameter colour), where Campbell's is #767676: Campbell is one click away.
- **Licence:** `THIRD_PARTY.md` and `crates/foreman-ui/assets/themes/vscode-dark-modern.LICENSE`. `release.ps1`
  copies both into the zip (checked: `Foreman/THIRD_PARTY.md`, `Foreman/licenses/vscode-dark-modern.LICENSE`).
- **Selftest:** 6c now expects the theme's bright black (#666666). Step 7m (`selftest_theme.rs`): starts in Dark
  Modern (panel #181818, ANSI 8 #666666), Appearance → Foreman Dark + Campbell paints #22252B / #767676 and
  saves them, then switches back. Shots `m5-31-tree-tabs`, `m5-31-settings-appearance`, `m5-31-settings-foreman-dark`. `m5-31-main-foreman-dark`
  catches the Settings window, which is on top, not the main window.
- **Environment:** git here refuses the worktree (dubious ownership: `.git` belongs to Administrators). Use
  `git -c safe.directory=*`, or `GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=safe.directory GIT_CONFIG_VALUE_0=*` for
  `release.ps1`. Global config was not changed.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-31\target`, after merging master c3f564e): build ok; `cargo test --release
  --workspace` 529 passed, 0 failed; `--selftest` 5 of 5 PASS; `--selftest-supervise` PASS (293 s);
  `--selftest-detach-shutdown` PASS; no `fm-selftest` rows left.

## 2026-10-28 — m5/35 hand sessions: orchestrator token, their own name, a visible override

- **The bug** (skills-9a, from WorkState's `agent_session`): René resumed mw-workflow in a Foreman tab. Before, it
  was `nomos-0c`, an orchestrator on his user registration, named through the session-header helper. After, it was
  `hand-Nomos-1`, a worker minted by Foreman, and lost work_status/work_update mid-queue. Cause: `hand.rs` minted
  `role: "worker"`, and its `--mcp-config` overrides the user-scope `workstate` entry (mint/7).
- **Role**: every hand launch (`+` and resume) mints `hand::ROLE` = orchestrator. The bearer is unchanged (repo
  role's static token = orchestrator, else the workspace's = controller), and the server caps the mint by it
  (`AgentSessionEndpoints.cs`: 403 "a role is never raised"). A refused mint opens the tab plain with the reason
  ("cannot mint an orchestrator one (…)"); there is no worker fallback. Supervised sessions (`executor.rs`) are
  untouched and keep controller/orchestrator.
- **Name, measured in WorkState's source (read-only)**: a minted token's name cannot change after the mint.
  Resolution order (a) in `docs/workstate/11-sessions.md`: a `wss_` token is its session, and both
  `X-WorkState-Agent` and `X-WorkState-Client` are ignored. `work_session` is refused for a minted session's name
  (`SessionStore.ClientKeys.cs:132`, "its name comes from its token"). There is no rename endpoint: PATCH takes
  only `claudeSessionId`. So "let the header win" is not possible, and the name has to be right at mint time.
  The helper (`session-header.mjs`) never reaches a hand session, because `--mcp-config` replaces the whole
  `workstate` entry, `headersHelper` included.
- **So**: a resume mints with the name the tree shows (`Session.name`: the `claude agents` name, else the
  transcript's custom title / agent name / ai title, `Transcript::title`), cut by `hand::session_name` to one
  sanitised line of at most 64 characters. A fresh `+` mints `hand-<folder>-<n>` and keeps it for the life of
  that claude. Its next resume carries whatever name it has given itself by then. `hand-<folder>-<n>` stays the
  key of `mcp/workstate-<key>.json` and `supervisor/hands.json` in every case, because a title is no file name.
  Fallbacks (both with a notice in the panel): when another open tab already holds that WorkState name
  (`replace: true` would revoke its token), and on a 409 (a name held by another launcher, e.g. a row the helper
  created with no launcher). Both use the generated name.
- **Not covered**: renaming a running `+` session (needs a WorkState rename for minted sessions, or a re-mint
  that a running claude cannot pick up). A resumed session lands on a new `foreman/hand` row, not on its earlier
  helper-named row (`nomos-0c`). That row has no launcher and the launcher rule refuses sharing it. Its claims
  stay there.
- **Visible override**: `hand.rs`'s header documents it. The tab card adds `WorkState  Foreman token ·
  orchestrator · <session>` for a minted hand tab (`Tab::workstate`, `tab_card::Card::workstate`).
- **Follow-up** `backlog/9`: after a WorkState restart, detect sessions whose MCP connection dropped and say
  which need /mcp → Reconnect.
- Tests: `hand::tests::hand_resume_mints_an_orchestrator_under_the_sessions_own_name` (fake server: resume
  body `name: mw-workflow, role: orchestrator`, same bearer, file key `hand-alpha-1`, second tab → generated
  name, title cut, blank title, name free after release), `…_name_held_by_another_launcher_falls_back_…` (409 →
  second POST with `hand-alpha-1`), `…_refused_orchestrator_is_not_minted_as_a_worker` (403 → one POST, plain),
  `tab_card::…_known_fields_in_order` (the WorkState line). Red: `ROLE = "worker"` plus the generated name
  always → 5 of 11 hand tests fail.

## 2026-10-28 — m5/36 × on Messages rows, sender contrast, placeholder token

- **Dismiss.** Every Messages row has a × (`msg-dismiss-<workspace>/<file>`, tooltip "Dismiss here"), the
  Inbox's `glyph_button(Icon::Close)`. `dismissed_messages.rs` keeps them in `<FOREMAN_HOME>/messages-dismissed.json`,
  `{"workspaces": {"<w>": ["<outbox file>", …]}}`, written at once (tmp + rename, like `starred.json`). The key is the
  outbox file name, so a newer note from the same sender (another file) shows. The outbox is never touched.
  `FleetView::listed_messages` drops dismissed ones before the count, so the header count and the hidden section
  follow. After each poll, `prune` forgets dismissals whose file is no longer among the workspace's listed
  (`MESSAGES_SHOWN`) messages; a workspace the poll did not list (empty outbox or read error) keeps its own.
- **Sender contrast.** The sender was painted in the row's status colour (Dark Modern `info` = #868686 after m5/31's
  fit, 4.5:1), which read as greyed out. It is now `text` in bold (9.0–9.8:1 on the info/warning tint), and the
  kind stays in the tint and the left edge. The workspace and age were `text_muted` (3.2:1 under Foreman Dark). They
  are now `quiet`: `secondary` falls to 4.42 on the hover tint. `MESSAGE_TINT` 0.10 / `MESSAGE_TINT_HOVER` 0.16 live in
  `themes.rs` and are checked in `the_newer_elements_read_under_each_bundled_theme`.
- **Placeholder (René, c5).** `text.placeholder` is the token. `fit_placeholder` makes it opaque (blended over the
  editor background) and moves it to about 3:1 when it falls outside [2.5, 4.0) on the panel or the editor background. Dark
  Modern `#FFFFFF56` → `#6B6B6B` (3.07–3.31), Foreman Dark `#6A6F79` as is (2.77–3.03). Test:
  `the_placeholder_reads_as_a_hint_under_each_bundled_theme`, which also fits a placeholder as bright as the text, and one nearly the background.
  **Measured:** the search box already painted `#6B6B6B` on `#1F1F1F` (pixel-sampled, shots
  `m5-36-search-placeholder-before/after` are identical), so gpui honours a text colour's alpha. The hint that read as
  content was the compose box's "Type here; Enter adds a line.", painted in `text_muted`, which Dark Modern sets to
  `#CCCCCC` = `text`. It now uses the placeholder.
- Step 7n (`selftest_messages.rs`): two notes from `fm-selftest-dismiss` dropped in the scratch outbox, with the pointer
  off the panel (m5/18 hold). × on the older: count 2 → 1, newer still shown, the file holds it after a
  `DismissedMessages::load` and a poll, outbox file kept. The rest dismissed → section gone. A newer note shows it
  again (count 1), and × hides it again. Shots `m5-36-messages-before`, `m5-36-messages-newer`.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-36\target`): build ok; `cargo test --release --workspace` 534 passed, 0
  failed; `--selftest` 5 of 5 PASS (138/102/104/98/102 s); `--selftest-supervise` PASS (295 s);
  `--selftest-detach-shutdown` PASS; no `fm-selftest` rows left.

## 2026-10-28 — m5/37 × on the notices above the tree

- **The box.** `render_left` drew `setup_notices`, `supervise_errors`, `hand_notices` and `action_error` with no ×, and
  `hand_notices` was only ever appended to, so a hand-token notice stayed until restart. Each now has its own ×
  (`glyph_button(Icon::Close)`, tooltip "Dismiss", key `notice-dismiss-<setup|supervise|hand|action>-<n>`, n = its
  place among the shown notices of that kind). `view/notices.rs`: `plain_notices` lists them in the drawn order,
  `dismiss_notice(kind, text)` removes that one (one entry of the setup/hand list; the action error only if it
  still reads the same).
- **Re-raised shows again.** Setup, hand and action notices are removed from their list, so the next push shows.
  Supervise errors are replaced on every poll, so a dismissed one is kept by text in `supervise_dismissed`, and
  `keep_dismissed` forgets it the first poll that no longer reports it; the same error coming back then shows.
  The `ws-setup-notice` × (`dismiss_workstate_notice`) is untouched.
- **Selftest trap.** `hits` holds the last *painted* frame's targets. Right after a state change, a key like
  `notice-dismiss-hand-1` can still be the previous frame's (another notice's ×), and a click there dismisses the
  wrong one (run 1 failed so). Step 7o's `hand_x` waits until the frame paints exactly one × per hand notice.
- Step 7o (`selftest_notices.rs`): two hand notices and an action error; × on the first leaves the other two; the
  same notice pushed again shows and its × removes it; the rest are dismissed. Shots `m5-37-notices-before/after`.
  Unit: `notices::tests::a_dismissed_supervise_error_shows_again_once_it_has_cleared`, `…each_kind_has_its_own_dismiss_key`.
  Red: hand × clears every hand notice → 7o FAIL; `keep_dismissed` a no-op → the unit test fails.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-37\target`): build ok; `cargo test --release --workspace` 539 passed, 0
  failed; `--selftest` 5 of 5 PASS (106/102/98/111/105 s); `--selftest-supervise` exit 0 (292 s);
  `--selftest-detach-shutdown` PASS (14 s); no `fm-selftest` rows left (`claude agents --json`).

## 2026-10-28 — m5/38 Delete sessions, bulk delete closed background sessions

- **Probe** (`fm probe rm-transcript`, claude 2.1.312; one `aw-m2-probe-ee27e8-rm` `--bg` session, cwd
  `<worktree>/target/aw-m2-probe-ee27e8/rm`, started hidden with the parent session's variables removed,
  `--strict-mcp-config`, `bgIsolation:"none"`): after its turn the transcript was 429 KB and there was no
  `<sessionId>/` dir (no subagents or tool results). `claude stop` → `stopped`; `claude rm` → `removed 294d5420`, and
  the row left `claude agents --json --all` at once. **rm leaves the transcript** (430 KB, untouched) and the project
  dir. **`claude --resume <uuid> -p …` still works after rm**: exit 0 in 5.0 s, it answered `ready` (its earlier
  reply, from the transcript), appended to the same `<uuid>.jsonl` (443 KB, no new file) and created no row. So
  rm alone does not make a session's text go away; deleting the transcript is what does. claude also leaves an empty
  `~/.claude/session-env/<uuid>/` dir; it is not deleted (nothing but the transcript and its sibling dir may be).
  The probe's own transcript went through the app's confined delete.
- **The one write under the claude dir** (`foreman_claude::forget`, René 2026-10-28): `forget_transcript(claude
  dir, sessionId, cwd)` deletes `projects/<dir>/<sessionId>.jsonl` and `projects/<dir>/<sessionId>/` in every project
  dir that holds them (the cwd's slug first), nothing else. The id must be a UUID (so no `..` or separator); a project
  dir, file or sibling dir that is a symlink or junction (reparse point) is refused; each path is canonicalised and
  must be `<canonical projects>/<dir>/<item>`; every path is checked before anything is deleted, so one refusal
  deletes nothing. `remove_dir_all` removes a junction inside the sibling dir without following it. Tests with a
  fake home: only the session's two items go (the rest of the tree is compared file by file), non-UUID ids incl.
  `..`, a claude dir given with `..`, junctioned sibling dir and project dir (refused, target kept), a junction inside
  the sibling dir (target kept), a symlinked transcript. Red: link check off → 2 of 9 fail (the junctioned project
  dir is still refused by the canonical check).
- **Menu.** `Delete…` (last item) on a session with no pid whose health is stopped or process gone, with a UUID:
  a background row (`claude rm`, then the transcript) or a closed interactive row from its transcript (transcript
  only). Not for a live process, a session open in a tab (`tab_runs`), or one this home's supervisor wants (Start
  resumes it). Another Foreman's `fm-` row is offered, and the question says so. Every delete asks, naming the
  session. rm's refusal (`kept … N unpushed commits … --discard-unpushed <sha>@<token>`, `RmRefusal`) becomes a
  second question with the count and the worktree ("Discard N unpushed commits and delete"); without a discard value
  it is an error line. A deleted row leaves the tree at once (`FleetView::deleted`, filtered from the poll and the
  scan for 60 s, then forgotten, so a session resumed by its uuid shows again) and the poll and scan are nudged
  (m5/28). A hand token minted for a resume carries its claude session id (`Hands::launch_for`) and is revoked on
  its Delete (`release_session`); a `+` session's token is already revoked when its claude exits.
- **Bulk.** The workspace menu offers `Delete closed background sessions…` when it has one. It skips ★ sessions,
  another Foreman's, this workspace's own wanted ones, ones open in a tab, and ones another controller launched.
  **What Foreman can tell about a launcher**: claude records no parent (no field in `claude agents`, none in the
  transcript lines; checked on this session's own transcript). rlarch's nested-claude writes its protocol header
  (`Your controller is the Claude Code session '<name [ref]>'`) into every brief; the first prompt either is that
  brief or points at `<repo>/.claude/briefs/brief-<name>.txt`. Foreman reads the first prompt (only as far as it),
  finds the header there or in the brief, and checks the controller's name against the live peer registry
  ("launched by controller X, still running" / "launched by controller X" / "launched from the brief …"). A session
  whose transcript is not found is skipped too (who launched it is unknown). A session another tool launched
  without such a brief is not recognised. The question gives the count and every skip with its reason. The run
  never discards: a session rm keeps joins the skipped list with its commit count. Afterwards an OK-only box lists
  what was deleted, skipped and why, and what failed.
- **Selftest step 7p** (`selftest_delete.rs`; merged as 7p after m5/37 took 7o): a fixture closed session `de1e7e00-…` with its `<id>/subagents/` dir in
  the fake claude dir: its menu ends with Delete…, the question names it, Delete → out of the tree in ~126 ms, file
  and dir gone, the 7h fixtures kept, still gone after a scan. Then the gate's own `fm-selftest-<hex>`: its live
  menu has no Delete; Stop from its menu; a fixture transcript with its id planted in the fake dir; Delete →
  `claude rm` and out of the tree in 0.8–1.0 s, not in `claude agents`, fixture gone. No bulk delete in the
  selftest: the gate's workspace is the user's real tree.
- **Selftests now delete their own transcripts**: cleanup collects `(sessionId, cwd)` of every own `fm-selftest`
  session (the gate's step-2 session, 7l's interactive claude, the supervise run's rows) and deletes them in claude's
  real dir with the same confined code. Empty project dirs stay. A 7l claude given no prompt wrote no transcript
  (`0 transcript(s)`).
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-38\target`): `cargo build --release --workspace` ok; `cargo test --release
  --workspace` 556 passed, 0 failed; `--selftest` 4 of the first 5 (run 5 FAIL step 7b: "the menu of
  selftest-added · ./beta: not within 5 s", m5/26's repo menu, not touched here; m5-37's gate ran beside it), then
  5 of 5 PASS (115/121/127/121/120 s); `--selftest-supervise` PASS (308 s); `--selftest-detach-shutdown` PASS; no
  `fm-selftest` rows or transcripts of this worktree left.

## 2026-10-28 — m5/40 every WorkState mint sends claudeSessionId

- **Why**: skills-9a's rule (René approved): a launcher may take over a name registered with no launcher only when
  that row's `claude_session_id` equals the mint's `claudeSessionId`. WorkState already reads it from the mint body
  (`AgentSessionEndpoints.cs:18`) and stores it with coalesce. `MintRequest::claude_session_id` sends it as
  `claudeSessionId`, left out when `None`, and never as a guess.
- **Probe** (`fm probe session-id`, claude 2.1.312; one hidden `aw-m2-probe-<hex>-sid` session, cwd
  `<CARGO_TARGET_DIR>/aw-m2-probe-<hex>/sid`, the parent's variables removed, `--strict-mcp-config`,
  `bgIsolation:"none"`), run twice with the same result:
  - `claude --bg --session-id <uuid>` is **accepted and ignored**: no error, `backgrounded · d6fc8fba`, and the row's
    `sessionId` was the daemon's own (`d6fc8fba-…`, not the `bc4948a5-…` asked for). No transcript under the asked id.
  - `claude --bg --resume <full uuid>` **keeps it**: the same `id`/`sessionId`, one row, and the same `<uuid>.jsonl`
    grew (422 KB → 429 KB).
  - A foreground `claude -p --session-id <uuid>` **takes it**: exit 0 in 4.9 s, `<uuid>.jsonl` written. That is the
    shape of the `+` tab (a foreground claude), so the `+` path relies on it.
  - Cleanup: `claude stop` + `claude rm` through `Claude::remove`, transcripts through `forget_transcript`
    (`probe_rm::cleanup`). claude leaves an empty `memory/` dir in the probe's project dir; it is not deleted (only
    the transcript and its sibling dir may be, m5/38).
- **So**:
  - Hand resume (`hand.rs` `launch_for`, from `view.rs` `resume` → `hand_claude`): the resumed `session_id`, only if
    it is a full UUID (`is_session_id`; an 8-hex short id is not sent).
  - Hand `+`: a new v4 (`foreman_claude::new_session_id`), `Launch::Minted::new_session`, and the launch script runs
    `claude --session-id '<uuid>' --mcp-config '<file>'`. The stale branch (file gone) starts plain `claude` without
    it. `MintedSession.claude_session_id` holds the id for both, so Delete's `release_session` finds a `+` token too.
  - Supervisor resume (`executor.rs` `remint`): the row's `sessionId` it resumes, also stored in `MintedSession`, so
    `bind_claude_sessions` no longer PATCHes it again after a resume. A resume that started a copy (`woke_in_place`)
    differs from the stored id and gets PATCHed with the copy's.
  - Supervisor launch (workspace, repo controllers, relay; all `--bg`): **no id in the mint**, since `--bg` ignores
    `--session-id`. The row's id reaches WorkState by the existing PATCH once the row shows it. Foreman cannot send
    anything better at mint time for a fresh `--bg` launch.
- Tests: `mint::tests::mint_http_client_sends_the_claude_session_id_when_known` (fake server body);
  `hand::tests::hand_mint_carries_the_repo_and_none_outside_a_repo` (`+`: body `claudeSessionId` = `new_session`,
  script `claude --session-id '<it>' --mcp-config`, `MintedSession` filled, each `+` its own uuid),
  `hand_script_passes_only_the_path_and_no_strict_flag` (exact `+` script, stale branch without it),
  `hand_resume_mints_an_orchestrator_under_the_sessions_own_name` (resume body = resumed uuid, a short id sends
  none), `deleting_a_resumed_session_revokes_only_its_own_token`; `executor::tests::mint_launch_writes_only_…`
  (launch: `claude_session_id: None`), `mint_relaunch_and_resume_replace_the_token_on_the_same_session` (resume:
  the row's uuid), `mint_patches_the_claude_session_once_per_mint` (after a resume: one mint with the uuid, no PATCH).
  Red: hand mint `claude_session_id: None` and `remint` minting with `None` → 3 of 12 hand tests and 2 of 19
  supervisor mint tests fail.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-40\target`, `plugins` junction beside it): `cargo build --release
  --workspace` ok, no warnings; `cargo test --release --workspace` 559 passed, 0 failed; `--selftest` 5 of 5 PASS
  (124/121/120/120/125 s); `--selftest-supervise` PASS (310 s); `--selftest-detach-shutdown` PASS (16 s); no
  `fm-selftest` or `aw-m2-probe` rows left (`claude agents --json --all`).

## 2026-10-28 — m5/41 one terminal font size, kept in settings

- **One value.** `Tab.font_size` is gone; `FleetView::term_font_size` is the size every terminal grid paints with
  (only the active tab paints, so another tab takes the new size the moment it is shown: 30–48 ms in the selftest).
  `set_term_font_size` is the one path for Ctrl+= / Ctrl+- / Ctrl+0, Ctrl+wheel and Settings → Appearance: it
  rounds and clamps to `MIN_FONT_SIZE..=MAX_FONT_SIZE`, repaints, and replaces a pending write task (dropping a
  gpui `Task` cancels it), so the file is written once, `FONT_SAVE_AFTER` (500 ms) after the last step.
- **settings.toml** `terminal_font_size` (px, integer; a float is rounded). Read with `Settings::terminal_font_size()`:
  unset → 14, out of range → clamped to 6..72. An invalid value (string, bool, array, nan) reads as unset through
  a lenient deserializer instead of failing the whole file. 14 is saved as unset (`set_terminal_font_size`). The
  bounds live in `foreman_core` (u32) and in `term_element` (px); `the_settings_bounds_are_the_painters` pins them equal.
  Read once at window start; a hand edit of the file while Foreman runs is not picked up until the next start.
- **Settings → Appearance** "Terminal font size": − value + and Reset. The page reads the fleet's value on every
  render and now re-renders on a fleet notify while Appearance is shown (as About did), so a zoom in a terminal shows
  there at once.
- **Selftest step 7q** (`selftest_zoom.rs`): two `+` tabs; Ctrl+=×2 in the second → 16 px, both grids laid out with
  `cell_metrics(16)` (the first on showing), settings.toml 16 after ~490 ms; 3 Ctrl+wheel notches → 19 px with the
  file still at 16 right after the spin, then 19; Settings → Appearance shows 19, its + → 20 in the open tab and the
  file, Ctrl+- in the terminal → 19 on the page; Ctrl+0 → 14 and the key gone from the file. Step 6c reads the shared
  size. Shot `m5-41-settings-font-size`.
- Not covered: a zoom within 500 ms of quitting is not saved.
- Tests: `settings::tests::terminal_font_size_is_14_unless_set_and_clamped`,
  `terminal_font_size_saves_clamped_and_the_default_as_unset`, `view::term::tests::the_settings_bounds_are_the_painters`,
  round trip extended. Red: clamp removed from `terminal_font_size()` → 1 of 2 fail.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-41\target`): `cargo build --release --workspace` ok, no warnings; `cargo test
  --release --workspace` 561 passed, 0 failed; `--selftest` 5 of 5 PASS (~123 s each; plus 6 earlier PASS while
  building the step); `--selftest-supervise` PASS (317 s); `--selftest-detach-shutdown` PASS; no `fm-selftest` rows of
  this worktree left (one `fm-selftest-e430b8` with cwd in the main checkout is another run's, not touched).
