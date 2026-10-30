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

Measured with `crates/foreman-app/diag/states.ps1` (retired in m5/44.15; see git history before 014d286) and `target/release/foreman.exe --selftest-detach <n>` /
`--selftest-detach-shutdown` / `--selftest` (CLI 2.1.308, Windows 11, worktree m1-7).

- **The defect.** The M1 gate's `--selftest` failed 2 of 4 from the main checkout with `fm-selftest-… is listed but
  stopped — the close stopped it instead of detaching`. **Nothing stopped it.** A control session that is never
  attached (`diag/states.ps1` (retired in m5/44.15; see git history before 014d286), raw `claude agents --json --all` row every 0.5 s) goes `state=done` on its own with
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

*Now: WorkState is the hosted server (`https://workstate.dikaia.ai`, m5/48) with OAuth and per-session minted tokens (m5/43); the local stack at `localhost:5290` and the static-token routes below are gone.*

Measured with `cargo run -p fm -- probe launch` (runs 1–4; runs 3–4 are the final code, identical answers) and
`crates/fm/probe/resume.ps1 -Variant bare|prompt|name` (retired in m5/44.15; see git history before 014d286) (CLI 2.1.308, Windows 11). Sessions `aw-m2-probe-<hex>-*`,
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
- **Cap**: live `fm-` rows with `state=working` take slots of `Policy::max_working` (default 3 then; 20 and per home since m5/45); Resumes before
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

*Now: no `workstate_env`, `.env` or static token; Foreman signs in with OAuth (m5/43) against `[workstate] url`, default the hosted server (m5/48).*

