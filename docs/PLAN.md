# Foreman — plan

A native Windows-first app (Rust + gpui) where you register a **platform** (a folder of git repos),
press **Start**, throw tasks/roadmaps at it, and watch a tree of Claude Code sessions build them.
Everything that needs a human (a permission park, a decision, a migration) surfaces in the app, and
**Attach** opens that session in an in-app terminal tab.

The repo folder is still `AutoWork`; the product, crates (`foreman-*`) and CLI (`fm`) are Foreman.
Measured facts behind every decision here are in [`LEARNINGS.md`](LEARNINGS.md); the work record is
the WorkState project `autowork` (runs `m0`, `ws`).

---

## 1. The one principle

**Deterministic supervision lives in Rust. Judgement lives in skills.**

The draft had an LLM (platform-controller) watching other LLMs, respawning them, telling them about
each other, and a cron job asking it for a password. Everything in that list except "coordinate the
work" is deterministic, and an LLM doing deterministic work is slow, costs tokens, and drifts after
compaction — the exact failure the health-checker was trying to catch.

| Concern | Draft owner | Owner |
|---|---|---|
| start / restart / stop controllers | platform-controller | **Foreman supervisor** (reconcile loop) |
| "who are the other repo-controllers" | platform-controller tells them | **roster** served by Foreman (`fm roster`), always current |
| liveness, blocked-on-permission | cron + ask the LLM | `claude agents --json --all` (`waitingFor`) + Claude Code **hooks** (`Notification`) — both proven in M0 |
| skill drift after compact | password-phrase probe | **`SessionStart` hook, source `compact`**: its stdout reaches the model (proven for manual compact in M0) |
| decompose a roadmap across repos, sequence, escalate | platform-controller | platform-controller (unchanged) |
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

New: the app, the supervisor, the `fm` CLI, the relay, and two skills (`platform-controller`,
`repo-controller`).

---

## 3. Architecture

```
┌──────────────────────── Foreman.exe (gpui) ─────────────────────────┐
│  UI: platforms tree · work browser · inbox · terminal tabs         │
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
  fm-<p>-platform                                  ──MCP──▶ WorkState (.NET, docker, :5290)
  fm-<p>-<repo>                                            └ Postgres
  fm-<p>-<repo>-<unit>  (launched by repo-controllers via nested-claude)
```

### Cargo workspace

| crate | job |
|---|---|
| `foreman-core` | domain types (Platform, Repo, Session, Role, Health), TOML config, the repo scanner |
| `foreman-claude` | the only code that knows the `claude` CLI: launch, poll `agents --json --all`, `logs`, `stop`, `rm`, `respawn`; version check so a CLI change fails loudly |
| `foreman-supervisor` | reconcile loop, restart policy, health evaluation, hook ingest, relay, concurrency budget. **No UI dependency** — liftable into a tray daemon |
| `foreman-workstate` | typed client for WorkState REST/SSE; brings the compose stack up on tear-up and checks the MCP registration from each repo's cwd |
| `foreman-term` | Zed's `terminal` vendored without `settings`/`theme_settings`/`task`/`release_channel` (Variant B, M1) + a grid painter adapted from `terminal_view/src/terminal_element.rs` |
| `foreman-app` | the gpui application |
| `fm` (bin) | tiny CLI hooks and sessions call: `fm hook <event>`, `fm roster`, `fm inbox` (the relay's watcher) |
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

- Names are the address and the join key: `fm-<platform>-platform`, `fm-<platform>-<repo>`,
  `fm-<platform>-<repo>-<unit>`, `fm-<platform>-relay`. The app filters `agents --json --all` by prefix.
- Every session is launched with an explicit `--mcp-config` carrying **its role's token** (controller /
  orchestrator / worker). M0 found that nested sessions otherwise inherit the user-scope registration —
  a worker could write evidence as the controller.
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
5. **work health** — from WorkState: expired leases, blocked items, open blocker findings.

### The mailbox — decided: a relay session

Foreman is not a Claude session and cannot call `SendMessage`. The peer-pipe protocol was refused as
research (classifier), so: Foreman launches `fm-<p>-relay`, which arms `Monitor` on `fm inbox`; Foreman
drops `{to, body}` files, the relay sends them with native `SendMessage`, and every message sent *to* the
relay comes back as an outbox file. Measured: ~6 s app→target, ~14–18 s round trip, ~5 model calls per
round trip, zero while idle, no held messages across permission modes. To do (M3): the watcher emits the
JSON itself (saves a turn), re-arm the 30-min Monitor, observe a held message. Controllers reach Foreman
with `SendMessage("fm-<p>-relay", …)`.

