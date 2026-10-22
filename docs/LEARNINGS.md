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