Measured with `crates/foreman-app/diag/kill.ps1`, `diag/resume.ps1` (retired in m5/44.15; see git history before 014d286) and `target/release/foreman.exe
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
  `settings.toml` `max_working` (default 20), the `busy ≤ − N +` stepper in the left header, re-read every tick.
  `foreman_supervisor::tick_all(supervisors, workspaces, snapshot, max_working, now)` ticks every platform on one snapshot
  in turn with one `Budget`: `max` − this home's live `fm-` rows that are mid-turn (m5/45) − wanted sessions still settling after
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

*Now: the `.env` token lines below are gone; see m5/43.*

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

Measured with `crates/foreman-app/diag/relay.ps1` (retired in m5/44.15; see git history before 014d286) (the relay exactly as Foreman launches it: rendered
`roles/relay.md`, `--plugin-dir`, `--add-dir <home>/relay/<w>`, `--strict-mcp-config`, auto, `bgIsolation:"none"`;
messages dropped with `fm send`, answers read from the outbox), `diag/timeline.py` on the transcripts (retired in m5/44.15; see git history before 014d286), and
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

Measured with `crates/foreman-app/diag/insession.ps1` (retired in m5/44.15; see git history before 014d286) (sessions exactly as Foreman launches them: `--plugin-dir
plugins/foreman` with this build's `fm.exe` installed, `--strict-mcp-config`, `--settings` env + `bgIsolation:"none"`),
`diag/busy.ps1` (retired in m5/44.15; see git history before 014d286) and `foreman.exe --selftest-supervise` step w (CLI 2.1.309, Windows 11, worktree m3-2). Sessions
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
- **Busy signal: `status`, not `state`.** `diag/busy.ps1` (retired in m5/44.15; see git history before 014d286) (the relay selftest's target prompt: answer messages, then
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

Measured with `crates/foreman-app/diag/skills.ps1` (retired in m5/44.15; see git history before 014d286) (a relay, a workspace session and a repo session exactly as
Foreman launches them: rendered `roles/*.md` as the prompt, `--plugin-dir plugins/foreman` with this build's
`fm.exe`, `--settings` env FOREMAN_HOME/ROLE/WORKSPACE(/REPO) + `bgIsolation:"none"`, `--allowedTools "Bash(fm:*)"
"Bash(<abs fm>:*)"`, auto, `--strict-mcp-config` and **no WorkState config**; FOREMAN_HOME and cwd under the
worktree's `target/`), `diag/endstate.ps1` (retired in m5/44.15; see git history before 014d286), and `foreman.exe --selftest-supervise` (CLI 2.1.309, Windows 11, worktree
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
- **Claude Code reads the end of a turn into `state`, and the daemon acts on it.** `diag/endstate.ps1` (retired in m5/44.15; see git history before 014d286), one turn
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
  drop the relay has already received the workspace session's `ready`. Not reproduced on purpose: `diag/relay.ps1` (retired in m5/44.15; see git history before 014d286)
  (no workspace session) 3/3 rounds delivered, `diag/skills.ps1 -Drop` (retired in m5/44.15; see git history before 014d286) (a drop right after the workspace's
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
  `crates/foreman-term/src/terminal.rs` (now `scroll.rs`); they pin the port's behaviour for Foreman's mode. Selftest step 6
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

*Now: the mint's bearer is the OAuth access token (m5/43), not a static token from `.env`; `crates/foreman-supervisor/src/mint.rs`.*

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

*Now: `client_from_settings` and `workstate_env` are gone; the client takes its bearer from the OAuth sign-in (m5/43), `crates/foreman-workstate/src/client.rs`.*

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

*Now: "not configured" means not signed in; there is no `.env` to choose (m5/43).*

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
- **Tests**: `terminal.rs` (now `mouse.rs`) `test_claude_mouse_modes_report_sgr_click_drag_release` (claude's modes → `CSI <0;6;3M`,
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
- The busy budget counts only this home's rows (m5/45): another home's mid-turn `fm-` rows take no slot.
- Tests: `fleet::tests::another_homes_fm_rows_are_foreign_shown_and_never_ranked_blocked`,
  `reconcile::tests::another_homes_parked_session_raises_nothing` (a guard: reconcile already only looks at wanted
  names). Red: `is_foreign` → `false` fails the fleet test. `--selftest-supervise` step e2 injects a fake
  `fm-elsewhere-<hex>-park` (blocked on a permission prompt, cwd in the testbed's repo) into every snapshot
  (`FleetView::injected`, which the supervisor sees too): after two ticks it is listed as another Foreman's with the
  tag painted, no Inbox row, and the testbed's own park is still Blocked in the Inbox (`m5-10-another-foreman.png`).

## 2026-10-26 — m5/3 first run

*Now: the `.env` offer and `workstate_env` are gone; the three states are worded in `crates/foreman-app/src/workstate_setup.rs` for the OAuth sign-in (m5/43).*

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

*Now: hand sessions are minted with the OAuth access token (m5/43), not a role's static token.*

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
- Now `scan` lists a root with `.git` (a folder, or a linked worktree's file) as a repo: first, named after its folder (`<folder>-root` when a child has that name), path = root, shown as `<name> (root)`. Consumers checked by tests: the registry and `fm want` cwd (`tests/want.rs`), `assign::repo_of` (children keep theirs), the `git_repo` mint key (`mint.rs`), branch via the worktree's gitdir. The relay never uses repo paths. The name is a `LineEdit` (`line_edit.rs`; now `foreman_ui::text_input`, `crates/foreman-ui/src/text_input.rs`): click/arrows/Home/End/Shift/Ctrl+A, a blinking caret, space→`-`, and a refused character shows a hint. Errors sit tinted next to Save, Save while scanning says so, and the window is re-activated after the picker. Step 7b drives it with posted messages and adds a one-repo workspace (shot `add-review-single-repo`).
- Contrast (WCAG, One Dark panel #22252b, `text` 10.9:1): `text_muted` was 3.2:1, and 1.8:1 in a 0.5-opacity row. The tree now uses `text` blends: quiet 0.78 (7.2:1), secondary 0.62 (5.1:1), dimmed rows 0.8 (5.2 / 3.8). `left.rs::the_tree_text_is_readable_against_the_panel` computes them (now `themes.rs::the_contrast_floors_hold_under_each_bundled_theme`, `crates/foreman-ui/src/themes.rs`) from the theme (shots `cut-label-tooltip`, `m5-26-selection` show the tree).
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
- **`text_input.rs`** (now `crates/foreman-ui/src/text_input.rs`) replaces `line_edit.rs`. `LineText` is the gpui-free model: grapheme steps, Ctrl+arrow
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

*Now: the mint's bearer is the OAuth access token (m5/43); the static role tokens below are gone.*

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

## 2026-10-28 — m5/42 one text input everywhere: the task box and the Find bar

- **Before**: the task box (`compose.rs`) and a terminal's Find bar (`view/term.rs` `Find::key`) each took the
  window's KeyDowns by hand (key_char appended, Backspace popped, no selection, no IME, a "▏" glued into the text).
  Both are now `text_input::TextInput`, the m5/30 input; `compose.rs` keeps only the box's state (workspace,
  reply-to, error / sent / route) and `first_line`. No other hand-made text entry is left: `settings_window`'s
  `key_down` only closes on Esc, and `FleetView::key_down` forwards key_char to the terminal's program, which is
  terminal input, not a text box.
- **Multi-line** (`TextInput::multi_line`): one element for both kinds. It shapes with `text_system().shape_text`
  (one `WrappedLine` per text line, wrapped at the box width minus 2 px for the caret; one line: no wrap, scrolled
  left). The painted rows go to the gpui-free `LineText` as the `Rows` trait (byte range per visual row, x within the
  row, offset at an x), so Up/Down (keeping the column of the first vertical move), Home/End per visual row and
  row_of's wrap rule (an offset at a wrap is the next row's start; End stops before the wrap's last character so
  the caret stays on its row) are unit-tested on a monospace `LineRows`. While the last paint is older than the text
  (keys dispatched with no frame between), `LineRows` of the text lines stands in. The box's height is a
  `request_measured_layout` that shapes at the width it is given: 3 to 8 rows, then it scrolls (the wheel too,
  only when the text is taller than 8 rows); the caret is scrolled into view after each edit or move.
- **Keys**: `enter` is `Confirm` (a new line in the task box, the event on one line), `shift-enter` `ConfirmBack`
  (the Find bar's previous match, a new line in the task box), `ctrl-enter` `Submit` (Confirm on both), `up`/`down`
  (+ shift) propagate on one line, `ctrl-home`/`ctrl-end` (+ shift) go to the text's ends. Paste keeps line breaks
  (CRLF/CR → LF) in the task box and turns them into spaces on one line (`pasted`). KEYS.md updated.
- **Focus**: the box's and the bar's inputs are in `FleetView::input_focused`, so `key_down` leaves their keys to
  them and to WM_CHAR. The old `focused` flags and the terminal's mouse-down that cleared them are gone: a click
  outside an input focuses the window (m5/30's root listener), Esc closes and focuses the window, Send refocuses the
  box after its press took the keys, and a click on the Find bar beside its input focuses the input.
- **Selftest typing**: a dispatched `KeyDownEvent` with a key_char never reaches a TextInput (on Windows characters
  come through WM_CHAR). `window.dispatch_keystroke` does: when nothing took the key it hands key_char to the
  input handler of the last paint, which exists only once a frame painted the input focused. So the steps wait
  for `is_focused && caret_painted` first. Step 6c types the Find query that way and now also checks that
  Shift+Enter goes back to the match Enter left. Supervise step t types two lines, Up (caret on line 1), Down
  (line 2), Ctrl+End, Ctrl+Shift+Left selects `WRONG.`, types `nothing.` over it, and Ctrl+Enter sends the
  result to the testbed workspace session (checked in its transcript); before the Esc check it clicks back into the
  box, because the Approve click gave the keys to the window.
- **Step f depended on René's live sessions** (fixed in m5/45: the budget counts only this home's rows). While `fm-nomos-workspace` and `fm-nomos-nomos` were mid-turn, neither of the
  testbed's two workspaces ever launched (`not both launched within 180 s (first live [None, None])`), with this
  unit's build and with main's (6b3df5a) alike. The selftest should not count sessions outside its testbed.
- Tests: `text_input::tests::up_and_down_keep_the_column_across_shorter_lines`,
  `home_end_and_vertical_moves_follow_wrapped_rows`, `shift_up_and_down_select_across_lines`,
  `a_paste_keeps_its_lines_on_several_and_joins_them_on_one`, `word_moves_cross_line_breaks`;
  `view::term::tests::find_steps_up_from_the_bottom_and_wraps` on `find_step`. compose.rs's key tests went with its
  key handler. Red: `move_vertically` without the goal column → 1 of 14 `text_input` tests fail.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-42\target`): `cargo build --release --workspace` ok, no warnings; `cargo test
  --release --workspace` 565 passed, 0 failed; `--selftest` 5 of 5 PASS (119–121 s); `--selftest-detach-shutdown`
  PASS; no `fm-selftest` rows of this worktree left. `--selftest-supervise`: FAIL at step f, twice on this build (434 s, 440 s) and once on main 6b3df5a (437 s), all `not both launched within 180 s (first live [None, None])` while fm-nomos-workspace and fm-nomos-nomos were busy (see above); its step t (the task box) passed in both runs on this build.

## 2026-10-28 — m5/43 Foreman signs in to WorkState with OAuth; no .env

- **Why**: WorkState moves to `https://workstate.dikaia.ai` (skills-9a runs the move); no static token moves with it,
  and René decided there is no `.env` fallback. Every WorkState call Foreman makes — the read API v1, `/api/stream`,
  and every `POST/PATCH/DELETE /api/agent-sessions` (hand `+` and resume, the supervisor's workspace and repo
  controllers) — now carries the access token of one OAuth sign-in. Mint bodies are unchanged (`claudeSessionId`
  etc.). Contract: Skills `docs/workstate/10-read-api.md` rule 5, `14-agent-launch.md` "Minting with OAuth".
- **Gone**: `settings.toml` `workstate_env`, `foreman-core` `dotenv.rs` (`read_env_value`, `has_env_key`),
  `foreman-workstate` `credentials.rs` (`client_from_settings`), `tokens::token_from`/`token_key`, the first-run
  "Use ~/.workstate/.env" offer and `FOREMAN_WORKSTATE_INSTALL`. An old file's `workstate_env` line is ignored and
  dropped at the next save; an old `…/mcp` `workstate_url` means its origin.
- **Server URL**: `workstate_url` is an origin (`Settings::workstate_origin`, now `WorkStateSettings::origin`; default `http://127.0.0.1:5290` until
  the cut-over, now `DEFAULT_WORKSTATE_URL`; `workstate_mcp_url` = origin + `/mcp` for minted configs). `validate_workstate_url`: http(s) only,
  a host, an optional port, any path dropped, plain `http` only for 127.0.0.1/localhost/::1.
- **The client** (`foreman-workstate/src/auth.rs`, `Auth`), written on the ureq stack the crate already had:
  discovery `<origin>/.well-known/oauth-protected-resource/api` → `authorization_servers[0]` → its
  `/.well-known/oauth-authorization-server` (RFC 8414 path insertion; the issuer compared without its trailing
  slash; S256 required when the server lists methods). **DCR afresh at every interactive sign-in** (the controller's
  point from skills-9a: the server's DB is restored with `--clean` at the cut-over, and an unknown `client_id` fails
  on the server's own page in the browser, where Foreman never sees `invalid_client`). Public client,
  `token_endpoint_auth_method: none`, grants `authorization_code` + `refresh_token`, redirect
  `http://127.0.0.1/callback`. Authorize: PKCE S256 (32-byte verifier), a 16-byte `state` checked on return,
  `resource=<origin>/api` exactly once, `scope=workstate.read workstate.sessions offline_access`, loopback listener on
  `127.0.0.1:<ephemeral>/callback`, 5 min, the browser answered with "You can close this tab". The token requests
  send no `resource` (the grant carries it).
- **Tokens**: the refresh token only in Windows Credential Manager (`secret.rs`, `CredWriteW`/`CredReadW`/`CredDeleteW`,
  generic credential `Foreman/WorkState:<issuer>`, persist local machine), its `client_id` beside it in
  `<home>/oauth/servers.json` (with the discovered endpoints; nothing secret). The access token lives in memory and
  is refreshed when less than 5 min is left. **Refresh tokens rotate**: every refresh stores the new one, under
  `<home>/oauth/refresh.lock` (`File::lock`), and reads the stored one fresh, so the app and `fm` (a separate process
  that refreshes on each `fm work`) never redeem the same one twice. A refresh refused with `invalid_client` or
  `invalid_grant` drops the pair (credential + client_id) and leaves "Sign in again" in memory, with no retry until
  the next sign-in; an unreachable server is just "unreachable" and the next call tries again. A 401 on a read drops
  the held access token and retries once with a fresh one; `/api/stream` asks for the bearer at each (re)connect.
- **Sign out**: the refresh token to `revocation_endpoint` (`token_type_hint=refresh_token`): the whole authorization
  goes, and every `wss_` minted under it with it. Then the credential and the held tokens. A failed revoke is a notice.
- **TLS**: ureq had no TLS at all (`default-features = false`). `ureq` feature `native-tls-no-default` with
  `TlsProvider::NativeTls` + `RootCerts::PlatformVerifier` (schannel, Windows' own root store; no ring/C build, no
  bundled roots), one agent for every call (`foreman_workstate::http_agent`); the stream's raw socket gets
  `native_tls::TlsConnector` for `https://`. New crates: `native-tls`, `schannel` (+ `der`, `rustls-pki-types` via
  ureq); `getrandom 0.3`, `base64`, `sha2`, `windows` were already in the tree. Nothing else was worth a dependency:
  an OAuth crate would bring its own HTTP stack.
- **Roles**: the OAuth principal is the operator as controller ("a role is never raised"), so `AppTokens` mints
  workspace → controller, repo → orchestrator, hand → orchestrator. The relay still has no WorkState role and
  launches without WorkState, as before (it only carries messages; it was never minted with the `.env` either).
  `TokenProvider::token` is now `Result<Option<RoleToken>, String>`: `Ok(None)` not signed in (launch without
  WorkState), `Err` signed in but no token now (unreachable, sign in again) → the launch fails with the reason
  instead of silently starting without WorkState.
- **Settings → WorkState**: Status ("Signed in on <url> as <name>" — the token's `name`/`sub` when it is a readable
  JWT, else just the URL; "Not signed in."; "Sign in again: <why>."), Sign in / Cancel sign-in / Sign out / Check now;
  Server URL in the shared `TextInput` + Apply (Enter applies). The notice above the tree says "Not signed in to
  WorkState at <url>…" with **Sign in** (`ws-setup-signin`; Cancel while the browser waits). `workstate_setup::detect`
  is `/health`, then `Auth::access_token` (a due refresh happens there, every 30 s).
- **Changing the URL** (`FleetView::set_workstate_url`): not signed in → saved at once. Signed in → sign in on the new
  server first; only then the URL is saved. `Supervision::tick` sees the origin change and calls
  `Supervisor::server_changed` on every supervisor: each minted, still-wanted session goes on `remint_due`, so the
  next tick stops its live row and the resume (full uuid) mints on the new server with `replace: true` into the same
  config file. Open hand tabs get " · restart to use <url>" on their card and one notice. The old server's grant is
  revoked in the background (a failure is a notice). A server that cannot be signed in to leaves the URL as it was.
- **How René signs in the first time**: start Foreman → the notice "Not signed in to WorkState at
  http://127.0.0.1:5290" → **Sign in** (or Settings → WorkState → Sign in). The browser opens WorkState's sign-in
  page; log in as `operator` and approve; the tab says "You can close this tab" and the notice goes. For the hosted
  server: Settings → WorkState → Server URL `https://workstate.dikaia.ai` → Apply; if signed in on the local one,
  the browser opens the hosted server's sign-in first, then Foreman switches, restarts its supervised sessions with
  hosted tokens, asks to restart open claude tabs and signs out of the local server. Check without printing a token:
  Credential Manager (Windows Credentials → Generic) shows `Foreman/WorkState:https://workstate.dikaia.ai/`.
- **Tests** (never the real servers): `FakeWorkState::enable_oauth` makes the fake its own authorization server
  (`FakeAuth`: PRM, AS metadata, DCR, authorize as an immediate 302 the test follows, token with PKCE check and
  rotating refresh that refuses a redeemed token, revoke of the grant, 401 + `WWW-Authenticate` on `/api/*`
  without a live token; `forget_clients`, `fail_refresh`, `deny`, `set_access_ttl`). `tests/oauth.rs` (10): sign-in
  shape (one resource, scope, S256, loopback, state, no token in any file of the home, no token in `Debug`), refresh
  rotation, a fresh process refreshing from the store, state mismatch (no exchange, nothing stored), denied /
  timed out / no browser, fresh DCR per sign-in, wiped server → sign in again → new registration, refused refresh
  without retries, sign-out revoke, 401 then refresh-and-retry. `secret.rs`: Credential Manager round trip under a
  random `fm-test-<hex>` prefix, deleted after. `fm` `tests/work.rs`: the test signs in into Credential Manager
  under a random prefix (`FOREMAN_CREDENTIAL_PREFIX`), `fm work` refreshes and stores the rotated token, the entry is
  deleted on drop. App: `tokens::tests` (provider, mint + revoke over HTTP with the fake AS's bearer),
  `workstate_setup::tests`, hand tests ported to the fake AS (+ a broken sign-in opens plain), supervisor
  `mint_on_a_new_server_stops_and_resumes_every_minted_session_with_its_new_url`,
  `mint_without_a_bearer_now_fails_the_launch_with_why`. Red: not storing the rotated refresh token and reusing a
  cached client_id → 2 of 10 OAuth tests fail.
- **Selftest**: every selftest home's `workstate_url` is a closed loopback port, so no status check reaches a real
  WorkState; the sign-in never opens a browser (`signin::use_http_browser`), and its credentials sit under
  `fm-selftest-<hex>` and are deleted at cleanup. Step 1w signs in to the fake's AS. Step 7d: Settings shows "signed
  in … as Fake Operator"; Sign out revokes the grant and the notice offers Sign in; the notice's Sign in signs in
  again and the Inbox paints the fake's WorkState rows read with the new tokens; Apply of another fake server's URL
  signs in there, saves it and revokes the old grant; a closed port's URL is refused with the reason and the URL
  stays; plain `http` to another machine is refused; closed port → unreachable notice with Retry. Step 7's
  read-only check accepts only access tokens the fake issued.
- **Not covered**: the real end-to-end sign-in on 127.0.0.1:5290 and on the hosted server (the controller's and
  René's: the `operator` password is René's). The hand tab restart notice is unit-free (UI only). A selftest that
  crashes mid-run leaves one `fm-selftest-<hex>:http://127.0.0.1:<port>/` credential (worthless: its fake server is
  gone). The URL field has no "revert" button: Esc does nothing.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-43\target`, `plugins` junction beside it): `cargo build --release
  --workspace` ok, no warnings; `cargo test --release --workspace` 576 passed, 0 failed; `--selftest` 5 of 5 PASS
  (124/127/128/124/122 s; idle frames p50 ~41-46 ms with own work p50 ~6 ms while three of René's sessions ran);
  `--selftest-detach-shutdown` PASS (23 s); no `fm-selftest` rows left. `--selftest-supervise` FAIL twice at step f
  ("not both launched within 180 s"), every step before it PASS: the busy budget counts every live `fm-` row on the
  machine, and René's `fm-nomos-workspace`, `fm-nomos-nomos`, `fm-nomos-nomos-letterhead-nomos-3` were mid-turn
  throughout, so step f's limit of 1 never had a free slot. Not touched (not ours); to be rerun when they are idle.

## 2026-10-28 — m5/45 the busy budget counts only this home's sessions; default limit 20

- **Per home.** `tick_all(supervisors, workspaces, snapshot, max_working, now)` builds its `Budget` from the rows
  `foreman_supervisor::is_foreign` does not reject: named `fm-<workspace>-…` after a registered workspace (or one a
  supervisor ticks), or wanted by a supervisor's desired state. Another `FOREMAN_HOME`'s mid-turn rows take no slot.
  Two Foremans never coordinated their launches anyway, so a shared count only made each one block the other: a
  running selftest lowered René's budget, and René's busy `fm-nomos-*` sessions kept `--selftest-supervise` step f
  (limit 1) from launching at all.
- **One ownership test.** `is_foreign` moved from `fleet.rs` into `foreman-supervisor` (desired.rs); `fleet::is_foreign`
  adapts the app's `workspaces` + desired map to it. The fleet's dimming and the budget cannot disagree on whose a row is.
- **Default 20** (René, via the controller): `DEFAULT_MAX_WORKING` 3 → 20, equal to `MAX_WORKING_LIMIT`. The left
  header's selftest step now presses − then + (a + at the bound does nothing). `--selftest-supervise` sets its own
  limit of 1 for step f and nothing else there depends on the default; its step f line now prints `limit 20 → 1`.
- Text: the header's busy tooltip and Settings → "Busy sessions at most" say the limit is this Foreman's sessions
  and that another Foreman's are not counted.
- Tests: `executor::tests::another_homes_busy_sessions_take_no_slot` (two foreign busy rows; limit 1: alpha goes,
  its own busy row holds the slot, beta goes once alpha is idle — step f on fixtures; a row this home wants under a
  foreign-looking name still counts); `two_started_workspaces_never_exceed_the_shared_budget_together` now counts a
  registered-but-unsupervised workspace's busy row. Red: `tick_all` without the ownership filter → 1 of 84
  foreman-supervisor tests fails.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-45\target`): build ok, no warnings; `cargo test --release --workspace` 566
  passed, 0 failed; `--selftest-supervise` PASS (328 s) with fm-nomos-workspace, fm-nomos-nomos and
  fm-nomos-nomos-letterhead-nomos-3 busy at its start, step f max 1 working over 31 polls; `--selftest` 5 of 5 PASS
  (116–123 s) on a second series — the first series had 4 of 5, run 4 failing step 7j's 1500 ms Refresh bound while a
  selftest of the main checkout ran alongside; `--selftest-detach-shutdown` PASS; no `fm-selftest` rows of this
  worktree left.

## 2026-10-28 — m5/46 Alt+V image paste: selftests turned the user's screenshot into PNG-only

- **Alt+V reaches claude.** "No image found in clipboard. Use alt+v to paste images." is the text of claude 2.1.312's
  `chat:imagePaste` handler (bound to alt+v on win32; Ink reads Foreman's `ESC v` as meta+v). It shows when
  `checkImage` (`powershell -NoProfile -NonInteractive -Sta -Command "…ContainsImage()…"`) exits non-zero *and*
  the text fallback (`Get-Clipboard -Raw`) returns nothing. So the clipboard held neither a WinForms-visible
  image nor text.
- **The terminal is innocent.** With a CF_DIB image on the clipboard the exact check exits 0 as a plain process, in a
  raw ConPTY (`examples/pty_dump.rs`) through pwsh → cmd → powershell with pwsh 7's PSModulePath, with Foreman's
  `TERM`/`TERM_PROGRAM=zed`/`ZED_TERM`/`COLORTERM`/`LANG` added, and in a real Foreman tab. René's Foreman is High
  IL like his WT. Its tabs differ from a WT pwsh only by `__COMPAT_LAYER=DetectorsAdminProtection` (inherited from
  Total Commander and from elevated shells; this session has it too and the check passes), `COMMANDER_*`, the Zed
  vars, and system conhost instead of WT's OpenConsole.
- **Cause: gpui's clipboard round trip.** `read_from_clipboard` turns CF_DIB into `Image { format: Bmp }`. Writing it
  back puts only the registered `PNG` format on the clipboard (`gpui_windows/src/clipboard.rs` `write_image`).
  WinForms `ContainsImage` asks for Bitmap, so it says no. Measured: DIB → check 0; gpui read + write → check 1.
  Selftest steps 6b, 6c and 7l "restored" the user's clipboard that way, so every gate on this machine (5
  `--selftest` runs per unit, several units a day, often in parallel) turned René's screenshot into a PNG-only
  entry that claude cannot paste anywhere. They also skipped the restore when gpui read nothing, which left
  `fm-paste-…` on the clipboard. No product path writes an image: OSC 52, copy and the text input write text.
  That René's own failure happened this way is inferred, not seen: it matches the symptom exactly (no image, no
  text), and nothing else in the tab reproduced it.
- **Fix:** `selftest_clipboard.rs` (`Snapshot::take`/`restore`: every HGLOBAL format byte for byte; GDI-handle
  formats are skipped and Windows synthesizes CF_BITMAP from CF_DIB again). 6b, 6c and 7l use it and always
  restore. A selftest step that borrows the clipboard uses this, never gpui's read/write.
- **Step 7r** (`selftest_paste.rs`): it saves the clipboard, puts a 2×2 CF_DIB on it, borrows and restores it as the
  other steps do, then types claude's exact check (through `cmd`, so pwsh's own `powershell.exe` env fix-up does
  not apply) into a Foreman shell tab and asserts exit 0. Then it puts the user's clipboard back (held ~0.9 s,
  9.6 s once under load). Red: the borrow restored through gpui → `FAIL claude's image check in a Foreman tab
  exited 1 …`. Unit test: `selftest_clipboard::tests::the_test_image_is_a_2x2_32_bit_dib`.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-46\target`): build ok, no warnings; `cargo test --release --workspace` 581
  passed, 0 failed; `--selftest` 5 of 5 PASS (122–127 s) on a second series. The first series had 3 of 5: run 1
  FAILed 7k (typed `5e1f` read `%E!F`, Shift held during the typing) and run 3 quit at 27 s before a verdict;
  neither touches the clipboard code. `--selftest-supervise` PASS (316 s); `--selftest-detach-shutdown` PASS; no
  `fm-selftest` rows of this worktree left (the listed ones were m5-47's live run).

## 2026-10-28 — m5/47 terminal font: Cascadia Code with ligatures, a font setting, fallback glyphs fitted

- **Setting.** `settings.toml` `terminal_font_family` (unset or not installed: `default_terminal_font_family`, the
  first installed of `TERMINAL_FONT_PREFERENCE` = Cascadia Code, Cascadia Mono, Consolas; matched in any case,
  spelt as installed) and `terminal_ligatures` (unset: on; on is saved as unset). A value of the wrong type reads
  as unset through `lenient` and costs nothing else in the file. `FleetView` holds `term_font_family`,
  `term_ligatures` and `installed_fonts` (`all_font_names()`, read once at start); `term_font()` builds the
  `term_element::TermFont { family, size, ligatures }` every grid paints with, and `set_term_font` saves at once
  and repaints. The next prepaint lays the grid out with `cell_metrics(&TermFont)` and resizes the pty, as a
  size change does (m5/41).
- **Settings → Appearance**: "Terminal font" lists the installed monospace families (`is_monospace`: `i`, `M`,
  `W`, `.` advance alike; found on a background thread once per process, `MONO_FONTS`), with Default; a family
  in the file that is not installed shows a note. "Ligatures" On/Off.
- **Ligatures keep the grid.** Runs are shaped with `force_width` as Zed's terminal does; Cascadia Code's `->`
  is `calt` glyph substitution with one glyph per cell, so `b` stays on its cell. `BatchedTextRun::cell_starts`
  maps glyph byte indices to cells, and `on_cells` checks every cell-starting glyph is at its cell's x (±1 px); a
  font whose ligature glyph spans several cells fails that and the run is shaped again with `no_ligatures()`
  (calt, liga, clig off; gpui's `disable_ligatures` turns off calt only). Runs still split on any style change.
- **Fallback glyphs fit.** DirectWrite gives a missing character glyph 0 (.notdef), not an error, so
  `advance()` alone cannot tell; `drawn_by_fallback` compares the font's own advance with the shaped width
  (`layout_width`, which uses the fallback). A non-ASCII character outside U+2500–257F that is drawn by a
  fallback font or is wider than its cells (1, 2 for a wide char, `Cell::is_wide_char`) is its own `fit` run:
  `paint_fitted` measures its ink (`typographic_bounds`), scales the font size down so ink and advance fit the
  cells, and paints under a content mask of those cells. Box drawing stays in the font; block elements stay
  rects. The per-(font, char) answer is cached.
- **Selftest step 7s** (`selftest_font.rs`; 7s after m5/46 took 7r): `record_painted_glyphs` keeps what the last grid paint drew (char,
  glyph id, x, ink, fitted). A `+` tab prints `◯x a->b`. Cascadia Code: `◯` inks 0.0..8.2 of 0.0..8.2 (Cascadia
  has it), `x` at 8.2, `b` at 49.2, `->` glyph ids differ from a `no_ligatures` shape. Appearance → Consolas:
  cells 8.20 → 7.70 px, 113 → 121 columns, laid out in ~33 ms, saved; `◯` fitted (14 → 6.86 px), inks 0.5..7.2 of
  0.0..7.7. Ligatures off: `->` plain, `b` still at 49.2. Default: both keys gone from the file. Shots
  `m5-47-term-default-font`, `m5-47-settings-terminal-font`, `m5-47-term-consolas`.
- Red: fitting disabled in `layout_grid` → 7s FAIL "`◯` inks 0.96..14.75, outside its cell 0..7.70".
- Tests: `settings::tests::the_default_terminal_font_is_the_first_installed_of_…`,
  `terminal_font_family_is_the_one_set_when_installed_else_the_default`,
  `terminal_font_family_and_ligatures_load_save_and_survive_bad_values`, round trip extended;
  `term_element::tests::a_fitted_character_is_a_run_of_its_own_…`, `cell_starts_skip_zero_width_characters`,
  `ligatures_off_disables_every_ligature_feature`.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-47\target`): `cargo build --release --workspace` ok, no warnings; `cargo
  test --release --workspace` 586 passed, 0 failed; `--selftest` 5 of 5 PASS (120–131 s); `--selftest-supervise`
  PASS (303 s); `--selftest-detach-shutdown` PASS (14 s); `claude agents --json --all`: 0 `fm-selftest` rows.

## 2026-10-28 — m5/48 sessions minted on another WorkState server are re-minted; the hosted server is the default

- **The bug**: after the cut-over René's `fm-nomos-workspace` kept `mcp/workstate-fm-nomos-workspace.json` on
  `http://127.0.0.1:5290` (ECONNREFUSED on `/mcp`). m5/43 re-minted only when `Supervision::tick` saw the origin in
  `settings.toml` change between two ticks, and only `FleetView::sign_in_to(…, from: Some(old))` revoked the old grant.
  His URL was saved by one attempt (which then hung on the server) and signed in to by a plain Sign in, so the
  origin had already changed while not signed in, and nothing was re-minted. A minted record did not say which
  server its token came from.
- **Origin on every token**: `MintedSession.origin` (`api_base` of the minted MCP URL) in
  `<home>/supervisor/<workspace>.sessions.json` and `supervisor/hands.json`. A record without one (a file from
  before) takes the origin of the URL in its config file at load (`MintedSessions::adopt_origins`, via the new
  `foreman_claude::mcp_config_url`, which reads only `url`, never the header); without a config file it stays unknown.
  `MintedSessions::elsewhere(origin)` lists the records not on `origin` (unknown counts as elsewhere).
- **One path** (`Supervisor::server_changed(Option<origin>)`): `Supervision::tick` calls it on every supervisor every
  tick with the configured origin when `Auth::status` says signed in there (`supervise::signed_in_server`, no network),
  else `None`. The first call with an origin it has not checked puts every wanted session minted elsewhere on
  `remint_due` (stop of the live row, resume with a fresh `replace: true` mint into the same file), once per origin.
  This one call covers start-up, a plain sign-in and the URL change; the old "origin differs from last tick" branch is gone.
- **Inbox**: a wanted session whose single live row still holds another server's token (checked while it was not
  live, a failed stop) gets `NeedsKind::OtherServer` ("other server") with `Remedy::Remint`; `retry` treats it like
  TokenExpired.
- **Never the wrong server by id**: PATCH (`bind_claude_sessions`), DELETE (`revoke_unwanted`) and a hand
  `release` skip a record whose origin is not the signed-in server's: its session id means nothing there (or another
  session). Those tokens go with their server's grant.
- **Hand tabs**: `Tab.hand_key` (the `hands.json` key) and `Tab.restart_to`. `FleetView::sync_server` runs whenever the
  status becomes Ready on an origin not yet synced (start-up, after any sign-in): open hand tabs whose token is
  elsewhere get " · restart to use <url>" on their card, plus one notice (`hand::restart_notice`).
- **Old grants**: `FleetView::leave_servers` signs out of each origin left behind (the supervisors'
  `Report::left_servers`, the hands' elsewhere and `Hands::take_left` of released ones, the URL change's `from`), once
  per origin per run, on the background executor; failures make one notice. `Auth::sign_out` now drops the held token,
  reads and deletes the credential under the locks, and only then posts the revoke, with its own 4 s agent
  (`REVOKE_TIMEOUT`). Before, it held the auth mutex and `refresh.lock` across a 15 s call, so revoking an unreachable
  old server stalled every `access_token` and so every mint. The local server is gone, so that is now the normal case.
- **Default** (controller): `DEFAULT_WORKSTATE_URL` = `https://workstate.dikaia.ai`. The Unreachable fix no longer
  offers to start a local server. Selftests and tests keep their own fake or closed ports; the `fm` probe keeps
  `127.0.0.1:5290` (an invalid token must not reach the real server).
- Tests: `tokens::tests::a_plain_sign_in_on_another_server_restarts_each_session_minted_on_the_old_one` (fake A and
  fake B with their own OAuth, the URL saved alone → `None`, a plain sign-in on B → stop, resume, the file names B with
  B's token, the record's origin is B, nothing DELETEd on A); `executor::tests::mint_of_unknown_origin_is_reminted_once_…`,
  `mint_on_another_server_offers_re_mint_and_is_never_revoked_or_patched_on_this_one`, the m5/43 test on the new
  signature; `mint::tests::minted_sessions_of_unknown_origin_take_their_config_files_…`;
  `hand::tests::a_hand_tab_minted_on_another_server_is_told_to_restart_and_left_to_its_grant`,
  `a_hands_file_from_before_m5_48_takes_its_config_files_origin`; `tests/oauth.rs`
  `signing_out_of_an_unreachable_old_server_is_quick_once_and_blocks_no_other_token` (a revoke endpoint that accepts
  and never answers: the other server's token in < 1 s meanwhile, the sign-out fails within the timeout, a second one
  is silent; a closed port fails at once). Red: `server_changed` scheduling nothing → the end-to-end test fails.
- **Flaky, not from this unit**: `fm --test work` `work_errors_are_one_line_without_the_token` fails about half the
  time with `sign in … again: the server refused the refresh (invalid_grant)` mid-sequence; 6 of 10 failed with
  HEAD's `auth.rs` swapped in, 4 of 10 with this one. Not investigated.
- **Not covered**: no selftest step for the plain sign-in path (unit-tested); the hand restart notice is drawn by the
  view only.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-48\target`): `cargo build --release --workspace` ok, no warnings; `cargo test
  --release --workspace --no-fail-fast` 594 passed, 0 failed (the `fm` flake above failed in an earlier run);
  `--selftest` 5 of 5 PASS (123–126 s); `--selftest-supervise` PASS (303 s); `--selftest-detach-shutdown` PASS (12 s);
  no `fm-selftest` rows of this worktree left (the four listed were m5-49's).

## 2026-10-28 — m5/49 fallback glyphs sized like Windows Terminal: full size, overhang into a blank neighbour

- **Which font draws what** (selftest 7s `probe`, 14 px, measured on this machine). Cascadia Code (8.20 px
  cells): `⏵` U+23F5 is Segoe UI Symbol, advance 14.0, ink 4.4 × 8.8; `◯` U+25EF 8.2 × 8.2, `≥` `≤` 6.5 × 7.5
  and `→` 7.1 × 7.1 are Cascadia Code's own (advance 8.2). Consolas (7.70 px): `⏵` Segoe UI Symbol as above,
  `◯` Segoe UI Symbol advance 15.7, ink 13.8 × 13.8; `≥` `≤` (5.9 × 8.5) and `→` (7.4 × 5.4) are Consolas's own.
  gpui's `get_font_for_id` does not know fallback faces, so `probe` matches the shaped run's font id against
  `resolve_font` of the usual fallback families (`FALLBACKS`).
- **Why `⏵⏵` looked like `››`.** m5/47 fitted ink ∪ advance into the cell. Segoe UI Symbol's `⏵` is 14.0 px
  wide by advance but its ink is only 4.4 px, so it was shrunk to 8.2 px (59%, ink 5.1 px high) for no reason.
- **What Windows Terminal does** (microsoft/terminal @ f356093, read in `.repos/terminal/src/renderer/atlas`):
  - Fallback: `IDWriteFontFallback::MapCharacters` from the configured family (`AtlasEngine.cpp:987`); its
    `scale` out-parameter is ignored (`assert(scale == 1)`, `:1037`). The fallback face is shaped at the same em
    size as the main font (`fontEmSize = font->fontSize`, `:1137`). This is the same system fallback DirectWrite
    gives gpui, so WT very likely picks Segoe UI Symbol for `⏵` too. I did not check that in a running WT: UI
    automation is only allowed on our own Foreman.
  - Grid: each glyph's advance is forced to its cells (`AtlasEngine.cpp:929`; complex runs `:1173`), so the
    next glyph starts on its own cell whatever the font's advance is.
  - Size and overhang: the glyph is rasterised at full size from its black box (`GetGlyphRunWorldBounds`,
    `BackendD3D.cpp:1496`) and placed at the cell's x plus the glyph's own bearing, on the main font's baseline
    (`BackendD3D.cpp:1122`, `:1182`). It is **never scaled to the cell, never centred, and not clipped**: it
    overhangs into any neighbour, blank or not. The only exceptions are a ligature-wide glyph
    (`overlapSplit`, `:1562`), which is split so each cell gets its own colour (still not clipped), and
    builtin glyphs (box drawing, blocks, `BuiltinGlyphs.cpp:1785`), which are drawn by code into one cell.
- **Foreman's rule** (`paint_fitted`, `term_element.rs`): full size on the terminal font's baseline (the
  baseline of a shaped `m`), at the font's own x, like WT. Two differences from WT, both on purpose:
  - Ink must not cover a non-blank cell. It may overhang into `room`: the blank cell after it, and the blank
    cell before it unless a fitted character two cells back can already claim that cell (`give_room`, computed
    per line in `layout_grid` from an `Inked` map; one cell each way).
  - If the ink does not fit, it is first moved inside the cells plus room, and only then scaled about its centre,
    just enough to fit (`(cells + room) / ink width`, ink only, not advance). The mask is the cells plus room.
  - No 85% floor in the code. A floor would break the m5/47 guarantee: `◯` in Consolas needs 0.56 to stay out of
    `x`'s cell. Every glyph measured here that needs no shrinking (all of them in Cascadia Code) stays at 100%,
    and the selftest makes the 85% a check.
- **Selftest 7s** prints `⏵⏵ auto mode on`, `◯general-purpose`, `a → b`, `≥ ≤` and checks the `⏵⏵` row: each
  `⏵` ≥ 85% of its unscaled ink height, the first one inside its own cell, the second up to `a`'s cell.
  Cascadia Code: `⏵` at 3.8..8.2 of 0.0..8.2 and 13.5..17.8 of 8.2..24.6 (it overhangs the blank by 1.4 px), both
  100% high. `◯x` still keeps `x`'s cell clear (Consolas `◯` fitted 14 → 7.81 px). `PaintedGlyph` gains
  `ink_height`.
- **Before and after** shots (`FOREMAN_SHOTS`): `C:\fmbuild\m5-49\shots\before\m5-49-term-fallback-glyphs.png`
  shows `››` half as tall as the text. `...\after\...` shows two full-height triangles, as WT draws them.
  `◯`, `→`, `≥`, `≤` are unchanged because Cascadia Code has them.
- Red: the new 7s check against m5/47's `paint_fitted` → FAIL "`⏵` in cell 0 inks 5.126953px high, under 85%
  of its unscaled 8.75px (fitted at 8.203125px)".
- Test: `term_element::tests::a_fitted_character_may_overhang_a_blank_neighbour_that_no_other_fitted_one_claims`.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-49\target`): build ok, no warnings; `cargo test --release --workspace
  --no-fail-fast` 587 passed, 1 failed: `fm` `work_errors_are_one_line_without_the_token`. It flakes on its own
  (7 of 10 runs of the test binary fail, "refused the refresh (invalid_grant)" instead of "HTTP 403:
  insufficient_scope"). `fm` depends on neither foreman-ui nor foreman-app, so this unit's change cannot cause
  it. `--selftest` 5 of 5 PASS (125–137 s). `--selftest-supervise` FAIL once at step s ("the tree does not
  overflow a 480 px window"), then PASS (298 s). `--selftest-detach-shutdown` PASS (13 s). 0 `fm-selftest` rows.

## 2026-10-29 — m5/44.2 terminal links open only on Ctrl+click (audit U1: A-26, A-49)

- **Rule** (René, m5/44 decision 5): an OSC 8 link or a detected URL opens only on Ctrl+click
  (`modifiers.secondary()`), for any scheme, with no allowlist. A plain click selects or positions as usual.
- **A-26 confirmed and fixed.** `Terminal::mouse_up` called `cx.open_url(link.uri())` for any OSC 8 cell under
  a plain left click. That branch is gone. A plain release now only queues `FindHyperlink(open)` when Ctrl is
  held. A Ctrl press on a link is already captured in `mouse_down` (`find_hyperlink_at_point` finds OSC 8 cells as
  well as regex URLs), so both kinds go through `ProcessHyperlink` → `Event::Open`.
- **A-49 confirmed and fixed.** `Event::Open` was dropped by the `_ => {}` arm in `view.rs`. It now goes to
  `open_link`: `Url` → `cx.open_url`; `PathLike` (a `file://` link, decoded) → `cx.open_with_system`, joined to
  the line's cwd when it is relative. Foreman passes no path regexes, so a bare path in the output is never a
  `PathLike`.
- **Hover:** the Ctrl-gated hover state (`last_hovered_word`, set only while Ctrl is held) is unchanged. Nothing
  draws it: `term_element.rs:5` drops hyperlink hover, and OSC 8 cells are always underlined (`:981`), as in WT.
  Ctrl-gating that underline would mean editing `foreman-ui`, which is outside this unit's files. Not done.
- **Hit-test pitfall:** the removed path found the cell with `content_index_for_mouse`, which rounds. y = 10 px on
  a 20 px line is row 1 there but row 0 for `grid_point`. A click test at y = 10 misses the link on the old code
  and passes green for the wrong reason. The tests click at y = 5. `content_index_for_mouse` is `#[cfg(test)]` now
  (only its own tests use it).
- **Tests:** `terminal.rs` (now `links.rs`) `tests::hyperlinks::{test_plain_click_opens_no_link, test_ctrl_click_opens_osc8_link_of_any_scheme,
  test_ctrl_click_opens_plain_url}` subscribe to `Event::Open` and read the test platform's `opened_url`; nothing
  real is opened. `view.rs` `an_open_event_opens_any_scheme_and_resolves_relative_paths` drives `open_link` with
  fake openers.
- **Red:** with the old `mouse_up` branch restored, `test_plain_click_opens_no_link` FAILS: "a plain click reached the
  platform opener, left: Some(\"vscode://file/c:/x.exe\")".
- **Pitfall:** `Copy-Item` keeps the source's mtime, so restoring a backup older than the last build leaves cargo
  running the stale binary. Touch the file after restoring it.
- **Gate** (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-2\target`): build ok, no warnings; `cargo test --release --workspace`
  all green (no F-05 flake this time); `--selftest` 5 of 5 PASS; `--selftest-supervise` PASS;
  `--selftest-detach-shutdown` PASS; 0 `fm-selftest` sessions.

## 2026-10-29 — m5/44.3 OAuth and WorkState client robustness; Credential Manager limits (U2)

- **c5, secrets of any length** (`secret.rs`): `CredWriteW` takes at most 2560 bytes (`PART_BYTES`,
  CRED_MAX_CREDENTIAL_BLOB_SIZE). A longer secret goes to `<target>`, `<target>#1`, …, and the first
  credential has the comment `parts=<n>`. A secret of 2560 bytes or less is one credential with no comment,
  exactly as before. `set` writes the parts first and the first credential last, then deletes leftover
  parts; `delete` removes every part. WorkState 1.0.0's refresh tokens were longer than 2560 bytes
  ("Credential Manager: The stub received bad data."), and 1.0.1 returns to short ones.
- **A refresh token the store refuses** stays in memory (`Inner::unsaved`). The server has already spent the
  old one, so dropping the new one would end the grant. `AuthStatus::SignedIn { unsaved: Some(why) }` reports
  it; the settings window shows it as a warning and `fm work` prints `warning: …`. Another process cannot
  see that token: its refresh is refused, and that drops the pair.
- **F-08 / A-27**: `access_token` takes `inner` only to read or set `held`, `broken` and `unsaved`; it never
  holds it across `refresh.lock` or the token request. After it gets the file lock it checks the held token
  and `broken` again, so two threads still produce one refresh and one rotation. Red: `inner` held across
  `token_request` → `status_answers_while_a_refresh_is_in_flight` fails (`status() waited on the refresh:
  Err(Timeout)`).
- **F-09 / A-28**: `load_known` falls back to the default only on NotFound. Any other read or parse error is
  `AuthError::Store` ("… is damaged (…); repair or delete it, then sign in again"), `status` returns
  `SignInAgain` with it, and sign-in, refresh and sign-out never save over the file. Red: the old `.ok()`
  chain → `a_damaged_servers_json_is_reported_and_never_written_over` sees `NotSignedIn`.
- **A-61 fake**: ids are `fake-<kind>-<16 random hex>` (client, code, rt, at, jti). Refusals carry
  `error_description` (`redeemed`, `unknown`, `revoked`, `refused on request`, …) and are listed by
  `FakeAuth::refusals()`. `stall_tokens`/`stalled` hold `/connect/token` open; `set_refresh_len` pads refresh
  tokens. `auth.rs` still reads only `error`.
- **A-45**: an SSE line or event over 64 KiB (`MAX_LINE`) ends the stream as `Lost`. **Skipped: the
  `X-WorkState-Api` check on the stream.** The real server sets that header only under `/api/v1`
  (Skills `mcp/WorkState.Api/App/V1/ApiV1.cs:35-37`), although doc 10 rule 4 says every response carries
  it. A check would cut Foreman off the real `/api/stream`. That mismatch is the server's to fix.
- **A-48 `health.rs`**: `connection refused` is `ureq::Error::Io` with `ErrorKind::ConnectionRefused` (ureq
  3.5's `try_connect` also synthesises it); `timed out` is `Error::Timeout` or `ErrorKind::TimedOut`.
  Nothing is matched on text or on Winsock numbers any more.
- **F-05 / A-77, the evidence.** `fm/tests/work.rs` now names each refresh token by its place in the fake's
  issue order (`rt0`, `rt1`, …; never the token) and reports, for each `fm`, what the store held before and
  after, what `fm` sent, and the fake's refusals.
  - 20 runs of the single test passed 20. 20 runs of the whole binary (its 4 tests in parallel) failed 2:
    - "store before rt2, after none; this fm sent [rt1]; all sent [rt0, rt1, rt1]; refusals [invalid_grant:
      redeemed]". The test process read rt2, then the next `fm` read rt1, which was already redeemed.
    - `work_overview…`: right after `fm` stored the rotated token, the test read back an older one.
  - A probe of 4 processes × 4 threads, each writing and reading back its own random-prefix target, showed
    that Credential Manager **loses writes made concurrently from several processes**:
    "wrote v-2-82, read v-2-4, still v-2-79 after 300 ms"; "wrote v-0-90, read None, still None after
    300 ms". A re-read never healed one. HEAD's `secret.rs` (no part deletes) does the same. 4 threads in one
    process: 0 of 1200.
  - So this is a **lost update across processes, not a stale read**. Decision 3's one re-read would read
    the same stale value, so it is **not applied**, and token handling is unchanged.
  - Narrowed in the test: a signed-in `Home` holds `CREDENTIAL_WRITERS`, so only one home and its `fm`
    children write at a time. After that, 20 of 20 whole-binary runs passed.
  - **Open (production):** any processes writing generic credentials at once, such as the app and `fm`, two
    gates, or another program, can revert Foreman's rotated refresh token. The next refresh then fails with
    `invalid_grant` and needs "Sign in again". Proposed: keep the refresh token in a DPAPI-encrypted file
    under `<home>/oauth`, written under `refresh.lock`, which no other process writes. That changes the
    credential targets, so it is the controller's and René's call.
- Tests: `secret::tests::a_secret_over_the_blob_limit_is_split_and_a_short_one_stays_one_credential`;
  `tests/oauth.rs` `a_6_kb_refresh_token_rotates_through_credential_manager`,
  `a_refresh_token_the_store_refuses_is_kept_in_memory_and_reported`,
  `status_answers_while_a_refresh_is_in_flight`, `concurrent_callers_share_one_refresh`,
  `a_damaged_servers_json_is_reported_and_never_written_over`,
  `the_fake_names_why_it_refuses_a_refresh_and_its_ids_are_not_guessable`;
  `stream::tests::an_endless_line_or_event_is_refused`; `health::tests::a_server_that_never_answers_is_timed_out`.
  Red for c5: HEAD's `secret.rs` → the 6 KB test finds nothing stored.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-3\target`, a02ee77): build ok, no warnings; `cargo test --release
  --workspace --no-fail-fast` 604 passed, 0 failed. `--selftest` 6 runs: 1–4 PASS (123–129 s), run 5 FAIL
  at step 7r ("claude's image check in a Foreman tab exited 1 with the test image borrowed and put back on
  the clipboard"; the clipboard and claude paste path, untouched here, and not a known flake), run 6 PASS
  (124 s). `--selftest-supervise` PASS (299 s), `--selftest-detach-shutdown` PASS (13 s). 0 `fm-selftest`
  rows from this worktree; 4 rows of another checkout's run, left alone.

## 2026-10-29 — m5/44.4 shared foundations in foreman-core (audit U3)

- **New in `foreman-core`** (listed in `CLAUDE.md`): `fs::{write_atomic, load_json_or_notice, append_capped,
  rotated}`, `paths::{user_home, claude_dir, normalize, path_key, linked_git_dir, main_checkout}`,
  `time::{now_ms, epoch_ms, modified_ms, hex6}`, `url::origin`; `Settings::{update, load_with_notices}`;
  `Registry::load_all`. In other crates: `foreman_workstate::{pct_encode, server_reason}` (`http.rs`) and
  `foreman_claude::mcp_config_path`.
- **`write_atomic`'s temp name is `<file>.<pid>-<seq>.tmp`**, unique per process *and* per call. A per-process
  name, as `desired.rs` had, still lets two threads of the app clobber each other. Red check: with a fixed
  `<file>.tmp`, `concurrent_writers_never_clobber_each_others_temp_file` fails 3 of 3 (8 threads; every
  writer's `write_atomic` errors on the shared temp file). It passed 20 of 20 with
  the real name. The name ends in `.tmp`, so the relay's `*.json` watcher never takes a half-written drop.
- **One origin rule** (`url::origin`): trim, then cut at the first `/`, `?` or `#` after `://`; spelling
  kept, no lowercasing, so the keys already in `servers.json` and the minted records stay the same.
  `every_origin_the_tests_use_is_unchanged` checks every origin in the repo's tests against the `/`-only
  cut `api_base` used. Only a URL with a `?`/`#` before any `/`, or with surrounding whitespace, now gives
  a different origin; none is stored.
- **`CLAUDE_CONFIG_DIR`** now moves the peer registry (`sessions_dir`) with the transcripts. Without the
  variable, the path is byte-identical (`claude_dir_without_the_variable_is_byte_identical_to_home_dot_claude`).
  `user_home` treats an empty `USERPROFILE` as unset and falls back to `HOME`; before, an empty
  `USERPROFILE` meant no home at all. `fm/tests/relay.rs` removes `CLAUDE_CONFIG_DIR`, so a set variable
  cannot point it at the real registry.
- **Settings keys are lenient one by one.** A key with a value of the wrong type keeps its default, and
  `load_with_notices` names it ("settings.toml: max_working is not valid (…); its default is used"). Only
  a file that is not TOML at all is still an error. Before, one bad key failed the whole file: the
  supervisor ran on all-default settings and the app's saves were skipped. Now the other keys hold, and
  a save drops the bad value. The app shows no notice yet (U6a/U6b).
- **Behaviour changes, all improvements:**
  - an event log or `foreman.log` that cannot rotate (the file held open) still gets the line;
  - `same_root` also folds `.`/`..` and `\?\`;
  - `human.rs`'s pacing ledger cleans up its temp file;
  - the relay's `last-drop` stamp is written atomically;
  - revoke deletes a session's config through `mcp_config_path`, so its name check applies;
  - a failed save names the target file, not its temp file.
- **Left for later, with the reason:**
  - `auth.rs`'s `split_origin` and `pct`, the remaining A-04/A-05 sites: m5/44.18 is rewriting that file.
  - A-05's status-and-body read: `client.rs` checks the API header and maps a failed body read to
    `Transport`, while mint ignores a failed body read. One helper would change one of them.
  - Mint's test HTTP server is not moved onto `FakeWorkState` (A-05): `fake::Seen` records no
    `Content-Type`, which the mint tests check, and `fake.rs` is outside this unit.
  - `fm/src/main.rs` and `repos.rs` keep `Registry::list`: a CLI that stops on a corrupt file says which one.
  - `fm/tests/work.rs`'s test prefixes (A-09) are m5/44.18's file.
  - `foreman-app` (A-02 app files, A-03 callers, A-08 `tab_card.rs`, A-09 `selftest.rs` `hex6`, A-46's
    notice) is U6a/U6b's.
  - `mint::git_repo` still runs git: the main-checkout root sent to WorkState is what git prints (Mint).
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-4\target`, 19fb774): build ok, no warnings; `cargo test --release
  --workspace` 628 passed, 0 failed. `--selftest` 5 of 5 PASS (128–135 s). `--selftest-supervise` PASS
  (302 s), `--selftest-detach-shutdown` PASS (15 s). No `fm-selftest` rows left.

## 2026-10-29 — m5/44.18 the refresh token in a DPAPI-encrypted file; Credential Manager dropped

- **Why Credential Manager is gone:** it loses writes that several processes make at the same time
  (m5/44.3, F-05). Another program writing generic credentials could revert Foreman's rotated refresh token,
  so the next refresh got `invalid_grant` and "Sign in again". René decided on 2026-10-29 to move the token
  into a file that only Foreman writes.
- **Layout** (`secret.rs` `FileStore`): `<home>/oauth/secrets/<sha256(issuer) hex>.bin`, one file per key.
  Each file holds only the DPAPI blob: `CryptProtectData` in current-user scope (no
  `CRYPTPROTECT_LOCAL_MACHINE`), `CRYPTPROTECT_UI_FORBIDDEN`, and the entropy `Foreman/WorkState refresh
  token`. Another user, or a program of the same user that lacks the entropy, cannot unprotect the file.
  A file that will not decrypt is an error ("cannot decrypt …"), so `status` is `SignInAgain`. Nothing
  else is written: no plaintext, no log line, and errors never carry the token.
- **Writes:** the sealed bytes go to `<hash>.<random>.tmp` in the same directory and are synced, then
  renamed over the file. A rename, read or delete is retried up to 8 times on PermissionDenied or a sharing
  violation (a scanner or a concurrent reader). `set` and `delete` happen only under `refresh.lock`. That
  covers `access_token`'s rotation and its invalid_grant delete, sign-in's first store (`auth.rs`
  `sign_in`, the `refresh_lock()` block around `keep_refresh`) and sign-out. The atomic write stays in
  `secret.rs` until m5/44.4's `foreman_core::fs::write_atomic` lands; a later unit switches the call over.
- **Migration:** `get` with no file reads Credential Manager under `Foreman/WorkState:<issuer>`
  (`LEGACY_PREFIX`; `CredentialStore::get` joins m5/44.3's split parts). It writes what it finds with
  `MoveFileExW` **without** `MOVEFILE_REPLACE_EXISTING`, then deletes the credential and all its parts.
  `status` reads the store without `refresh.lock`, so the migration may run without it. Because the
  no-replace rename never overwrites a file, a migration that loses a race cannot overwrite a token
  that `fm` rotated meanwhile; it reads the winner's file instead. `delete` also deletes a leftover
  credential, so sign-out cannot bring a token back. A signed-in user stays signed in across the upgrade
  with no new sign-in.
- **Test isolation by home:** `FOREMAN_CREDENTIAL_PREFIX` (`PREFIX_VAR`) is removed. `default_store(home)`
  is per home; the selftest and `fm` tests use scratch homes. Their fake issuers
  (`http://127.0.0.1:<port>/…`) never match a legacy target, so the migration finds nothing there. Unit
  tests use `MemoryStore` or `FileStore` with a fake legacy store. The split-parts migration test uses a
  `CredentialStore` under a random `fm-test-…` prefix of its own.
- **fm tests:** `CREDENTIAL_WRITERS` and the serialized `Home` are gone. Red check: master 1c09179 with only
  the lock disabled failed 4 of 60 runs of the `work` binary ("fm stored the rotated refresh token",
  "and stored the rotated refresh token"), after 20 of 20 passed in a first batch. This unit passed 60 of
  60 runs of the same binary, plus 20 of 20 runs of `cargo test -p fm`.
- **For U11 (SecretStore into foreman-core):** the audit's "keep `PREFIX_VAR`" no longer applies. Move
  `FileStore` and keep `CredentialStore` only as the legacy reader for the migration.
- **Risk:** an older Foreman or `fm` still running during the upgrade keeps writing Credential Manager.
  A rotation it makes after the migration is not seen by the new build, so the next refresh is refused
  and the user must sign in again. Restart every Foreman process at the upgrade.
- Tests: `secret::tests::a_file_store_keeps_one_encrypted_file_per_key_under_the_home`,
  `a_file_another_program_sealed_is_refused`, `a_legacy_credential_moves_into_the_file_once_and_is_deleted`,
  `a_migration_never_replaces_a_file_already_written`,
  `a_split_legacy_credential_migrates_and_every_part_is_deleted`; `tests/oauth.rs`
  `a_6_kb_refresh_token_rotates_through_the_file_store`; `fm/tests/work.rs` unserialized.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-18\target`): build ok, no warnings; `cargo test --release
  --workspace --no-fail-fast` 613 passed, 0 failed; `--selftest` 5 of 5 PASS (130–147 s);
  `--selftest-supervise` PASS (321 s); `--selftest-detach-shutdown` PASS (14 s); 0 `fm-selftest` rows from
  this worktree (6 listed belong to m5-44-4's run, left alone).

## 2026-10-29 — m5/44.6 supervisor correctness (audit U5)

- **A-36, `DesiredState::update`:** read-edit-save runs under `<home>/supervisor/<workspace>.lock`
  (`foreman_core::fs::lock_file`, the `refresh.lock` pattern, now shared; `auth.rs` uses it too). The
  check-then-write retry loop is gone, and `edit` runs exactly once (`FnOnce`). Red check:
  `concurrent_updates_lose_no_edit` (6 threads × 15 wants) kept 19 of 90 on master and keeps 90 of 90 now.
  Plain `save` (selftests, `migrate_home`) does not take the lock. The UI thread still calls `update` for
  Start/Stop: the lock is held for one small read and write, and moving it off the UI thread is in
  `view.rs` (U6a's and U4's files).
- **A-33, stops:** every `claude stop` result is kept per name. After `STOP_FAILURES_SHOWN` (3) failures
  in a row, the name gets a `NeedsKind::StopFailed` Inbox row ("stop failed", Attach). A success, or a
  tick with no stop due, clears it. It is still stop and never rm, retried every tick.
- **A-34, Re-mint:** a name stays in `remint_due` until its stop succeeds. It is dropped when there is no
  live row (the resume mints anyway) or the name is no longer wanted. With two or more live rows it waits
  (R6). `retry` adds no duplicate entry.
- **A-35, resume output:** `resumed()` returns `InPlace`, `Copy(id8)` (an 8-hex id that differs) or
  `Unknown`. `Unknown` (empty, or new CLI text) is a notice. `confirm_resumes` settles it against the
  next snapshots: its own id live confirms it; another live row of the name is a copy and a counted
  failure; if neither shows up, it waits up to `settle`. `record_result` on a failure clears
  `started_at`, so a copy is not counted a second time as "exited within".
- **A-48:** `foreman_core::FM_PREFIX` replaces the `fm-` literals in `session.rs`, `budget.rs`,
  `desired.rs` and `migrate.rs` (the app's own literals are left to U6). The budget recognises a relay by
  `is_relay(name, workspaces)`, the exact `fm-<workspace>-relay` of a known workspace, instead of the
  `-relay` suffix. `Budget::new` and `working` take the workspaces. Red case:
  `a_repo_session_whose_slug_ends_in_relay_takes_a_slot` (`fm-a-b-relay` is repo `b-relay` of `a`).
  This took no slot under the suffix rule.
- **A-52 (note for ARCHITECTURE, m5/44.17):** `Supervisor::tick` runs on the poll thread. Each launch can
  block for up to `WRITE_TIMEOUT` (60 s), plus a `claude agents` call before every `--bg`. During that
  time the session list and event polling stall, and later launches act on an old snapshot (R6 and R13
  cover duplicates). Not changed. A later unit can move the launches off the poll thread.
- **A-58:** `forget.rs` gets `find_transcript_looks_in_the_cwds_project_dir_first_and_reads_only_files` and
  `a_sibling_dir_that_cannot_go_fails_after_the_transcript_went_and_a_second_call_finishes`. The second
  holds a file in the sibling dir with `share_mode(0)`. The transcript is already deleted when the
  error is returned, and a second call removes the rest.
- **A-60:** tests for `starting()` on a stopped workspace and around a hold, a trust refusal on resume,
  and a row whose cwd is gone (a resume again after the backoff, never a launch beside it).
  `sessions_dir` delegates to `paths::claude_dir`, whose `CLAUDE_CONFIG_DIR` handling foreman-core
  already tests. The `human.rs` two-writer ledger is not in U5's files and is left open.
- **New shared component:** `foreman_core::fs::lock_file` (listed in CLAUDE.md).
- **Kind added:** `NeedsKind::StopFailed`. `view/left.rs` gets its colour (error) and label.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-6\target`): build ok, no warnings; `cargo test --release
  --workspace --no-fail-fast` 645 passed, 0 failed; `--selftest` 8 runs: runs 2–6 PASS five in a row, run 7
  FAIL at step 7c "window not found" (F-06b), run 1 FAIL at 98 s, step not captured, run 8 PASS;
  `--selftest-supervise` PASS (327 s); `--selftest-detach-shutdown` PASS (16 s); 0 `fm-selftest` rows from
  this worktree (4 listed belong to m5-44-5, left alone).

## 2026-10-29 — m5/44.5 one process helper in foreman-core (audit U4)

- **`foreman_core::process`:** `command(program)` (CREATE_NO_WINDOW), `run(cmd, timeout, Kill)` (stdin
  null, both pipes drained on threads, exit code + stdout + stderr, `RunError::{Spawn, Timeout}`),
  `git(dir, args)` (30 s), `shell_execute(verb, target, args, cwd)` (ShellExecuteW, `Err(code)`),
  `started(pid)` / `alive(pid)`. Every spawn outside the PTY and the test modules goes through it: the
  claude runner (`Claude::exec`), the free `list_sessions`/`cli_version`, `daemon_status`, `icacls`, mint's
  `git`, the fm probes, the selftest launch/stop/rm, `git init`, `fm` and the screenshot script, the
  sign-in browser, the elevation hand-off's `runas`, and Open folder (ShellExecute instead of a bare
  `explorer.exe`). `foreman_claude::no_window`, `view::process_alive` and peer's private
  `process_start` are gone; `peer::started` re-exports core's.
- **Tree kill, but not for claude:** `Kill::Tree` puts the child in a job object (no kill-on-close: a
  normal exit leaves descendants running) and `TerminateJobObject`s it on timeout. The claude background
  service is a descendant of the `claude --bg` that started it (measured: the daemon's parent pid is a
  gone `claude --bg`), so a tree kill of a timed-out `--bg` would kill the service and every session in
  it. Every `claude` call therefore uses `Kill::Child`, as before.
- **`pwsh` here is a Store app-execution alias** (`%LOCALAPPDATA%\Microsoft\WindowsApps\pwsh.exe`): the
  real pwsh is not a descendant of the caller, so no job reaches it. The tree-kill test uses System32
  `powershell`. A timed-out `pwsh` child of Foreman can leave the real one running.
- **One CLI failure variant:** `ClaudeError::Failed` is gone; `list_sessions()` and `cli_version()` run
  through `Claude` and fail as `CliFailed` ("claude failed (exit N): …", with trust and daemon detection).
  Timeouts unchanged: 10 s for the free `list_sessions`/`--version`, 60 s for `Claude`'s calls, 10 s for
  `daemon status`. The argv of every claude call is unchanged; the selftest launch keeps its own argv
  (no `--`) through `Claude::run_in`.
- **Behaviour changes:** `daemon status` on timeout kills its child instead of leaking a thread and a
  process every 15 s (A-31). The selftest's stop/rm get the 60 s timeout, so a hung CLI no longer holds
  the SCRATCH lock (A-32); rm refusals are read by `RmRefusal::parse`, not a third parser (A-10). The
  probes and the selftest's `git`, `fm` and screenshot spawns flash no console. `fm probe launch`, the
  static-token route (`with_session_token`, `SessionIdentity::headers`, `write_session_mcp_config`) and
  the 127.0.0.1:5290 URL are deleted (A-54/A-55). `SessionIdentity` itself stays: the executor builds it.
- **Left alone:** the PTY shells and spawns inside test modules. `executor.rs`'s doc link now names
  `with_minted_token` (fixed after m5/44.6 merged).
- Tests: `process::tests::{output_and_exit_code_come_back, a_missing_program_is_a_spawn_error_with_its_kind,
  a_timeout_kills_the_tree_or_only_the_child, git_runs_in_the_given_dir,
  this_process_is_alive_and_a_spent_pid_is_not}`; launch.rs
  `minted_token_lands_in_an_owner_only_file_per_session_not_in_argv` replaces the session-token test. Red
  check: with the job object disabled, `a_timeout_kills_the_tree_or_only_the_child` fails (1 of 5).
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-5\target`): build ok, no warnings; `cargo test --release
  --workspace` 634 passed, 0 failed; `--selftest` 5 of 5 PASS; `--selftest-supervise` PASS (329 s);
  `--selftest-detach-shutdown` PASS (17 s); 0 `fm-selftest` rows from this worktree. After merging master
  a6b50cd (m5/44.6): `cargo test --release --workspace` 646 passed, 0 failed.

## 2026-10-29 — m5/44.8 Work tab, settings window, persistence (audit U6b)

- **Persistence (A-02 app files):** `stars.rs`, `dismissed_messages.rs` and the WorkState inbox's
  `workstate-dismissed.json` load through `fs::load_json_or_notice` and save through `fs::write_atomic`.
  The notice texts of stars and messages are unchanged. The inbox's dismissals used to be read with
  `.ok()` and written non-atomically with `let _ =`; now an unreadable file and a failed save land in
  `WorkInbox::notice`. **Not shown yet:** the notice box is `view/notices.rs` + `view/left.rs` and the
  `FleetView` fields, m5/44.7's files during this unit; wiring `WorkInbox::notice` into `plain_notices`
  is a few lines once 44.7 has merged.
- **`write_atomic` flushes the temp file before the rename** and retries a rename that fails with a
  sharing violation (32) as well as PermissionDenied. Both come from `secret.rs`'s own write, which
  `FileStore::set` now replaces with `write_atomic`, so the refresh token keeps its durability. Every
  other caller (settings, registry, relay drops, mint records, desired state) gains it: a crash leaves the
  old file or the new one, never an empty one.
- **`secret.rs` keeps one private write: the migration's no-replace rename** (`MoveFileExW` without
  `MOVEFILE_REPLACE_EXISTING`). `write_atomic` always replaces, and the migration must never overwrite a
  token that `fm` rotated meanwhile (m5/44.18).
- **A-38:** `Stream::subscribe` returns an `async_channel::Receiver` (unbounded, `send_blocking` on the
  stream thread). The hub awaits `recv()`; no background executor thread sits in a 1 s `recv_timeout`
  loop any more. Dropping the hub drops the `Stream`, which ends the thread and so the loop.
- **A-27, the settings side:** `SettingsView::auth` is `Option<AuthStatus>`, read on the background
  executor after each settings reload (`read_auth`, newest read wins). Until it answers, the Status row
  shows neither Sign in nor Sign out. `work_view::load_client` already ran off the UI thread.
- **Settings:** `change` goes through `Settings::update`; `reload` uses `load_with_notices` and shows bad
  keys in the window's error line (44.4 left the app side open). The help text takes "every N s" from
  `RECHECK_EVERY` and the font bounds from core's constants (A-19). The Status row and the sign-in hint
  (`workstate_setup.rs`) said the grant is kept in Credential Manager; since m5/44.18 it is a file in the
  Foreman home, encrypted for the Windows account. The plugins text had runs of 14 spaces (A-51) and said
  rlarch belongs in `extra_plugin_dirs` (A-63); a user-installed plugin loads without it.
- **A-17 debounce:** one `work_view::after_debounce(cx, f)`; the overview, board, item, journal and inbox
  keep their own pending flags.
- **A-44:** undo keeps 100 steps (`UNDO_STEPS`); `clipped`'s cut-label map keeps 512 labels and drops the
  one painted longest ago (gpui has no frame id to clear it per frame, and a cached view repaints without
  running the canvas); the journal's `prepend` stops at `KEPT` (2000) keeping the newest, and `older` stays
  true. The journal still renders every event each frame (a list virtualisation is a later unit).
- **A-24:** `Auth::issuer_of(origin)` replaces `signin.rs`'s own parse of `servers.json`.
- **A-04/A-05 in `auth.rs`:** `split_origin` delegates to `url::origin` and `form_encode` to
  `pct_encode`. Byte-identical for every issuer the server can send; only an issuer with `?`/`#` before
  its first `/` or surrounding blanks would split differently, and RFC 8414 forbids both.
- **A-54:** `Stars::contains` deleted; `selftest_closed.rs` and the tests read `ids()`.
- Stale comments that named Credential Manager as the store (`settings.rs` `workstate_url`, `fm/src/work.rs`)
  deleted.
- Tests: `text_input::tests::undo_keeps_the_newest_steps_only`, `clipped::tests::labels_painted_longest_ago_go_first`,
  `journal::tests::older_pages_stop_at_the_cap_and_keep_the_newest`, `tests/oauth.rs`
  `issuer_of_names_the_signed_in_origins_issuer_only`, the inbox's round-trip test now checks the notice;
  `split_origin` is pinned by a table. Red check: with each cap disabled, the three cap tests fail (3 of 3).
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-8\target`, 6cdca8e): build ok, no warnings; `cargo test --release
  --workspace --no-fail-fast` 650 passed, 0 failed. `--selftest` 5 of 5 exit 0 / PASS (128–155 s).
  `--selftest-supervise` PASS (310 s), `--selftest-detach-shutdown` PASS (14 s). 0 `fm-selftest` rows from
  this worktree (1 row of m5-44-7's run, left alone).

## 2026-10-29 — m5/44.7 app UI-thread work and hands (audit U6a)

- **New shared components:** `foreman_ui::nudge::{nudge, Nudge, Nudged}`, a coalescing wake-up for a loop on a
  gpui executor (`wait(executor, timeout)` returns on a nudge or the timeout; `take()` without waiting).
  `foreman_core::process::wait_any_exit(pids, timeout)` blocks until one pid exits (WaitForMultipleObjects;
  a pid it cannot open is skipped). For CLAUDE.md's list (m5/44.17).
- **A-37, no timer polling:** the poll loop sleeps until Refresh (nudge), `REFRESH_EVERY`, or an interactive
  session's process exits (`wait_any_exit` on the background executor). The transcript scan and the WorkState
  recheck sleep until nudged or their interval; a changed workspace list nudges the scan. Each `Tab` has
  `watchers`, woken by its terminal's Wakeup/title/close events and by `remove_tab`: the shell-prompt wait
  (`type_in_shell_when`) reads the content once the output has been still `SHELL_SETTLED`, not every 50 ms; the
  hand tab checks its token file on an event (at most every `HAND_CHECK_GAP`, `ended` off the UI thread, no
  `try_lock`); the detach waits on the close event or the pid's exit. The left-panel hold sleeps to its
  deadline. Zed's marker handshake is not used.
- **A-29, quit:** Quit detaches the Attach tabs, then revokes every hand token through
  `hand::release_all_within(hands, QUIT_RELEASE_WITHIN = 5 s)` on the background executor, then closes. The
  window stays responsive meanwhile; a server that answers gets every revoke; what the deadline cuts off stays
  in `hands.json` for the next start. Revokes start only after every detach, so a failed detach (the window
  stays open) never revokes a live hand tab's token. `on_app_quit` (any other end) only tries within gpui's
  200 ms `SHUTDOWN_TIMEOUT`.
- **A-30:** a failed `hands.json` save is in `Launch::Minted{notice}` and in `release`'s notice.
- **Start/Stop off the UI thread:** `desired_writer` runs them in order (one channel, one write at a time) on
  the background executor; `DesiredState::update` waits on the lock file there. Selftest step q now waits up to
  5 s for the save (measured 23 ms).
- **A-27 app side:** `set_workstate_url` reads `AuthStatus` on the background executor; a save failing after
  that is an action error. `sign_in`, `sign_out`, `leave_servers` use `Auth::configured_origin`.
- **A-39:** the context menu opens without IO; the SendMessage name is looked up off the UI thread and joins
  the menu if it is still open (selftest `mark` waits for it already).
- **A-46:** the app lists through `Registry::load_all`; a corrupt workspace file is a notice ("A workspace is
  left out, the others run: …") and the others are supervised.
- **A-03:** every app settings write is `Settings::update` (left width, run-as-admin, server URL, font size,
  font family, max working). A non-TOML `settings.toml` is now an action error instead of a silent skip.
- **A-17:** `dismiss_popup`, `pwsh(args)`, `Tab::new`, `fleet::display_name`. Audit partly wrong: of the six
  "name, else short id" sites only `view.rs` ×2 and `manage.rs` are that; `delete::label` is "name (id)",
  `left.rs` shows "(unnamed)"; `item.rs` is U6b's.
- **A-50:** Ctrl+wheel steps once per notch (`ScrollDelta::Lines`) and by distance for a touchpad
  (`ScrollDelta::Pixels`, 48 px per step, carried between events: `term::zoom_steps`). Moving the handler into
  `term_element.rs` (foreman-ui, U7's) is not done.
- **Also:** `FM_PREFIX` for the app's `fm-` literals in `view.rs`, `left.rs`, `fleet.rs` (`menu.rs` is U10's);
  `new_shell_cwd` uses `paths::user_home`; hand config paths go through `mcp_config_path`; the inbox file's
  `WorkInbox::notice` is a dismissible notice (`Notice::Inbox`).
- **A-59:** tests `hand::tests::{quit_release_returns_by_its_deadline_and_still_revokes_a_server_that_answers,
  a_hands_json_that_cannot_be_saved_is_a_notice}`, `view::tests::{only_open_hand_tabs_holding_a_token_from_elsewhere_restart
  (stale_tabs), a_corrupt_workspace_file_is_skipped_and_named}`, `term::tests::ctrl_wheel_steps_once_per_notch_and_a_touchpad_by_its_distance`,
  `process::tests::a_wait_returns_when_one_pid_exits_and_times_out_on_live_ones`. Re-mint and `server_changed`
  with a failed stop are supervisor tests (m5/44.6's).
- **Red checks:** the old synchronous release → the quit test fails (`Some([])` instead of `None`, 9 s); `let _ =
  minted.save()` → "the failed save is reported" fails; `Registry::list` → the corrupt-file test fails; one step
  per event → the zoom test fails (left 10, right 2).
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-7\target`, merged with master 25fa8ab): build ok, no warnings;
  `cargo test --release --workspace` 656 passed, 0 failed; `--selftest` 5 of 5 PASS (121–134 s);
  `--selftest-supervise` PASS (302 s); `--selftest-detach-shutdown` PASS (14 s); 0 `fm-selftest` rows.

## 2026-10-29 — m5/44.15 fm and plugin cleanup (audit U13)

- **A-56:** `fm hook` reads only `FOREMAN_WORKSPACE` and the role as given (no `FOREMAN_PLATFORM`, no
  `platform` → `workspace`); `fm tell` signs only with `FOREMAN_SESSION` and is refused without it. Every
  supervised launch passes it (`LaunchConfig::session_settings`). A session from before m2x/2 or m5/4 that is
  resumed gets no role text and cannot `fm tell` until Foreman relaunches it.
- **A-54:** `Transcripts::projects`, `assign::repo_of` (its test helper calls `repo_of_with(…, main_repo)`) and
  the `API_HEADER` re-export are gone. Audit wrong on `Registry::update`: `view/manage.rs` `set_repo_enabled`
  calls it, and `load_all` does not replace a read-modify-write; it stays.
- **A-55 / decision 4:** `crates/foreman-app/diag/` (8 `.ps1` and `timeline.py`, which exists, contrary to the
  audit) and `crates/fm/probe/resume.ps1` are retired in 014d286; the LEARNINGS lines citing them say so.
  `crates/fm/probe/hook-timing.ps1` is cited nowhere and stays (not in the brief).
- **A-63 / A-64:** the plugin README lists `FOREMAN_SESSION`, says rlarch loads user-installed with no
  `extra_plugin_dirs`, and states the WorkState rule (https://workstate.dikaia.ai, Foreman's OAuth sign-in, a
  minted token per session, no static token or local stack, `fm work` on the same sign-in). The repo-controller
  launches units with `agent_launch` (`workstate: {role: "worker", url: "https://workstate.dikaia.ai"}`; the
  url always, its default is local), escalates an `agent_signin` refusal, and keeps `Start-NestedAgent.ps1` as the
  fallback when the tool is absent. Still not measured: whether `--strict-mcp-config` hides rlarch's `agents`
  server in a Foreman session (LEARNINGS 2026-10-26).
- **Red checks:** master's `hook.rs` → `the_workspace_role_text_names_foreman_workspace_and_nothing_else` fails
  (prints the role for `FOREMAN_ROLE=platform`); master's `relay.rs` →
  `tell_writes_the_outbox_as_the_session_and_is_refused_outside_one` fails (tell without `FOREMAN_SESSION` exits 0).
- **Concurrent selftests break 7r:** with m5/44.14's `--selftest` running at the same time, step 7r failed 2
  of 5 (both borrow the one clipboard); 5 of 5 PASS once alone.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-15\target`, on master 199241c): build ok; `cargo test --release
  --workspace` 656 passed, 0 failed; `--selftest` 5 of 5 PASS (alone); `--selftest-supervise` PASS;
  `--selftest-detach-shutdown` PASS; 0 `fm-selftest` rows.

## 2026-10-29 — m5/44.14 selftest hygiene and F-06 (audit U12)

- **Shared helpers (A-18):** `selftest::{until, until_with, until_app}` (poll every 25 ms, fail "what: not within
  <duration>") replace the 10 local copies and `selftest_manage::until_app`; `selftest_delete` and
  `selftest_first_run` keep a two-line adapter that adds their state to the error, `selftest_title` and
  `selftest_term` adapters keep their call shape. `selftest_supervise`'s `until` polls `claude agents` rows every
  500 ms, an async CLI read, and stays. `selftest_clipboard::{text, put_text}` replace the read/write lambdas;
  `read_own` reads through `text`.
- **Step 7l (A-78, F-06a):** the audit's cause holds: the old check compared a session row's absolute y in a
  tree that re-sorts every 2 s, and Inbox/notice rows above it come and go. It now compares `tree-area` from the
  frame before the right-click with the frame that painted the toast. Only the **bottom** is asserted: height is
  bottom − top, and a notice above legitimately moves the top. Red: the toast without `.absolute()` →
  `FAIL step 7l: … tree area top 298.7 → 298.7, bottom 820.0 → 788.0, height 521.3 → 489.3`.
- **Step 7c (A-79, F-06b):** both hypotheses fixed. The step waits for `tray::is_shown` and an `on_next_frame`
  callback on the settings window before each close, and records `selftest::log_offset()` before the close and
  scans only what follows (`teardown_errors_since`; a rotated log is scanned whole). `tray::close` waits, every
  10 ms up to 1 s, until the window is hidden and `PeekMessageW(hwnd, PM_NOREMOVE | PM_QS_POSTMESSAGE |
  PM_QS_INPUT | PM_QS_PAINT)` finds nothing queued, instead of a fixed 100 ms. Sent messages are excluded:
  peeking for them would dispatch them inside the task. m5/22's discipline is unchanged (hide outside the update,
  `on_window_should_close` → false).
- **FAIL lines name the step:** the gate runs each step through `selftest::step(name, …)`, which records the
  current step; the watchdog, panic hook, teardown-log and "quit before a verdict" lines print it. m5/44.6's
  run-1 failure at 98 s printed none of these, so it was one of them, most likely the quit or a panic.
- **Clipboard across selftests:** the clipboard is system-wide, so parallel `--selftest` runs broke each other's
  7r (m5/44.3 run 5, m5/44.15 2 of 5). Steps 6b, 6c, 7l and 7r now hold the named mutex
  `Local\Foreman-selftest-clipboard` (`selftest_clipboard::borrow`) from their save to their put-back, waiting up
  to 60 s and saying so. The mutex is taken by polling `WaitForSingleObject(…, 0)` on the UI thread, which owns it
  and releases it on drop; an abandoned one counts as taken. 7r also records `GetClipboardSequenceNumber` after
  its put-back and, on exit 1, says whether another process wrote the clipboard since.
- **Other shared machine state:** the keyboard modifier state (`GetKeyState`) is the other one; `post_keys`
  already waits it out (m5/30), and m5/46 run 1's 7k `5e1f` → `%E!F` is the only sign of it. Not covered here.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-14\target`, merged with master c27f4b7): build ok, no warnings;
  `cargo test --release --workspace` 658 passed, 0 failed; `--selftest` 10 of 10 PASS in a row (120–131 s; before the merge 4 of 4
  without the mutex, then 8 of 8 with it); `--selftest-supervise` PASS (281 s); `--selftest-detach-shutdown` PASS (13 s);
  0 `fm-selftest` rows.

## 2026-10-29 — m5/44.9 terminal performance and clipboard (audit U7), TERM_PROGRAM=foreman (A-66 decision 2)

- **One paste path (A-13):** `Terminal::paste_clipboard(cx) -> bool` pastes the clipboard's text and returns false
  when it holds none (an image). Ctrl+V / Ctrl+Shift+V / Shift+Insert (`view/term.rs` `terminal_key`) return false
  then, so the key reaches the program as before (m5/46). Right-click without a selection (`term_element.rs`
  `register_mouse_listeners`) now does the same: an image-only clipboard sends the Ctrl+V keystroke through
  `try_keystroke` instead of doing nothing.
- **Selection text on demand (A-40):** `Content::selection_text` is gone; `make_content` no longer builds the string on
  every sync under the term lock. `Terminal::selection_text()` reads it when asked (selftests 6b/6c, the wheel-line
  helper, unit tests).
- **Process refresh throttled (A-41):** `PtyProcessInfo::emit_title_changed_if_changed` runs at most once per
  `REFRESH_INTERVAL` (1 s). A call inside the interval schedules one refresh at its end, so the last output burst is
  always followed by one. Input with `\r` (`write_input`) calls `refresh_soon`, and the next wakeup refreshes at once.
  BreadcrumbsChanged (the console title claude sets) is not throttled. 7l over 5 runs: registered 1.1–2.1 s, row
  marked 2.3–3.2 s.
- **Theme files parsed once (A-42):** `themes::parse_file` caches each `<home>\themes\*.json` by (modified ms, length).
  `catalog` and `resolve` still list the folder on each call, so a new or changed file shows up, but an Appearance
  change no longer re-reads and re-parses every file. The listing still runs on the UI thread. Moving it off means
  changing `settings_window.rs` (m5/44.11's file), so that is left for later.
- **Bounded caches (A-44, term/ui parts):** `term_element` `WIDTH_PER_PX` is cleared when it reaches 4096 entries,
  because a font change leaves the old FontIds' entries behind. `record_cwd_change` keeps one entry per
  scrollback position (`cwd_at_line` only ever finds the last one). Once the scrollback is full it clears the
  history, which `cwd_at_line` ignores from then on anyway.
- **Clipboard tests (A-62):** `test_paste_clipboard_brackets_or_turns_newlines_into_returns` (bracketed paste strips
  inner ESC; `a\r\nb\n` → `a\rb\r`), `test_paste_clipboard_leaves_an_image_to_the_caller`, `test_copy_trims_trailing_blanks`
  (the `Copy` event through `sync`; red without `trim_line_ends`: `Some("ab\ncd\t")`). The image-only case is tested on
  `paste_clipboard`; `terminal_key` needs a FleetView and only maps a false onto `return false`.
- **TERM_PROGRAM=foreman (A-66):** `insert_terminal_env` sets `TERM_PROGRAM=foreman`, `TERM=xterm-256color`,
  `COLORTERM=truecolor` and `TERM_PROGRAM_VERSION` (Foreman's version), with no `ZED_TERM`. What claude 2.1.312's bundle
  does with these (grepped read-only from `claude.exe`):
  - `ZED_TERM` turns on **strikethrough** (`dDn`: a terminal list ending `…||!!i.WT_SESSION||!!i.ZED_TERM||VTE≥4400`)
    and **synchronized output** (DEC 2026). For DEC 2026, `Se` is the env fallback ("likely") that the capability
    probe uses when it gets no answer. Foreman's alacritty answers no XTVERSION (`CSI >0q`), so claude never sends
    DECRQM 2026 ("DECRQM 2026 not asked (no XTVERSION reply)"), and `deadline()` settles the probe on that "likely".
    Without `ZED_TERM`, both would be off on Windows (no `WT_SESSION`). Foreman's vte processor honours 2026
    (`StdSyncHandler`). The tab env therefore sets `CLAUDE_CODE_FORCE_SYNC_OUTPUT=1` and
    `CLAUDE_CODE_FORCE_STRIKETHROUGH=1`, claude's own overrides, which give the same result.
  - `"zed"` as TERM_PROGRAM: claude offers `/terminal-setup`, which for zed writes a Shift+Enter binding into
    `~/.config/zed/keymap.json`. That file is Zed's, and Foreman never reads it. With `foreman` the command is not
    offered. Shift+Enter in Foreman already sends `\n` (`mappings/keys.rs`).
  - Unchanged: colours (`COLORTERM=truecolor` → level 3), OSC 8 hyperlinks (neither `zed` nor `foreman` is in claude's
    list and there is no `WT_SESSION`: off both ways), Alt+V (`chat:imagePaste` is bound per platform, and only
    iTerm/Apple_Terminal change keys; 7r passed 5/5), the scroll/wheel and keyboard-dialect checks (vscode, mintty,
    ghostty, iTerm, JetBrains only). The terminal name in claude's telemetry and `/status` reads `foreman`.
  - Not verified: whether `claude --bg` forwards `CLAUDE_CODE_FORCE_STRIKETHROUGH` to its worker. Its forward list has
    `TERM_PROGRAM*`, `WT_SESSION` and `ZED_TERM`. Sync output in daemon mode comes from the attaching client's
    `syncOutput`, which reads the tab env.

## 2026-10-29 — m5/44.11 foreman-ui foundations: text_input, button, banner, hit, fonts (audit U9)

- **Moved into foreman-ui:** `text_input.rs` (git mv, with its docs and tests), `hit.rs` (`Hit`, `HitTargets`,
  `hit_target`, and the `fully_visible` test), and the new `button.rs`, `banner.rs` and `fonts.rs`. foreman-ui depends
  on no app crate: the button and banner builders take `&HitTargets`, a key and colours; the app's `Icon` and `Tooltip`
  stay in `view/icon.rs`, and the call sites pass `icon.svg(..)` and chain `.tooltip(..)`. foreman-ui gained
  `unicode-segmentation` and the `Wdk_System_SystemServices` feature, and foreman-app lost both.
- **Buttons (A-20):** `bordered` (the old `view::button`), `outline` (item back, journal toggle and "load older",
  Attach), `pill` (attach/resume/tab), `icon` (row ×/+, tab close), `toolbar` (the action bar), `ghost` (the base,
  for the star and the new-shell +), and `small`/`choice` (the settings window, which has no hit map). The left
  panel's `icon_button`/`glyph_button`/`star_button` are kept as one-line app bindings of Icon + Tooltip.
  `titlebar::icon_button` stays: it is the only caption-style button (no pressed fill) and has one caller.
  gpui's `hover()` debug-asserts when it is set twice, so each variant sets its own hover rather than overriding a base.
- **Banners (A-21):** `banner::tinted(color)` (status text on its `TINT` = 0.12), `plain` (text on
  `element_background`), `dismissable(body, ×)`, `tint`. Callers add margins and override the shape (item.rs's
  warning box: `px_2 py_1 rounded_sm` with a border); `text_xs`, `rounded_sm` and `px/py` overrides replace exactly the
  fields they set, so the result is pixel-identical. The confirm box keeps its own layout and uses `banner::tint`.
- **A-15:** the caret blink uses foreman-term's `CURSOR_BLINK_INTERVAL` and `blink_phase` (same 530 ms).
- **Fonts:** `fonts::UI` ("Segoe UI"), `fonts::MONO` ("Cascadia Mono") and `fonts::icons()` (Fluent vs MDL2) replace
  the literals in view.rs, left.rs, icon.rs, tab_card.rs, item.rs, settings_window.rs and titlebar.rs.
- **Hit keys:** every key is unchanged. `button::*` registers its key, so the journal toggle also records a plain
  `work-journal` hit beside its `work-journal=open|closed` one.
- **view/term.rs** (m5/44.9's file during this unit) still names `crate::text_input` and `super::hit_target`; `main.rs`
  `use foreman_ui::text_input;` and view.rs's `use foreman_ui::hit::{.., hit_target}` keep both paths valid.
- **Pixel check:** `FOREMAN_SHOTS` selftest runs on master 7b1f58e and on this branch, compared per pixel: the
  appearance/theme/font shots are identical; the WorkState settings shots differ only in the fake server's port; the
  notice band of `m5-37-notices-before` (rows 395–680) is identical; the other differences are live session rows,
  counts and timings.

## 2026-10-29 — m5/44.12 foreman-ui widgets: menu, confirm, toast, notices, tabs, hover card, split, matcher (audit U10)

- **Moved into foreman-ui** (no app dependency; each builder takes `&HitTargets`, keys, colours or icons, and
  callbacks of the form `Fn(&mut V, …, &mut Context<V>)`, wrapped in `cx.listener` inside the widget, so the owner
  stays any view `V`): `menu` (`ContextMenu<A>`, `MenuItem<A>`, `item`), `confirm` (`Confirm<A>` with
  `ask`/`report`), `toast` (`ToastSlot`), `notices` (`NoticeKind`, `render`, `dismiss_key`, `keep_dismissed`,
  `remove_one`), `tabs` (`tab_strip`, `TabHeader`, `cycled`, `sanitize`), `hover_card` (`HoverCard`, `tooltip`,
  `LAST_CARD`), `split` (`SplitPane`) and `matcher` (`find_all`, `prefix`, `highlighted`, `count`).
- **MenuAction split (A-22):** the menu carries `MenuAction` (what a row offers); a yes to the question carries
  `Confirmed` (`Remove`, `StopSession`, `Delete{discard}`, `DeleteClosed`). The `*Now` variants and the `Dismiss`
  sentinel are gone: a report is `Confirm::report(text)` (`action: None`, no Cancel). The m5/38 texts, buttons and
  the order of the discard question are unchanged.
- **App keeps:** the menu builders, `Notice` and its sources, `Tab`/`TabKind`/`Detach` and the detach (CLI), the
  card's fields (`tab_card::Card`, now `hover_card()` → `HoverCard`), the grouped search results, and `clipped`
  (the tab label is passed in as an element). The Work tab's 42 % board/item split is fixed, not a `SplitPane`.
- **SplitPane:** its drag payload carries the divider id, so two splits do not move each other; the max width comes
  from the split's own bounds (= the old viewport rule, since the split spans the window).
- **A-16:** `left.rs`'s test-only contrast copy (4.5/3 floors on One Dark) is gone; `themes.rs` asserts the real floors.
- **A-54:** `tab_card::branch_at` is gone; callers use `foreman_core::branch_at`, and its test moved to `scan.rs`.
- **A-17 (Esc):** Esc already goes through one `dismiss_popup` on master; nothing to do. `cycled` and `view/term.rs`'s
  `find_step` are not copies: find starts at the bottom and steps up, so they stay apart.
- **Pixel check:** `FOREMAN_SHOTS` on master 5ace88c and on this branch (39 shots each): 18 identical (menus repo and
  workspace, confirm-remove, add review, settings, fonts, work board/journal, search); every other difference is
  a random session id, fake-server port, start time or refresh timing (each one looked at).
- **Shared target dir trap:** building a `git archive` copy of master into the same `CARGO_TARGET_DIR` makes cargo
  see the worktree's edited crates as fresh (workspace members hash by relative path; fingerprints by mtime). Touch
  the sources afterwards, or build master by detaching the worktree. A master copy outside the worktree also fails
  the selftest's trust check, and a `foreman.exe` copied alone shows the "fm.exe is missing" notice (7f fails).

## 2026-10-29 — m5/44.10 terminal dead code and split (audit U8: A-53, A-57, A-80)

- **Reachability, not grep:** every `pub` item in foreman-term was narrowed to `pub(crate)`, then widened back only
  where foreman-app or foreman-ui failed to compile (`cargo check --workspace --all-targets --exclude foreman-term`).
  The lib's own dead-code warnings then listed what no product path reaches. The narrowed visibility stays, so a
  future dead item warns again. Methods named alike elsewhere (`.clear()`, `.select_all()`) fooled the grep count;
  the compiler did not.
- **Deleted (A-53, decision 1):** headless/no-PTY subprocess terminals, display-only output replacement and the
  output generation counter, ANSI text parsing, the `actions!` block, the init-command marker handshake, vi mode,
  foreground-command naming, `Terminal::title` and `title_override`, `focus_in/out`, `clone_builder` and its
  `CopyTemplate`, `try_modifiers_change`, `select_word_at_event_position`, `with_renderable_cells`, the
  interactive-with-completion mode, `pid_getter`, `Hyperlink::{new,id,uri}` and the owned hyperlink variant, the
  `Clear` event and `clear_saved_screen`, `Event::OutputReplaced`, plus the alacritty helpers only these used. 24 test functions go: 16 that ran on Windows (they tested deleted code) and 8
  `cfg(not(windows))`/`cfg(unix)` ones that never ran (A-57). foreman-term 138 → 114 test functions.
- **Kept on purpose:** `TerminalBuilder::new`'s signature (the call site is `view.rs`, which this unit may not
  edit), so `is_remote_terminal`, `activation_script` and the path-hyperlink regex parameters still work as before;
  `TerminalMode` is now a one-value type. The events nobody reads (`Bell`, `BlinkChanged`, `SelectionsChanged`,
  `NewNavigationTarget`) stay too: without them `view.rs`'s `_ => return` arm is unreachable and warns. Both are a
  small follow-up in `view.rs`.
- **Not wired, so deleted:** focus reporting (`CSI I`/`CSI O` under DECSET 1004) was never called by the app;
  Ctrl-hover link underlining on a bare modifier change likewise. Wiring either is a new feature, not a restore.
- **A-57:** `urlencoding` and `percent-encoding` are gone; `url` is the one URL crate. The `file://` decode in
  `process_hyperlink` was unreachable (hyperlinks.rs already turns every `file://` match into a path), and the
  OSC 8 percent-decode ran only for `PathStyle::Posix`, which no caller passes on Windows.
- **Display-only terminal is a test fixture:** `TerminalBuilder::new_display_only`, `write_output`/`write_raw_output`
  and the parse buffer are `#[cfg(test)]`; `TerminalType::DisplayOnly` stays as a variant (allowed dead outside tests).
- **Split (A-80):** `terminal.rs` 6,651 → 104 lines (struct, re-exports); `bounds`, `builder`, `content`, `cwd`,
  `events`, `input`, `links`, `mouse` (selection), `process`, `scroll`, `search`, `test_support`; colours in
  `mappings/colors.rs`. A line-multiset diff before/after shows only imports, visibility and rustfmt re-wraps.
  foreman-term src 11,119 → 9,087 lines (8,702 after the deletion, before per-file imports).
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-10\target`, merged with master 1049b90): build ok, no warnings;
  `cargo test --release --workspace` 651 passed, 0 failed; `--selftest` 5 of 5 PASS (125–141 s; 4 of 4 before the
  merge); `--selftest-supervise` PASS (311 s); `--selftest-detach-shutdown` PASS (13 s); 0 `fm-selftest` rows.

## 2026-10-29 — m5/44.13 namespaced settings, settings page registration, SecretStore in foreman-core (audit U11)

- **settings.toml tables:** `[sessions]` (max_working, extra_plugin_dirs, run_as_admin, relay_idle_secs),
  `[workstate]` (url), `[appearance]` (theme, terminal_colors), `[terminal]` (font_size, font_family, ligatures),
  `[window]` (left_width). Each is a `foreman_core::Section` with `KEY` and `FIELDS` (flat name → table name); the
  field methods moved with them (`settings.workstate.origin()`, `settings.terminal.font_size()`,
  `settings.sessions.relay_idle_ms()`).
- **Migration:** in `parse`, not `migrate_home`: a flat key moves into its table on every load, and the next save
  writes tables. No save at start, so a file stays flat until something changes. A key set both flat and in its
  table takes the table's value, with a notice. A test pins every shape the repo has written (defaults, test homes,
  LEARNINGS, `workstate_env`, the float font size), and a flat file with all 11 keys round-trips. To check it, put
  each table's keys back at their flat names and compare with the original. `FOREMAN_SETTINGS_SAMPLE=<file> cargo
  test -p foreman-core -- --ignored a_real_settings_file` runs the same check on a real file. René's copy passed:
  same 4 values, nothing else.
- **Kept, not dropped:** `Settings::other` holds every table and key Foreman does not own, as read: a plugin's
  `[gitman]`, a newer build's key, a key inside an owned table that this build does not know, and `workstate_env`.
  Before, `workstate_env` was dropped at the next save; now it is kept and unused. `section::<T>(key)` and
  `set_section` are the plugin API, and they assert on an owned key.
- **Leniency:** each owned key is checked alone (as m5/44.4 did for the flat file). A section that is not a table
  (`terminal = 5`) keeps all its defaults, with a notice, and the rest of the file stands. Notices name
  `section.key`.
- **Downgrade:** an older build reads a table file as all defaults (serde ignores the tables) and drops the tables
  at its next save. René's values match the defaults except `left_width`.
- **Pages:** `foreman_ui::settings_page::{SettingsPages, SettingsPage, row}`. `register` panics on a duplicate id,
  and `resolve` falls back to the first page for an unknown id. The nav shows a Segoe icon per page. The app's ids
  are `settings_window::page::{SESSIONS, WORKSTATE, PLUGINS, APPEARANCE, ABOUT}`; `open_settings_at`, `show` and
  `page()` take and return them.
- **Not done from the audit's c8 sketch:** the `SettingsStore` entity with `Changed(section)` events, per-page
  models instead of `WeakEntity<FleetView>`, and the auth model off the UI thread. The brief did not ask for them,
  and they belong with the follow-up that gives GitMan its own pages.
- **SecretStore:** `foreman_core::secret` (a `git mv`; the diff is `crate::fs`, visibility, one doc pointer).
  `CredentialStore` is private: it is only the legacy reader `default_store` hands `FileStore`. `foreman_workstate::secret`
  re-exports it, and foreman-workstate no longer depends on `windows`. A test pins the file name of a key, the
  DPAPI entropy and the legacy prefix.
- **Credential Manager tests flake under load** (as F-05). `credential_manager_round_trip` and `split` each failed
  once in the full workspace run (set, then get → None), never in 8 runs of the crate alone. The three tests that
  write Credential Manager now take one mutex. After that: 2 full runs green.
- **Step 7j flake:** "a poll, a scan … after Refresh: not within 1.5s" failed 2 of 15 `--selftest` runs. Passing
  runs measure 0.5–1.3 s; the wait is the `claude agents` poll, which no settings read is on.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-13\target`, merged with master 7a48baf): build ok;
  `cargo test --release --workspace` 662 passed, 0 failed (678 before the merge removed foreman-term tests);
  `--selftest` 5 of 6 PASS on the merged build (run 2 failed 7j), 5 of 5 consecutive before the merge after a 7j
  failure in run 3; `--selftest-supervise` PASS (297 s); `--selftest-detach-shutdown` PASS; 0 `fm-selftest` rows of
  this worktree.

## 2026-10-29 — m5/44.16a comment pass over foreman-term and foreman-ui (audit U14a: A-69, A-70)
- **Comments only.** Every changed file, with its `//` comments stripped (outside string literals) and blank lines
  dropped, is identical to master (`codeonly.py`-style check over 27 files: 0 code differences); `git diff -w master`
  shows comment lines and 61 trailing comments cut from code lines (`keys.rs`' caret-notation `//1`..`//127`).
- **Counts (full-line comments):** foreman-term 513 → 303 (plus 61 trailing), foreman-ui 332 → 313. The audit's
  estimate was 87 and 24; most of foreman-term's cut is vendored-Zed restatement and test scaffolding.
- **Kept on purpose:** the GPL provenance headers (`Vendored from Zed … @ 03efd49…`) cut to the two-line form; upstream
  issue links in tests; protocol and platform quirks (claude's keyboard modes, ConPTY default titles, OSC colour-reply
  order); `m5/26` floors and `LEARNINGS m5/22` as pointers; `m5/44 decision 5` on Ctrl+click links; the shared
  components' `///` docs, trimmed only where they named a unit (`text_input.rs`, `themes.rs` "before m5/31").
- **Removed:** unit tags (m3/3, m5/9, m5/15, m5/23, m5/24, m5/29, m5/36, m5/41, m5/47, tf/6), history ("This used to
  be … I couldn't find it", "as the old field did", "Kept: … Dropped: …", the terminal.rs "Cut for Foreman" note),
  the SSH/WSL remoting and macOS locale essays, commented-out `NumpadEnter`, a duplicate xterm modifier table, section
  banners and `///Scroll the terminal`-style restatements.
- **No code-looks-wrong findings.** One stale name: hyperlinks' test comment called `sanitize_url_parentheses`, a
  function that does not exist (deleted with the comment).
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-16a\target`): build ok; `cargo test --release --workspace` 651 passed,
  0 failed; `--selftest` first series 4 of 5, then 1 of 2 (runs 2 and 7 failed at 72 s, run 7 on step 7j's
  1.5 s Refresh bound while another gate's foreman ran, as in m5/45; run 2's log was not kept), second series 5 of 5 PASS
  (120–138 s); `--selftest-supervise` PASS (317 s); `--selftest-detach-shutdown` PASS (13 s); 0 `fm-selftest` rows.

## 2026-10-29 — m5/44.16b comment pass over foreman-claude, foreman-supervisor and foreman-core (audit U14b: A-65, A-71–A-73)
- **Comments only.** Every changed .rs file (27), with its `//` and `/* */` comments stripped outside string and char
  literals and blank lines dropped, is identical to master: 0 code differences.
- **Counts (full-line comments):** foreman-claude 331 → 303, foreman-supervisor 405 → 387, foreman-core 302 → 290.
  Below the audit's estimate (57, 67, 25): most of what it counted are LEARNINGS evidence pointers (`m0/2` measured,
  `m2/1` probe, `m2/4`), which stay as pointers, as in U14a; the crate docs' health and reconcile tables are spec.
- **Removed:** unit tags as narration (m2x/2, m5/7, m5/8, m5/10, m5/25, m5/26, m5/32, m5/33, m5/38 with its name and
  date, m5/40, m5/41, m5/43, m5/47, m5/48, mint/7, "M2's"), "a later tray daemon", "before m2x/2" / "before m5/7";
  the essays in `transcript.rs`, `human.rs`, `daemon.rs`, `forget.rs`, `relay.rs` ("Since m5/4 … Since m5/32") and
  `events.rs` cut to two or three lines; `budget.rs`' paraphrases of the crate docs' shared budget turned into
  pointers; lib.rs' Held section no longer restates `daemon.rs`; restatements (`Load, edit, save over it`, `Every
  process now running`, `A whole transcript, read in one go`, …).
- **A-65:** `mint.rs`' "static role token" (module docs, `Minter`, `MintedSession::role`) deleted: the bearer is the
  app's OAuth access token. `executor.rs`' `with_session_token` pointer was already gone. No code-looks-wrong findings.
- **Gate trap:** with `CARGO_TARGET_DIR` outside the repo, `fm`'s `tests/want.rs` fails ("No Foreman plugin for …
  fm.exe") until `plugins/foreman` sits in a folder above the target dir, as `C:\fmbuild\gate\plugins` does.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-16b\target`, `C:\fmbuild\m5-44-16b\plugins\foreman` copied from the repo):
  build ok, no warnings; `cargo test --release --workspace` 662 passed, 0 failed. Selftests not run (per brief).