### Data placement

- platform registry: `%APPDATA%\Foreman\platforms\<name>.toml`
- runtime session state: in memory, rebuilt from `agents --json --all` + hooks (sessions outlive the app)
- work, plans, evidence, journal, notes: WorkState (one project per platform; a roadmap is a run)

---

## 4. Flows

**Create platform** — pick `<dir>` → scan for folders whose `.git` is a **directory** (a `.git` *file* is a
worktree; `*.worktrees/` skipped) → review, untick → save. `P:\Projects\Magic\Nomos` yields 13 repos.

**Start** — WorkState up + registration checked → `fm-<p>-relay` → `fm-<p>-platform` with
`/foreman:platform-controller` → **repo-controllers start on demand** when the platform-controller assigns
that repo work.

**Throw a task** — text box → relay → platform-controller decomposes into per-repo runs with cross-repo
`blocks` → one approval in the app → repo-controllers run their queues with `/rlarch:controller`.

**Something needs you** — permission park, a `blocked` item with a decision reason, a migration/deploy
escalation → Inbox row → **Attach** opens `claude attach <id>` in a terminal tab.

**Stop / tear down** — workers → controllers → platform → relay. Stop before removing a worktree; run
`dotnet build-server shutdown` (or the toolchain's equivalent) first and retry the delete after ~3 s — a
stopped session or a build server holds handles briefly (seen on every unit in M0).

---

## 5. Skills to write (`plugins/foreman/skills/`)

- **`platform-controller`** — planner and lead; owns cross-repo contracts and sequencing; routes
  escalations to the human. Never restarts anything, never writes code. Waits on the relay.
- **`repo-controller`** — owner of one repo; runs its queue via `/rlarch:controller`. Migrations, deploys,
  prod data always escalate.
- `controller` — unchanged, from rlarch.

---

## 6. Milestones

**M0 — prove the foundations — done** (run `m0`, 5/5 units verified by the controller, 4 follow-ups in the
backlog). WorkState fit and fixed; state model, hooks, relay and terminal measured; see §3.

**M1 — fleet viewer (read-only) — done** (run `m1`, 7 units; master `3448f97`). `foreman.exe`: left panel
platforms → repos → sessions with health (live, every 2 s), right panel terminal tabs; Attach opens
`claude attach <id>`, closing a tab detaches with Ctrl+Z and never kills anything. `fm platform add|list|show|
remove`, `fm sessions`. Crates: `foreman-core`, `foreman-claude` (read side), `foreman-term`, `foreman-ui`,
`foreman-app`, `fm`. Health derives **liveness from the pid**; `state` is the current turn (`done` + live pid =
idle). Open: the human hand check (backlog 9-d).

**M2 — supervisor**: Start/Stop, launcher port (role tokens, `--plugin-dir`), reconcile + restart with
backoff, hook ingest, Inbox, concurrency cap. Proves auto-compact re-injection on a long session (6-d).
Needs from M1's findings: a "trust needed" state per repo — `claude --bg` refuses a workspace the user never
trusted, and the user must accept that prompt, Foreman must not; never restart a session whose pid is alive
(the `done`-with-live-pid case); WorkState `agents` records (skills-5d, scheduled before M2).

**M3 — skills + relay**: `fm` CLI, relay hardening (7-d), `platform-controller`, `repo-controller`. First
end-to-end task on a small test platform, **not Nomos**.

**M4 — work browser**: native views over WorkState (tree, journal, threads, evidence), task box.

**M5 — hardening**: per-platform cost budget, tray daemon, Nomos as first real platform.

---

## 7. Risks

- **Usage limits.** The supervisor needs a global cap on concurrently busy sessions from M2.
- **`claude` CLI surface** (`agents --json` fields, `--bg`) is not a stable contract; all of it behind
  `foreman-claude`, version-checked.
- **Classifier refusals.** Anything that looks like credential access or guard-weakening is refused for
  nested sessions whatever the brief says. Design for supported routes.
- **Depth.** platform → repo → controller → orchestrate subagents: four hops. Complete worker briefs from
  the store (fixed in ws/2–ws/4) are what keeps it honest.
- **Closing the app stops supervision** until M5's daemon.
