# Foreman — plan

A native Windows-first app (Rust + gpui) where you register a **workspace** (a folder of git repos),
press **Start**, throw tasks/roadmaps at it, and watch a tree of Claude Code sessions build them.
Everything that needs a human (a permission park, a decision, a migration) surfaces in the app, and
**Attach** opens that session in an in-app terminal tab.

The repo folder is still `AutoWork`; the product, crates (`foreman-*`) and CLI (`fm`) are Foreman.
Measured facts behind every decision here are in [`LEARNINGS.md`](LEARNINGS.md); the work record is
the WorkState project `autowork` (runs `m0`, `ws`).

---

## 1. The one principle

**Deterministic supervision lives in Rust. Judgement lives in skills.**

The draft had an LLM (workspace-controller) watching other LLMs, respawning them, telling them about
each other, and a cron job asking it for a password. Everything in that list except "coordinate the
work" is deterministic, and an LLM doing deterministic work is slow, costs tokens, and drifts after
compaction — the exact failure the health-checker was trying to catch.

| Concern | Draft owner | Owner |
|---|---|---|
| start / restart / stop controllers | workspace-controller | **Foreman supervisor** (reconcile loop) |
| "who are the other repo-controllers" | workspace-controller tells them | **roster** served by Foreman (`fm roster`), always current |
| liveness, blocked-on-permission | cron + ask the LLM | `claude agents --json --all` (`waitingFor`) + Claude Code **hooks** (`Notification`) — both proven in M0 |
| skill drift after compact | password-phrase probe | **`SessionStart` hook, source `compact`**: its stdout reaches the model (proven for manual compact in M0) |
| decompose a roadmap across repos, sequence, escalate | workspace-controller | workspace-controller (unchanged) |
| run a repo's queue, verify, land, own its DBs/deploys | repo-controller | repo-controller → `/rlarch:controller` per work package (unchanged) |

---

## 2. What exists and gets reused

- **`/rlarch:controller`** + **`/rlarch:nested-claude`** (Skills, `feat/workstate`) — the discipline and the
  mechanics. M0 itself was run with them: 9 nested agents, every unit verified by the controller. The
  Rust launcher ports `Start-NestedAgent.ps1` and keeps its trap fixes. One more learned in M0: resolve a
  session's id from the `agents --json --all` row whose name you chose, never from free text.
- **WorkState** (`Skills/mcp`, .NET 10 + Postgres in Docker, HTTP MCP on :5290) — the system of record.
  Reused as a .NET service, not rewritten. Using it for M0 found 9 defects; 7 are fixed and live on
  `feat/workstate` (ws/1–ws/7: empty-server stream, worker brief complete and worker-shaped, notes never
  consumed unseen, every DB guard a teaching refusal, exact criteria writes, qualified assumption refs).
  Open: ws/8 claim-per-worktree (decided by the human), F-05 record-note threads.
- **`claude` CLI 2.1.308** — `--bg`, `--name`, `agents --json [--all]`, `attach`, `logs`, `stop`, `rm`,
  `respawn`, `--plugin-dir`, `--mcp-config`, `--resume <id> -p`. The whole process API.

New: the app, the supervisor, the `fm` CLI, the relay, and two skills (`workspace-controller`,
`repo-controller`).

---

## 3. Architecture

```
┌──────────────────────── Foreman.exe (gpui) ─────────────────────────┐
│  UI: workspaces tree · work browser · inbox · terminal tabs        │
│        ▲ state (entities)                                          │
│  supervisor: reconcile desired vs observed, restart w/ backoff,    │
│              concurrency budget, health ladder                     │
│        ▲              ▲               ▲                 ▲           │
│  claude adapter   hook ingest     relay files      workstate client │
│  (agents --json   (fm hook →      (inbox/outbox)   (REST + SSE)     │
│   --all, --bg…)    local sink)                                      │
└──────┬───────────────┬───────────────┬─────────────────┬───────────┘
       │ spawns         │ hooks          │ SendMessage     │
  claude --bg sessions ─┘     fm-<p>-relay ─┘ (native)      │
  fm-<p>-workspace                                 ──MCP──▶ WorkState (.NET, docker, :5290)
  fm-<p>-<repo>                                            └ Postgres
  fm-<p>-<repo>-<unit>  (launched by repo-controllers via nested-claude)
```

