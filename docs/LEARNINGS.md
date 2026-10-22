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
