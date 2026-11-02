# Pre-publication audit: secrets, client names, personal data (C103)

- **Date:** 2026-11-02 (unit `public/4`, worktree `public-4`, branch `public/4`).
- **Range scanned:** every commit reachable from every ref (`git log --all`): root `aa74d64` (2026-10-22) to `8047a7c`
  (master and `public/4`; `origin/master` at `30c145c` is an ancestor). 457 commits, 289 of them non-merge; gitleaks reports
  288 commits scanned: it skips the root commit's diff, so the root was scanned on its own (`git show aa74d64 | gitleaks stdin
  --redact --exit-code 1`: no leaks found).
- **Tree scanned:** `8047a7c`, 370 tracked files, plus the working tree (`gitleaks detect --no-git`).
- **No secret value appears in this file.** Findings are named by commit, path, line and kind only.

## Tools and commands

| What | Command |
|---|---|
| Secrets, all history | `gitleaks detect --source . --log-opts="--all" --redact --report-format json --report-path <target>\gitleaks.json` (gitleaks 8.30.1, default rules) |
| Secrets, gate | `gitleaks detect --source . --log-opts="--all" --redact --exit-code 1` (reads `.gitleaks.toml`) |
| Secrets, working tree | `gitleaks detect --source . --no-git --redact --exit-code 1` |
| Names, history | `git log --all -p --no-merges -U0` once, every added line matched against the terms below; lines whose text is no longer in the tree listed as history-only |
| Names, tree | `git grep -nIE <term> HEAD` per term |
| Commit metadata | `git log --all --format='%an <%ae>|%cn <%ce>'` |

Terms swept (René, 2026-11-02: the known names): any e-mail address; `C:\Users\renel`, `renel`, the machine name; `P:\Projects\…`,
`P:\system\…`, `C:\Tools\…`, `C:\fmbuild\…`, any other drive-letter path; `Nomos`, `Magic\`; Italian legal vocabulary
(`CCNL`, `sentenza`, `tribunale`, `ricorrente`, `avvocato`, `cassazione`, `licenziamento`, `giudice`, `udienza`, `causa`, `codice
civile`); `dikaia`; every `http(s)://` host; JWTs (`eyJ…`), `ws_/wss_/wsk_/sk_/ghp_/github_pat_` tokens; UUIDs; `René`/`Larch`.

## Result in one paragraph