### Cargo workspace

| crate | job |
|---|---|
| `foreman-core` | domain types (Workspace, Repo, Session, Role, Health), TOML config, the repo scanner, the relay's files (`relay`: `send`, `read_outbox`, `relay_ready`, the `fm inbox` watcher) |
| `foreman-claude` | the only code that knows the `claude` CLI: launch, poll `agents --json --all`, `logs`, `stop`, `rm`, `respawn`; version check so a CLI change fails loudly |
| `foreman-supervisor` | reconcile loop, restart policy, health evaluation, hook ingest, relay, concurrency budget. **No UI dependency** — liftable into a tray daemon. m2/3: desired state (`<FOREMAN_HOME>/supervisor/<workspace>.toml`), pure `reconcile`, backoff memory, `Supervisor::tick` over a `Launcher` trait; deps foreman-core + foreman-claude only |
| `foreman-workstate` | typed client for WorkState REST/SSE; brings the compose stack up on tear-up and checks the MCP registration from each repo's cwd |
| `foreman-term` | Zed's `terminal` vendored without `settings`/`theme_settings`/`task`/`release_channel` (Variant B, M1) + a grid painter adapted from `terminal_view/src/terminal_element.rs` |
| `foreman-app` | the gpui application |
| `fm` (bin) | tiny CLI hooks and sessions call: `fm hook <event>`, `fm want`, `fm unwant`, `fm roster`, `fm inbox` (the relay's watcher), `fm send`, `fm messages` |
| `plugins/foreman/` | the two skills + `hooks.json`, loaded with `--plugin-dir` so every session runs the version Foreman ships |

### Terminal — decided: `foreman-term` (Variant B, m1/1)

`crates/foreman-term` is Zed's `terminal` @ `03efd492` vendored with `settings`, `theme_settings`, `task` and
`release_channel` cut (a plain `TerminalConfig` global + `TerminalPalette` instead). Measured against Variant
A (Zed's crates as git deps): identical runtime — ~98 MiB private, p50 16.67 ms / p95 ~18 ms, 0 frames over
50 ms in 12 runs incl. a concurrent `cargo build`, keystroke-to-grid 106–137 ms — while B needs **no CMake**,
cold-builds in 103 s vs 187 s, pulls 350 crates vs 487 and ships 12.0 MB vs 17.3 MB. `gpui`/`util`/`collections`
stay git deps, so `Cargo.lock` is still re-seeded from Zed's lockfile on a rev bump. M0's one 896 ms frame did
not reproduce for either variant; untested: inactive window, a .NET build as load, link-phase load.
Variant A is retired (m1/2): `term-spike` builds only `foreman-term`, and nothing in the workspace needs CMake.

### Session identity and permissions

- Names are the address and the join key: `fm-<workspace>-workspace`, `fm-<workspace>-<repo>`,
  `fm-<workspace>-<repo>-<unit>`, `fm-<workspace>-relay`. The app filters `agents --json --all` by prefix.
- Every session is launched with `--mcp-config <FOREMAN_HOME>/mcp/workstate-<session name>.json --strict-mcp-config`
  (m3/2; one file per session, was per role). The file holds the role's token (workspace → controller, repo →
  orchestrator, unless the role has its own line) and the session's WorkState identity headers
  (`X-WorkState-Agent: <name>`, `-Launched-By: foreman/<w>`, `-Origin: foreman/<w>[/<repo>]`, `-Cwd`,
  `-Foreman-Role`; Skills `docs/workstate/11-sessions.md`); no `X-WorkState-Session`, because `--bg` ignores
  `--session-id` and the uuid is unknown at launch. It is created owner-only (one ACE: the user, full control)
  before it is filled, and only its path appears in argv and in the daemon's `roster.json`
  (`LaunchSpec::with_session_token`); a resume re-applies it. M0 found that nested sessions otherwise inherit
  the user-scope registration — a worker could write evidence as the controller.
- Repo sessions start on demand: `fm want <w> <repo> [--prompt-file]` adds `fm-<w>-<repo slug>` (role `repo`,
  cwd = the repo, auto, `bgIsolation:"none"`) to the desired state, `fm unwant` retires it (stopped, never rm),
  `fm roster <w> [--json]` lists the workspace's sessions. Controllers run `fm` themselves: the plugin's `bin/`
  is on the session's PATH and every launch passes `--allowedTools "Bash(fm:*)" "Bash(<abs fm>:*)"`.
- `settings.toml` `extra_plugin_dirs` are passed as more `--plugin-dir` after Foreman's own (the rlarch plugin
  for `/rlarch:controller`). Env on `claude --bg` never reaches the session, and
  `--settings` env persists the value in `~/.claude/daemon/roster.json`, so no env-var route (m2/1).
