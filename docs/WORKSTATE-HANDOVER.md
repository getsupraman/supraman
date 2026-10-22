# WorkState handover — from the Foreman/AutoWork controller session

From session `autowork-7c` (repo `P:\Projects\LarchSys\AutoWork`), 2026-10-22, to the WorkState owner
session `skills-5d`. The full record is in WorkState itself: project `autowork`, runs `m0` (done), `ws`
(the WorkState fixes), `m1`, `backlog`. `work_history(scope:"ws")` and `work_get` on any `ws/*` ref give
the detail; this file is the map.

## 1. Changes made to WorkState (all on `feat/workstate` in `P:\Projects\LarchSys\Skills`, none pushed)

Every unit: a nested agent on its own branch + worktree under `P:\Projects\LarchSys\Skills.worktrees\`,
then the controller reviewed the diff, ran the full suite itself, merged `--no-ff` into `feat/workstate`,
rebuilt the live image (`docker compose -f mcp/compose.yml up -d --build`) and re-checked on the live server.

| unit | merge | what |
|---|---|---|
| ws/1 | `b8360fc` | empty server streams: `/api/stream` no longer 503s before the first workspace exists (dashboard showed "stream down") |
| ws/2 | `abcf1f1` | worker brief renders BINDING rules + routes, GOAL and intent in full (no `…`), and lists every rule (no cap of 5). Resume brief unchanged (35-line budget) |
| ws/3 + ws/4 | `b860616` | worker brief NEXT is worker-shaped (`work_report`, never `work_status`); criteria + notes in full. Notes are consumed **by the ids rendered** (before: all notes to the item were marked consumed though only 3 were shown) |
| ws/5 + ws/6 + ws/7 | `deb3672` | every DB guard reachable from a write tool is a teaching REFUSED (claim_one_writer_per_tree escaped as "An error occurred"); `Guarded()` net over all 11 write tools; deferral numbers follow-ups in the target scope; `work_update` refuses unknown criterion ids (was OK + silent drop); qualified assumption refs (`ws/A1`) resolve, ambiguous bare ones refuse (bare `A1` used to resolve BOTH runs' A1) |
| ws/8 | `9415eff` | **claims per working tree**: `work_status(…, tree:"<path or branch>")`; path keys server-wide, name keys per workspace. Migration `20261022160602_ClaimPerTree` (applied to the live DB; backup taken first: `P:\system\temp\claude\P--Projects-LarchSys-AutoWork\79421798-69cd-47b9-ac37-bc5d20f1f15d\scratchpad\ws-pre-ws8.dump`, pg_dump -Fc, 126 KB). tools/list orchestrator/controller budget 6064 → 6097 |
| ws/10 | `a439808` | read side keyed by item id: `work_get`/`work_tree` merged same-ref items across runs (m0/2 showed ws/2's criteria, evidence, commit). URIs qualified when a ref repeats |
| ws/9 | `2b88614` | `plugins/`: controller §2 + nested-claude §7 pass `tree:`; **rlarch 2.0.1 → 2.1.0** |

Suite went 254 → 300 passing, 0 failing, no budget ceiling raised.

Also done, outside the code: registered WorkState at user scope with the **controller** token
(`claude mcp add --scope user …`); an older local-scope registration still exists for `Skills/mcp` and
shadows it there.

### In flight right now

`aw-ws-11` (id `99909a65`), branch `ws/11-threads-backlog-refs` off `2b88614`, worktree
`P:\Projects\LarchSys\Skills.worktrees\ws-11`, three commits:
- ws/11 — record notes (from: only) are not open threads in `work_threads`
- ws/12 — deferring to `"backlog"` lands in a **project-level** backlog run (reuse an existing one), and closing
  a run carries out any legacy in-run backlog. Changes existing assertions in DeferralTests.cs:62,
  SchemaGuardTests (4th refusal) and possibly a WriteToolTests transport test — deliberately, see §2.
- ws/13 — every ambiguous-ref refusal suggests strings that resolve (it suggested the same ambiguous ref; a run
  is addressable as `<run>/<run>`, which nothing names)

Not merged yet. I will not merge it until René and you have agreed who lands it.

## 2. What René said about WorkState (verbatim where I have it)

- Original brief: *"The MCP p:\Projects\LarchSys\Skills\mcp\ may be re-used as .NET process under the hood, or we
  implement it new - decide by your self. Note: the feat/workstate branch in Skills repo was never realy tested.
  It started as experiment and should be production ready by now, but I hadn't time to integrate it into my
  workflow, therefore its untested but the ideas should hold."* — I chose to reuse it.
- *"when opening http://localhost:5290/ I see "stream down"."* → ws/1.
- *"you have permission to change and fix the WorkState service in Skills repo"*
- Claim model question → he chose **"Claim per worktree"** → ws/8.
- ws/9 (plugins/ skill update + version bump) → **"Yes, update + bump"**.
- Next steps → **"Finish ws, then M1"**.
- On my refusing to change existing assertions for ws/12: *"the WorkState mcp/app and the skills are still a WIP,
  its rules where never proven on a real task therefore changing rules is the normal process to grow this system
  more mature and stable."* — I retired the brief rule that forbade editing assertions; the new one forbids
  *weakening* a test, allows deliberate contract changes with the spec edit in the same commit.
- Via your message: the :5290 UI is bad, no overview, worse than the old markdown ledger/plan. (I had not heard
  that from him directly.)

## 3. What Foreman needs from WorkState (priority order)

Foreman is a native gpui app supervising platforms of Claude sessions (plan: `docs/PLAN.md`). WorkState is its
system of record: one project per platform, a roadmap is a run, per-repo slices are runs/tasks with cross-repo
`blocks`, workers claim units per worktree.

1. **A stable JSON read API for a native client** — not rendered text. Foreman's UI needs: projects with rollups;
   a run's tree (status, criteria rollup, claim holder + tree, commit, blocked-by); item detail (criteria,
   evidence split into *claims* `work_report` vs *verifications* `work_verify`, findings, notes, links);
   threads; journal since a seq. Plus the SSE `/api/stream` (works since ws/1) to know when to re-read. Auth for
   a local native client that is not the browser cookie flow.
2. **Agents/sessions as first-class records.** Today I write `environment` notes ("agent aw-ws-8 id b0ce16b7,
   branch …, worktree …"). Foreman needs: an agent row (name, claude session id, role/token, run/unit, tree,
   launched-by), linked to its claim, so the UI can show *who* holds a unit and Attach to it.
3. **A human inbox query**: everything waiting on a person — items `blocked` with a decision reason, escalations,
   blocker findings, expired claims — across a project, with the call that resolves each.
4. **Role tokens per session** work already; Foreman will launch each session with `--mcp-config` carrying its
   role's token (nested sessions otherwise inherit the user-scope controller token — a worker can then write
   evidence as the controller).
5. **Tool-schema stability across upgrades**: a session's tool schema is fixed at its start (after ws/8 this
   session still had no `tree:`). Additive params are fine; anything Foreman's sessions must call needs to exist
   before they start, so tell me before adding required parameters.
6. Backlog that lives at project level and milestone tagging for deferred items (ws/12 covers the first half).

## 4. Known bugs / gaps not fixed

- F-08: a note addressed to an item stays unconsumed forever once the item is superseded; the attention block
  repeats it on every call (live: "1 note(s) addressed to 8-d").
- From ws/10's not-done: the attention line "N note(s) addressed to <ref>" groups by bare ref (pinned
  deliberately by AttentionRefTests); link targets, search hits, history rows and the resume brief's NEXT still
  print bare refs.
- From ws/8: path-keyed claims (run serialization_key, repo_root, path tree) are server-wide by design; the Down
  migration fails if two workspaces hold live claims on the same name; the watch-floor LiveClaims block does not
  show the tree.
- From ws/5: deferral into a run with a *cancelled* backlog phase refused generically (ws/12 resolves it).
- `work_verify(command:)` must equal `verify_cmd` literally or the close guard refuses — correct but surprising;
  the refusal could say "omit command to use verify_cmd".
- Two record-keeping costs at controller scale: closing N units of one run needed N sequential claims before
  ws/8; every follow-up needs `in_progress` before `done` even when it is a pure record.

## 5. Views I'd want from the :5290 redesign (and what Foreman will build natively)

- **Overview**: every project → runs with done/open/blocked counts, what is in progress right now and by which agent.
- **Run board**: the tree, each row with status, criteria x/y, claim holder + tree + lease, commit, blocked-by.
- **Inbox**: what needs a human, newest first, each with its reason and resolving call.
- **Unit page**: the agent's claim (`work_report`) next to the controller's evidence (`work_verify`), red-checks,
  findings, the decision/correction notes — "claimed vs proven" at a glance.
- **Journal**: a timeline per run, filterable by kind (transitions, verifications, findings, decisions).
- **Backlog**: deferred work grouped by milestone/target.
