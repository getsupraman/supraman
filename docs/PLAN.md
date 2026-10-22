# AutoWork — plan

A native Windows-first app (Rust + gpui) where you register a **platform** (a folder of git repos),
press **Start**, throw tasks/roadmaps at it, and watch a tree of Claude Code sessions build them.
Everything that needs a human (a permission park, a decision, a migration) surfaces in the app, and
**Attach** opens that session in an in-app terminal tab.

---

## 1. What I'd change in the draft — the one principle

**Deterministic supervision lives in Rust. Judgement lives in skills.**

The draft has an LLM (platform-controller) watching other LLMs, respawning them, telling them about
each other, and a cron job asking it for a password. Everything in that list except "coordinate the
work" is deterministic, and an LLM doing deterministic work is slow, costs tokens, and drifts after
compaction — the exact failure the health-checker is trying to catch.

| Concern | Draft owner | Proposed owner |
|---|---|---|
| start / restart / stop controllers | platform-controller | **app supervisor** (reconcile loop) |
| "who are the other repo-controllers" | platform-controller tells them | **roster** served by the app (`aw roster`), always current |
| liveness, blocked-on-permission | cron + ask the LLM | `claude agents --json` + Claude Code **hooks** pushing to the app |
| skill drift after auto-compact | password-phrase probe | **`SessionStart` hook with `compact` source** re-injects the skill; canary probe kept as a secondary check |
| decompose a roadmap across repos, sequence, escalate | platform-controller | platform-controller (unchanged) |
| run a repo's queue, verify, land, own its DBs/deploys | repo-controller | repo-controller → `/rlarch:controller` per work package (unchanged) |

So the platform-controller becomes a **planner/coordinator**, not a babysitter. It is still the lead
the repo-controllers listen to, but nobody depends on it to stay alive.

---

## 2. What already exists and gets reused

- **`/rlarch:controller`** (Skills, `feat/workstate`) — the per-work-package discipline: queue in
  WorkState, brief, auto-mode launch, verify, land, record. Used as-is by repo-controllers.
- **`/rlarch:nested-claude`** — the mechanics (`claude --bg`, naming, READY/BLOCKED/DONE, launch
  traps). The Rust launcher is a port of `Start-NestedAgent.ps1` and must keep every trap it fixed
  (prompt as one quote-free line naming a brief file, variadic flags closed, brief under
  `.claude/briefs/`, refuse a `-RepoRoot` that is not a working tree, auto mode by default).
- **WorkState** (`Skills/mcp`, .NET 10 + Postgres, HTTP MCP on :5290) — the system of record for
  work: items (`Open…Done`), claims/leases, evidence (`work_verify` vs `work_report`), findings,
  notes, an append-only journal, and already a REST read side + SSE `/stream`. **Reuse it as a .NET
  process under the hood; do not rewrite it.** The rules-in-Postgres design and the acceptance runs
  are worth more than having one language. The app talks to it over REST + SSE for reads; agents keep
  writing through MCP.
  - Note: `feat/workstate` is **ahead of `master`** (master merged an earlier tip). Build from
    `feat/workstate`.
- **`claude` CLI 2.1.x** — `--bg`, `--name`, `agents --json` (has `state: "blocked"` for background
  sessions, `status: idle|busy` for interactive), `attach`, `logs`, `stop`, `rm`, `respawn`,
  `--plugin-dir`, `--mcp-config`. This is the whole process API; no screen-scraping needed for status.

What is genuinely new: the app, the supervisor, the `aw` helper CLI, and two skills
(`platform-controller`, `repo-controller`).

---

## 3. Architecture

```
┌──────────────────────── AutoWork.exe (gpui) ────────────────────────┐
│  UI: platforms tree · work browser · inbox · terminal tabs         │
│        ▲ state (entities)                                          │
│  supervisor: reconcile desired vs observed, restart w/ backoff,    │
│              concurrency budget, health ladder                     │
│        ▲                 ▲                    ▲                     │
│  claude adapter     hook ingest           workstate client          │
│  (agents --json,    (127.0.0.1 HTTP,      (REST + SSE /stream)      │
│   --bg, stop, rm)    fed by `aw hook`)                              │
└────────┬───────────────────┬────────────────────┬──────────────────┘
         │ spawns            │ hook POSTs          │
   claude --bg sessions ─────┘                     │
   aw:<platform>:platform          ──MCP work_*──▶ WorkState (.NET, docker, :5290)
   aw:<platform>:<repo>                            └ Postgres
   aw:<platform>:<repo>:<unit>  (launched by repo-controllers via nested-claude)
```

### Cargo workspace