- Every session is launched with `--plugin-dir` pinned to Foreman's skills (the installed rlarch plugin
  lagged the branch by a major version).
- Auto mode everywhere; the brief is the authorization (controller §2). Some acts are refused by the
  classifier however they are authorized — M0's peer-protocol research was refused at launch even with
  the human's words quoted. Design for the supported route; never route around a refusal.

### Health signals, cheapest first (all measured in M0)

1. **process** — row present in `agents --json --all`; `state`: `working` → `blocked` → `stopped`; `pid` and
   `status` exist only while the process lives. Plain `--json` drops stopped rows, so use `--all`.
2. **blocked** — `waitingFor: "permission prompt"` + `status: waiting`, and a `Notification` hook
   (`notification_type: permission_prompt`) 1–3 s earlier → Inbox row with **Attach**.
3. **stalled** — busy but no transcript growth / hook events for N minutes. Exception: a session waiting
   on a `Monitor` (the relay) shows `busy` forever and costs nothing — health is "its watcher is alive".
4. **drifted** — a `SessionStart` hook with `source: compact` → its stdout re-injects the role skill.
   Every `--resume` also fires `SessionStart(resume)`, so the injected text must be idempotent and tagged.
   Built (m2/2): `plugins/foreman` (loaded with `--plugin-dir`) runs `fm hook <event>` for SessionStart,
   UserPromptSubmit, Notification, Stop, PreCompact; each appends one line to
   `<FOREMAN_HOME>/events/<session_id>.jsonl` (read with `foreman_core::EventReader`), and SessionStart prints
   `FOREMAN-ROLE <role> v<sha8>` + `<FOREMAN_HOME>/roles/<FOREMAN_ROLE>.md`. Proven after a real auto-compaction:
   the compact-time injection is in the model's context on the next turn. A launch needs `--plugin-dir`,
   `--settings '{"env":{"FOREMAN_HOME":…,"FOREMAN_ROLE":…}}'` and `fm plugin install` once per fm build.
5. **work health** — from WorkState: expired leases, blocked items, open blocker findings.

### The mailbox — decided: a relay session

Foreman is not a Claude session and cannot call `SendMessage`. The peer-pipe protocol was refused as
research (classifier), so: Foreman launches `fm-<p>-relay`, which arms `Monitor` on `fm inbox`; Foreman
drops `{to, body}` files, the relay sends them with native `SendMessage`, and every message sent *to* the
relay comes back as an outbox file. Measured: ~6 s app→target, ~14–18 s round trip, ~5 model calls per
round trip, zero while idle, no held messages across permission modes. Controllers reach Foreman
with `SendMessage("fm-<w>-relay", …)`.

Built (m3/1): one relay per workspace, `fm-<w>-relay` (role `relay`, `plugins/foreman/roles/relay.md`), a wanted
session of the workspace's desired state, launched before `fm-<w>-workspace` on Start and stopped after it on
Stop; cwd = the workspace root, `--add-dir <FOREMAN_HOME>/relay/<w>`, auto, `--strict-mcp-config` and no
WorkState; outside the busy budget. Files `<FOREMAN_HOME>/relay/<w>/{inbox,outbox}`: `foreman_core::send` (= `fm send
<w> <to> <body>`) drops `{to, body}`; the relay's Monitor runs `fm inbox --dir <inbox>`, which prints each drop as
one JSON line and moves it to `sent/`, and the relay calls `SendMessage` straight from that event; everything
sent to the relay comes back as `outbox/msg-NNNN.json` (`read_outbox`, `fm messages <w>`); `ready.json` is
rewritten on every arm. A resumed relay is told to re-arm. The app shows relay health on the workspace row
(live + armed within 31 min) and a Messages section under the Inbox (sender, time, first lines, click for
all, Attach to the sender).