No real secret anywhere in the history: gitleaks finds one hit, a fake token in a test, now allowlisted. No client matter and no
Nomos data: the legal vocabulary sweep has **zero** hits in all history, and nothing from `P:\Projects\Magic\Nomos` was copied in;
`Nomos` occurs only as DIKAIA's product and repo names (`Nomos`, `Nomos.Pdf`, `Nomos.Extract`, workspace `nomos`) used as test data.
Every `wss_` token in history is a test fake (`wss_selftest`, `wss_from_a`, `wss_from_b`, `wss_run_token_7f3c`). The only host other
than public/example ones is `workstate.dikaia.ai` (the default WorkState server, René's public company domain: left). Personal data
that does go public: René's personal e-mail as author and committer of all 457 commits (history only), his Windows user and machine
name in two research docs (fixed in the tree, still in history), and his folder layout (`P:\Projects\LarchSys\…`, `C:\fmbuild\…`) in
test fixtures and the internal docs.

## Gitleaks summary

| Rule | Hits (all history) | Verdict |
|---|---|---|
| `generic-api-key` | 1 | test fixture, allowlisted |
| every other default rule | 0 | — |

After `.gitleaks.toml`: `gitleaks detect --source . --log-opts="--all" --redact --exit-code 1` → `288 commits scanned … no leaks
found`, exit 0. Working tree (`--no-git`): no leaks found.

**Allowlisted items** (`.gitleaks.toml`, one entry, rule `generic-api-key` only, path AND line must both match):

| Commit | Path:line | Kind | Reason |
|---|---|---|---|
| `bcb2c2f` (and the same line after the rename) | `crates/foreman-claude/src/peer.rs:318`, now `crates/supraman-claude/src/peer.rs:344` | `peerToken` in a JSON key file written to a temp dir | Test fixture: the value is the hex digits `0`–`f` twice, written by the test into a `tempfile::tempdir()`; it was never a token of a real session. The allowlist matches only that exact line in that file, so a different token in `peer.rs` still fails the gate. |

## Fixed in the tree

| File | Kind | Replaced with |
|---|---|---|
| `docs/research/scheduler.md:344` | Windows machine and user name (`<machine>\<user>` as the `.git` owner) | `<host>\<user>` |
| `docs/research/scheduler.md:361` | Windows user name (`user=…`) | `user=<user>` |
| `docs/research/scheduler.md:364` | user home path (twice) | `C:\\Users\\<user>` |
| `docs/research/gitkraken-12.6.1.md:3` | user home path of the analysed install | `%LOCALAPPDATA%\gitkraken\app-12.6.1` |
| `crates/supraman-core/src/registry.rs:122-125` | test sample pointing at the Nomos checkout on René's machine (`P:\Projects\Magic\Nomos\…`) | `C:\work\demo\…` (folder names kept; the tests only round-trip the sample) |

## Left in the tree, listed (a fix would touch an asserted fixture or a documented default)

| Where | Kind | Recommendation |
|---|---|---|
| `crates/supraman-core/src/schedule/tests/golden/{daily,interval,weekly}.xml`, `schedule/tests/registrar.rs:21,70-81`, `schedule/tests/foreign.rs:35` | `C:\Users\rene\…` home in golden task XML and path tests (asserted byte for byte) | accept (a first name and a fake home); or change input and goldens together in one later unit |
| `crates/supraman-core/src/schedule/tests/registrar.rs:286-310`, `docs/research/scheduler-probe/register-probe.ps1:4-5` | `C:\fmbuild\sched-3\…`, `C:\fmbuild\sched-1\…`, `P:\Projects\LarchSys\AutoWork` (fake exe paths asserted in the XML; probe defaults) | accept |
| `crates/supraman-app/src/brand.rs:61`, `crates/supraman-app/assets/brand/NOTICE.md:4` | path of the brand kit on René's machine (`P:\Projects\LarchSys\getsupraman\brand`); the test compares against it when it exists | accept, or name the kit's repo instead (`SUPRAMAN_BRAND_KIT` already overrides) |
| `crates/supraman-app/src/fleet.rs:623-638,1038-1039`, `tab_card.rs:111-117`, `supraman-claude/src/launch.rs:419,538,570,577`, `transcript.rs:862`, `supraman-core/src/paths.rs:96-102`, `supraman-term/src/events.rs:349,356` | `P:\Projects\…` test paths; `fleet.rs:1038` describes René's real registry (workspaces `foreman`, `gaia`, `larchsys`; repos `AutoWork`, `Skills`, `Gaia`, `NodeApi`, `node-api-dotnet`) | accept: folder and repo names only, no data |
| `crates/supraman-app/src/hand.rs:505-541`, `README.md:49`, `docs/RELEASE.md:19,141,144,199` | `C:\Tools\supraman` as the example install folder | accept (documented example) |
| `crates/supraman-core/src/settings.rs`, `url.rs`, `crates/sm/tests/work.rs`, `plugins/supraman/README.md`, `supraman-app/src/settings_window.rs` | `https://workstate.dikaia.ai`, the default WorkState server | leave (René's public domain; a default URL is behaviour) |
| tests across `sm`, `supraman-core`, `supraman-app`, `supraman-supervisor` | `Nomos`, `Nomos.Pdf`, `Nomos.Extract`, `nomos`, `pipeline-tools` as workspace and repo names | accept: product and repo names, no client data |
| ~20 doc comments in `crates/` (`schedules.rs`, `approval.rs`, `tray.rs`, `view.rs`, `desired.rs`, `sm/tests/*.rs`, …) and `scripts/new-signing-key.ps1:3` | `René` as the user who approves | accept, or say "the user" in a later comment pass |
| `docs/research/scheduler.md:350,370` | power-scheme GUID with CPU model; a probe session's UUID | accept (no account data) |

## History only

| Commit | Path | Kind | Recommendation |
|---|---|---|---|
| all 457 commits | commit metadata | René's personal (gmail) address as author and committer; name `René Larch` | René decides: **accept** (GitHub shows it on every commit once public) or **rewrite** all SHAs (`git filter-repo --mailmap`, breaks every recorded sha in WorkState and LEARNINGS). Either way, set a public or noreply `user.email` for this repo before the next commit. |
| `6df1fa2` | `docs/research/scheduler.md` | Windows machine and user name, user home path (fixed in the tree above) | accept: a user name and a machine name, no credential |
| `ad19138` | `docs/research/gitkraken-12.6.1.md` | user home path (fixed in the tree above) | accept |
| `9b781c9`, `aa74d64` | `crates/foreman-core/src/registry.rs`, `docs/PLAN.md` | `P:\Projects\Magic\Nomos` path (the registry sample is fixed above; `PLAN.md:192` and `LEARNINGS.md:457,490` still name it in the tree) | accept: a folder path, no Nomos content |
| `bcb2c2f` | `crates/foreman-claude/src/peer.rs:318` | fake `peerToken` in a test (allowlisted) | accept |
| renamed files (`crates/foreman-*`, `crates/fm`, `plugins/foreman`) | — | the same fixture paths and names as their `supraman` successors in the table above, nothing else | accept |

Nothing else is history-only: the 23 files ever deleted are renames (`foreman-*` → `supraman-*`, `fm` → `sm`), `plugins/foreman`, and
`scripts/cargo.ps1` (a PATH line for Visual Studio's CMake).

## Internal docs

None of these holds a secret, a client matter or Nomos data (sweep above). What each holds and whether it reads as fit to publish:

| Path | What is in it | Recommendation |
|---|---|---|
| `docs/LEARNINGS.md` (5606 lines) | Every unit's findings since m0: gate results with `C:\fmbuild\<unit>` target dirs (72), `P:\Projects\…` paths incl. `P:/Projects/Magic/Nomos` (457, 490), ~70 mentions of René and his decisions, session names (`autowork-7c`, `skills-5d`, …), WorkState refs | René decides: publishable as a development log (no secrets, no client data); trim if the personal working style and paths should stay private |
| `docs/PLAN.md` | The original plan (Foreman era), milestones, `P:\Projects\Magic\Nomos yields 13 repos` (192), "Nomos as first real workspace" | trim or remove: superseded by WorkState and `docs/ROADMAP.md` |
| `docs/WORKSTATE-HANDOVER.md` | A 2026-10-22 handover to the WorkState owner session: paths of René's Skills repo and worktrees, a DB migration id and the path of its backup under `P:\system\temp\claude\…` (20), how role tokens are registered (no token values) | remove from the tree: internal to WorkState's development, nothing a user needs |
| `docs/research/scheduler.md` | Task Scheduler design and probe results; René's repo paths and probe output (user and machine name now neutralised) | publishable after this audit |
| `docs/research/scheduler-probe/**` | The probe program and its register script, defaults pointing at `C:\fmbuild\sched-1` and `P:\Projects\LarchSys\AutoWork` | publishable, or remove with the probe phase done |
| `docs/research/gitkraken-12.6.1.md` | Analysis of an installed third-party app (GitKraken 12.6.1) for the GitMan roadmap item | René decides: it describes another vendor's app internals; remove if that should not be published |
| `docs/research/m5-44-audit.md` | A read-only code audit and proposed refactor units | publishable |
| `docs/research/tree-redesign.md` | UI proposal with ASCII mock-ups showing `P:\Projects\LarchSys`, `P:\Projects\Gaia` | publishable |
| `probes/**` (m0.2-states, m0.3-hooks, m0.4-relay, sched2.9-desktop, sched2.11-osc8) | PowerShell/C# probes of claude and Windows behaviour; no personal paths or names | publishable, or remove as finished experiments |
| `spikes/term-spike/**` | The m0.5 gpui terminal spike | publishable, or remove as superseded by `supraman-term` |
| `docs/icon-proposals/*.svg` | Three rejected icon proposals | remove (superseded by the brand kit) |
| `.claude/` | not tracked (`.claude/briefs/` is ignored) | — |
| `docs/images/*.png` | README screenshots from `--selftest-shots` fixtures (public/6) | publishable |