| crate | job |
|---|---|
| `aw-core` | domain types (Platform, Repo, Session, Role, Health), TOML config, the repo scanner |
| `aw-claude` | the only code that knows the `claude` CLI: launch (port of `Start-NestedAgent.ps1`), poll `agents --json`, `logs`, `stop`, `rm`, `respawn`; version check so a CLI change fails loudly |
| `aw-supervisor` | reconcile loop, restart policy, health evaluation, hook event ingest, concurrency budget. **No UI dependency** — liftable into a tray daemon later |
| `aw-workstate` | typed client for WorkState REST/SSE; starts the compose stack on tear-up |
| `aw-term` | terminal element: **gpui + `alacritty_terminal` directly**, ConPTY on Windows |
| `aw-app` | the gpui application |
| `aw` (bin) | tiny CLI the sessions and hooks call: `aw hook <event>`, `aw roster`, `aw wait`, `aw canary` |
| `plugins/autowork/` | the two skills + `hooks.json`, loaded with `--plugin-dir` at launch so every session runs the version this app ships |

### Why gpui + alacritty, not Zed's `terminal`/`terminal_view` crates

**Decided: GPL is fine** (private tool). So reuse Zed's `terminal` (+ `terminal_view`'s element) rather
than rewriting it. Open question for the M0 spike: depend on Zed's crates by git rev (simple, but
`terminal` pulls `settings`, `theme`, `theme_settings`, `task`, `release_channel` — a big build) or
vendor `terminal` + `terminal_element.rs` and cut the settings/theme coupling (more work, lean
binary). Measure both build time and binary size before choosing.

### Session identity

Names are the address and the join key: `aw:<platform>:platform`, `aw:<platform>:<repo>`,
`aw:<platform>:<repo>:<unit-ref>`. The app filters `agents --json` by prefix, so it ignores your other
sessions, and parentage is derivable from the name (and recorded in WorkState as an `environment`
note).

### Health signals, cheapest first

1. **process** — present in `agents --json`, not exited.
2. **blocked** — `state: "blocked"` or a `Notification` hook (permission prompt) → red, shows in
   Inbox with **Attach**.
3. **stalled** — busy but no transcript growth / hook events for N minutes.
4. **drifted** — a `SessionStart` hook with source `compact` fired → the hook itself re-injects the
   role skill as context (deterministic, no LLM cooperation needed). Secondary: a per-launch canary
   the app hands the session; asked only when the session is idle, never mid-work.
5. **work health** — from WorkState: expired leases, blocked items, open blocker findings.

### The mailbox (app → session)

**Preferred, if the M0 probe holds: speak the peer protocol natively.** Every session registers in
`~/.claude/sessions/<pid>.json` with `messagingSocketPath` (a named pipe `\\.\pipe\LOCAL\cc-msg-…`),
`peerProtocol: 1`, `peerFeatures`, and a sibling `<pid>.<hash>.key`. That is what `SendMessage` /
`ListAgents` ride on. If the app can (a) write to a session's pipe and (b) register itself as a peer,
then the app *is* a session to the fleet: it sends tasks directly, and controllers report with
`SendMessage("foreman", …)`. It is an undocumented internal protocol, so it lives behind `aw-claude`
with a `peerProtocol` version check, and the mailbox below stays as the fallback.

**Fallback mailbox.** There is no CLI to send a message into a background session. So: the app writes the task into WorkState (an inbox item under the
platform project) and controllers wait on it with `Monitor` running `aw wait --for <name>`, which
blocks on WorkState's SSE and exits when something addressed to them lands. Idle waiting costs no
tokens. Session ↔ session coordination stays on native `SendMessage`/`ListAgents`.

### Data placement

- platform registry: `%APPDATA%\AutoWork\platforms\<name>.toml` (root dir, repos, per-repo overrides:
  enabled, toolset, verify command, max workers)
- runtime session state: in memory, rebuilt from `agents --json` + hooks on every start (sessions
  outlive the app; reopening reconciles)
- work, plans, evidence, journal, memory-like notes: WorkState (one project per platform; a roadmap is
  a run; per-repo slices are runs/tasks linked with cross-repo `blocks`)

---

## 4. Flows

**Create platform** — pick `<dir>` → scan one level (configurable depth) for folders whose `.git` is a
**directory**; a `.git` *file* is a worktree, so `Nomos.*.worktrees/` are skipped; read each repo's
`CLAUDE.md`/`AGENTS.md` presence as a hint → review list, untick what you don't want (`y_legacy`) →
save. For `P:\Projects\Magic\Nomos` that yields 13 repos.

**Start** — WorkState up (compose) → health OK → launch `aw:<p>:platform` with
`/autowork:platform-controller` → **repo-controllers start on demand**, when the platform-controller
assigns that repo work (or pinned always-on per repo). Thirteen idle controllers is thirteen sessions
to supervise and thirteen chances to drift for nothing.