### Data placement

- workspace registry: `%APPDATA%\Foreman\workspaces\<name>.toml`
- per workspace desired state (Start/Stop, wanted sessions): `%APPDATA%\Foreman\supervisor\<name>.toml`;
  app settings (busy limit, WorkState `.env`): `%APPDATA%\Foreman\settings.toml`
- runtime session state: in memory, rebuilt from `agents --json --all` + hooks (sessions outlive the app)
- work, plans, evidence, journal, notes: WorkState (one project per workspace; a roadmap is a run)
- a home written while a workspace was called a platform (before m2x/2) is migrated by
  `foreman_core::migrate_home` at app start and before every `fm` registry command

---

## 4. Flows

**Create workspace** — pick `<dir>` → scan for folders whose `.git` is a **directory** (a `.git` *file* is a
worktree; `*.worktrees/` skipped) → review, untick → save. `P:\Projects\Magic\Nomos` yields 13 repos.

**Start** — WorkState up + registration checked → `fm-<p>-relay` → `fm-<p>-workspace` with
`/foreman:workspace-controller` → **repo-controllers start on demand** when the workspace-controller assigns
that repo work.

**Throw a task** — text box → relay → workspace-controller decomposes into per-repo runs with cross-repo
`blocks` → one approval in the app → repo-controllers run their queues with `/rlarch:controller`.

**Something needs you** — permission park, a `blocked` item with a decision reason, a migration/deploy
escalation → Inbox row → **Attach** opens `claude attach <id>` in a terminal tab.