## 2026-10-29 — m5/44.16d comment pass over foreman-app (audit U14d: A-65, A-67, A-68)
- **Comments only.** Every changed file (55), with its `//` and `/* */` comments stripped outside string and char
  literals and blank lines dropped, is identical to master: 0 code differences. No trailing comments were cut.
- **Counts (full-line comments):** foreman-app 1598 → 294. The UI files (`view.rs`, `view/*`) 431 → 44, the selftests
  about 640 → 140, `hand.rs` 102 → 13, the rest about 450 → 97. The audit's estimate was 330.
- **Kept on purpose:** platform and protocol quirks (WM_CHAR after an unhandled key-down, posted keys not in
  GetKeyState, ShowWindow inside a gpui update, ConPTY titles, PSReadLine's DECTCEM redraw, CF_DIB → PNG-only),
  claude CLI behaviour (`--resume` keeps the id only in full, the daemon's respawn after ~12 s, transcript shapes),
  timing bounds and their reasons, `SAFETY:` lines, C21 pointers, and `LEARNINGS mX/Y`, `F-04`, `F-06b`, `R13`
  as pointers to evidence. `hand.rs`'s 28-line module essay is two lines (the deliberate `--mcp-config` override, and
  the pointer to Skills `docs/workstate/11-sessions.md` on names).
- **Removed:** unit tags as narration (bare `(m5/4)`, `(m4/2b)`, `as before m5/20`, …), history ("written before
  m5/32", "placeholder; m5/14 replaces this"), step-label comments in the selftests, module essays, and docs that
  restate a name or signature.
- **Contradicting comments (deleted):** `view.rs`'s `TitleChanged` arm said claude names the console through it; the
  OSC title arrives as `BreadcrumbsChanged` (A-65), so that arm may never fire for claude. `work_view/inbox.rs`
  `kind_label` was documented as `blocked_finding` → "blocker finding", but the code gives "blocked finding".
- **Gate needs the plugin:** with `CARGO_TARGET_DIR` outside the repo, `fm`'s `want` tests find no `plugins/foreman`
  above `fm.exe` and fail; a copy at `C:\fmbuild\m5-44-16d\plugins\foreman` fixes it.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-16d\target`): `cargo build --release --workspace` no warnings;
  `cargo test --release --workspace` 662 passed, 0 failed. Selftests not run (brief: parallel comment passes share
  the clipboard and keyboard); the controller runs them on master.

## 2026-10-29 — m5/44.16c comment pass over foreman-workstate, fm and plugins/foreman (audit U14c: A-74, A-75, A-76)
- **Comments only.** Every changed .rs file, with its `//` and `/* */` comments stripped outside string literals and
  blank lines dropped, is identical to master (21 files, 0 code differences; the check flags a one-character code
  edit, red-checked on `hook.rs`). The plugin's markdown lost only unit tags (m2/1, m2/2, m3/2, m3/3, m3/4, m3/6);
  every string `crates/fm/tests/skills.rs` asserts is unchanged.
- **Counts (full-line comments):** foreman-workstate 241 → 144, fm 149 → 96. The audit's estimate was 37 and 33.
- **Kept on purpose:** pointers to Skills `docs/workstate/10-read-api.md` / `12-install-harness-identity.md` and to
  `docs/LEARNINGS.md` m2/2, m5/38, m5/40 (the probes' answers); C21 (read only); the no-token invariants; OAuth and
  SSE quirks (loopback port ignored, `retry:` ignored, `invalid_client`/`invalid_grant` drop the pair, the lock
  around a refresh); the shared components' `///` docs (`pct_encode`, `server_reason`).
- **Removed:** unit and finding tags (m5/19, m5/43, m3/4, m0/3, F-05, F-08, F-09, "c2:"), the 19-line `auth.rs`
  essay, the stream and probe module essays, `// ---- GET …` banners in `model.rs`, test-module summaries that list
  their tests, and doc comments that restate a signature or a usage string.
- **Left in markdown (not narration, so out of U14c's scope):** `workspace-controller/SKILL.md` says "call the Skill
  tool even when the listing does not show it" twice (A-76).
- **Code-looks-wrong (not touched):** `foreman-workstate/src/auth.rs` `save_known` writes `servers.json` through a
  fixed `json.tmp` and a hand-made rename, which CLAUDE.md reserves for `foreman_core::fs::write_atomic`.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-44-16c\target`): `cargo build --release --workspace` 0 warnings;
  `cargo test --release --workspace --no-fail-fast` 660 passed, 2 failed: `fm` `tests/want.rs`
  `want_adds_a_repo_session_once_and_unwant_retires_it` and `a_workspace_whose_root_is_a_repo_wants_its_session_in_the_root`.
  Master's own `crates`/`plugins` fail these two the same way in this target dir: an `fm.exe` under `C:\fmbuild` has no
  `plugins/foreman` next to it or above it ("No Foreman plugin for …"). Selftests not run (parallel passes, per brief).

## 2026-10-29 — m5/51 one border on the Server URL field, shared TextInput padding

- **One border:** Settings → WorkState → Server URL drew the field inside a second bordered box
  (`settings_window.rs`). The box is gone; `ws-url` keeps only its id and 280 px width, and the field's own border
  is the only one, like every other TextInput.
- **Padding lives in `foreman_ui::text_input`:** `PAD_X` 8 px beside, `PAD_Y` 3 px above and below, inside the 1 px
  border. A single-line field is 26 px high (was 22, with 4 px beside and 1 px above). `TEXT_LEFT` (border +
  `PAD_X`) is exported for hit targets that click into the text, and `TextInput::text_bounds` returns where the
  last frame painted the text.
- **Callers checked:** left-panel search (`view/results.rs`), Settings Server URL, the task box (`open_compose`,
  multi-line: now `PAD_Y`/`PAD_X` too), Find in terminal (`view/term.rs`; its 28 px bar holds the 26 px field), the
  add-workspace name (`view/manage.rs`, painted in `view/left.rs`). Only Server URL had a wrapper border. The
  add-name text hit target hard-coded the old 5 px inset; it uses `TEXT_LEFT` now (one line in `view/left.rs`).
- **Result line:** the search summary under the box gets `pt_2` (was `py_1`); its left edge stays at the result
  rows'.
- **Hit keys unchanged.**
- **Tests:** `text_input::tests::the_text_has_room_inside_the_border` (red: `PAD_X` back to 4 px → 1 failed).
  `--selftest` 7k checks the painted search text starts at least 17 px in and 8 px down in `search-box` (master
  paints 13 and 6, so it fails there). 7n opens the task box, types a line and writes the `m5-51-compose` shot.
- **Gate:** `cargo build --release --workspace` clean; `cargo test --release --workspace` 663 passed, 0 failed.
  Selftests run by the controller: in the agent environment 7c fails every run, also on master UI code and with no
  other selftest running ("window not found" on `close_settings`), while the controller's own run of 2b0fb17
  passes it.
- **Before/after (`FOREMAN_SHOTS`, `C:\fmbuild\m5-51\shots\{before,after}`):** taken with 7c's bail turned into a
  print in a local, uncommitted build; "before" is the same build with master's field geometry, Settings wrapper and
  summary padding. `m5-43-settings-signed-in`: two nested boxes around the URL → one. `m5-30-search-id`: "5e1f"
  12.7 px in and 6 px down in the search box → 16.7 and 8, and the summary line 4 px lower. `m5-51-compose`: the
  task text clear of the border. The 7k check printed 12.67/6 on master geometry (it needs 16/7: red) and passed
  after; that run PASSed every step.
- **7f in the controller's run of 44241a8:** "192.0 px under the Inbox (158.0 px besides the search box)". Here the
  same commit paints Inbox 66.0..262.7 and the tree from 302.7: 40 px, of which the search box is 34, so 6 px (master
  build: 36, 30 and 6). The tree shows this machine's live sessions, so what sits between them depends on the
  environment; 7f's failure message now lists every hit key painted in the gap with its height.
- **7f, the cause:** the gap held the setup notice `setup-0`, "No Foreman plugin for <exe>: … sessions cannot be
  started from this build." (`supervise::setup` → `foreman_core::find_plugin_dir`). An exe whose folder and every
  folder above it lack `plugins\foreman` shows it. `C:\fmbuild\m5-51` has one, so my runs never painted the notice;
  the controller's build dir has none. The `inbox-ws-*` keys in the gap are the Inbox's own second row, overflowing
  its 25 % `max_h` and clipped: their bounds lie below `inbox-area`, but they take no space. 7f now subtracts the
  Messages (`messages-area`), notices (new `notices-area` key on the notices section) and search box sections, the
  way it subtracted the search box alone. Anything else in the gap still fails. Reproduced with `plugins`
  renamed: tree at 454.7, `notices-area` 145 + `search-box` 34 of 192, 13 px left → 7f passes. Red: `notices-area`
  left out of the list → the controller's exact failure.

## 2026-10-29 — m5/52 the one-time migrations that have run are removed

René, 2026-10-29: "the one time migration code could be removed now". The controller saw René's 438c727c0bea
start and migrate: `settings.toml` is in tables and the refresh token is in `oauth\secrets\<hash>.bin`.

- **Inventory** (what each reads, since when, verdict):
  - `foreman_core::migrate_home` (m2x/2, 2026-10-23): `platforms/` → `workspaces/`, and `supervisor/<w>.toml`'s
    `platform` field, role and `fm-<w>-platform` session renamed. It ran at every app start and `fm` registry
    command, so René's home went through it six days ago. **Removed** with `DesiredState`'s
    `#[serde(alias = "platform")]` and its tests (core, supervisor, `fm/tests/workspace.rs`). A home that still has
    `platforms/` now shows no workspaces and is fixed by renaming the folder.
  - Flat `settings.toml` keys → tables (m5/44.13, `settings.rs` `take` and `Section::FIELDS`' flat names).
    **Removed.** `FIELDS` lists the table's key names only. A key outside every table, whether an old flat name
    or `workstate_env`, gets the notice "`<key>` is not in a table; it is ignored". It keeps no value but stays
    in `Settings::other`, so a save loses nothing and the human can move it by hand.
  - Credential Manager → `FileStore` (m5/44.18): `CredentialStore`, `LEGACY_PREFIX`, the move in
    `FileStore::get`, the no-replace `write` with `MoveFileExW`, and the split-parts code. **Removed** with their
    tests and the `FileStore::new(home, legacy)` parameter, so `FileStore::new(home)`. `foreman-core` no longer
    needs `getrandom`, `Win32_Security_Credentials` or `Win32_Storage_FileSystem`. A token still only in
    Credential Manager now means one new sign-in.
  - `fm hook`'s `FOREMAN_PLATFORM` / `FOREMAN_ROLE=platform` fallback (m2x/2): already gone. `fm/tests/hook.rs`
    pins that it is not read. **Nothing to do.**
  - `WorkStateSettings::origin` dropping a `…/mcp` path: **kept**. It is the one origin rule (`foreman_core::url`),
    not a migration. Only the comment that called it "an older file's" URL changed.
  - `MintedSession`'s `#[serde(default)]` fields (`claude_session_id`, `last_active`, `origin`), stars,
    dismissals, `hands.json`, `servers.json`: **kept**. They are optional fields of today's format, not a reader
    of an old one.
  - `foreman_workstate::model`'s legacy agent label: **kept**. WorkState, another program, writes it.
  - Claude's files (transcripts, `claude agents`, hook payloads): **kept**, written by another program.
  - `selftest_supervise.rs:125` "as a legacy desired state has it: the relay first": a selftest file belongs to
    m5/44.19, so not touched. It reads no old format: `set_relay` puts the relay first today too.
- **Tests on today's shapes, no real files:** `a_tables_file_as_written_today_loads_and_saves_unchanged`, a
  four-table file shaped like René's with made-up values, loads with no notice and saves back the same tables.
  `a_token_file_in_todays_layout_loads` builds `<home>/oauth/secrets/<sha256 hex>.bin` by hand with a DPAPI blob
  under today's entropy. Then `FileStore` and `default_store` both read it. Every other settings test now
  writes tables; `fm/tests/work.rs`'s scratch home wrote a flat `workstate_url` and now writes `[workstate] url`.
- **Red checks:** notice loop off → `a_key_outside_a_table_is_a_notice_and_the_rest_of_the_file_stands` fails;
  file name `{:X}` instead of `{:x}` → `a_token_file_in_todays_layout_loads` fails. Both restored.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-52\target`): release build ok; `cargo test --release --workspace` 652
  passed, 0 failed. `--selftest` ran once and passed steps 1 to 7b, then failed 7c "window not found" (F-93, as the
  brief expects). The controller runs the selftest gate.

## 2026-10-29 — m5/58 box drawing and block elements drawn cell-exact, like Windows Terminal

- **Why René's table looked broken.** Box drawing went through the font. In Cascadia Code at 14 px, `─` is narrower
  than its 8.20 px cell, so a row of them is dashed. `│` is shorter than the 18 px line (line height 1.3 × size,
  `LINE_HEIGHT_FACTOR`), so verticals stop short of the horizontals in the rows above and below.
- **What Windows Terminal does** (microsoft/terminal @ f356093, `.repos/terminal/src/renderer/atlas`): it never
  shapes U+2500–U+259F. `BackendD3D::_drawBuiltinGlyph` (`BackendD3D.cpp:1582`) gives `DrawBuiltinGlyph`
  (`BuiltinGlyphs.cpp:1824`) one whole-pixel cell (`cellSize`). Each glyph in the `BoxDrawing` table
  (`BuiltinGlyphs.cpp:139–964`) is up to 4 instructions: a shape plus begin and end `Pos` values. A `Pos` is a
  fraction of the cell plus a multiple of the light line width (`Pos_Lut`, `:97`).
  - Thickness: `lightLineWidth = max(1, round(cellW / 6))` (`:1845`), heavy is twice that (`:1869`). This is a
    sixth of the cell **width**, not the font's underline thickness or the cell height, so the brief's wording
    does not match WT. I followed WT's source. The corner radius is `min(5 × light, min(w, h) / 2)` (`:1846`).
  - Snapping (`:1886–1889`): a coordinate is rounded after subtracting half the stroke along the stroke's
    width, then the half is added back, so a stroke's edges land on whole pixels. A horizontal line's ends are
    not offset, so `─` runs from 0 to w exactly and the next cell starts where it ends.
  - Shapes: lines are D2D `DrawLine` with flat caps (`:1913`), double-line corners are hollow rects
    (`DrawRectangle`, `:1919`) clipped to the cell, arcs `╭╮╯╰` are rounded rects that extend past the cell
    (`Pos_Max` = 1.5) and are clipped (`:1925`, clip `:1826`), diagonals `╱╲╳` are lines, blocks are filled rects.
    `░▒▓` are 25/50/75 % stipples done in the shader.
- **Foreman** (`foreman_ui::builtin_glyphs`, painted by `term_element::BuiltinCell`): `GLYPHS` is WT's table
  converted 1:1 by a script, covering all 160 entries of U+2500–U+259F. `marks` repeats `DrawBuiltinGlyph`'s
  arithmetic in device pixels. Our cells are fractional (8.203125 px), so each cell's device rect is its own
  logical edges × scale, rounded. Neighbours share those edges, and a column keeps one rect on every row.
  The light width comes from the nominal cell width (`cell_width × scale`), so it is the same in every cell.
  Lines and hollow rects become quads; diagonals and arcs become gpui stroked paths under a content mask of the
  cell. A glyph's quads are made disjoint (`disjoint`), so a dimmed (α 0.7) or selected colour is even where
  strokes cross. Two differences from WT:
  - Shades are the foreground colour at 25/50/75 % opacity, not a stipple, as before.
  - Zed's 8×24 subcell block-element painter is gone. Its quads were not snapped, and blocks are now in the same
    table.
  - Characters outside U+2500–U+259F keep the font path, with m5/47 ligatures and m5/49 fitting unchanged.
    Every mark is inside its cell, so m5/47's rule that ink never covers another cell holds.
- **Selftest 7t** (`selftest_box.rs`) prints `┌───┬───┐ │   │   │ ├───┼───┤ └───┴───┘` in rgb(255, 0, 255) at
  14 and 23 px. It captures the window (`selftest_manage::capture`, the PrintWindow of `scripts/shot.ps1`, now
  shared with `shot_window`) and maps the table's cells into the PNG. The window frame is taken as equally
  wide below and on both sides. The step finds each horizontal stroke's pixel row (rows 0, 2, 3) and each
  vertical stroke's pixel column (cells 0, 4, 8), then requires unbroken ink along each one between the outer
  strokes. At scale 1.5 it passed: 12×27 px cells, horizontals at y 221/275/302 unbroken over x 720..=819, and
  20×45 px cells.
- Red: 7t with box drawing routed to the font again (`if false && builtin_glyphs::is_builtin(…)` in
  `layout_grid`) → FAIL "Cascadia Code at 14px: cell 0: the vertical stroke at x 720 does not reach the
  horizontal ones, a gap at y 233 between y 221 and 302".
- Tests: `builtin_glyphs::tests` (6: coverage of the range, quads inside the cell on whole pixels, horizontals
  0..w and verticals 0..h that meet at every junction, heavy = 2 × light and double = two light lines, disjoint
  crossings, dashes/arcs/diagonals/shades) and `term_element::tests::box_drawing_and_block_elements_are_builtin_cells_between_text_runs`
  and `a_selected_builtin_cell_is_drawn_in_the_selection_colour`.
- Step 7c ("close_settings logged 1 window-teardown line") failed in 10 of 11 runs in this agent session, with and
  without `FOREMAN_SHOTS` (F-93). This stopped the before/after screenshots, so the controller takes them:
  `FOREMAN_SHOTS=<dir>` writes `m5-58-box-drawing-14.png` and `-23.png` from step 7t.

## 2026-10-29 — m5/44.19 refactor follow-ups F-88 … F-93 and the stream header check (A-45)
- **Stream header (A-45):** `stream.rs` `HttpReader::feed` refuses a 200 without `X-WorkState-Api: 1` (or with
  another version) as the v1 client does; the fake's 401 sends the header too, as WorkState 1.1.0 does.
  Tests: `stream::tests::http_reader_wants_read_api_v1`, `tests/fake.rs` `a_stream_answer_without_the_v1_header_is_refused`
  (red on master: Connected instead of Lost).
- **F-92:** `auth.rs` `save_known` writes through `foreman_core::fs::write_atomic`. Test
  `servers_json_is_written_past_another_writers_temp_name` (a directory at `servers.json.tmp`; red on master).
- **F-91 closed, the finding was wrong:** WorkState's inbox kind is `blocker_finding` (Skills `ApiV1.Inbox.cs:73`),
  so `kind_label` already reads "blocker finding"; there is no `blocked_finding`. Pinned in `keys_and_labels`.
- **F-90 closed, the arm is live:** `TerminalEvent::TitleChanged` is the foreground process changing
  (`pty_info.rs` `emit_title_changed_if_changed`), which is when a shell tab starts or leaves claude, so it drives
  `resolve_tabs`. claude's OSC title is `BreadcrumbsChanged`. One line says so at the arm.
- **F-88, 7j:** no nudge can be lost: `Nudged` is an unbounded channel drained only before a load starts. The bound
  was tight: Refresh measured 0.45–0.79 s over 20 runs here (0.5–1.3 s in m5/44.13 beside other selftests). Without
  the transcript nudge the next scan is ≥ 7.5 s away, so `REFRESHED` is 3 s; red (nudge removed) → "1 poll(s) …,
  0 scan(s), row shown false". A failure now names the late part.
- **F-88, 6b:** it holds `selftest_clipboard::borrow` across copy, read and paste. The loss is gpui's: Windows
  `write_to_clipboard` (`gpui_windows/src/clipboard.rs:70`) gives up silently when `OpenClipboard` fails while another
  process has the clipboard open, but `Chord::Copy` (`copy(Some(false))`) still clears the selection, so the
  retries copied nothing. 6b now reselects before a retry when the selection is gone, as 6c always did.
- **F-89:** the hold is right for synthetic input; it is released by `on_hover(false)` or a 2 s rest, and an OS
  mouse message from the real cursor counts as the pointer leaving. The next 1 px nudge then arms a new hold that
  already includes the message, so "held: true" with a moved row. `FleetView::left_holds` counts arms; step t0
  retakes (up to 3) when the hold was armed again while waiting, and still fails when one hold let the row move.
- **Machine-wide selftest mutex** (controller request): `selftest_clipboard::hold_machine` holds
  `Local\Foreman-selftest` for `--selftest`, `--selftest-supervise`, `--selftest-detach-shutdown` from `main`, waits
  up to 20 min ("setup: waiting for another selftest (pid N)" every 30 s, pid from `%TEMP%\Foreman-selftest.pid`).
  Test `a_selftest_waits_for_the_machine_and_names_the_holder` (red: take always succeeds).
- **F-93, 7c "window not found" from agent sessions:** a Foreman started from an agent session is not the
  foreground process, so Settings never becomes the foreground window and stays the thread's *active* window after
  `SW_HIDE` (measured: active = the closing HWND, foreground another process's window). `DestroyWindow` then sends
  it the deactivation after gpui dropped it, and gpui's `window.rs` callback logs "window not found". `tray::close`
  now hands activation to another visible window of the thread (`SetActiveWindow`, no foreground right needed)
  before the drain. 7c here: 7 of 7 failed before, 3 of 3 and then 10 of 10 passed after.

## 2026-10-29 — m5/60 supervise step s: the left column overcommitted its height

- **Symptom** (150 %, master 2d15564): step s scrolled `ws-row-…-u` from 624.7..654.7 to 451.3..481.3 in a tree
  visible 363.3..480 and no further, every run.
- **Cause**: not the scroll arithmetic. The tree had reached its max scroll offset, and its own box ended at 497.3,
  17.3 px below the 480 px window. A dump of the areas at 1400×480: Inbox 66–176.7 (its 25 % cap), Messages
  183.3–316.7 (30 %), notices squeezed to 0, search box 329.3–363.3, tree 134 px = `TREE_MIN_SHARE` of the
  446.7 px panel. Inbox and Messages were `flex_shrink_0` (m3-era, from before the tree had a minimum), so 25 + 30
  + 30 % plus header, search box and seams overflowed any panel where those fixed parts exceed 15 %, about
  565 px. m5/51's 4 px taller search box pushed the 480 px step over the edge. The clip at the window hid the
  overflow, so the last row could never be revealed by the harness, nor by a user.
- **Fix** (`view/left.rs`): Inbox and Messages keep their caps but shrink (`min_h_0`, default shrink); the notices
  get `flex_shrink(NOTICES_SHRINK = 1000)`, so they still give way first and Inbox/Messages shrink only once the
  notices are at 0. In tall windows nothing changes. After: tree 346..480 (30 %), the row scrolls to y 446.
- **Other reveals**: a search result's reveal only uncollapses and clears the search; the results render in the
  same tree container, so the fix covers them. The Inbox scrolls inside itself, and its box was always inside the
  panel; it is now smaller in short windows.
- **Red check**: `--selftest-supervise` on master fails step s as above. With the fix it passes 3 of 3.

## 2026-10-29 — m5/50 sessions painted right after the first list; loading row; Refresh icon turns

- **Startup timeline** (`view.rs` `timeline`, log lines `startup <phase>: N ms (at +M ms)` in `foreman.log`,
  printed and checked by selftest step 3s). Measured in `--selftest` on this machine, 401 rows, in ms from the
  FleetView being built:
  - **Before** (2 runs): version check 69–89, then setup 4–6, then registry 0, `list_sessions` 560–563, main repos
    and events 19, supervisor tick 334–355, all one after the other: the first paint with sessions at **+1038–1048**.
  - **After** (6 runs): version check (~70) runs beside setup and the first read; registry 0, `list_sessions`
    456–499, events 17–22; the snapshot is applied and painted **1–15 ms after the list returns**, at
    **+498–548**; the tick (300–333) runs after that paint. The transcript scan is 0–8 ms with the selftest's fake
    claude dir (a real one is bigger; it already runs beside the poll).
  - The supervisor tick, not `claude agents`, was half the wait. The 1.3 s before the window in the selftest is
    its own setup (step 2 launches a session), so the real app's pre-window time is not measured here.
- **The read/tick split:** `Poller::read` reads the registry and `list_sessions` and keeps that snapshot;
  the app applies it; `Poller::tick` then hands exactly that snapshot to `Supervision::tick` and drops it
  (`snapshot.take()`), so a tick never runs on an older list or twice on one. `refreshes` counts a poll once both
  halves are applied. The first read also loads each `DesiredState` so Start/Stop is right before the first tick.
  When `claude --version` fails, the first read's sessions and snapshot are dropped, as the old `load(false)` did.
- **Loading state:** until the first snapshot (or its failure) is applied (`sessions_read`), the tree shows a
  "Loading sessions…" row (hit key `loading-sessions`), never an empty tree; not shown when the CLI failed (its
  notice says why). Step 3a checks it is painted with no rows before the first list and gone after.
- **Turning Refresh icon:** shared `foreman_ui::button::toolbar_turning` (gpui `with_animation` +
  `Transformation::rotate`, one turn a second, paints `<key>-turning` while turning). `FleetView::refreshing` is true
  until the first poll is applied and from a click until a poll that started after it is applied
  (`refreshes_answered` = the `refresh_asks` read before that poll's read); a click while refreshing asks nothing.
  Step 7j clicks, sees it turn, clicks again (asks unchanged), and waits for it to stop.
- **Red checks** (each reverted, never committed): loading row's hit target removed → 3a "not within 15s";
  the refreshing guard in `refresh_now` removed → 7j "a second click while refreshing asked again (2 → 3)"; tick
  moved back before the apply → 3s "the first list painted 317 ms after claude agents returned, over 150 ms".
- **Gate:** `--selftest` 5 of 6 PASS, one 7r failure (another process wrote the clipboard during claude's image
  check), passed on rerun; `--selftest-supervise` failed step s (the reveal) before m5/60 was merged and passed
  on the merged build; `--selftest-detach-shutdown` PASS.

## 2026-10-29 — m5/53 tabs reorder by drag and drop

- **Shared part** (`foreman_ui::tabs`): every tab header has `.on_drag(DraggedTab { id, title })`; the preview is
  a chip with the title. `tab_strip` takes `on_move(from, to)` (indices into the headers it was given); the drop
  on a tab resolves `from` from the dragged id against the ids of that render, so a stale payload index cannot
  move the wrong tab, and a payload from another strip or an id no longer there is ignored. `reorder(vec, from,
  to)` is remove-then-insert, so the dropped tab takes the target's place. m5/54 reuses `DraggedTab` (its `id`) as
  the payload a terminal accepts.
- **Drop marker:** `drag_over::<DraggedTab>` on the target tab clears its borders and draws a 2 px
  `drop_target_border` bar on the side the tab lands (left when it comes from the right, right otherwise),
  Zed's `pane.rs` rule. Setting only the colour also recolours the tab's own 1 px right border, so both sides lit
  up; `border_0()` first fixes that.
- **No side effects:** only the tabs have a `DraggedTab` drop listener; a drop anywhere else (below the strip,
  the empty strip after the tabs, the `+`) is taken by nothing, and gpui clears the drag on mouse up. A drag
  never fires the tab's click, so the active tab stays active (it is kept by id) and nothing is closed or
  detached.
- **Persistence:** open tabs are not persisted across a restart (`FleetView::tabs` starts empty, no settings key
  holds them), so the order lives for the session, in `FleetView::tabs`; Ctrl+Tab (`cycled` over that Vec)
  follows it.
- **Selftest 7u** (`selftest_tabs::reorder`): three tabs [A, B, Work], A clicked; B dragged 200 px below the strip
  (unchanged, A active); B dragged onto A (`[B, A, Work]`, A active, screenshot `m5-53-tab-drag` mid-drag); six
  Ctrl+Tab/Ctrl+Shift+Tab presses follow the new order; B dragged onto the last tab (`[A, Work, B]`) and Ctrl+Tab
  from A goes to Work. The drag is dispatched mouse move/down, eight moves with the left button held, a 100 ms
  wait so a frame paints the marker, then up.
- **Red checks** (reverted with `git checkout`, never committed): the drop's `on_move` call removed → 7u "tab 23
  dropped on tab 22: (order, active) ([22, 23, 24], Some(22)), expected ([23, 22, 24], 22)"; the marker side
  swapped → `the_marker_is_on_the_side_the_tab_lands` fails.

## 2026-10-29 — m5/54 drop a tab or session row onto a terminal to paste its SendMessage name

- **Shared parts:** `foreman_ui::tabs::DraggedSession { session_id, title }` is the session row's drag payload,
  beside `DraggedTab`, with the same chip preview (`chip`). A row has no tab id, so it carries the `session_id`.
  `ToastSlot::show_why` is the toast without the ✓ and the success border, for "nothing was done, because…".
  `foreman_term::Terminal::paste` is public: the one paste path (bracketed when the program enabled it, line
  breaks as returns otherwise) that `paste_clipboard` also takes; it never reads or writes the clipboard.
- **Drop target** (`view/drop.rs`): an absolute overlay over the terminal grid, `invisible()` until a
  `DraggedTab` or `DraggedSession` hovers it (`drag_over` → `visible()`), then `drop_target_background` with a
  2 px `drop_target_border`: Zed's `pane.rs` pattern. The overlay is absolute, so the grid and the PTY size never
  change during a drag; a hidden div paints no listeners, but the hovered frame before the mouse up is visible, so
  the drop lands. `can_drop` keeps the look off the tab's own header and off a row whose session runs there.
- **What a drop does** (`dropped`, unit-tested): onto its own tab, or a tab/row whose session is the one the
  target runs → nothing; a tab that runs no claude (a pwsh tab, Work) → pane toast "X runs no claude session:
  nothing pasted."; otherwise the name is looked up off the UI thread (`manage::send_name_of`, the menu's lookup)
  and pasted into the target tab's terminal, or the pane toast "X has no SendMessage name yet: nothing pasted.".
  A tab's session: `Attach` maps its 8-hex id through `sessions`, a shell tab uses `tab.claude`. The pane toast
  is a second `ToastSlot` (`pane_toast`) over the right pane, so it shows where the drop happened.
- **Strip unchanged:** the strip's tabs still take `DraggedTab` for reorder (7u passes); a session row dropped on
  the strip is taken by nothing.
- **Selftest** (`selftest_drop::run`, inside 7l, which has a live claude with a SendMessage name and a plain pwsh
  tab): the claude tab dragged onto the plain terminal (screenshot `m5-54-tab-drop`) → its prompt line ends with
  the name and no new prompt appears 500 ms later (no Enter); Esc; the session row dragged on → the same; Esc;
  the plain tab onto its own terminal → nothing, no toast; a `bare` pwsh tab onto it → nothing pasted, toast
  "bare runs no claude session: nothing pasted."; the clipboard keeps its sentinel and its sequence number.
  The "no name yet" path is covered by the unit rule and the shared toast, not driven live: a session with no
  peer name is not something the selftest can make without writing into `~/.claude/sessions`.
- **Red checks** (reverted, never committed): the own-tab condition taken out of `dropped` →
  `a_drop_pastes_another_sessions_name_and_never_its_own` fails ("a plain shell onto its own terminal", NoSession
  vs Nothing); the overlay left out of the terminal pane → selftest FAIL 7l "the claude tab dropped on the plain
  terminal: the prompt line reads \"PS …\selftest-cwd>\", expected it to end with \"fm-selftest-957064-mark\"".
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-54\target`): build ok, no warnings; `cargo test --release --workspace` 663
  passed, 0 failed; `--selftest` 5 of 5 PASS; `--selftest-supervise` PASS; `--selftest-detach-shutdown` PASS; no
  `fm-selftest` sessions left.

## 2026-10-29 — m5/55 workspaces as named sets of folders; tree design A

- **Model** (`foreman_core::model`): a `Workspace` is `name`, `home` (optional; `home()` falls back to the first
  folder), `created_at`, `[[scans]]` (`path`, `depth`) and `[[folders]]` (`name`, `path`, `kind = "git"|"plain"`,
  `branch`, `has_claude_md`, `has_agents_md`, `enabled`, `added` for one added by hand). A file in the root-based
  shape (`root` + `[[repos]]`) deserialises through `Stored`: `home = root`, one scan of the root whose depth is the
  deepest `a/b` folder name (1 for every file so far), the repos as git folders in order. It is written back in the
  new shape on the next save (Rescan, Enable/Disable). The folder `name` is stored, so `fm-<ws>-<repo>` never
  changes with the shape; new names are made unique by slug (`unique_folder_name`, `-2`, `-3`).
- **Session names kept** (`crates/foreman-supervisor/tests/workspace_files.rs`): a committed root-based fixture
  (a root repo `Plat-root`, `Nomos.Pdf`, `_Logo`, `group/deep`) and René's `larchsys.toml` and `nomos.toml` (read
  from the untracked `.claude/briefs/` copies when present; both were checked here) derive the same workspace,
  relay and repo session names from the raw TOML, after load, and after a save in the new shape.
- **Ownership** replaces `RootTaken`: `owners(workspaces)` gives each enabled folder (by `path_key`) to the first
  workspace in registry order that enables it. `Registry::claim_folders` disables, in a workspace being added (fm
  `workspace add`, the app's Add workspace) or in the folders a Rescan found, those another workspace owns, and
  reports them (`notice: <folder> is supervised in workspace <owner>; disabled here`; the app's toast). `fm want`
  refuses a folder another workspace owns. The same folder can now be registered twice; `CoreError::RootTaken` is
  gone.
- **Rescan** runs every scan again: a folder found keeps its name and `enabled` (matched by path, not name), a new
  one is added, a scanned one gone from its scan is dropped, a folder added by hand stays (missing or not).
- **Mapping** (`assign`): the deepest folder holding the cwd, then a git folder that is the worktree's main
  checkout, then `in_home`. `fleet::build` groups `in_home` by the first path segment below the home into unlisted
  folder rows (`listed: false`, `folder` tag; a session in the home itself shows under `<home folder> (home)`), so a
  session in `P:\Projects\LarchSys\NodeApi` shows under larchsys with no edit, and under gaia once gaia lists it.
- **The workspace session** runs in `home()` with `--add-dir` for every folder and scan outside it (`other_dirs`,
  nested ones collapsed); the relay runs in `home()`. A workspace with neither home nor folder cannot Start (an
  error, not a launch). Plain folders get repo sessions like git ones; the mint sends no repo for them
  (`git_repo` is `None` outside a repo, unchanged), and the workspace-controller skill binds only `kind=git`
  folders (`fm workspace show` now prints `home:`, `scans:` and `kind=`/`path=` per folder).
- **Tree design A** (`fleet.rs`, `view/left.rs`, `view/results.rs`, `view/menu.rs`):
  - the workspace session and relay rows show their role (`workspace`, `relay`) in place of the name; the relay
    row's state column carries the relay status word (`ready`, `starting`, `expired`, `direct`) while it is alive,
    and the workspace row shows the relay label only when there is no relay row;
  - a wanted `fm-<ws>-<folder>` sits first under its folder; other wanted sessions (the selftest's park) stay in
    the supervised block;
  - folder rows: `folder` (plain), `supervised in <ws>` (another workspace owns it: no `+`, no Enable; a click
    reveals the folder there), `also in <ws>, …` (other workspaces list it); a plain folder shows no branch; the
    root-repo label is now `<name> (home)`;
  - a session under several workspaces paints in each; the occurrence in the workspace that wants it, else the
    first, keeps today's keys; every other occurrence carries `also in <owner>` and every key of the row
    (`session-row-`, `star-`, `attach-`, `resume-`, `tab-link-`, `marked-`, `foreign-`, `waiting-`, `hook-`,
    `session-name-`, `session-id-`, the element id, the search result's `result-session-`) ends in `@<its ws>`.
    `hidden_stopped` counts a session once however many workspaces hide it.
  - Without the `↗` of the redesign doc: `also in <ws>` is plain text, since the UI font's coverage of the arrow
    was not checked.
- **Trust and access** (probe, CLI 2.1.312, scratch `target/aw-m5-55-probe-afc2cc`, sessions
  `aw-m5-55-probe-afc2cc-*`, stopped and removed, no trust setting read or written): a `--bg` session whose cwd
  is a trusted plain folder, with `--add-dir` to a freshly `git init`ed folder (untrusted as a cwd) and to a folder
  under `%TEMP%` outside every trusted tree, launched in auto mode with **no trust and no access prompt**, read
  both notes and wrote a file into each. The same `git init`ed folder as the cwd is refused (`Workspace not trusted.
  Run \`claude\` in … once and accept the trust prompt, then retry.`, exit 1). So trust is asked only for the
  cwd: the home must be trusted, the other folders need nothing. **Recommendation: keep the default (the first
  folder) and let the user set a trusted parent as home**; a home that is an untrusted repo shows the existing
  "trust needed" Inbox row at Start, and the repo sessions of untrusted repos need trust as before (per repo).
- **Trust follows any `.git`:** inside this trusted checkout, a folder holding a fake `.git` (only a HEAD) and one
  holding a copy of this checkout's `.git` file (`gitdir: …/worktrees/m5-55`; `git rev-parse --show-toplevel`
  names the copy) were both refused as cwds: `Workspace not trusted …`. So a selftest cannot make a git folder that
  claude launches in; the first run of step v failed exactly there (the plain folder's session ran, the git one's
  never did).
- **Selftest step v** (`selftest_supervise::virtual_step`, before step g): workspace `<testbed>-v`, home unset, of
  this checkout (git, a linked worktree: kind git with its branch) and the plain `VPlain` inside the testbed's home;
  each folder's repo session is wanted with a one-word, no-tool prompt and the workspace saved as started; both go
  live, each first under its folder in `-v`; the plain one's shows under the testbed too as a home folder with
  `also_in = -v`; `session-row-<id>` for both and `session-row-<id>@<testbed>` painted; not started → both stopped
  and still listed (stop, never rm). Not two siblings, as c5 asked: the only trusted git folder a selftest may
  launch in is this checkout, and its siblings are other units' worktrees. Step g still expects 2 supervised rows:
  the testbed's repo session is unwanted in step w before g, so design A moves nothing there.
- **Selftest 7b changed on purpose:** it asserted the old refusal of a folder already registered ("already
  registered, as workspace …"). It now saves the same folder again (as `selftest-add`), checks alpha is disabled
  there (owned by `selftest-added`) and beta enabled, waits for `supervised-in-selftest-add/alpha` to paint, and
  removes the second workspace.
- **Red checks** (reverted with `git checkout`, never committed): `home: s.home` instead of `s.home.or(s.root)` →
  `the_users_workspace_files_keep_their_session_names` fails (larchsys's home read as its first folder
  `P:\Projects\LarchSys\AutoWork`; the committed fixture alone does not catch it, as its first folder is the root);
  the second-occurrence owner taken as the first workspace always → `the_workspace_that_wants_a_session_keeps_its_keys…`
  fails (`fm-larchsys-gaia` marked `also in gaia` in larchsys, which wants it); `claim_folders` not disabling →
  `check_new_refuses_a_taken_name_and_a_second_workspace_on_a_folder_gets_it_disabled` fails.
- **Not built here** (m5/56, m5/59): editing folders, home and scans in the UI and `fm`; hiding disabled folders
  from the tree; the `kind`/owner columns m5/56 adds to `fm workspace show` beyond `kind=` and `path=`; design B.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-55\target`): build ok, no warnings; `cargo test --release --workspace` 678
  passed, 0 failed; `--selftest` 5 of 5 PASS; `--selftest-supervise` PASS (the run before step v's fix failed at
  step v, above); `--selftest-detach-shutdown` PASS; no `fm-selftest` sessions left.

## 2026-10-29 — m5/56 create and edit workspaces (UI and fm), disabled folders out of the tree

- **One edit path** (`foreman_core::Registry::edit`, `edit.rs`): `Edit::{AddFolder { path, enable }, RemoveFolder,
  SetEnabled, SetHome, AddScan, RemoveScan}` loads the workspace under `workspaces/<name>.toml.lock`, applies one
  change, checks it and saves through `save` (`write_atomic`); `Edited` names the folders added, removed and claimed.
  Both `fm workspace …` and the app's Edit folders… call it, so neither has a copy of the rules:
  - a path must be a directory (made absolute); its kind (git or plain) is read from the disk (`Workspace::add_folder`);
  - adding a folder that another workspace supervises adds it disabled (`claim_folders`, notice as in m5/55);
    enabling it is refused (`CoreError::SupervisedElsewhere`, a user error, exit 1);
  - **removing a scanned folder puts it on its scan's `exclude`** (`[[scans]] exclude = [...]`, `skips=` in `fm
    workspace show`), else the next Rescan or `add-scan` would add it back; adding it again by hand takes it off;
  - `AddScan` of a path already scanned takes the new depth and runs every scan again (`rescan`); `RemoveScan` takes
    the folders only it found (not added by hand, under no other scan) with it;
  - `SetHome(None)` goes back to the first folder.
  `Workspace::find_folder` (name, ignoring case, slug, then path) is the lookup for every `<folder>` argument; `fm
  want`'s `find_repo` uses it.
- **Stopping a folder's session** (`DesiredState::release(home, ws, folders)`): the repo sessions of the folders
  taken away (removed, a removed scan's, or disabled in the app) that the workspace *wants* are unwanted, i.e.
  retired, so the supervisor stops a live one (stop, never rm); a workspace that wants none of them is not written,
  so no desired state appears for a workspace that had none. The app asks first (`foreman_ui::confirm`,
  `Confirmed::Edit`): "Remove folder X in workspace W? Its supervised session fm-W-x is stopped (stop, never rm) and
  can be resumed; no files are deleted." (`folders::stops` decides when to ask). `fm workspace disable` releases too;
  `fm` asks nothing, like `fm unwant`.
- **fm** (`crates/fm/src/edit.rs`): `workspace new <name> [--home <dir>] [--folder <dir>]...` (checks every
  directory before writing; folders enabled), `add-folder <ws> <dir> [--enable]` (disabled without it, per the
  item), `remove-folder`, `enable`/`disable` (beyond the item: the CLI had no way to flip the flag), `set-home`,
  `add-scan [--depth N]`, `remove-scan`. `workspace show`, `add` and every edit print `owner=<ws>|-` per folder
  (`with_owners`: the registry with the workspace as it is now; `-` for a disabled folder nobody enables). The
  workspace-controller skill's table names `owner=` and `skips=`.
- **App**:
  - the Add workspace button opens a menu (`menu-add-from-folder`: today's picker and review; `menu-add-empty`: the
    review with only the name, saved with no home, folder or scan, and Edit folders… opens on it);
  - Edit folders… (workspace menu `edit-folders`, `view/folders.rs`) is a panel in the notices area like the Add
    review. It lists the folders with ☑ (`edit-enable-<f>`), kind, path, `home` or `Home` (`edit-home-<f>`) and
    `Remove` (`edit-remove-<f>`). A folder another workspace supervises shows `supervised in <ws>` in place of the
    box. The scans each have `Remove` (`edit-scan-remove-<i>`). Below sit a shared `TextInput` path field
    (`edit-path`; Enter adds it) and `Add folder…`/`Add scan…` (an empty field opens the folder picker, or takes
    `pick_folder` in a selftest), `Set as home`, `Reset` (home back to the first folder) and `Done`. A folder added
    in the app is enabled.
  - **Adopting a running session** (controller decision (b), 2026-10-29): there is no adopt action. A folder added
    that holds a live session Foreman did not launch shows it under the folder by containment, and the toast says
    "<session> in <folder> now shows here" (plus "; Enable to supervise a session of its own." when the folder is
    disabled). Stopping it and resuming its UUID as `fm-<ws>-<folder>` (option (a)) is a backlog follow-up.
- **René's rule (c5)** (`fleet::build`): a disabled folder is in the tree only while it holds a session (open or
  closed-and-shown); a disabled folder's sessions were dropped from the tree before, and now show under it with its
  `disabled` tag. `Supervised::disabled_folders` keeps the empty ones for search (`search_tree`); a result for one
  carries a `disabled` tag (`result-disabled-<ws>/<f>`), and a click on it opens Edit folders… for that workspace.
  A folder `supervised in <ws>` stays in the tree (design A, m5/55).
- **Selftests:**
  - 7b no longer finds the disabled `beta` in the tree; it enables beta through the workspace menu's Edit folders…
    (`edit-enable-beta`) and closes it with `edit-done`. Its workspace menu list gains `edit-folders`.
  - Every `add-workspace` click is followed by `menu-add-from-folder`.
  - **A closed menu still painted in the next frame takes a press as a click outside it** (`on_mouse_down_out`), so a
    right-click right after a click outside opened nothing; 7b waits for a frame without `menu-remove` first.
  - **Step x** (`selftest_supervise::edit_step`, after v), all through the window: Add workspace → Empty →
    `<testbed>-x` saved empty; Edit folders… adds the sibling testbed folders `XGit` (a fake `.git`: git kind, never
    launched, see m5/55's trust finding) and `XPlain` (plain) by the picker hook; the testbed is set as home by the
    path field; `fm-<testbed>-x-xplain`, wanted, runs and sits first under XPlain; XGit disabled leaves the tree and
    search shows it `disabled`; Remove XPlain asks the question, then the file loses it, the desired state retires the
    session, and it stops and stays listed.
  - The tree check filters `listed` folders, since the testbed's own sessions in the home show as unlisted folder rows.
- **Red checks** (reverted with `git checkout`, never committed):
  - the tree keeping every disabled folder (master's behaviour) → `a_disabled_folder_without_sessions_leaves_the_tree_and_stays_for_search` fails;
  - `RemoveFolder` not adding to `exclude` → `scans_add_their_repos_keep_a_removed_one_out_and_take_theirs_when_removed` fails;
  - `release` not unwanting → `release_unwants_only_the_wanted_sessions_of_the_folders_and_writes_nothing_otherwise` fails.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-56\target`): build ok, no warnings; `cargo test --release --workspace` 686
  passed, 0 failed; `--selftest` 5 of 5 PASS; `--selftest-supervise` PASS (step x included); `--selftest-detach-shutdown`
  PASS; no `fm-selftest` sessions left. No m5-44 audit section names m5/56, so no audit finding was in scope.

## 2026-10-29 — m5/59 tree design B: the workspace row is the workspace session

- **Model** (`fleet.rs`): `WorkspaceNode` carries the workspace session (`own`) and the relay (`relay`) apart from
  `supervised`, which keeps only wanted sessions of no folder (the selftest's park). `SessionRow::role` is gone:
  neither session has a tree row. `session_count` counts the rows under the workspace row; `rows()`/`into_rows()`
  walk all of them, the two first (menus, delete, `mark_second_occurrences`, the selftests). Search still finds both
  by name and shows them as result rows.
- **Workspace row** (`view/left.rs` `render_workspace_row`): chevron · the workspace session's health dot (hollow when
  it is stopped, gone, unknown or not wanted) · name · home (clipped, `ws-root-<ws>`) · row count · a 16 px relay slot
  · four fixed 20 px icon slots: Work `work-<ws>`, Task… `task-<ws>` (started, relay accepts), Attach
  `attach-ws-<ws>` or Resume `resume-ws-<ws>`, Start `start-<ws>`/Stop `stop-<ws>`. A slot whose action does not
  apply stays empty, so nothing shifts. Blocked tints the row amber (0.12/0.18, as a session row); `waiting for: …`
  is a second line (`ws-waiting-<ws>`). The row counts as selected while the active tab is its Work tab or attaches
  its session.
- **Relay** is one glyph in its slot (`relay-<ws>`, `signal_high` green ready, `load_circle` amber starting,
  `warning` amber expired; nothing for `direct` or stopped), with `RelayStatus::label` as its tooltip; the workspace
  card's `relay` row names `direct`/`stopped` too.
- **Hover icons** (`foreman_ui::button::revealed`): an `icon` button at opacity 0 that a `group_hover` of its row
  brings to 1, or always shown when the row is selected. Opacity 0 still paints, so its hit key exists and a selftest
  click lands (checked in gpui: `with_element_opacity` multiplies, it never skips). Folder rows: `+`
  `new-claude-<ws>/<f>` and a terminal `terminal-<ws>/<f>` (new key). Session rows: star, then Attach `attach-<id>` or
  Resume `resume-<id>`, then Tab `tab-link-<id>`. **Two states stay visible without a hover**: a filled star and the
  Tab icon (the session is open in a tab not shown now); hiding them would hide a fact, not an action.
  **Keyboard focus**: tree rows take no keyboard focus in Foreman, so only hover and selection reveal.
- **Menus** (`view/menu.rs` `workspace_items`): after Remove…, the workspace session's full `session_items` under a
  `workspace session · <name>` caption and the relay's under `relay · <name>`, keys renamed `session-…`/`relay-…`
  (`section_key`; `stop-session` → `session-stop`/`relay-stop`) so they never clash with the workspace's own. Every
  item of the old rows' menus is kept, Star and Delete… included. The send-name lookup runs for the workspace menu
  too. Shared: `foreman_ui::menu::MenuItem::under(caption)` draws a rule and a caption above an item.
- **Folder rows**: `git_branch` or `folder` icon in place of `./` (`folder_label`; `(home)` kept); the branch without
  parentheses; the `folder` tag is gone (the icon says it). Search results and the Add review keep `repo_label`.
- **Hover cards** (`row_card.rs` over `foreman_ui::hover_card`): workspace: home, folders (`(disabled)` marked), state,
  session (name · health · id), waiting, hook, relay, WorkState (the project an open Work tab read; `WorkView::project`).
  Session: state, waiting, id, cwd, worktree (`<folder> of <main checkout>` when the cwd's `.git` is a file), branch
  (`branch_at`, read when the card shows), hook, claimed item, also in. A child's tooltip wins over the row's card
  (gpui shows the last tooltip request, children paint after parents), so the clipped root path still shows its
  full-path tooltip (7b's check passes).
- **Icons** copied from Zed (`.repos/zed/assets/icons`): `play_outlined`, `stop`, `maximize`, `rotate_ccw`, `screen`,
  `pencil`, `list_todo`, `terminal`, `git_branch`, `folder`, `signal_high`, `load_circle`.
- **Selftests**: step b reads the workspace session from `own`, then `workspace_row_step` checks `attach-ws-<ws>`
  paints, no `session-row-<id>` exists for it, and the workspace menu holds `stop, work, task, session-attach,
  session-stop, session-copy-name, session-copy-id, session-star` under the `workspace session · <name>` caption;
  step r checks the relay glyph paints while the relay is ready and the relay has no row; step g expects the
  workspace session on the workspace row and 1 row (park) in the block (was 2 rows: workspace, park). 7b's
  workspace menu list is unchanged (that workspace never started, so it has no section); 7b gains the two hover
  cards (`hover_cards`, shots `m5-59-workspace-card`, `m5-59-session-card`). 7f and t0 pass with `task-`/`stop-`
  now 20 px icons at the row's right end.
- **Before/after** (`FOREMAN_SHOTS`, `C:\fmbuild\m5-59\shots\{before,after}`; before = master 8671f05 in this
  worktree detached; its `--selftest-supervise` stopped at step v only because a detached HEAD has no branch, after
  the shots): started workspace (`m5-14-messages`): before `▾ selftest-… [Work] 2 sessions started d… [Task…] [Stop]`
  then a `workspace` and a `relay` row; after `▾ ● selftest-… P:\…\selftest-… 1` with the icons hidden until hover
  and no session or relay row. Unstarted (`m5-31-tree-tabs`): `○ larchsys P:/Projects/LarchSys 8`, folders with the
  git-branch icon and `master 4`, session rows without pills. Cards: `m5-59-workspace-card`, `m5-59-session-card`.
- **Red checks** (reverted with `git checkout`): the workspace session pushed into `supervised` →
  `the_workspace_that_wants_a_session_keeps_its_keys_and_its_rows_sit_by_role_and_folder` fails; the relay section
  dropped from `workspace_items` → `the_workspace_menu_holds_every_item_of_its_workspace_session_and_relay_under_a_caption` fails.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-59\target`): build ok, no warnings; `cargo test --release --workspace` 689
  passed, 0 failed; `--selftest` 5 of 5 PASS; `--selftest-supervise` PASS; `--selftest-detach-shutdown` PASS; no
  `fm-selftest` sessions left. No m5-44 audit section names m5/59, so no audit finding was in scope.

## 2026-10-30 — m5/64 a folder unchecked here and enabled elsewhere leaves this workspace; no folder badge

- **Rule** (`fleet::build`): a folder disabled in a workspace and enabled in another (so `supervised_in` is set,
  `owners`) is not in that workspace's tree, and neither are its sessions; they show where the folder is enabled.
  Before, such a folder stayed with a `supervised in <ws>` tag and its sessions (m5/55 design A), and a session
  under it was a second occurrence keyed `@<ws>`. A disabled folder no other workspace enables keeps m5/56's rule
  (in the tree while it holds a session). `RepoNode::also_in` lists only the other workspaces that *enable* the
  folder, and only while it is enabled here; `supervised in <ws>` stays for a folder enabled in both.
- **Home folders too:** a session in the home that another workspace places in one of its enabled folders
  (`placed_in`: `assign_with` over every workspace, open and closed sessions, enabled folders only; a linked
  worktree counts through `main_of`) no longer makes an unlisted home-folder row. Without it, René's larchsys
  (scan of `P:\Projects\LarchSys`, which finds only git folders) would still show gaia's plain `NodeApi` as an
  unlisted row with `also in gaia`. Unlisted rows carry no `also in`. This goes past the item's wording (it names
  disabled folders) and follows its "sessions show only where the folder is enabled"; the controller may narrow it.
- **Search** (`search_tree`, `disabled_folders: true`): the hidden folder stays, without sessions, and its result
  carries `supervised in <ws>` (`RepoHit::supervised_in`, key `result-supervised-in-<ws>/<f>`); a click reveals the
  folder in that workspace (`reveal_folder`). The `disabled` result tag is unchanged.
- **Badge:** the `folder` tag of a plain folder row is gone (`folder_tags`); the kind icon says it. m5/59's notes
  said so already, but the tag was still painted.
- **Tests:** `a_folder_unchecked_here_and_enabled_elsewhere_leaves_this_workspace_with_its_sessions` (René's shape:
  foreman {AutoWork, Skills}, gaia {Gaia, NodeApi plain, node-api-dotnet}, larchsys home `P:\L` with AutoWork, Gaia,
  Skills disabled plus Healthy, Old, Quiet; a worktree session of AutoWork; search for `autowork` and `quiet`).
  `overlapping()` now enables Gaia in both, so the second-occurrence tests keep a folder enabled in two.
- **Selftests:** 7b no longer waits for `supervised-in-<again>/alpha`; alpha is out of the second workspace's tree
  (beta painted), and a search for `alpha` paints `result-supervised-in-<again>/alpha`. Step v: the plain folder's
  session is not under the testbed and `session-row-<id>@<testbed>` is not painted (the testbed's home holds VPlain,
  `-v` enables it).
- **Red checks** (reverted with `git checkout`, never committed): the folder supervised elsewhere kept in the tree
  (master's branch) → the new test fails with larchsys `[AutoWork 2, Gaia 1, Healthy 1, Skills 1, Other, Old]`,
  René's screenshot; the home filter off → both overlap tests fail (`NodeApi` unlisted under larchsys); the search
  label off → the new test fails at "search finds it in larchsys, labelled with its workspace".

## 2026-10-30 — m5/63 the Edit folders dialog fits its box

- **Cause**: each folder row was one flex line whose name, kind (`git (<branch>)`), Home and Remove were `flex_none`;
  only the path shrank. A long name or branch pushed Home/Remove past the dialog's right edge, and the
  `supervised in <ws>` tag in place of the ☑ shifted the rest of its row.
- **Row now** (`view/folders.rs`): a two-line details block (`flex_1`, `min_w_0`) and a `flex_none` button column at
  the row's right end. Line 1: a fixed 16 px state column (☑/☐, `edit-enable-<f>`), the name (`edit-name-<f>`), the
  git-branch or folder icon and the branch (`edit-branch-<f>`, shrinks 8× faster than the name). Line 2: the path
  (`edit-folder-path-<f>`, middle ellipsis), starting under the name; for a folder supervised elsewhere the
  `supervised in <ws>` tag (`edit-elsewhere-<f>`) starts line 2 at the state column's x, so no row shifts. Every
  label goes through `crate::clipped::clipped`, so a cut one has its full text as a tooltip. Home and the `home`
  tag sit in a 48 px `justify_end` slot, so Remove sits at the same x in every row. The title, home line and scan
  rows are clipped too; the dialog and its lists take `min_w_0`. The dialog scrolls with the notices area
  (`overflow_y_scroll`), which already held it.
- **Selftest** (7b, `selftest_manage::edit_folders_fit`): with the second registration of `selftest-add` (alpha
  `supervised in selftest-added`, beta), it sets `left_width` to `MIN_LEFT_WIDTH` (now `pub(crate)`), opens Edit
  folders…, adds `target/selftest-edit-long/a-folder-with-a-rather-long-name-for-edit-folders` on branch
  `copilot/fix-the-edit-folders-dialog-overflowing-its-box` through the picker hook, and waits until every row's
  state cell, Home and Remove lie inside `edit-folders` with Remove's right edge equal in all rows; then the path
  and branch must be painted cut (`clipped::cut_at`). Shot `m5-63-edit-folders-min` (also taken when the check fails).
- **Scaling**: gpui lays out in logical pixels, so 125 % and 150 % change only the device pixels; the selftest ran at
  this machine's 150 %. At the minimum width a long branch can shrink to nothing next to its icon; the name keeps
  room first.
- **Red check** (master's `folders.rs`, reverted by restoring the file, never committed): 7b fails `every Edit
  folders row inside the dialog` with `edit-remove-a-folder-…` at x 759 against the dialog's right edge at 267.
  Before/after shots: `C:\fmbuild\m5-63\shots\{before,after}\m5-63-edit-folders-min.png`.
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-63\target`): build ok, no warnings; `cargo test --release --workspace` 689
  passed before and 690 after merging master (m5/64), 0 failed; `--selftest` 5 of 5 PASS before the merge; after it
  4 of 5, the fifth `FAIL step 7: board load: own work per frame p10 19.21 ms, over the 16 ms budget` beside the
  other units' selftests, PASS on rerun; `--selftest-supervise` PASS; `--selftest-detach-shutdown` PASS; no
  `fm-selftest` sessions of this worktree left. No m5-44 audit section names m5/63.

## 2026-10-30 — m5/62 startup: transcripts read head and tail from a persisted cache; a loading line until every source has loaded

- **The slow source was the transcript scan.** René's log showed the first paint with sessions at +858 ms, but the
  closed sessions came from the first scan after that, and it read every in-root transcript **whole**. The first
  scan also ran before the registry was applied, with no roots, and logged its 0 ms as "transcripts", so the log
  never showed the real cost. `~/.claude/projects` on this machine (René's): 2,871 files, 3.1 GB, 14-day window
  ~2,720 listable. Measured with `cargo test --release -p foreman-claude -- --ignored --nocapture`:
  - **Before** (whole-file read, naive `windows().any` byte search 5× per short line, one thread): the real dir
    (every project dir) **17.0–17.9 s**; the generated fixture (400 transcripts, 426 MB, `scan_timing_on_four_hundred_transcripts`)
    **1.9–3.1 s**, warm OS cache.
  - **memmem alone** (`memchr` was already in the tree via serde_json; one `"type":"` pass instead of five
    `format!`ed needles), one thread: fixture **110–155 ms**. The byte search, not IO, was most of the 2.8 s.
  - **+ 8 parallel readers** (`read_all`, largest file first): fixture **35–66 ms**.
  - **+ head and tail** (`read_head_and_tail`): fixture with two 60 MB files (540 MB) **73–95 ms**; the real dir
    **350–385 ms reading 340 MB of 3.1 GB** (was every byte). The time left there is listing all 1,237 project dirs
    (~250 ms; the app lists only its roots' dirs).
  - **Restart from the cache** (`<home>/cache/transcripts.json`, 4 MB for the real dir): **0 bytes of transcripts
    read**, 165–182 ms in the benchmark (listing plus JSON); the app shows the cached rows before even that scan.
- **Head and tail** (`foreman-claude/src/transcript.rs`): over every real file's meta lines, the last
  `custom-title`/`agent-name`/`ai-title`/`last-prompt` sits within 34 KB of the end (claude appends them again
  near the end), 2,692 of 2,730 files have a `custom-title`, none has a `summary`, and the first user line is within
  35 KB of the start. So a file over 256 KB is read from the start to its cwd and first prompt, then from the end in
  64 KB blocks (×4 each time, aligned to a whole line) until the tail holds a `custom-title`, the name that outranks
  the rest; a tail without one grows until it meets the head, which is the whole read. The latest-wins rule holds
  for what the tail read. `title`, `cwd`, `first_prompt` and `listable` come out as the whole read gives them:
  `the_head_and_tail_read_names_each_session_as_the_whole_read_does` (generated: no custom title, latest near the
  end, one only early, a command before the prompt, a half-written last line) and, over the real dir,
  `a_real_claude_dir_names_as_the_whole_read_does` (`FM_BENCH_CLAUDE_DIR`, ignored): 2,722 transcripts, 0 differ.
  `agent_name`/`ai_title`/`last_prompt` may be older than a whole read's when a `custom-title` decided early; nothing
  outside `transcript.rs` reads them. A grown file is still read from its cached offset.
- **The cache** (`Transcripts::with_cache`, `cached`, `has_cache`): the file entries (path, length, mtime in ns,
  offset, transcript) with `CACHE_VERSION` (bump when `Transcript::feed` reads something new), written with
  `write_atomic` at once after a file was read from its start, otherwise at most every 5 min; entries of files gone or
  out of the window are dropped. An unreadable or older cache is ignored.
- **René's decision (warm start):** the watcher loads the cache in the background beside the first poll; when it
  holds files, its rows are shown at once (`startup transcripts from the cache done`) and the scan rereads only
  changed, new and deleted files quietly and updates the rows in place. The loading line names the transcripts
  (`transcripts n/N`, from `scan_with`'s progress) only when there is no cache.
- **Every source logs `startup <source> done: N ms (at +T)`**: version check, setup, registry, list_sessions, main repos
  and events, desired state, supervisor tick (relays, messages), transcripts (files read, listed), transcripts from
  the cache, workstate status, workstate inbox, and `every source done`. Step 3s fails when one is missing.
- **Loading state** (`FleetView::loading`, `loaded`): the slim line `Loading sessions… session list, transcripts
  120/400, WorkState` under the action bar (hit key `loading-sessions`), the tree shown under it, until the first poll
  and tick are applied, the transcripts are read or cached, WorkState's status is known and its first inbox read
  (when one started) is applied; then latched off. The Refresh icon turns until then, and after a click until both
  the poll and the transcript scan that started after it are applied (`transcripts_answered`).
- **Selftests:** 3a checks the line is painted before the first list, never gone while a source is pending, and gone
  only once all are done. 7w (`selftest_startup.rs`): a new transcript is listed and saved in the cache; one appended
  line later, a warm `restart_transcripts` with the scan held (`transcripts_hold`, a `PollHold`) shows the cached
  name, every source loaded, no loading line; released, the background scan renames the row; then with the cache
  deleted, a cold restart paints the line naming the transcripts while the scan is held and loses it after.
- **Red checks** (reverted with `git checkout`, never committed): tail never grown → the head-and-tail test fails
  (`"prompt 6"` against `"title 6.100"`); `with_cache` ignoring the file → the cache test fails ("only the grown file
  is read", 2 against 1); `loading` never naming transcripts → `--selftest` FAIL at 7w ("a cold start: the loading
  line naming the transcripts while the scan is held: not within 15s").
- **Gate** (`CARGO_TARGET_DIR=C:\fmbuild\m5-62\target`, master ae3713f merged, m5/63 and m5/64 in): build ok, no
  warnings; `cargo test --release --workspace` 692 passed, 0 failed; `--selftest` 5 of 5 PASS in a row (3a: every
  source done 523–617 ms after the loading line; 7w warm row 28–52 ms, background rename 63–85 ms);
  `--selftest-supervise` PASS; `--selftest-detach-shutdown` PASS; no `fm-selftest` session from this worktree left.
  The first gate run on the merged tree failed `--selftest` 3 of 5 at step 7 (board load own work p10 19.5–20 ms
  over the 16 ms budget; 8–13 ms in every other run) and `--selftest-supervise` once at step c ("the stopped pid …
  is still running") while another unit's rustc and foreman were busy; each passed on rerun. Before the m5/63 merge
  all passed. No m5-44 audit section names m5/62.

## 2026-10-30 — m5/67 session rows: claude icon, `kind · state`, one open action that double-click also runs

- **Row** (`view/left.rs` `render_session`): the claude mark (`Icon::Claude`, Zed's `ai_claude.svg`, monochrome) in the
  health colour (`health_color`: running green, idle blue, blocked amber, stopped/interrupted/closed grey) · name ·
  `kind · state` (the state word in its colour, for colour-blind reading) · `also in` · star · one 20 px action slot.
  The hex id is off the row; the hover card keeps it, Copy id keeps it, search still matches it and, while a search
  matched the id, the result row shows the short id with the mark (`session-id-<id>`, hit key; never in the tree).
  Blocked keeps its amber tint and `waiting for:` line. Keys kept: `session-row-`, `attach-`, `resume-`, `tab-link-`,
  `star-`, `marked-`, `session-name-`, `foreign-` (now on the name).
- **`interrupted`** replaces `process gone` (`fleet::health_label` for `Health::Orphaned`); the card adds "the process
  ended while the session was working" (or `blocked`, from `last_state`). `fm roster` keeps `orphaned (…)`: the
  workspace-controller skill documents that output.
- **One open action** (`fleet::Open::of`, unit-tested per case; `FleetView::open_of`/`open_row`), shown in the slot
  and run by a double-click on the row (`click_count() == 2`; a single click still does nothing in the tree):
  another Foreman's → nothing; open in the tab shown now → nothing; open in another tab → switch (Tab icon, shown
  without hover); live background → Attach; `offers_resume` → Resume (full UUID, unchanged); a live interactive
  session outside our tabs → nothing (resuming would start a second claude on its transcript); a stopped `fm-`
  session → nothing, "Start the workspace from its menu". The two refusals show a `show_why` toast and an `open`
  line in the hover card says what a double-click does or why not. Search results: a single click runs only
  Tab/Attach (as before), the row's own double-click does the rest; the outer click ignores `click_count() > 1`, so
  nothing runs twice. The m5/54 row drag is unchanged (no movement, no drag).
- **Another Foreman's session**: no badge, no action, no double-click; the row is muted (opacity, no strikethrough,
  c8) and the name's tooltip says "another Foreman session".
- **Own selftest run** (controller's decision D): `FleetView::own_prefix` (→ `fleet::Supervised::own_prefix`, the
  4th argument of `fleet::is_foreign`) names `fm-<prefix>` and `fm-<prefix>-…` this Foreman's own; only the
  selftest sets it (`selftest::start`: the supervise testbed's `name_prefix`, else `fm-selftest-<run_hex()>`; the
  main session and 7l's `-mark` share `run_hex`). Without it the selftest's own `fm-selftest-<hex>` row is foreign
  (`foreman_supervisor::is_foreign`, unchanged) and loses its Attach. Step e2's injected row is now another run's
  `fm-selftest-<reversed hex>x-park`, still foreign.
- **Selftests**: step 4 attaches by double-clicking the row, then a second double-click with its tab shown opens
  nothing; 7h resumes the closed session by double-click (exactly one tab); 7l: a double-click on the row brings
  its tab back; 7k: the id mark painted on the result row, no `session-id-` in the tree after Esc; supervise step v:
  a double-click on the stopped `fm-` row opens no tab and toasts the Start hint; e2: the foreign row paints no
  action and a double-click changes neither tabs nor toast. Not driven live: the "interactive live elsewhere" case
  (unit test only).
- **Red checks** (reverted with `git checkout`, never committed): the foreign arm of `Open::of` off →
  `a_rows_one_open_action_follows_tab_then_attach_then_resume_and_refuses_the_rest` fails; the own-prefix check off
  → `another_homes_fm_rows_are_foreign_shown_and_never_ranked_blocked` fails; `process gone` back → `labels` and
  `an_interrupted_session_says_its_process_ended_and_another_foremans_says_whose` fail. Before decision D the first
  `--selftest` failed at step 3 (the selftest's own row foreign, no `attach-`).
- **Before/after** (`FOREMAN_SHOTS`, `C:\fmbuild\m5-67\shots\{before,after}`; before = master ae3713f in this worktree,
  detached): `m5-31-tree-tabs`/`m5-14-terminal-tab` (dot · state column · name · `kind · id`, the `another Foreman`
  badge → claude mark · name · `kind · state` · star), `m5-30-search-id` (the id mark now on its own chip),
  `m5-10-another-foreman` (after only: muted, tooltip, no badge, no action).
- Gate (`CARGO_TARGET_DIR=C:\fmbuild\m5-67\target`, master 44c235a merged): build ok, no warnings; `cargo test
  --release --workspace` 694 passed, 0 failed; `--selftest` 8 runs, 5 PASS (2, 4, 6, 7, 8), 3 FAIL (1, 3, 5) all at
  step 7's frame budget (own work p10 16.7–19.2 ms over 16 ms) while other units built and ran selftests; master
  ae3713f failed the same check twice on this machine the same hour (step 7 p10 16.6 ms, step 8 p10 18.6 ms), and
  the passing runs' step 7 own work (p10 11.7–13.1 ms) matches master's (12.1–13.1 ms). `--selftest-supervise` PASS;
  `--selftest-detach-shutdown` PASS; no `fm-selftest` session from this worktree left. No m5-44 audit section names
  m5/67.

## 2026-10-30 — m5/65 remember last tabs: the tabs open at close come back on the next start

- **File** `<home>/tabs.json` (`saved_tabs.rs`: `SavedTabs { tabs, active }`, `load` through `load_json_or_notice`,
  `save` through `write_atomic`). Each tab is one of `shell {title, cwd}`, `claude {title, cwd, session_id, name}`
  (a hand tab), `attach {title, cwd, id, session_id, name}`, `work {title, workspace}`; exited tabs are left out.
  A hand tab's `session_id` is the claude resolved under its shell (`Tab::claude`, so a `/clear` is followed), else
  the uuid it was launched with (`Tab::launched_session`: the resumed uuid, or the `+` tab's `new_session` from
  m5/40); `name` is the tree's name for that uuid, else the one it was launched with, so the resume mints under it (m5/35).
- **When it is written:** `persist_tabs` runs on every render and sends the snapshot to one writer task only when
  it changed (the writer keeps only the newest pending snapshot). That catches every change without a hook per
  path: open, close, drag (m5/53), activate, a claude resolved under a shell. Nothing is written until the start's
  reopen is done (it would write an empty list over the file), nor once Quit is asked (`quit_asked`: the quit detaches
  and removes the attach tabs, which must not reach the file), nor while the selftest closes tabs for its restart.
  A failed write is one notice (`Notice::Tabs`), cleared by the next good write.
- **On start** (`view/reopen.rs` `reopen_tabs`): after the first session list is applied **and** the last run's hand
  tokens are released (`hands_ready`, set after `Hands::release_all`; a reopened hand tab minted before it would be
  revoked by it). The plan (`saved_tabs::plan`, pure, unit-tested) runs in the background (it reads the folders and
  looks for transcripts with `find_transcript`): shells start a new pwsh in their cwd; Work tabs open if the
  workspace is still registered; an attach tab re-attaches when its session is alive, else becomes a placeholder
  with Resume (nothing restarts unasked); a stopped `fm-` session is dropped (the supervisor starts it); a claude
  tab resumes with `claude --resume <full uuid>` and a fresh mint (`hand_claude` → `launch_for(cwd, name, uuid)`),
  or becomes a placeholder when `resume_tabs_on_click` is set. Dropped, with one notice naming each and why: a
  folder gone, a workspace gone, a session with no transcript (deleted, or a `+` tab never prompted), a short or
  missing id (never resumed: a short id starts a new session), a claude session already running in another
  terminal (a resume would run it twice). Reopened tabs come first in the saved order, then any the user opened
  while they loaded; the saved active tab is active unless the user had one open before.
- **Placeholder** `TabKind::Saved(SavedTab)`: no terminal, a text and a Resume button (`resume-tab-<id>`). Resume
  attaches when the session runs again by then, else resumes (`claude --bg --resume` then attach for a background
  session, the hand resume for a claude tab), refuses with a toast when a claude session runs elsewhere, and puts
  the new tab in the placeholder's place. It is saved as the tab it stands for, and `tab_runs` counts it as its
  session's tab, so the tree row's open action (m5/67 `Open::Tab`) shows it instead of opening a second tab.
  `open_row` itself is not used for the reopen: it needs a tree `SessionRow` and titles the tab `workspace · repo ·
  name`, while a reopened tab keeps its saved title.
- **Settings → Sessions** (`[sessions]`): `reopen_tabs` "Reopen last tabs on start" (default on, saved as unset) and
  `resume_tabs_on_click` "Resume reopened claude tabs" At once / On a click (default at once, René's decision).
- **Selftest 7x** (`selftest_reopen.rs`, before 7p while the run's own session lives): Work, shell, claude
  (resume of the fake transcript `RECENT` through the fake claude), attach (the run's own session), and a shell in
  a temp folder; Work dragged first, claude active; `tabs.json` must equal the snapshot. Restart in-process
  (`close_tabs_unsaved` closes every tab as a quit does, file unchanged, the folder deleted; `restart_tabs` reopens
  as a start does): 4 tabs in the same order with the same active tab, the fake claude ran `--resume <uuid>`, the
  attach tab re-attached, one notice names the dropped shell. Then with `resume_tabs_on_click`: the claude tab is a
  placeholder, nothing runs for 1 s, its Resume resumes it in place. An attach tab is closed only once `claude
  attach` has painted (a Ctrl+Z typed before it reads keys leaves `Detach::Failed`); a failed detach is retried.
- **Red checks** (reverted, never committed): the attach plan without the liveness filter →
  `an_attached_session_reattaches_only_while_it_runs_and_never_restarts_unasked` fails; `persist_tabs` never writing →
  7x "the five tabs in tabs.json: not within 15s"; the saved active tab ignored → 7x "after the restart … active:
  Some(0) … expected … active: Some(2)".
- **Gate** (`CARGO_TARGET_DIR=C:\fmbuild\m5-65\target`, master 590fdcc with m5/67 merged): build ok, no warnings;
  `cargo test --release --workspace` 698 passed, 0 failed; `--selftest` 5 of 5 PASS in a row (7x: 4 tabs back in
  30–37 ms) after one run failed step 7's frame budget (F-94, own work p10 17.6 ms while other units' Foremans ran);
  `--selftest-supervise` PASS; `--selftest-detach-shutdown` PASS; no `fm-selftest` session left. No m5-44 audit
  section names m5/65.

## 2026-10-30 — m5/66 refactor leftovers (F-14, F-18, F-48, F-49, F-50, F-54, F-66, F-87) and F-94

- **F-14 (A-05):** `foreman_workstate::read_reply` (`http.rs`) reads an answer whole (status, headers, body); the
  read API client, OAuth's `read` and the mint's `HttpMinter::send` all use it. A body that cannot be read is now a
  transport failure (`Unreachable`/`Transport`) in all three; OAuth and the mint used to take it as an empty body.
  Request bytes are unchanged. The mint's tests run on `FakeWorkState`, which now routes by `<METHOD> <target>`
  before the bare target and records `content_type`; the mint's private HTTP server is gone. `pct_encode` and
  `server_reason` were already shared (m5/44 U3).
- **F-18 (A-09):** `selftest::hex6` is `foreman_core::time::hex6`; no copy is left.
- **F-48 (A-42):** the Settings window lists `<home>\themes` (`themes::catalog`) and resolves a picked theme
  (`themes::resolve`) on the background executor; `themes::apply_resolved` paints on the UI thread. A pick counter
  drops a resolve that a later pick overtook. Step 7m waits up to 5 s for the pick to paint. The startup
  `apply_theme` stays synchronous: no window is shown yet.
- **F-49 (A-43):** `foreman_core::prune::prune(home, now, KEEP_MS)`, 30 days unmodified: `events/*.jsonl` and
  `.jsonl.1`, each workspace's `outbox/*.json` (never `ready.json`), `inbox/sent`, `inbox/rejected`, then the
  `answered.json` entries of gone outbox files. A drop still waiting in the inbox is never removed. The supervision
  runs it once an hour on the poller thread, against the real clock (its tests tick with a 2027 clock).
  `answered.json` writes now hold `answered.lock` (`mark_answered` and the prune). `EventReader::poll` drops the
  cursor of a log that is gone.
- **F-50 (A-44, the parts named in m5/66):** `Transcripts::listed` keeps only the project dirs still present;
  `DesiredState::retired` keeps the newest `RETIRED_KEPT` (100) names. The other A-44 parts belong to earlier units.
- **F-54 (A-48 `transcript.rs`):** a user line is found by its top-level `type` read with serde (`line_type`), so a
  spaced or reordered line counts and a nested `"type":"user"` does not; the serde read runs only while the first
  prompt is missing. The `"cwd"` prefilter no longer needs the colon. claude writes compact JSON, so
  `CACHE_VERSION` stays (no cached transcript reads differently).
- **F-66 (A-60 ledger):** two writers lost entries: `direct()` loaded `direct.json`, slept, delivered and saved
  the whole file. `record` now appends under `direct.lock` (load, push, save); the pacing read stays unlocked.
  Test `two_writers_lose_no_ledger_entry` (2 threads × 40): 80 kept; without the lock 40.
- **F-87 (A-81): kept, partial.** `HitTargets` already lives in `foreman_ui::hit` (m5/44 U9). Settings reads and
  calls `FleetView` for the WorkState status, the sign-in flow, the CLI probe, fonts and the terminal font size,
  because the main window shows and saves the same state; moving each to its own entity touches ~30 sites in
  `view.rs` and the selftests for no behaviour change. Recorded as staying until a feature needs one of them apart.
- **F-94:** `selftest/frames.rs`. `baseline` waits up to 60 s for a second with other processes under 35 % CPU
  (`BUSY_CPU`, was 75 %). `judge`: other processes at 35 % or more (baseline or phase) → PASS with `machine busy (X %
  CPU by others): frame budget not judged`; otherwise own work p10 over 16 ms FAILs as before, and the interval
  checks skip on a slow baseline as before. A slow-frame baseline alone (p50 36–43 ms at 15–22 % CPU, seen in every
  step 8 here) still judges own work. Test `a_busy_window_is_a_note_and_a_quiet_overrun_fails`.
- **Red checks** (reverted with `git checkout`, never committed): ledger without the lock → 40 of 80; the old byte
  match → the spaced-line test gets no prompt; no `listed` retain → the removed worktree dir stays; no cursor retain
  → 1 cursor left; prune without the `ready.json` exception → the prune test fails; no `retired` cap → 105 names;
  the own-work budget only when busy → the quiet-overrun test fails.
- **Gate** (`CARGO_TARGET_DIR=C:\fmbuild\m5-66\target`): build ok, no warnings; `cargo test --release --workspace`
  704 passed, 0 failed; `--selftest` 5 of 5 PASS (run 5 step 7: `machine busy (58 % CPU by others): frame budget
  not judged`); `--selftest-supervise` PASS; `--selftest-detach-shutdown` PASS; no `fm-selftest` session left.

## 2026-10-30 — m5/68 Open folder crashed Foreman: ShellExecuteW off the UI thread, panics logged

- **Cause, reproduced.** Open folder called `ShellExecuteW` inside the click's handler, on the UI thread. For a
  folder, shell32 calls into Explorer over COM and pumps the thread's messages while it waits. gpui's window
  procedure then runs queued foreground tasks (`WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD`), and a task that borrows the
  App panics (`RefCell already borrowed … async_context.rs:83`): the click's dispatch holds the App mutably. A panic
  inside an `extern "system"` window procedure cannot unwind, so the process aborts with `0xc0000409`, WER's BEX64
  subcode 7, as René saw. Stack: `ShellExecuteW` → `RealShellExecuteA` → `CallWindowProcW` → gpui's window procedure
  → a foreground task → `AppCell::borrow_failed`. A foreground task arriving within the pump is what a working
  Foreman always has (refresh, streams, terminals); the repro makes it certain with a 1 ms foreground loop.
  `ShellExecuteW` on a missing path or on a windowless exe (`rundll32.exe`) returns without pumping, so neither
  reproduces it.
- **Fix.** `FleetView::open_folder` runs the open on the background executor and puts a refusal into the action
  error (`Cannot open <path>: ShellExecuteW returned <code>`), as before. gpui's `open_with_system` also spawns, but
  only logs a failure. Sign-in already opened the browser on the background executor (`sign_in_to`). Elevate's
  `runas` runs in `main` before `application().run`, so no window exists to re-enter. Neither changed.
- **Seam.** `FleetView::open_with` (default `shell_execute("open", …)`). Selftest step 7b swaps in `pumping_open`,
  which pumps its calling thread's messages for 300 ms the way shell32 does and then calls the real `shell_execute`
  on a missing folder. So no Explorer window opens, and the step checks that the call ran off the UI thread, that
  the app stayed alive with foreground tasks running, and that the refusal reads as the action error.
- **Panic hook.** `foreman_ui::log_panics` (after `init_logger`, app and selftests) logs `thread '<name>' panicked at
  <file:line:col>:\n<message>\nbacktrace:\n…` as `[ERROR foreman::panic]`, then runs the hook that was set. In a
  release build the message and location are exact. The backtrace names the frames from `foreman.pdb`: std's frames
  carry file:line, ours only the nearest public symbol, sometimes wrong (no debuginfo in the release profile). Without
  the pdb beside the exe, our frames are bare addresses. `scripts/release.ps1` now ships `foreman.pdb`. A
  non-unwinding panic runs the hooks before it aborts, so its line is written first. Test
  `foreman-ui/tests/panic_log.rs`.
- **Red check.** `open_folder` calling the opener on the UI thread → selftest step 7b aborts with `0xc0000409`,
  `RefCell already borrowed`, stack `pumping_open` → `FleetView::run_action`.
- **Explorer tabs.** On Windows 11, a folder opened by ShellExecute can become a tab in an Explorer window that is
  already open. `WM_CLOSE` to the window titled with that folder then closes the whole window and its other tabs.
  Tests should not open Explorer at all.
- **Gate** (`CARGO_TARGET_DIR=C:\fmbuild\m5-68\target`): build ok; `cargo test --release --workspace` 705 passed,
  0 failed, 3 ignored; `--selftest` 5 of 5 PASS; `--selftest-supervise` PASS; `--selftest-detach-shutdown` PASS;
  no `fm-selftest` session left (`claude agents --json --all`).

## 2026-10-30 — sched/1 scheduler design and probes

- **Doc.** `docs/research/scheduler.md`: the schedule TOML, the approval (fingerprint + HMAC-SHA256 with key
  `foreman/schedule-approval/v1` in the DPAPI `FileStore`), the script contract, the run record, the life of
  `fm schedule fire`, Task Scheduler registration (COM `ITaskService` with XML), profiles and caps, and eight
  questions for the controller.
- **No new crate for the HMAC.** `hmac` 0.12.1 is already in `Cargo.lock` (through `hkdf`) and its `digest` 0.10
  matches the `sha2` 0.10.9 that `foreman-core` uses.
- **The supervisor never touches `fm-<ws>-sched-…`**: `reconcile` acts only on wanted or retired names. But
  `revoke_unwanted` revokes every name in `supervisor/<ws>.sessions.json` that is not wanted, so a scheduled run's
  minted session must stay out of `MintedSessions`. Such a run is not foreign, so it counts against `max_working`.
- **Console.** `fm.exe` and `foreman.exe` are console programs (the app `FreeConsole`s after it starts). A task
  running one shows a console window, so the task runs a GUI-subsystem `fmw.exe`.
- **Task folders.** `schtasks` and `Unregister-ScheduledTask` leave the folder behind; only COM
  `ITaskFolder::DeleteFolder` removes it.
- **DPAPI needs a logged-on token.** "Run whether logged on or not" without a stored password (S4U) has no DPAPI
  user key, so a schedule's task must be `InteractiveToken`.
- **git ownership.** `P:\Projects\LarchSys\AutoWork` is owned by `BUILTIN\Administrators`, its `.git` by René. The
  non-elevated fix is `git config --global --add safe.directory P:/Projects/LarchSys/AutoWork`, set once by René. Until
  then `git_repo()` returns `None` silently, so a mint carries no repo binding.
- **Probes**, run by the controller under René's grant at 16:59 (`register-probe.ps1`; the unit's own `schtasks /create`
  had been refused by the auto-mode classifier):
  - **(a)** The task ran unelevated, with no console window. `claude --bg`, `agents --json --all` and `stop` worked.
    The service at the time (pid 49204) was **elevated**, yet the unelevated client reached it on 2.1.314; this
    contradicts m5/25 (2.1.311). From the unelevated side `claude daemon status` reads `NotRunning`, so
    `daemon_mismatch()` cannot detect the elevated service there.
  - **(b)** DPAPI `FileStore` read and an OAuth refresh (against a fake server) worked from the task.
  - **(c)** git exited 128, "dubious ownership" (A2 confirmed).
  - **(d)** WakeToRun and StartWhenAvailable were read back; wake timers are enabled; `powercfg /waketimers` showed
    none while the machine was awake. The wake itself was not tested.
  - The task and its folder are gone.
- **Gate** (`CARGO_TARGET_DIR=C:\fmbuild\sched-1\target`): build ok; `cargo test --release --workspace` 705 passed,
  0 failed, 3 ignored; selftests skipped (docs-only unit).
## 2026-10-30 — m5/61 Adopt: an unmanaged background session continues as `fm-<ws>-<folder>`

- **claude 2.1.314 cannot rename a session, so Adopt makes a copy under a new id.** Probed on own sessions in
  `target/m5-61-probe`. `claude --bg --resume <uuid>` with no other flag wakes the same uuid "with its saved
  options": its old `--name`, its old `--mcp-config`, and no `FOREMAN_*` env. So it gets neither the supervised name
  nor a mint. `claude --bg --name fm-x --permission-mode … --resume <uuid> -- <prompt>` printed `note: background
  session fc5bf027 keeps its own saved options, so the flags you passed started a copy as f050c443`. The copy has a
  new `sessionId`, its transcript is written under the new name (`custom-title`/`agent-name`), and it carries the
  whole conversation: asked, it answered the word the original was told to remember. The original row stays (`done`,
  no pid). The CLI has no rename command, and the daemon's roster is not Foreman's to write. The brief's "same UUID
  runs as fm-…" therefore became "the conversation continues as fm-…, a copy under a new id" (controller decision A,
  2026-10-30; René can veto).
- **Flow.** The session menu's `Adopt…` (`menu-adopt`) asks first (`Confirm::ask`): "Adopt <name> (<id>) into <ws>?
  It is stopped (stop, never rm), then its conversation continues as fm-<ws>-<folder>, supervised: a copy under a new
  id with a fresh WorkState token. The original stays stopped and listed." When the workspace is not started, it
  adds that the copy starts on Start. Yes stops it (`claude stop`, off the UI thread). Only then does
  `supervise::adopt` want `fm-<ws>-<folder>` through `DesiredState::update`, as `adopted_session`: the repo
  session with `fork_from = <full uuid>` and the rendered repo role as its prompt for a later fresh launch. The
  supervisor's launch of a wanted session with `fork_from` is `LaunchSpec { resume: Some(uuid) }` (argv `--bg --name
  <n> --resume <uuid> …`), with every supervised flag, a fresh mint, and the adopt prompt. A live row of that uuid
  fails the launch ("runs again: stop it, then Retry"), so two sessions never go on from one history.
  `Supervisor::settle_forks` clears `fork_from` (`DesiredState::forked`, in the file too) once the name's row shows
  another uuid. From then on it is an ordinary wanted session, resumed by its own uuid. The original is never rm'd.
- **The prompt sent into the copy** (`adopt_prompt`): "Foreman adopted this conversation: from now on you are
  fm-<ws>-<folder>, the repo session for <folder> in the Foreman workspace <ws>, supervised by Foreman. Follow the
  FOREMAN-ROLE instructions of this session's start as after a resume." The role text itself is not sent: `fm hook`
  injects it at SessionStart (`FOREMAN_ROLE=repo` comes in the copy's `--settings` env), and `repo.md` already says
  what to do after a resume. Sending the launch prompt as well would put the role in twice.
- **When there is no Adopt** (`fleet::adoption`): it is offered for a background session (not `fm-`, not muted,
  not closed) that is running, idle, blocked or stopped, under a folder enabled here that this workspace supervises
  (`owners`). This holds on any occurrence of the row, not only the first. In the selftest, YPlain lies inside step
  v's folder (this checkout, git, by then disabled), so the session shows first under v's checkout, because a
  disabled folder stays in the tree while it holds a session (m5/56 c5). Its YPlain row is the `@<ws>` occurrence.
  Only the folder the row sits in decides. An orphaned row is excluded because the daemon may still respawn it. If
  `fm-<ws>-<folder>` is wanted or has a row, the menu has no Adopt and the session's hover card says why (`adopt
  none: fm-… is wanted already, and a folder has one supervised session`). Replacing it was not chosen: it would
  stop a supervised session, which needs its own question. Muted rows (another Foreman's) have no Adopt.
- **An interactive session has no Adopt.** Its `claude agents --json` row has no `id`, and `claude stop <sessionId>`
  answers `No job matching '<id>'. Run 'claude agents' to list running sessions.` (exit 1). `stop` addresses only
  background jobs, so Foreman cannot stop a session that runs in someone's terminal. (Measured with an unknown uuid;
  no interactive session was started, because that needs a console.)
- **A failed copy**: reconcile's `LaunchFailed`/`GaveUp` for a wanted session with `fork_from` carry
  `Remedy::Adopt { from }`, and the detail starts with `adopting <id8>: `. The Inbox row shows Retry
  (`inbox-retry-<name>`, as before) and Start fresh (`inbox-fresh-<name>`: `DesiredState::start_fresh` drops
  `fork_from`, then Retry). The folder's row shows a warning tag `adopt failed` (`adopt-failed-<ws>/<folder>`) with
  the reason and "Retry or Start fresh in the Inbox". The original stays stopped. The session row's primary action
  (`fleet::Open::of`, also the double-click) is unchanged: Adopt stops a session, so it belongs behind a menu item
  and a question, not behind a double-click.
- **Tests:** `desired::an_adopted_session_forks_until_its_own_row_shows_and_start_fresh_drops_the_fork`,
  `reconcile::an_adopted_session_is_launched_as_a_copy_and_a_failed_copy_offers_retry_or_start_fresh`,
  `executor::an_adopted_session_launches_a_copy_of_its_uuid_with_the_supervised_flags_and_forks_once` (no copy while
  the original is live; `--resume <uuid>` right after `--name`, the mint, the adopt prompt, never a Resume of the
  original's row; `fork_from` kept while the row shows the original's uuid and cleared in memory and file once it
  shows its own; later resumed as itself), `launch::a_launch_that_resumes_names_the_session_right_after_its_own_name`,
  `fleet::adopt_is_offered_for_an_unmanaged_background_session_of_an_enabled_folder_without_a_supervised_one`,
  `supervise::adopt_wants_the_folders_session_forking_the_uuid_with_the_repo_role`, the menu and card tests.
- **Selftest step y** (`selftest_supervise::adopt_step`, after x): workspace `<testbed>-y` with the plain folder
  `YPlain` (inside the testbed), started, wanting nothing. The run launches its own unmanaged `selftest-<hex>-adoptee` there (default
  mode, no MCP), which is told a word to remember, and waits for its first turn to end. Then, through the window:
  right-click its row under YPlain (`session-row-<sid>@<ws>`, see above) → `menu-adopt` → the question (checked for the name, "stop, never rm", "a copy under a new id",
  "The original stays stopped and listed") → `confirm-yes`. It checks that `fm-<testbed>-y-yplain` is live with
  another sessionId, the original is stopped with its id and name, `fork_from` is cleared in the file, and the copy's
  transcript carries the word and the adopt prompt. It also checks that the original's card now says why there is
  no Adopt. Cleanup removes the adoptee through `Scratch::session_name` and the copy by the run's prefix.
- **Red checks** (each reverted, never committed): the fork launch without `spec.resume` →
  `an_adopted_session_launches_a_copy_…` fails; `settle_forks` not called → the same test fails (`fork_from` never
  clears); a failed copy offering `Remedy::Retry` → the reconcile and executor tests fail; `adoption` without the
  "wanted already" check → `adopt_is_offered_…` fails. Step y itself failed twice before passing. First, Adopt
  required the first occurrence and the row under YPlain was a second one ("nothing painted under menu-adopt"). Then
  its last check looked the stopped original up in the tree, which hides stopped rows.
- **Not covered:** a failed copy is not driven in the selftest (Retry, Start fresh and the folder's `adopt failed`
  tag have unit tests for their logic only). The live-original refusal is unit-tested.
- **Gate** (`CARGO_TARGET_DIR=C:\fmbuild\m5-61\target`): build ok, no warnings; `cargo test --release --workspace`
  711 passed, 0 failed, 3 ignored; `--selftest` 5 of 5 PASS; `--selftest-supervise` PASS (step y included);
  `--selftest-detach-shutdown` PASS; no `fm-selftest` or adoptee session left. No m5-44 audit section names m5/61,
  so no audit finding was in scope.

## 2026-10-30 — sched/2 the schedule model: `foreman_core::schedule`

- **Module.** `crates/foreman-core/src/schedule/` (`mod.rs` types and `validate`, `store.rs` load/edit, `approval.rs`
  fingerprint, MAC and `approve`, `next.rs` next fire, `runs.rs` run records and the schedules' part of `prune`).
  The file is `<home>/schedules/<ws>.toml`, edited only through `schedule::edit(home, &Workspace, closure)` under
  `schedules/<ws>.lock` + `write_atomic`: a closure, not an `Edit` enum, since a schedule has many fields.
- **What `edit` enforces (C100).** It puts back `[schedule.approval]` as it was, whatever the closure did; a change of
  any covered field sets `pending`; `active` holds only when the stored approval's fingerprint equals the current one
  (so resuming a paused, still-approved schedule works, and nothing else activates). A covered edit stays `pending`
  even when it restores the approved value: only Approve moves it back.
- **One covered struct.** The edit's "covered changed" check and the fingerprint share one serde struct
  (`CoveredJson`), the edit's copy without files or the command path, so a field cannot be covered by one and not the
  other. A slash command is covered by `{"name","path"}` (the folder's `.claude/commands/<a>/<b>.md` for `/a:b`, else
  the user's when that exists); its content hash goes to `approval.command_sha256`, not to the fingerprint.
- **`approve` is in core**, as the brief asks (the doc lists it under sched/7): it creates the key under
  `schedules/approval.lock` on the first approval (`getrandom` 0.3, 32 bytes, hex, `APPROVAL_KEY`), writes the approval
  and sets `active`. Only the window's Approve may call it. `verify_approval` is checks 2 and 3 of a fire;
  `effective_status` reads an `active` one that does not verify as `pending`.
- **No new package.** `hmac` 0.12.1 and `getrandom` 0.3 are new direct dependencies of `foreman-core`, both already in
  `Cargo.lock`; chrono gains the `clock` feature `foreman-ui` already enables.
- **Time zones without `chrono-tz`.** `next_fire`/`previous_fire` are generic over `chrono::TimeZone`; the app passes
  `Local` (`next_fire_local`). The DST tests use a test-only Europe/Berlin zone with the EU rule (last Sunday of March
  and October, 01:00 UTC), so they do not depend on the machine. An interval steps in wall-clock time from `at` while
  under 24 h, and restarts at `at` each day; a gap collapses its fires into the first minute after it.
- **`git rev-list --exclude` with `--branches` takes the name below `refs/heads/`.** `--exclude=refs/heads/x` matched
  nothing, so a worktree's own branch counted as "elsewhere" and a worktree with a commit only it had was removed. The
  prune test caught it; `--exclude=<branch>` is right.
- **Prune (Decision 8).** The run worktrees are found on disk (`<folder>/.claude/worktrees/fm-sched-*` of every
  registered folder), not from run records, which age out themselves. One is removed (`git worktree remove`, no
  `--force`, then `branch -D`) only when `status --porcelain` is empty and `rev-list HEAD --not --exclude=<branch>
  --branches --remotes --tags` counts 0; the rest stay, listed by `run_worktrees` for sched/7. The state dir of a
  removed schedule goes at once; a workspace whose schedule file does not parse keeps all of its state.
- **The `sched-` slug.** `repo_session_name` returns `None` for a slug starting with `SCHED_SLUG_PREFIX`.
- **A checkout named `sched-…` hit the reservation.** `--selftest-supervise` step v named its git folder after the
  checkout, so in worktree `sched-2` it failed "no name for sched-2". The step now names that folder `VGit`
  (like its `VPlain`), so the selftest no longer depends on where it is checked out.
- **Gate** (`CARGO_TARGET_DIR=C:\fmbuild\sched-2\target`): build ok; `cargo test --release --workspace` 727 passed,
  0 failed, 3 ignored; `--selftest` 5/5 PASS; `--selftest-supervise` PASS (after the `VGit` fix);
  `--selftest-detach-shutdown` PASS; no `fm-selftest` session of this worktree left.