**Throw a task** — text box on the platform → WorkState inbox item → platform-controller wakes,
decomposes into per-repo slices with dependencies, presents the plan in the app for one approval (the
controller skill's "one approval per queue" rule, lifted a level) → assigns → each repo-controller runs
its queue with `/rlarch:controller`, launching workers in worktrees.

**Something needs you** — permission park, `blocked` item with a decision reason, a migration/deploy
escalation → Inbox row with context and **Attach** → tab opens `claude attach <id>` in an in-app PTY.

**Stop / tear down** — stop workers → controllers → platform (order matters: a worktree pins its
branch; stop before `rm`), reclaim build servers, optionally stop WorkState.

---

## 5. Skills to write (`plugins/autowork/skills/`)

- **`platform-controller`** — planner and lead. Reads the platform `CLAUDE.md` (cross-repo map), owns
  cross-repo contracts and sequencing, turns roadmaps into per-repo runs, routes escalations up to the
  human. Does **not** restart anything and does **not** write code. Wait loop on `aw wait`.
- **`repo-controller`** — owner of one repo: its rules, envs, migrations, databases. Runs the repo's
  queue via `/rlarch:controller`. Migrations, deploys, prod data **always escalate** (consistent with
  controller §8) — "owns" means it is the only one allowed to *propose* them.
- `controller` — unchanged, from rlarch.

Permission inheritance ("receives permissions from the parent"): the app launches controllers with a
toolset profile from the platform config; controllers launch workers with a profile **no wider** than
their own (`never launder permissions`, controller §8), auto mode everywhere.

---

## 6. Milestones

**M0 — prove the foundations (before writing UI)** — run as a `/rlarch:controller` queue in
WorkState, so the run itself is the first real use of the store. Units:

| unit | outcome | gate |
|---|---|---|
| M0.1 WorkState fitness | running image == `feat/workstate` HEAD; scripted acceptance (ACCEPTANCE.md §7) passes | script output |
| M0.2 session state model | table of `agents --json` `state`/`status` values observed across launch → busy → permission park → unpark → idle → stop → exit, for `--bg` and interactive | observations recorded as notes |
| M0.3 hooks in `--bg` | `Notification` (permission), `SessionStart` (`compact`, via `/compact`), `Stop` fire inside a `--bg` session and can POST/write to a local sink; `SessionStart` `additionalContext` re-injection observed | a log file with one line per event |
| M0.4 peer-protocol probe | can an external process deliver a message to a session's `messagingSocketPath`, and can it register as a peer visible in `ListAgents`? Read-only investigation first; a yes/no with evidence | a minimal Rust or PS probe that delivers one message, or a documented no |
| M0.5 gpui + terminal spike | Windows window with one Zed terminal running `claude attach <id>`; idle RAM, frame time under a CPU-saturating build; git-dep vs vendored build time/size | numbers |

M0.2–M0.4 are independent of M0.5; M0.1 goes first (the record depends on it).

**M1 — fleet viewer (useful on day one, read-only)**
Scanner + platform registry; left panel from `agents --json` grouped by platform/repo, health dots;
right panel terminal tabs with Attach. Already works with the sessions you run today.

**M2 — supervisor**: Start/Stop, launcher port, reconcile + restart with backoff, hook ingest, Inbox.

**M3 — skills + mailbox**: `aw` CLI (`hook`, `roster`, `wait`, `canary`), `platform-controller`,
`repo-controller`, `--plugin-dir` loading. First end-to-end task on a small test platform, **not
Nomos**.

**M4 — work browser**: native views over WorkState (tree, journal, threads, evidence), task box,
pending/running/done. The SolidJS dashboard stays for the browser; the app doesn't embed a webview.

**M5 — hardening**: concurrency/cost budget per platform, tray daemon (supervision while the window is
closed), Nomos as first real platform.

---

## 7. Risks

- **Usage limits.** A platform plus N repo-controllers plus their workers can saturate a subscription
  fast. The supervisor needs a global cap on concurrently busy sessions from M2, not M5.
- **`claude` CLI surface** (`agents --json` fields, `--bg` behaviour) is not a stable contract.
  Everything goes through `aw-claude`, which checks the version and fails loudly.
- **WorkState is untested in real use.** M0 exists for this. If it fails, the fallback is fixing it,
  not replacing it.
- **Depth.** platform → repo → controller → orchestrate subagents is four levels of delegation;
  each hop loses context. Briefs from the store (`work_brief(for: "worker")`) are what keeps it honest.
- **Closing the app stops supervision** until M5's daemon; sessions keep running but nobody respawns
  them.