**Stop / tear down** — workers → controllers → workspace → relay (the supervisor stops a relay after the
other wanted sessions of the tick). Stop before removing a worktree; run
`dotnet build-server shutdown` (or the toolchain's equivalent) first and retry the delete after ~3 s — a
stopped session or a build server holds handles briefly (seen on every unit in M0).

---

## 5. Skills to write (`plugins/foreman/skills/`)

Written (m3/3); `roles/workspace.md` and `roles/repo.md` only say who the session is, invoke the skill, and
what to do on start and after a compaction (re-invoke, `work_brief`).

- **`workspace-controller`** (`/foreman:workspace-controller`, session `fm-<w>-workspace`, controller token) —
  planner and lead. On start it announces `ready` to the relay once and waits (messages wake it; no polling).
  A task from `fm-<w>-relay` (the only voice of the human): reads the repos (`fm workspace show`, `fm roster`,
  the code) → WorkState project `<w>`, one run per repo `<task-slug>-<repo-slug>` with units (criteria + a
  verify that can fail), an integration run `<task-slug>` holding `<task-slug>-verify`, cross-repo `blocks`
  → **one approval** (`APPROVE?` + `work_tree` to the relay, waits for the answer) → `fm want <w> <repo>
  --prompt-file .foreman/prompts/<run ref>.md` for each repo with unblocked work (and a `SendMessage` when the
  session had a row before, since a resumed session gets no prompt) → answers what is its own, forwards the
  human's decisions as `DECIDE:` and passes answers back verbatim → runs the cross-repo check itself, reports
  `DONE:` in outcomes, `fm unwant`s idle repos. Never writes code, runs nested agents, restarts or stops
  sessions, approves for the human, or runs `fm send` (it speaks as the human).
- **`repo-controller`** (`/foreman:repo-controller`, session `fm-<w>-<repo-slug>`, orchestrator token) — runs
  the run it is handed with `/rlarch:controller` verbatim (the run and the approval exist already; nested
  agents `fm-<w>-<slug>-<unit>` in `../<repo>.worktrees/<unit>`, merge into the repo's checkout, close with
  evidence, never push). Escalates migrations, deploys, prod data, dependencies, public contracts, undecided
  questions to `fm-<w>-workspace` (never the relay) as `BLOCKED <unit>: …`, unit `blocked` meanwhile;
  reports `DONE <run>: …` with the merge shas. A direct request with no run is answered where it came from.
  Known gap: nested agents get no per-session WorkState config yet (skills-5d), so they must never close,
  verify or report their own unit.
- `controller`, `nested-claude` — unchanged, from rlarch (needs `extra_plugin_dirs`: the installed rlarch
  plugin has neither, m3/3).

---

## 6. Milestones

**M0 — prove the foundations — done** (run `m0`, 5/5 units verified by the controller, 4 follow-ups in the
backlog). WorkState fit and fixed; state model, hooks, relay and terminal measured; see §3.

**M1 — fleet viewer (read-only) — done** (run `m1`, 7 units; master `3448f97`). `foreman.exe`: left panel
workspaces → repos → sessions with health (live, every 2 s), right panel terminal tabs; Attach opens
`claude attach <id>`, closing a tab detaches with Ctrl+Z and never kills anything. `fm workspace add|list|show|
remove`, `fm sessions`. Crates: `foreman-core`, `foreman-claude` (read side), `foreman-term`, `foreman-ui`,
`foreman-app`, `fm`. Health derives **liveness from the pid**; `state` is the current turn (`done` + live pid =
idle). Open: the human hand check (backlog 9-d).

**M2 — supervisor — done** (run `m2`, units 1–4; master `dc72158`; gate from the main checkout: 252 tests,
`--selftest-supervise` 3/3, `--selftest` 2/2, `--selftest-detach 10` 10/10). Open: René's hand check on a real workspace.
Launcher port (`LaunchSpec`, role token in an owner-only `--mcp-config`, `--plugin-dir`), `plugins/foreman` +
`fm hook` (event log, role re-injection; auto-compact re-injection proven, 6-d), `foreman-supervisor` (pure
reconcile, restart with backoff, respawn grace, trust-needed, give-up), and in the app: **Start/Stop per
workspace** (desired state persisted in `<FOREMAN_HOME>/supervisor/`, supervision resumes on app start), the
**Inbox** (blocked → Attach, launch failed / gave up → Retry, trust needed → Run in terminal; Foreman never
answers the trust prompt), **one busy-session limit shared by all workspaces** (`settings.toml` `max_working`,
stepper in the app), hook signals on supervised rows. `foreman.exe --selftest-supervise` is the gate.
Workspace sessions are `bgIsolation:"none"`; the workspace role is still the M2 placeholder.

**M3 — skills + relay — done** (run `m3`, units 1–4, 6, 8; master `75c3bf1`; gate from the main checkout: 297 tests,
`--selftest` PASS, `--selftest-supervise` PASS 3 of 4, the miss being the classifier flake at step r, 7-d2). The relay `fm-<w>-relay` (§3 "The mailbox";
`fm inbox`/`send`/`messages`, app Messages), repo sessions on demand (`fm want|unwant|roster`, one WorkState config
per session with identity headers, extra plugin dirs, busy = `status=busy`), the `workspace-controller` and
`repo-controller` skills (§5), `fm send` refused inside a Foreman session. **First end-to-end task (m3/4)** on
two throwaway repos under `target/e2e`: thrown → plan → approval in the app → both repo-controllers ran a nested
unit each and merged → workspace-controller verified and reported DONE; ~12 min of work, Stop in 3.3 s (LEARNINGS
"m3/4"). Found there and fixed in m3/8: the skill listing is truncated, so sessions call the Skill tool for
unlisted rlarch skills; a Foreman session waiting on a peer is not an Inbox "blocked". Human step per new repo:
accept Claude's trust prompt. Backlog from René's look at the UI: terminal wheel scroll, block glyphs, `NO_COLOR`.

**M4 — work browser**: native views over WorkState (tree, journal, threads, evidence), task box.

**M5 — hardening**: per-workspace cost budget, tray daemon, Nomos as first real workspace.

---

## 7. Risks

- **Usage limits.** The supervisor needs a global cap on concurrently busy sessions from M2.
- **`claude` CLI surface** (`agents --json` fields, `--bg`) is not a stable contract; all of it behind
  `foreman-claude`, version-checked.
- **Classifier refusals.** Anything that looks like credential access or guard-weakening is refused for
  nested sessions whatever the brief says. Design for supported routes.
- **Depth.** workspace → repo → controller → orchestrate subagents: four hops. Complete worker briefs from
  the store (fixed in ws/2–ws/4) are what keeps it honest.
- **Closing the app stops supervision** until M5's daemon.
