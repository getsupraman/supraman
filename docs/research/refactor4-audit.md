# refactor4 audit: findings, measurements, test plan and proposed units

Unit refactor4/1, 2026-11-09/10, branch `r4-audit` at `bfa588a` (release 0.1.15). It audits everything since refactor3's audit (`76c9c63..bfa588a`: 205 commits, 344 files, +42,786 / −7,683 lines): GitMan 2 (GA0–GA10), the crate splits (supraman-git-view, supraman-work-view), worktree-wip, gitman-polish and tabview/2–5. It also covers the whole tree for the c4 topics: code reduction, duplicates, and Windows-only code outside a platform boundary.

This is a read-only audit. The only file it writes is this one. The measurements ran through `scripts/heavy-lock.ps1` at BelowNormal. The LNK2019 probe edited `crates/supraman-work-view/src/work_view/state.rs` temporarily and restored it from a copy, so `git status` is clean apart from this doc.

**Method**
- Four read-only reviewers ran in parallel. Each was briefed on CLAUDE.md's shared-components list, C144 (1)–(8), C57 and the protected logic. Their areas:
  1. supraman-gitman and supraman-git-view;
  2. supraman-app, supraman-work-view, supraman-ui, supraman-editor and supraman-term;
  3. core, claude, supervisor, workman and sm, plus every Windows-only spot in `crates/*`;
  4. the tests, the flakes and the selftests.
- They sized every function and file with a brace matcher, leaving out `#[cfg(test)]` modules and test files.
- I checked the riskiest claims myself:
  - M-4: `patch.rs` has no production caller.
  - T-3: `copy_dir` copies `*.lock`.
  - C-22: `write_mcp_config` is called only from tests.
  - P-x: the app's unused `windows` features.
  - A-325 (u12-narrowed): the `foreman/` labels that remain.
- Measurements ran in four steps, in order:
  1. `cargo test --profile unit --workspace --no-fail-fast`;
  2. `cargo build --profile unit --workspace`;
  3. the same test run again, warm;
  4. a gate-shaped `cargo test --profile unit -p supraman-git-view -p supraman-app`.
- After those, the LNK2019 probe ran, and the three tests that failed in step 4 were re-run alone.
- **Not run: any `--selftest*`.** None of the measurements needed one, and they disturb René's typing.

**Columns.** *Risk* is the risk of the fix. *Prot.* marks protected logic:
- WS: the Workman contract and C21 GET-only;
- Mint: the mint/revoke owner;
- CLI: the claude CLI protocols;
- Del: deletion confinement;
- Sched: C100/C101;
- Contrast: the contrast floors;
- C196: GitMan writes only on fixture repos.

A finding marked *Prot.* goes to René as a question (U12), never into a unit that changes it. **Beh.** marks a fix that changes what the user sees or what the app does.

**Id ranges**

| ids | area |
|---|---|
| L-x | the LNK2019 link failures and other build infrastructure |
| F-x | flaky tests |
| G-x | supraman-git-view |
| M-x | supraman-gitman |
| A-x | supraman-app |
| U-x | supraman-ui, supraman-editor, supraman-term |
| C-x | supraman-core, -claude, -supervisor, -workman, sm |
| P-x | Windows-only code outside a platform boundary |
| T-x | test layout, duplicates, selftests, sleeps |
| W-x | compiler warnings under the unit profile |

## Summary

- **LNK2019 has an established trigger, but its last step is not proven.**
  - Windows Defender's real-time scan is on for `P:\`; only `C:\Projects` is excluded. It holds files in rustc's new incremental session directories, so the closing rename fails with "Access is denied".
  - In `target\unit\incremental`, 70 of the 129 member sessions on disk were `-working` (never finalized) before this audit. During it, 34 of about 106 new sessions failed the same way.
  - Seven crates were left with an older finalized session under a newer unfinalized one. That is the state in which rustc's next build reuses work from the older session.
  - Every reported LNK2019 came with these notes (memory gate-stale-exe, 2026-11-09).
  - My probe reproduced the failed finalize six times (a handle held on one `.o`, 2 min in all) but not the link error. So the step from a stale session to undefined `.llvm.*` symbols is inferred, not shown.
  - The fix is U1 (heavy-lock removes a crate's incremental sessions when its newest one is unfinalized, and recovers once from a member's LNK2019), plus a Defender exclusion that is René's call.
- **The flakes share one cause, confirmed for one of them (F-3).**
  - `Fixture::copy()` copies the shared fixture file by file, `*.lock` included (`fixture.rs:322`).
  - Meanwhile, tests that only read open `GitView`s on `Fixture::shared()`. Their first `status_first` per git dir takes `index.lock` and rewrites the index.
  - A copy taken in that window holds a stale lock, and its own `git commit` then fails ("index.lock: File exists").
  - Three more tests failed under this audit's gate-shaped run and passed 2 of 2 alone; all three use `Fixture::copy()`.
  - The process timing test (F-1) is a 10 ms wall-clock budget under load.
- **The code is mostly within its limits.**
  - GitMan 2 (33k lines) has no function over 80 lines, no file over 1,500, no `env::var`, and no blocking call on the UI thread.
  - Its cost is duplication: 15 rev-parse helpers, `UNDER_WAY` five times, seven copies of "open a Writer and keep it", six actor skeletons and three `files_words`.
  - The app still has two files over the cap: `view.rs` at 2,657 lines (it shrank) and `selftest.rs` at 1,583 (it grew).
  - 10 functions in the app and ui are new over 80 lines or grew while over it.
- **Code reduction.**
  - Dead or test-only production code: over 250 lines in 30 items. The largest piece is `supraman-gitman/src/patch.rs`, 201 lines with no production caller.
  - Five unused `windows` features in the app.
  - Three compiler warnings in our code; the editor's two "warnings" are the linker's LNK4217 from tree-sitter's `wasm`.
- **Windows-only code** sits mostly in a few owners already: `process`, `secret`, `desktop`, `paths`/`shells` and `schedule::task_service`. About 1,340 production lines are `#[cfg(windows)]`, and about 450 more lines (shell and paths) assume Windows without a cfg. U9 and U10 propose `supraman_core::platform::{process, open, locale, fs, secret, desktop, shell, tasks}`, each with only a Windows implementation for now.
- **Numbers before** (details in Measurements):
  - **1,607 tests passed, 0 failed, 6 ignored**, in 58 test binaries.
  - **Full suite: 7 m 47 s** after master moved (compile 3 m 45 s); **4 m 31 s warm**.
  - `build --workspace`: 2 m 28 s.
  - A gate's test step for a git-view change: 3 m 49 s.

| Area | Ids | major | minor | nit | unrated | Prot. |
|---|---|---|---|---|---|---|
| L build infrastructure | 5 | 2 | 2 | 1 | – | – |
| F flakes | 11 | 4 | 7 | – | – | – |
| G git-view | 19 | 2 | 9 | 8 | – | – |
| M gitman | 15 | 1 | 6 | 8 | – | – |
| A app | 40 | 2 | 26 | 12 | – | 2 |
| U ui / editor / term | 13 | – | 2 | 11 | – | 1 |
| C core / claude / supervisor / workman / sm | 38 | 2 | 18 | 14 | 4 notes (C-15, C-28, C-30, C-32) | 14 |
| P platform boundary | 40 | – | – | – | 40 (grouped by module) | 15 |
| T tests and selftests | 30 | 2 | 5 | 1 | 22 (layout and sleep lists) | – |
| W warnings | 6 | – | – | – | 6 | 1 |
| **Total** | **217** | **15** | **75** | **55** | **72** | **33** |

**Refactor3's open followups**
- **14, Settings load on Git tab open:** not a UI-thread load. All three reads run off the UI thread. They are redundant, though (three reads per tab open), and two of them drop a read failure (G-4). U4.
- **15, the binary probe:** one probe (`blob::is_binary`, a NUL in 8000 bytes), but it calls UTF-16 text binary, so `invisible.rs`'s "Encoding changed" path cannot be reached (C-20). U9, **Beh.**
- **16, the process timing test:** F-1. U2.
- **17, C144 leftovers:** A-1 … A-22, C-1 … C-6. U8.
- **u12-narrowed (A-325):** the `foreman/` launch labels moved with wo-label's takeover. What remains is `APPROVAL_KEY = "foreman/schedule-approval/v1"` (`core/schedule/approval.rs:17`) and the run key `foreman/schedule/…` (`core/schedule/live.rs:71`). Both are Sched-protected, and renaming them invalidates every approval and live file. A question for René (Q6).

## Measurements

Release-shaped `unit` profile on René's 24-core machine, at BelowNormal, through heavy-lock, 2026-11-09 23:20 to 2026-11-10 00:10. Cargo's own `Finished … in` is the compile time. Wall time is the time holding the lock.

### Suite and gate

| Step | Compile | Wall | Result |
|---|---|---|---|
| `cargo test --profile unit --workspace --no-fail-fast`, first after master moved to `bfa588a` (all 13 members rebuilt) | 3 m 45 s | **7 m 47 s** | 1,607 passed, 0 failed, 6 ignored |
| `cargo build --profile unit --workspace` right after (members rebuilt in their non-test feature set) | 2 m 27 s | 2 m 28 s | ok |
| `cargo test --profile unit --workspace --no-fail-fast` again (sm and supraman-app rebuilt after the `build`) | 16 s | **4 m 31 s** | 1,607 passed |
| Gate-shaped `cargo test --profile unit -p supraman-git-view -p supraman-app` (the `-p` variant, first run since master moved) | 2 m 40 s | 3 m 49 s | **3 failed** (F-9 … F-11), 213 + 232 passed |
| The 3 failures alone, twice each | – | 4–8 s each | 6 of 6 passed |
| LNK2019 probe: 7 builds of supraman-work-view's tests, 6 edits, 3 with a held handle | 8 s each | 2 min in all | no LNK2019 |

- The tests take **about 245 s summed**, and cargo runs the binaries one after another. A gate's test step is therefore dominated by:
  - the git-view binary: 60 s;
  - the gitman integration binaries: 108 s summed over 17 binaries, each paying its own `Fixture` build;
  - supraman-term: 12.9 s;
  - supraman-core: 9.5 s;
  - supraman-app: 7.3 s.
- **The selftest was not timed here.** The gate is the test step above, plus `build --workspace` (the 2 m 28 s row is the cost after a test step changed the feature set), plus `--selftest`.
- **X-note:** `test` and `build --workspace` resolve features differently, so each alternation rebuilds the members of the other set. That was 2 m 27 s for the build after the suite, and 16 s for the suite after the build. Every gate pays this once (L-5).

### Tests per crate

Counted from the source (`#[test]`, `#[gpui::test]`). Lines were split into production, test (`#[cfg(test)]` modules, `tests.rs`, `*_tests.rs`, `tests/`, test-support) and selftest. Runtime is the summed `finished in` of the crate's binaries in the first run.

| Crate | tests | of them gpui | ignored | prod lines | test lines | selftest lines | test time |
|---|---|---|---|---|---|---|---|
| supraman-git-view | 216 | 176 | 0 | 15,365 | 7,314 | – | 59.9 s |
| supraman-gitman | 183 | 0 | 1 | 7,099 | 3,740 | – | 3.7 s lib + 104.4 s in 17 integration binaries |
| supraman-core | 260 | 0 | 3 | 8,218 | 4,631 | 27 | 9.8 s |
| supraman-app | 233 | 5 | 1 | 17,172 | 5,254 | 11,097 | 7.3 s |
| supraman-supervisor | 133 | 0 | 0 | 2,881 | 3,292 | – | 2.5 s |
| supraman-ui | 129 | 29 | 0 | 9,374 | 2,553 | 8 | 0.1 s |
| supraman-claude | 108 | 0 | 1 | 2,890 | 1,934 | – | 0.4 s |
| sm | 102 | 0 | 0 | 4,488 | 3,532 | – | 9.2 s in 13 binaries |
| supraman-term | 100 | 41 | 0 | 4,679 | 3,872 | – | 12.9 s |
| supraman-workman | 72 | 0 | 0 | 3,621 | 1,516 | – | 12.0 s |
| supraman-editor | 40 | 21 | 0 | 1,370 | 765 | – | 1.5 s |
| supraman-work-view | 40 | 16 | 0 | 3,562 | 1,140 | – | 2.1 s |
| **Total** | **1,616** | **288** | **6** | **80,729** | **39,543** | **11,132** | **~245 s** |

The source counts 1,616 tests, the run 1,613 (1,607 passed and 6 ignored). The difference is `cfg(windows)` and feature-gated tests.

### Incremental sessions (`target\unit\incremental`, members only)

| When | finalized | `-working` (failed rename) | crates whose newest session is unfinalized above a finalized one |
|---|---|---|---|
| Before the audit | 59 | 70 | 7 |
| New during the measurement steps | ~72 | 34 | 7 at the end: supraman_claude, supraman_editor, supraman_git_view, three supraman_supervisor variants, supraman_work_view |

`target\unit` holds 35 GB; DEVELOPING.md says 18 GB.

## Findings

### L. The LNK2019 link failures and build infrastructure

| id | where | finding | proposed fix | Risk | Sev. |
|---|---|---|---|---|---|
| L-1 | `Cargo.toml:13-20` `[profile.unit]` (members `incremental = true`, 256 codegen units, inherits release, so thin-local LTO) with Defender on `P:\` | **The trigger, established.** Rustc ends each incremental session by renaming `s-…-working` to its final name. On Windows the rename fails while any file inside is open; rustc's own test `tests/run-make/incremental-finalize-fail` holds one handle to cause exactly this. Defender real-time protection is on, and `P:\Projects\LarchSys\AutoWork\target` is not excluded (`Get-MpPreference`: exclusions `C:\Projects` only). Result: about a third of member sessions fail, with "note: did not finalize incremental compilation session directory …: Access is denied (os error 5)". | U1. And Q1: René adds a Defender exclusion for `P:\Projects\LarchSys\AutoWork\target` (admin; it also speeds every build). | low | major |
| L-2 | rustc's choice of session after a failed finalize | **The likely step to LNK2019, inferred, not reproduced.** The next build of the crate loads the newest *finalized* session. If an older one exists, that is a state older than the objects and rlib now in `deps\`. The undefined symbols in the reports are thin-local LTO's promoted names: `….llvm.<hash>` and promoted `drop_in_place` glue, which exist only between codegen units of one crate. That fits a CGU reused from the older session that names a promoted symbol a recompiled CGU no longer exports under that hash. It also fits "cleaning one crate moves it downstream": several crates sit in that state at once (7 now), and the link stops at whichever comes first. **Probe:** 6 rounds on supraman-work-view, alternating a held `.o` handle with edits to `state.rs`. The rename failed every time, but no LNK2019 appeared, so the exact rustc path stays unproven. | U1: after each cargo run, heavy-lock deletes `incremental\<crate>-<hash>\` of every member whose newest session is `-working` and unlocked. The next build compiles that crate in full: 8–60 s, never the 21 GB clean. On an LNK2019 whose symbols name a workspace crate, it runs `cargo clean --profile <p> -p <those crates>` and retries once, saying so on stderr. If LNK2019 still happens after U1 and Q1: `lto = "off"` for `[profile.unit]`. That stops the `.llvm.` promotion; measure its effect on compile and test time first. | low | major |
| L-3 | cargo's replay of cached diagnostics | A fresh crate's old "did not finalize" notes are printed again on every build until it recompiles (suite2.log replayed suite.log's 11 notes). The notes therefore cannot count failures. | U1 counts `-working` dirs instead of reading notes. | – | nit |
| L-4 | `docs/DEVELOPING.md` "Its artifacts live in `target\unit` (18 GB…)" | It is 35 GB now, including 106 never-collected `-working` dirs of about 44 MB each (about 4.6 GB). | U1 corrects the line; its cleanup bounds the growth. | – | minor |
| L-5 | the gate: `test -p …` → `build --workspace` → selftest | The members are built in two feature sets, so each gate rebuilds the app between its test and build steps (2 m 27 s here after a suite; tens of seconds after a small change). | Measure in U1 whether `cargo build --profile unit --workspace --all-targets`, or building the bin first, avoids the second set. If it does not, document the cost in DEVELOPING.md. | low | minor |

### F. Flaky tests

| id | test | cause | fix | Risk | Sev. |
|---|---|---|---|---|---|
| F-1 | `supraman-core/src/process.rs:885` `a_child_that_exits_at_once_costs_little_more_than_its_own_run` | It asserts that the median of 20 single timings of `run_bytes` is within **10 ms** of a bare `Command::output()` (`:902`). `run_bytes` also spawns suspended and adopts the child into a job (`:174-182`), starts two reader threads (`:418-429`) and joins them (`:144-155`). Under a full parallel run those threads are scheduled late. | The regression it guards is waiting out `PIPES_SETTLE` (1 s, `:252`). Assert `ours < PIPES_SETTLE / 2`, and keep the 10 ms number in the `#[ignore]`d `child_overhead_measured` (`:905`). Also loosen `:661` (`spawn_detached < 300 ms`) the same way. | low | major |
| F-2 | git-view `wip/write_tests.rs:114` (discard all, `:139-148`) | **Not found by reading.** The failure text was git's `-C` chdir error ("cannot change to … Invalid argument", `repo.rs:34`). Every `-C` here is the copy's own top or worktree. The only shared state is `Fixture::copy()` (F-3). | F-3's fix. Make `fixture::git_in` (`fixture.rs:33`) and `GitError::Failed` print the `-C` dir and whether it exists, so the next failure names it. | low | minor |
| F-3 | git-view `wip/tests.rs:127` `wip_view_shows_a_conflicted_file_in_file_view` | **Confirmed by reading.** `wip/tests.rs:145` opens a GitView on `Fixture::shared().worktree`. Its first `status_first` (`gitman/status.rs:112`) takes `repo/.git/worktrees/wt/index.lock` in the shared fixture and rewrites the index; the fixture forces that on purpose with `bin.dat`'s old mtime (`fixture.rs:271-273`). A `Fixture::copy()` taken then (`fixture.rs:84`) copies the lock (`copy_dir`, `:322`, copies every file). The copy's `f.conflict()` then runs `git commit -am` in wt and fails with "Unable to create …/worktrees/wt/index.lock: File exists". No background git is involved: background work, watchers and focus refreshes are off in tests (`background/mod.rs:72-82`, `refresh.rs:50-61`). | (a) `copy_dir` skips `*.lock` and `gitman-*.index`. (b) `copy()` copies a private template that no test ever opens. (c) The shared fixture's index is refreshed once when it is built, so `status_first` has nothing to write; keep the stale-mtime file only where `tests/write.rs:208` needs it. Add a no-lock check to `tests/fixture_copy.rs`. | low | major |
| F-4 | git-view `ref_panel/tests.rs:189` `hiding_a_branch_prunes…`, and the **stash-hide flake** (its stash half, `:199-205`) | `soloed()` (`:175`) is one repo shared by three tests (`:190`, `:448`, `:504`), built from `Fixture::copy()`. The first GitView in the process rewrites its index (`status_first`) while the others read it. A failed refresh read takes `apply_refresh`'s `Err` path (`refresh.rs:206-211`): old rows kept, `graph_failed` set. That fails "leaves the graph" or "hides every stash". Probable; not reproduced. | One `Fixture::copy()` per test (sharing saves one copy). The `click`/`redraw` helpers assert `graph_failed.is_none()`, so a failed read says why. | low | major |
| F-5 | git-view `diff/tests.rs:89` "the viewer is open" (`diff_view_reloaded_after_a_change_below_keeps_its_top_row_and_change`, `:242`) | `diff()` is `None` right after `open_diff` only through the early return at `git_view.rs:567` (`self.repo` is `None`): the first load failed. Nothing escapes `run_until_parked`: `background_spawn` is deterministic in the test dispatcher and the path has no timer. Leading hypothesis: F-4's, since `repo()` (`:41`) is one `Shared<Repo>` opened by six tests. Second hypothesis: `run_raw` keeps only the bytes read within `PIPES_SETTLE` after the child exits (`process.rs:151-155`), so a starved reader truncates git's stdout. | `open_in` (`git_view/tests.rs:16-32`) asserts `Load::Ready` and prints the problem text. Per-test copies (or F-3c). `PIPES_SETTLE` counts only while a reader makes no progress. | low | major |
| F-6 | chdir-flake | No `set_current_dir` exists in `crates/` at `bfa588a`. The process cwd is read only by production code (`sm/src/schedule.rs:41`, `:412`; `app/elevate.rs:58`), and `sm/tests/schedule.rs:48`, `:165` set the child's `current_dir`. | Close the Workman item, or ask it to name the test (Q8). | – | minor |
| F-7 | flaky-prune-worktree-test: `supraman-gitman/tests/branch.rs:246` (`remove_worktree` `:257`, `remove_dir_all(&f.worktree).unwrap()` `:260`) | `git worktree remove --force` and `remove_dir_all` fail with "Directory not empty" or "Permission denied" while Defender or the indexer holds `dirty.txt`, written just before (`:256`). The same trigger as L-1. A lock copied in by F-3 also marks the worktree locked. | Retry the removal (as `prune_settled`, `core/schedule/tests.rs:651-661`); F-3's fix. | low | minor |
| F-8 | flaky-live-refresh-tests: git-view `live_tests.rs` | (1) `until` (`:20-29`) polls real time: a 10 s deadline, 5 ms sleeps. (2) `:146` runs real watchers on `Fixture::shared()`, which react to other tests' index rewrites. (3) `refresh_tests.rs:90` asserts "no refresh" with real watchers armed, but a late batch from the load's own index write reaches `apply_wip` → `refresh_graph` (`refresh.rs:350-351`). (4) The thread-locals `FOLLOW` (`refresh.rs:50`) and `background::ON` (`background/mod.rs:72`) are never reset, so they leak between tests that share a thread. | Wake on the refresh, not on a sleep: a `Follow` test hook resolved when `refreshes` or the wip token changes. No watchers on shared repos (T-41). A drop guard from `follow()` and `start_for_test()` resets both thread-locals. | low | minor |
| F-9 | git-view `agent_rows_tests.rs:69` `selecting_an_agent_row_shows_the_session_and_attach_asks_for_it`: "git-agent-0 is painted" | Failed in this audit's gate-shaped run and passed 2 of 2 alone. The test uses `Fixture::copy()`; the agent row needs HEAD's or the WIP row, so it fits a first load that failed (F-3/F-5). | F-3, and `open_in` asserting `Load::Ready`. | low | minor |
| F-10 | git-view `branch/tests.rs:98` `a_pill_dropped_on_another_fast_forwards_that_branch_without_a_checkout`: "a row of 8a85419…" | The same run, the same pattern: no rows. | Same. | low | minor |
| F-11 | git-view `background/tests.rs:155` `tabs_of_one_repository_share_one_schedule…`: `[1]` ≠ `[2]` "the worktree's tab joins" | The same run. The second tab (on `f.worktree`) joins on its first remote facts, which come from its load. The worktree's load failing fits a lock copied into `repo/.git/worktrees/wt/`. | Same. | low | minor |

### G. supraman-git-view

GitMan 2's view layer, 15.4k production lines in 97 files. No function is over 80 lines (the longest, `render_overlay` at `ref_panel/overlay.rs:106`, has 47). No file is over 1,500 non-test lines (the largest, `git_view.rs`, has 978). No `env::var`, and no blocking call in a handler.

| id | where | finding | proposed fix | Risk | Sev. |
|---|---|---|---|---|---|
| G-1 | `git_view.rs:79` `struct GitView` (~65 fields) | §6: feature state lives on the hub view. Diff viewer: `diff`, `diff_mode`, `diff_layout`, `focus_diff`. Paging: `layout`, `history`, `rows`, `commits`, `more`, `paging`, `page_error`. Also the column menu and `find`. | Move the diff state and the paging state onto entities of their own before the next field. | med | major |
| G-2 | `layout.rs:36-48` `Opening::adopt`, `File::opening` | §8: a compat shim ("a tab saved before this file kept it") with no ref and no removal release. | Name its removal release (Q7), or delete it. | low | minor |
| G-3 | `background/maintain.rs:35-53` `Ran::last`/`record` | §5: a failed load of `maintenance.json` keeps its path, so the next `record` writes the default map over the file. The failure is only logged, and nothing takes `lock_file`. | Fall back to memory as `layout.rs:76` does (G-14), and hold `lock_file` around the write. | low | minor |
| G-4 | `background/mod.rs:107`, `remote/mod.rs:217`, `load.rs:174` | §1/§3, followup 14: `settings.toml` is read three times per tab open. Two of the reads (`Settings::load(..).unwrap_or_default()`) drop a failure, so `fetch_every_minutes = 0` silently becomes a fetch every minute when the file cannot be read. | One `supraman_gitman::read_settings(home) -> (GitmanSettings{git, remote, background}, Option<notice>)`, read once in `start()` off the UI thread and handed out. | low | major |
| G-5 | `live_tests.rs:28` | §7: a 5 ms sleep loop to a 10 s deadline. | F-8. | low | minor |
| G-6 | `occupancy.rs:32` `Live::set` | Test-only production code: only `background/tests.rs:239,244` and `branch/tests.rs:317` call it; production uses `set_sessions`. | The tests call `set_sessions`; delete `set`. | low | nit |
| G-7 | `wip_row.rs:59` `draft_commit` | Ungated `pub`, used only by supraman-app's `selftest_shots.rs:287`. | `test-support`, or name it a selftest hook. | low | nit |
| G-8 | `branch/menu.rs:153` `fn act` | A needless layer: it only forwards to `supraman_ui::menu::item` (23 call sites). | Call `item`. | low | nit |
| G-9 | `details/render.rs:28` `fn short` | Wraps `short_sha` in a `SharedString`. | Inline it. | low | nit |
| G-10 | the empty `Refs{…}` literal at `branch/mod.rs:153` and `remote/mod.rs:189`; HEAD's sha matched at `branch/menu.rs:141`, `commit/menu.rs:27`, `branch/mod.rs:452`, `:477`, `load.rs:120` | §1, 7 sites. | `Refs::empty()` and `Head::sha()` in gitman's `refs.rs`. | low | minor |
| G-11 | "open this repo's `Writer` off the UI thread and keep it": `stash.rs:129`, `branch/mod.rs:157`, `details/reword.rs:124`, `diff/picks.rs:104`, `undo.rs:67`, `remote/mod.rs:231`, `wip/mod.rs:168` | §1/§2, one lifecycle: 7 × ~13 lines, each with its own `writer: Option<Writer>`. | `WriteGate` opens and owns the tab's one `Writer`; the actors ask the gate for it. | med | minor |
| G-12 | the actor skeleton (gate, hits, writer, `asking: Option<Confirm>`, menu, `doing`/`outcome`, `written`, `ask`/`answer`/`busy`) in `StashActions`, `BranchActions`, `CommitActions`, `RemoteActions`, `UndoBar` and the WIP actions | §1: six copies. | A `write_gate` helper that holds the ask, busy and written handling; after G-11. | med | minor |
| G-13 | `files_words` ×3 identical (`commit/mod.rs:69`, `stash.rs:53`, `wip/actions.rs:55`); count-with-plural helpers at `diff/picks.rs:73`, `details/reword.rs:87`, `diff/invisible.rs:34`, `wip/load.rs:79`, `wip_row.rs:94`, `wip/compose.rs:45`, and work-view `overview.rs:22` | §1, 10 copies. | `supraman_core::text::count(n, one, many)`, listed in CLAUDE.md. | low | minor |
| G-14 | "load the store or fall back to memory": `layout.rs:76-82`, `find/frecency.rs:109-112` (and G-3 should use it) | §1. | `JsonStore::load_kept` in `supraman_core::store`. | low | minor |
| G-15 | `occupancy.rs:50` `Live::working_in` and `occupancy.rs:65` via `write/guard.rs:18` | §1: two `within` filters over the same sessions. | `who_works_in` maps over `Live::working_in`. | low | nit |
| G-16 | `background/mod.rs:316` `reason` (git's `fatal:` line, else the last line) | Git error parsing inside the view. | Move it to `supraman_gitman::write` (`WriteError::reason`). | low | nit |
| G-17 | `worktree_panel.rs:270` `to_string_lossy().replace('\\', "/")` | §1: the path rule exists. | `supraman_core::paths::normalize`. | low | nit |
| G-18 | `load.rs:165` "Install Git for Windows, or set [gitman] git to a git.exe." | Windows-only wording. | Pick the words per platform (U10). | low | nit |
| G-19 | `commit/mod.rs:276` `top.join("a/b").display()` | Copies a path with mixed separators. | `paths::normalize`. | low | minor |

### M. supraman-gitman

| id | where | finding | proposed fix | Risk | Sev. |
|---|---|---|---|---|---|
| M-1 | `write/rebase.rs:244-248` `write_marker` | C56: `REBASE_MARKER` JSON written with `fs::write` and read back by `read_marker`, possibly from another process. | `write_atomic`. | low | minor |
| M-2 | `write/files.rs:81-93` `ignore` | §5: a read-modify-write of `.gitignore`, which git reads concurrently, done with plain `fs::write`. | `write_atomic`. | low | minor |
| M-3 | `write/remote.rs:171-188` `fetch_each` | §1 ("never a worker pool of your own"): one unbounded `thread::scope` spawn per remote. | `supraman_core::pool::map(remotes, n, …)`, with progress through the existing channel. | low | minor |
| M-4 | `patch.rs`, all 201 lines (re-exported `lib.rs:33`: `DiffOf, FilePatch, Hunk, Line, LineKind, patch`) | **Dead.** No production caller in `crates/` (verified); only `tests/readers.rs:96-104` uses it. CLAUDE.md lists `patch` among the readers. | Delete it, its tests and its CLAUDE.md mention. | low | major |
| M-5 | `history.rs:59` `pub fn history` | Forwards to `History::start(..).page(..)`. Only tests use it: `layout/tests.rs`, `tests/paging.rs`, `tests/readers.rs`, git-view `refresh_tests.rs:112`. | Move it to `fixture` (test-support). | low | nit |
| M-6 | `history.rs:101` `History::pid` | Only tests use it (`git_view/stream_tests.rs:19`, `tests/paging.rs:50,66`). | `cfg(any(test, feature = "test-support"))`. | low | nit |
| M-7 | `write/locks.rs:193` `Scheduler::await_waiting` | Only tests use it (`tests/write.rs:35,43,63`). | Same. | low | nit |
| M-8 | `lib.rs:24-39`: `is_binary`, `COMMIT_HOOKS`, `WIP_READERS`, `GitmanSettings`, `check`, `judge`, `locate`, and `write/branch.rs:115` `local_name` | Exported wider than any caller needs. | `pub(crate)`, so the dead-code lint sees them (LEARNINGS dev-speed/2). | low | nit |
| M-9 | rev-parse helpers: `write/stash.rs:48` = `write/wip.rs:32` (`has_head`); `write/branch.rs:66` = `journal/restore.rs:97` (`exists`); `write/branch.rs:70` = `rebase.rs:226` (`rev`); `write/remote.rs:138` (`resolve`); inline `rev-parse HEAD` + `text().trim()` at `wip.rs:189,240`, `conflicts.rs:108,216`, `stash.rs:101-102`, `journal/mod.rs:307`, `snapshot.rs:31` | §1, about 15 sites. | `Session::{verify(name) -> Option<String>, rev(name) -> String, line(call)}` in `write/mod.rs`. | low | minor |
| M-10 | `WriteError::Io { what: format!("<verb> {}", path.display()), source }` at `files.rs:83`, `journal/mod.rs:218`, `journal/restore.rs:242`, `lines.rs:150,245`, `rebase.rs:247,288,301`, `snapshot.rs:28`, `wip.rs:160` | §1, 10 sites. | `WriteError::io(verb, path)`. | low | nit |
| M-11 | `GitError::Failed { args, code, stderr: lossy(..).trim() }` at `repo.rs:118,126`, `history.rs:150`, `write/mod.rs:303`, `rebase.rs:238` | §1, 5 sites. | `GitError::failed(args, &RawOutput)`. | low | nit |
| M-12 | `pub const UNDER_WAY` five times with the same text: `write/branch.rs:20`, `commit.rs:14`, `rebase.rs:18`, `remote.rs:18`, git-view `remote/mod.rs:40` | §1. | One, in `supraman_gitman::write`. | low | minor |
| M-13 | the remote-prefix strip: `layout/groups.rs:106` and `write/branch.rs:117` `local_name` | §1. | `RemoteBranch::branch()` in `refs.rs`. | low | nit |
| M-14 | `write/files.rs:23` `escaped`: `replace('\\', "/")` | Windows assumption: on Unix it corrupts a file name that holds `\`. | Normalise only under `cfg(windows)`, or take repo-relative `/` paths (U10). | low | minor |
| M-15 | `write/rebase.rs:321` `copy_over` | The same `\` → `/` assumption, for git's sh. | The same; document it as the Windows boundary. | low | nit |

### A. supraman-app

**Size (§6)**

| id | where | finding | proposed fix | Risk | Sev. |
|---|---|---|---|---|---|
| A-1 | `view.rs` | 2,657 non-test lines (2,903 at refactor3). Over the cap and shrinking. | Move `watch_transcripts` and the hand launch (`:1790-1900`) into `view/hand_tab.rs`. | med | minor |
| A-2 | `selftest.rs` | 1,583 lines, **grew +35** while over the cap (§6 forbids). | Move `drawn`, `until`, `until_app`, `until_with` and `frame_if_asked` (`~:500-550`) into `selftest/wait.rs`, together with A-29. | low | major |
| A-3 | `settings_window.rs:124` `SettingsView::new` | 81 lines (was 69): new over the cap. | Build the pages in `pages()`. | low | minor |
| A-4 | `selftest_work.rs:417` `run` | 127 lines (was 109): grew while over. | Split it into step fns. | low | minor |
| A-5 | `selftest_first_run.rs:75` `run` | 134 lines (was 125): grew while over. | The same. | low | minor |
| A-6 | `main.rs:207` `main` | 182 lines (was 179): grew while over. | Move the console, env scrub and mode dispatch into fns (with P-8). | low | minor |
| A-7 | `selftest.rs:1002` `gate` | 136 lines (was 133): grew while over. | Make it a table of steps. | low | minor |
| A-8 | `supervise.rs:332` `tick` | 128 lines (was 126): grew while over. | Split read / decide / apply. | med | minor |
| A-9 | `view/manage.rs:242` `run_action` | 99 lines (was 96): grew while over. | One method per arm. | low | minor |
| A-10 | `settings_window.rs:786` `render_appearance` 168 (was 167); `view/left.rs:674` `render_inbox` 104 (was 103) | Each grew by a line while over. | Section renders; `settings_window.rs` one module per page, as `gitman.rs` is. | low | nit |

Also over 80 lines but unchanged since refactor3; fold each into the unit that next touches its file:
- `view.rs:567` `FleetView::new` 315;
- `selftest_supervise.rs:239` `run` 262;
- `fleet.rs:291` `build` 215;
- `view/left/messages.rs:22` 192;
- `view/left.rs:1164` `render_session` 191, `:930` `render_workspace_row` 155, `:389` `render_left` 132, `:228` 107;
- `selftest_supervise.rs:940` 161, `:1251` 96, `:777` 82, `:680` 82;
- `selftest_schedules.rs:261` 161;
- `view.rs:883` `watch_transcripts` 143;
- `schedules.rs:368` `sections` 132;
- `selftest_manage.rs:666` 128;
- `view/schedules.rs:887` 119, `:590` 114, `:1024` 90;
- `selftest_reopen.rs:116` 108;
- `selftest_link.rs:89` 107;
- `selftest.rs:128` 103, `:320` 92, `:1258` 85;
- `view/results.rs:58` 91;
- `selftest_left.rs:179` 88;
- `settings_window.rs:591` 87, `:683` 85;
- `selftest_mark.rs:54` 87;
- `selftest_hand.rs:56` 82.

**FleetView, blocking calls, env reads (§3, §4, §6)**

| id | where | finding | proposed fix | Risk | Sev. |
|---|---|---|---|---|---|
| A-11 | `view.rs:402` `tab_keys: TabKeyState` (mode, closed-tab ring) | §6: new feature state as a FleetView field (tabview/3). | `Entity<TabKeys>` owned by `tab_keys.rs`; A-28's accessors go. | low | minor |
| A-12 | `view.rs:321` `occupancy: occupancy::Live` beside `view.rs:318` `work_live: Rc<RefCell<Vec<LiveSession>>>`, both filled at `:1290-1295` with two `LiveSession` types | §6/§1: two projections of the same live sessions. | One shared live handle that the Work tab reads too. | med | minor |
| A-13 | `view.rs:329` `work_repo_of` (a fn pointer only `--selftest-shots` overrides) | §6. | Pass it in `Opening`/`WorkInbox` from the selftest. | low | nit |
| A-14 | `settings_window.rs:454` `open_file`: `path.exists()` in a click | §4, new. | Check inside the `open_off_thread` task. | low | minor |
| A-15 | `settings_window.rs:1041` `render_plugins`: `is_file()` per plugin in a render | §4. | Compute when the settings load, off the UI thread. | low | minor |
| A-16 | `settings_window.rs:386` `open_themes_folder`: `create_dir_all` in a click | §4. | Create it in the background, then open. | low | minor |
| A-17 | `updates.rs:32` `install_update` (`is_file()` in a click); `updates.rs:302` `hand_over` (`fs::copy` and a spawn on the UI thread) | §4. | Into the background task. Update path; not protected, but Beh. timing only. | low | minor |
| A-18 | `view.rs:2225` detach loop: `process::alive` in a foreground `cx.spawn` | §4. | Background executor. | low | nit |
| A-19 | `work_view/inbox.rs:210` `WorkInbox::new` → `Dismissed::load`; `view.rs:578` `Settings::load` in `FleetView::new` | §4 at startup. | Load in the background, apply after. | med | minor |
| A-20 | `SUPRAMAN_SHOTS` read at `selftest.rs:238`, `selftest_manage.rs:532,540`, `selftest_schedules.rs:68`, `selftest_shots.rs:42` | §3: five readers. | `selftest_shots::dir()`. | low | minor |
| A-21 | `SUPRAMAN_SELFTEST_HOLD_SECS` at `selftest_supervise.rs:1324`, `selftest_work.rs:596` | §3: two readers. | `selftest::hold_secs()`. | low | nit |
| A-22 | `selftest_left.rs:92` | §7: a fixed 1,000 ms sleep after `close_settings`. | `until_with` on the window being closed. | low | minor |

**Code reduction, duplicates, Windows spots, comments**

| id | where | finding | proposed fix | Risk | Prot. | Sev. |
|---|---|---|---|---|---|---|
| A-23 | `hand.rs:444` `Hands::launch_for` (and `#[cfg(test)] launch` `:439`) | Dead in production since 6252b68 (A-306): production runs `launch_locked` (`hand.rs:223`). About 20 tests (`hand.rs:1127-1733`) exercise only `launch_for`, so the locked path is not the one tested. | Point the tests at `launch_locked(&Mutex::new(hands), …)` and delete `launch_for` (W-3). | low | Mint | major |
| A-24 | `view.rs:1824` `let (watch, mut changed)` | An unneeded `mut` (W-2). The brief said `:1830`. | Drop it. | – | | nit |
| A-25 | `view/term.rs:26,56-57,124-125` `Chord::ScrollPage` and the test at `term.rs:492-493` | Unreachable: `tab_keys.rs:50-51` binds `ctrl-shift-pageup/pagedown` to `TabKey::Move` ahead of `key_down`, and `passes` lets only New and Close through. `docs/KEYS.md:14` documents the move. | Remove the variant, its two arms and the two assertions. **Waits on René (Q4):** he may give the terminal Shift+PageUp/PageDown page scroll instead. | low | | minor |
| A-26 | `selftest_clipboard.rs:274` `put_test_image`, the non-Windows stub | Dead: no caller. | Delete it. | – | | nit |
| A-27 | `updates.rs:22` `open_release_page` | Forwards to `open_web`. | Inline it at `:33`; pass `open_web` as the fn pointer. | – | | nit |
| A-28 | `tab_keys.rs:119-137` `set_tab_keys`, `tab_keys()`, `pop_closed` | Forwarding accessors. | Go with A-11. | – | | nit |
| A-29 | `selftest.rs:522` `drawn` against `selftest_left.rs:109-123` `shown_and_painted` | §1: the same on_next_frame + refresh + until_with sequence (9 and 15 lines), copied in a5d4ed6. | `selftest::drawn_in(window)`; `shown_and_painted` calls it after its `is_shown` wait. | low | | minor |
| A-30 | a closed loopback port by hand (`TcpListener::bind("127.0.0.1:0")`): `selftest.rs:138,197`, `selftest_first_run.rs:169`, `selftest_supervise.rs:118` | §1/§7: `supraman_workman::fake::closed_base` exists but sits behind `fake`. | An always-built `supraman_workman::closed_base`, called everywhere. | low | | minor |
| A-31 | `selftest_messages.rs:37` `tell` ≈ `selftest_supervise.rs:1243` `inject` | §1 and C56: `{file}.tmp` + rename by hand. | `supraman_core` relay `tell`/`send`, or `write_atomic`, in one `selftest::drop_outbox`. | low | | minor |
| A-32 | the Git tab title (`file_name` else `display`, `format!("Git · {name}")`) at `view/git_tab.rs:45-46` and `view/right/tab_menu.rs:203-204` | §1. | `git_tab::title(folder)`. | – | | nit |
| A-33 | WM_CLOSE posting (`selftest.rs:1438`, `selftest_left.rs:127`); OpenProcess/TerminateProcess (`selftest_mark.rs:26`, `selftest_desktop.rs:116`) | §1, Win32 copies in selftests. | One `selftest/win.rs` (P-17); the kill through `supraman_core::process`. | low | | minor |
| A-34 | `view.rs:2467-2480` `powershell()`/`powershell_args()` (used at `hand.rs:1375`) | Shell argv built in the main view. | `view/shells.rs`. | low | | minor |
| A-35 | `hand.rs:87-100` the embedded PowerShell runner (`[Diagnostics.Process]::Start`) | A launch script inside the token-lifecycle module. | Move it to a launch-text module, leaving the token logic. | low | CLI | minor |
| A-36 | `view/start.rs:73-95` the `.cmd`/`.bat` refusal and `pwsh_command` lookup | Shell rules in a UI module. | With A-34's module. | low | | nit |
| A-37 | `main.rs:195` FreeConsole/GetConsoleProcessList inline in `main` | Windows-only, inline. | `platform::desktop::detach_console()` (P-8). | low | | nit |
| A-38 | Win32 across selftests: `selftest_capture.rs:11,50` (PrintWindow), `selftest_manage.rs:67,92,598` (mouse messages, PeekMessage), `selftest/frames.rs:28,157`, `selftest_clipboard.rs` | Windows-only outside a boundary. | `selftest/win.rs` (P-17). | low | | minor |
| A-39 | `view/shells.rs:77`, `selftest_tabs.rs:23`, `view/writes.rs:5` (new); `selftest_updates.rs:137`, `selftest_supervise.rs:676` (old) | C57: multi-line docs that narrate their callers or steps. The app has 71 multi-line blocks, 17 touched since refactor3, no past-tense narration. | One line each (U11). | – | | nit |
| A-40 | `inbox_card.rs` and its tests render `crate::work_view::cards` and `HOUSEKEEPING_KEY` | Work-view internals tested in the app. | Move both to supraman-work-view (T-31). | low | | minor |

### U. supraman-ui, supraman-editor, supraman-term

supraman-work-view has no function over 80 lines, no file over 1,500 lines and no C57 problem. Its findings are listed with the app's (A-12, A-19, A-40, G-13).

| id | where | finding | proposed fix | Risk | Prot. | Sev. |
|---|---|---|---|---|---|---|
| U-1 | `supraman-ui/src/menu.rs:61` `render` | 92 lines (was 66): new over the cap. | Section and rule rows rendered apart. | low | | minor |
| U-2 | `supraman-ui/src/details.rs:150` `render_section` | 90 lines (was 72): new over the cap. | One fn per `Line` kind. | low | | minor |
| U-3 | `supraman-ui/src/text_input.rs:735` `text_bounds` | Dead. | Delete it. | – | | nit |
| U-4 | `supraman-editor/src/diff.rs:105` `Diff::new_point` ("for a test") | Dead: not even a test calls it. | Delete it. | – | | nit |
| U-5 | `supraman-editor/src/rows.rs:100` `select_rows` | Only `git-view diff/pick_tests.rs:158` calls it. | `test-support`. | low | | nit |
| U-6 | `supraman-editor/src/lib.rs:130` `cursor_row`, `:75` `read_only_buffer`, `split.rs:220` `scroll_rows` | Only tests call them (`languages/tests.rs`, `tests/theme_switch.rs`, git-view `diff/tests.rs`); production uses `read_only_buffer_of`. | `test-support`. | low | | nit |
| U-7 | `supraman-ui/src/tree.rs:362` `TreeState::set_entries_given` | Only `tree.rs:903,909` (tests) call it; CLAUDE.md names it, but the ref panel calls `Tree::build_given`. | Gate it for tests and fix the CLAUDE.md line. | low | | nit |
| U-8 | `supraman-term/src/builder.rs:488` `resolve_path` (SearchPathW) | §1 against `paths::executable_on_path`; vendored. | Leave it (vendored Zed code), or call core at its one call site. | low | | nit |
| U-9 | `supraman-ui/src/lib.rs:136` `memory()` (GetProcessMemoryInfo) | A process fact in the UI crate. | `supraman_core::process::memory` (P-7). | low | | nit |
| U-10 | vendored term: `alacritty.rs:39`, `pty_info.rs:13` (GetProcessId), `builder.rs:229` (`LANG`) | Already split per platform upstream. | Leave it. | – | | nit |
| U-11 | `supraman-ui/src/lib.rs:176` | C57: an 8-line `FrameProbe` doc teaching how gpui draws. | One line. | – | | nit |
| U-12 | `supraman-ui/src/tabs.rs:150` | C57: a 6-line `tab_strip` doc restating CLAUDE.md's tabs line. | One line. | – | | nit |
| U-13 | `supraman-ui/src/themes/syntax.rs:106` | C57: a 3-line `fit` doc restating `nearest_holding` and `SYNTAX_FLOOR`. | One line; wording only. | – | Contrast | nit |

Comment counts (multi-line `//`/`///` blocks): supraman-ui 88 (21 touched since refactor3), supraman-editor 16 (3), vendored supraman-term 40 (0).

### C. supraman-core, supraman-claude, supraman-supervisor, supraman-workman, sm

No file in these crates is over 1,500 non-test lines. `supervisor/executor.rs` holds 962 production and ~1,725 test lines in one file.

| id | where | finding | proposed fix | Risk | Prot. | Sev. |
|---|---|---|---|---|---|---|
| C-1 | `supervisor/src/reconcile.rs:52` `reconcile` | 157 lines. | One helper per action kind. | med | | minor |
| C-2 | `sm/src/fire.rs:342` `fire_inner` | 129 lines. | Script, session and record stages. | med | Sched | minor |
| C-3 | `sm/src/run.rs:79` `session_stage` | 111 lines. | Split out the launch-with-token step and the cap setup. | med | Mint | minor |
| C-4 | `workman/src/stream.rs:135` `Stream::connect` | 100 lines. | Header parse and chunk loop apart. | med | WS | minor |
| C-5 | `supervisor/src/executor.rs:404` `Supervisor::tick` | 93 lines. | The revoke/absent phase out. | med | (Mint-adjacent) | minor |
| C-6 | `sm/src/run.rs:318` `watch` | 92 lines. | Extract the cap checks. | low | | minor |
| C-7 | `workman/src/fake.rs:544` `FakeWorkman::handle` (feature `fake`) | 131 lines. | A route table per path prefix. | low | | nit |
| C-8 | `sm/src/probe_hooks.rs:102` (132), `probe_isolation.rs:47` (121), feature `probes` | Dev-only. | Leave it. | – | | nit |
| C-9 | `sm/src/hook.rs:18,20,21`; `relay.rs:58,103-109`; `schedule.rs:798` | §3: `SUPRAMAN_WORKSPACE` (3 readers), `SUPRAMAN_ROLE` (2), `SUPRAMAN_REPO`, `SUPRAMAN_SESSION`; the writer spells them as literals at `supervisor/executor.rs:135-137`. | `supraman_core::home::SessionEnv{role, workspace, session, repo}` with name consts, used by writer and readers. | low | | minor |
| C-10 | `core/src/schedule/registrar.rs:23` `APPDATA` | §3: a second reader beside `home.rs:26`. | `home::default_home_from_env()`. | low | Sched | minor |
| C-11 | `sm/src/schedule.rs:134,209-215`: `MSYSTEM`, `ProgramFiles*`, `LOCALAPPDATA`, `EXEPATH`, `PATH` | §3: `PATH` and `ProgramFiles(x86)` have owners (`paths::process_path`, `program_files_x86`). | Move `git_roots` into `paths` (P-21). | low | | minor |
| C-12 | `claude/src/launch.rs:275` `USERNAME` | §3. | `paths` owner (P-12). | low | Mint | nit |
| C-13 | `core/src/askpass.rs:124` `DISPLAY` | In the owner's own module. | A note at `askpass`, or leave it. | – | | nit |
| C-14 | `term/src/builder.rs:229` `LANG` | Vendored. | Leave it. | – | | nit |
| C-15 | app env reads | See A-20, A-21. | – | – | | – |
| C-16 | `claude/src/daemon.rs:67-92` `is_elevated` = `:123-142` `pid_elevated` | §1: the same token-elevation read. | One `token_elevated(handle)` in `core::platform::process` (P-6). | low | CLI | minor |
| C-17 | `claude/src/proc_tree.rs:18-42` `snapshot` and `core/src/process.rs:770-786` (test `processes`) | §1: the same Toolhelp32 walk. | `core::process::snapshot`. | low | | minor |
| C-18 | `claude/src/forget.rs:126-136` `is_link`; `core/src/paths.rs:441-446` `is_app_alias` | §1: `FILE_ATTRIBUTE_REPARSE_POINT` twice. | `paths::is_reparse_point`. | low | Del | minor |
| C-19 | `core/src/process.rs:51` and `:188` | `CREATE_NO_WINDOW` defined twice; `desktop.rs:197` uses the crate's own. | One const (P-1). | – | | nit |
| C-20 | text vs binary: `gitman/src/blob.rs:6-7,66-69` `is_binary` (a NUL in 8000 bytes); viewer `git-view diff/load.rs:182` | Followup 15. `text::decode` (`core/text.rs:55-64`) and `write/lines.rs:58-64` accept UTF-16 with a BOM, but UTF-16 always has NULs, so the viewer shows it as binary, and `invisible.rs`'s "Encoding changed: UTF-16 LE → UTF-8" (tested at `invisible.rs:68`) cannot be reached from `load`. `core/shells.rs:233-235` uses `contains(&0)` to mean UTF-16 without a BOM. | `supraman_core::text::classify(bytes) -> Text(Encoding) \| Binary` (BOM first, then the NUL probe); `is_binary` and `load.rs` call it (U9, **Beh.**). | low | | major |
| C-21 | `supervisor/src/mint.rs:308-327` `MintedSessions::load`/`save`; `:680-694` `revoke_deleted_runs` (load, remove, save) | §5: `<home>/supervisor/runs/<name>.json` is written by `sm` (`sm/src/run.rs:247,280`) and edited by the app with no `lock_file`. | A per-file lock in `MintedSessions` around load…save (Q2). | med | Mint | major |
| C-22 | `claude/src/launch.rs:52` `with_minted_token`, `:186` `write_mcp_config`, `:191` `write_mcp_config_with`, `:154` `workman_mcp_config_with` | Production code only tests use (verified: the callers are `launch.rs:605-727`, `supervisor mint.rs:914`, `executor.rs:1504`, `app hand.rs:743`); it is the token-in-the-file path that CLAUDE.md says production never takes. | `#[cfg(test)]` or `test-support`, so production cannot write a token into a config (Q2). | low | Mint | minor |
| C-23 | `supervisor/src/executor.rs:146` `LaunchConfig::session_settings` | Test-only (`executor.rs:2061`). | Delete it; the test calls `settings_with`. | low | | nit |
| C-24 | `executor.rs:259` `with_daemon_check`, `:277` `with_minter`, `:282` `with_repo_of` | Test seams used only by executor tests. | `#[cfg(test)]`. | low | (Mint-adjacent) | nit |
| C-25 | `supervisor/src/mint/keeping.rs:87` `Keeping::with_margin` | Test-only (`keeping.rs:224`). | `#[cfg(test)]`. | low | Mint | nit |
| C-26 | `workman/src/auth.rs:165` `with_sign_in_timeout` | Only `tests/oauth.rs:23,185`. | Feature `fake`. | low | | nit |
| C-27 | `workman/src/client.rs:159` `Client::backlog` | Only `tests/client.rs:120,187`: a dead GET API. | Remove it with its tests, or keep it with a Workman ref (Q2). | low | WS | minor |
| C-28 | `claude/src/human.rs:49` `Sent::is_direct` | Selftest-only. | Keep it. | – | | – |
| C-29 | `core/src/install.rs:246` `rename_aside_on_this_thread` (a `#[doc(hidden)]` thread-local that changes `apply`) | A test hook inside production `apply`, used by `sm/tests/update.rs:215,441`. | A feature (Q2). | low | Del | minor |
| C-30 | `core/src/process.rs:163` `spawned_here` | Listed in CLAUDE.md for tests. | Keep it. | – | | – |
| C-31 | `sm/src/fire.rs:577` `recover_if_dead`, `:595` `recover_workspace_with` | Only `sm/tests/fire.rs:548-581` calls them; the app calls `recover_workspace` (`app schedules.rs:187`), but CLAUDE.md says `recover_if_dead` is "the app calls it each poll". | Fix the CLAUDE.md line and gate the extras (Q2). | low | Sched | minor |
| C-32 | `core/src/schedule/registrar.rs:182-216` `FakeRegistrar::{…}` | Test and selftest support by design (C101). | Keep it. | – | | – |
| C-33 | `workman/src/client.rs:38` `api_base` = `core::url::origin` | Two spellings of one rule: 35 callers use `api_base`, 6 `url::origin`. | Pick one. | low | | nit |
| C-34 | `workman/src/secret.rs:1-3` | A module that only re-exports `core::secret` (3 users). | Import from core. | – | | nit |
| C-35 | `claude/src/transcript.rs:489`, `lib.rs:87`, re-export of `paths::claude_dir` | One user (app `selftest.rs:56`). | Drop it. | – | | nit |
| C-36 | `claude/src/lib.rs:76` `pub use supraman_core::process::Output` | A forwarding re-export. | Import from core. | – | | nit |
| C-37 | `workman/tests/oauth.rs:366` (300 ms sleep, then `< 1 s` at `:369`), `:453` (200 ms); `core/schedule/tests.rs:645,657` (200 ms retries) | §7. | `FakeWorkman::seen_when`/`stalled`; the retries are acceptable (Windows fs). | low | | minor |
| C-38 | `core/src/secret.rs:41-45` `default_store` | Off Windows it is a `MemoryStore`, so secrets vanish silently. | The platform `secret` module (P-10) decides per OS; until mac/Linux land, a notice (Q2). | low | Mint | minor |

### P. Windows-only code outside a platform boundary

About 1,340 production lines are inside `#[cfg(windows)]`. About 450 more lines (shell and paths) assume Windows without a cfg. Every Windows dependency is already under `[target.'cfg(windows)'.dependencies]`. The proposal is `supraman_core::platform` with one module per concern; each holds a trait or a plain fn set with **only the Windows implementation** for now, and a `cfg(not(windows))` stub that says "not on this platform" instead of silently doing nothing.

| Module | ids | Where now | Lines | Prot. |
|---|---|---|---|---|
| `platform::process`: spawn flags, suspend and resume, job tree, liveness, Toolhelp snapshot, elevation, memory | P-1 … P-7 | `core/process.rs:45-55,185-239,432-590`; `claude/proc_tree.rs:18-50`; `claude/daemon.rs:67-147`; `ui/lib.rs:135-162` | ~250 | P-6 CLI |
| `platform::open`: ShellExecute, runnable extensions and `PATHEXT`, `runas` | P-32 … P-34 | `core/process.rs:510-528`; `ui/open.rs:30-60,89`; `app/elevate.rs:55-68` | ~60 | – |
| `platform::locale` | P-35 | `core/time.rs:118-146` (GetLocaleInfoEx) | 18 | – |
| `platform::fs`: replace a running exe, reparse points, hidden files, `exe_name(stem)` with `EXE_SUFFIX` | P-25 … P-28 | `core/install.rs:128-205`; `claude/forget.rs:126-136`; `core/paths.rs:441-446`; `core/scan.rs:96-106`; hard-coded `.exe` names in `core/plugin.rs:8`, `schedule/registrar.rs:14`, `install.rs:62-75`, `app/updates.rs:300-313`, `sm/schedule.rs:216`, `claude/proc_tree.rs:50`, `schedule/foreign.rs:18-20` | ~90 | P-25, P-26 Del; P-28 Sched (part) |
| `platform::paths`: data dir, user home order, `path_key` casing | P-23, P-24 | `core/home.rs:11-27`; `schedule/registrar.rs:21-23`; `core/paths.rs:7-15,149-153` | ~30 | P-23 Sched (registrar) |
| `platform::secret`: DPAPI store, owner-only ACL | P-10 … P-12 | `core/secret.rs:35-142`; `workman/secret.rs`; `claude/launch.rs:270-300` (`icacls` + `USERNAME`) | ~115 | Mint |
| `platform::desktop`: hidden desktop, tray and window, single instance, fonts, console; selftest harness (clipboard, capture, input, messages) behind a `selftest` feature | P-8, P-13 … P-17 | `core/desktop.rs:9-277`; `app/tray.rs:67-300` (HKCU registry read `:73-91`); `app/instance.rs:1-140`; `ui/fonts.rs:4-20`; `app/main.rs:193-205`; `selftest_*` (~360 lines) | ~850 | – |
| `platform::shell`: `%NAME%` expansion, `PATHEXT`, pwsh and Windows PowerShell lookup, App Execution Alias, shell detection (pwsh, cmd, WSL, vswhere), the [+] runner, Git Bash path rewrite, script kinds | P-9, P-18 … P-22 | `core/paths.rs:343-481`; `core/shells.rs:1-245`; `app/hand.rs:86-100`, `view/start.rs:74-101`, `view/left.rs:1107`; `sm/schedule.rs:191-223`; `sm/fire.rs:784-821`; `core/schedule/mod.rs:45,405-410` | ~480 | P-20 CLI (C113); P-9, P-22 Sched |
| `platform::tasks`: Task Scheduler COM, task XML, foreign tasks, run level (already behind `Registrar`, a good boundary: a move) | P-29 … P-31 | `core/schedule/task_service.rs`; `task_xml.rs`; `registrar.rs:21-40`; `foreign.rs`; `session.rs:12-14`; `claude/daemon.rs:112-114` | ~600 | **Sched** (C100/C101) |
| Stays: claude's named pipe (P-36, `claude/peer.rs`, already has a Unix-socket twin; CLI); vendored term (P-37) | | | | |

Dependency findings:
- **P-38:** five `windows` features in supraman-app have no `use` anywhere: `Media_Ocr`, `Graphics_Imaging`, `Storage_Streams`, `Foundation_Collections`, `Win32_System_WinRT` (verified; `app/Cargo.toml:44`). Drop them (U3).
- **P-39:** `Win32_Storage_Xps`, `Win32_System_DataExchange` and `Win32_UI_Input_KeyboardAndMouse` serve only the selftests. They go with P-17's feature.
- **P-40:** `embed-resource` is an ungated build-dependency (`app/Cargo.toml:46`). Fine; `build.rs:7` checks the target.

### T. Tests: layout, duplicates, selftests, sleeps

**Duplicated helpers**

| id | where | proposed shared home | Sev. |
|---|---|---|---|
| T-10 | `struct Holder` + a render wrapper, 12 copies in git-view (`tests.rs:34`, `wip/write_tests.rs:20`, `stash_tests.rs:15`, `branch/tests.rs:21`, `remote/tests.rs:16`, `undo_tests.rs:27`, `details/reword_tests.rs:14`, `agent_rows_tests.rs:20`, `ref_wips_tests.rs:24`, `worktree_panel_tests.rs:22`, `worktree_rows_tests.rs:21`), plus `work_view/tests.rs:172` and app `inbox_card/tests.rs:40` | `git_view::test_support::Holder` (the module exists with only `sizes()`); a generic `Holder<V>` in a `supraman_ui` test-support module | minor |
| T-11 | the open-a-tab helper (`init_globals`, `bind_keys`, `editor::init`, `add_window_view`, `run_until_parked`) in 15 git-view test files (`tests.rs:11,16`, `write_tests.rs:29,36`, `stash_tests.rs:23`, `branch/tests.rs:29,311-322`, `remote/tests.rs:24`, `undo_tests.rs:13`, `reword_tests.rs:22`, `agent_rows_tests.rs:30`, `ref_wips_tests.rs:55`, `worktree_panel_tests.rs:44`, `worktree_rows_tests.rs:39`, `background/tests.rs:50`, `diff/tests.rs:73`, `wip/tests.rs:13`) | `test_support::open(folder, Opts{home, live, editor})`, asserting `Load::Ready` (F-5) | major |
| T-12 | `click` ×8, `press` ×6, `right_click` ×3, `painted`/`repaint` ×8, `toast` ×8, `asking` ×4, `menu` ×2, `row_of` ×3, `head`/`sha`/`exists` (`branch/tests.rs:65-74`, `reword_tests.rs:99`, `undo_tests.rs:45`, `write_tests.rs:103`, `worktree_rows_tests.rs:61`) | `git_view::test_support` | minor |
| T-13 | gitman `tests/*.rs`: `write`/`write_in`/`undo` (`branch.rs:21,25`, `commit.rs:18,34`, `conflicts.rs:19`, `journal.rs:51,55`, `lines.rs:71`, `rebase.rs:21,25`, `remote.rs:40`, `stash.rs:12`, `wip.rs:15,19,23`); `git(f, dir, args)` = `git_in(&f.git, dir, args, &[], None)` (`branch.rs:13`, `commit.rs:22`, `journal.rs:86`, `rebase.rs:13`, `remote.rs:15`, `wip.rs:27`; git-view `commit/tests.rs:12`, `diff/tests.rs:23`, `cap_tests.rs:8`, `remote/tests.rs:63`) | `Fixture::{at, write_job, undo}` in `fixture.rs` | minor |
| T-14 | deadline poll loops: `live_tests.rs:20`, `selftest.rs` `until*`, `core/watch.rs:163`, `process.rs:760,806`, `sm/tests/fire.rs:409,432,450`, `sm/tests/update.rs` (6), `workman/tests/stream.rs:28`, `oauth.rs:448` | a `supraman_core` test-support `eventually(within, what, f)` that takes a wake source | minor |
| T-15 | the seven inbox kinds and their rows: `selftest_work.rs:46`, `inbox_card/tests.rs:10-55`, work-view `cards.rs:232` | `supraman_workman::fake::inbox_rows()` | nit |
| T-16 | derived shared repos: ref_panel `soloed()` `:175`, diff `repo()` `:41` | gone with F-4/F-5; otherwise `Fixture::derived(label, build)` refreshing the index first | minor |
| T-17 | 17 gitman integration binaries (`tests/*.rs`), each linking the crate and building its own `Fixture` (104 s summed, 4–17 s each) | One `tests/all/main.rs` with `mod branch; mod commit; …`: one link, one `Fixture::shared`, parallel across modules. Measure before and after. | major |

**Test layout after the splits**

| id | finding | proposal |
|---|---|---|
| T-30 | git-view mixes four layouts: sibling `foo_tests.rs` declared from `git_view.rs:38-61` (11 files); `dir/tests.rs` (8 dirs); `dir/foo_tests.rs` (`diff/{cap,media,pick}_tests.rs`, `details/reword_tests.rs`, `wip/{conflict,write}_tests.rs`); `#[path]` (`agent_rows.rs:255`) | One rule: `<module>/tests.rs`, and `<module>/tests/<topic>.rs` where large; move the siblings under their module. |
| T-31 | app `view/left/inbox_card.rs` tests work-view internals | Move to supraman-work-view (A-40). |
| T-32 | app `view/git_tab_tests.rs:30-45` tests `saved_tabs::plan` and legacy JSON | Move to `saved_tabs.rs`'s tests; keep `:12-28`. |
| T-33 | `live_tests.rs:79`, `:131` use no watchers | Into `refresh_tests.rs` (or T-40). |
| T-34 | `live_tests.rs:131,146`, `wip/tests.rs:145`, `layout.rs:221` run writing code on `Fixture::shared()` | A copy each, or F-3c. |
| T-35 | `supraman-app/Cargo.toml:22` puts gitman's `test-support` (tempfile, image) in the normal deps for selftest 7g's `Fixture::new_in` | Keep it, noted; or gate the selftest code behind a feature with P-39. |
| T-36 | `supervisor/src/executor.rs`: ~1,725 test lines inside the 2,690-line file | `executor/tests.rs`. |

**Sleeps and wall-clock asserts (§7)**

| id | where | what |
|---|---|---|
| T-50 | `core/process.rs:902` | 10 ms median delta (F-1) |
| T-51 | `core/process.rs:661` | `spawn_detached < 300 ms` |
| T-52 | `core/process.rs:692,739,827,948` | `< 20–30 s` ceilings; drop them, since the outcomes are asserted |
| T-53 | `core/process.rs:762,800,808` | 10–50 ms poll sleeps |
| T-54 | `core/watch.rs:194-195` | "one batch" relies on two writes inside a 150 ms debounce |
| T-55 | `core/schedule/tests.rs:645,657` | 200 ms retry sleeps (acceptable) |
| T-56 | `git-view live_tests.rs:27-28` | 10 s deadline, 5 ms sleeps (F-8) |
| T-57 | `sm/tests/fire.rs:250` (`< 20 s`), `:454` (fixed 500 ms before the restarter) | |
| T-58 | `sm/tests/update.rs:273` | fixed 500 ms, then asserts an absence (a negative wait) |
| T-59 | `sm/tests/mcp_header.rs:115` | `< 2 s` |
| T-60 | `app/hand.rs:1034` | `< 2 s` |
| T-61 | `app/tests/elevation_handoff.rs:37` | fixed 5 s (`#[ignore]`) |
| T-62 | `workman/tests/oauth.rs:366,453` (300/200 ms); `:369` (`< 1 s`), `:371` (`< 8 s`), `:382` (`< 3 s`) | C-37 |
| T-63 | `workman/tests/stream.rs:38` (20 ms); `:62` (`< 2 s`); `:83` (gap ≥ 180 ms) | |
| T-64 | `app/selftest.rs:1180` | fixed 200 ms after a drag |

### W. Compiler warnings under the unit profile

From the first full run (`cargo test --profile unit --workspace`) and `cargo build --profile unit --workspace`:

| id | where | warning | fix |
|---|---|---|---|
| W-1 | `supraman-git-view/src/git_view/background/wips.rs:58` | `method error is never used` (lib and lib test) | Delete it, or show it (the ref panel's WORKTREES header could show a failed read; CLAUDE.md names `error` on `Wips`). |
| W-2 | `supraman-app/src/view.rs:1824` | `variable does not need to be mutable` | A-24. |
| W-3 | `supraman-app/src/hand.rs:444` | `method launch_for is never used` | A-23 (Mint: Q3). |
| W-4 | `supraman-editor` lib test and `tests/theme_switch.rs` | Linker LNK4217: tree-sitter (`wasm` feature) imports `wasm_*`/`wasmtime_*` as dllimport, but they are statically linked from `wasmtime-c-api`. Harmless, but it shows as "1 warning". | Find whether supraman-editor needs tree-sitter's `wasm` (heavy-lock's cmake note says Zed's `language` pulls it in); if it does, accept it and say so in DEVELOPING.md. |
| W-5 | `supraman-app` bin and bin test | the same LNK4217 lines | W-4. |
| W-6 | every build | replayed "did not finalize" notes (L-3) | U1. |

## Test plan

**Remove or merge** (each line names the test that keeps the coverage):

| Test | Action | Coverage kept by |
|---|---|---|
| gitman `tests/readers.rs:96-104` (the `patch` readers) | remove with M-4 | – (the code goes) |
| app `hand.rs:1127-1733`, ~20 tests on `launch_for` | retarget to `launch_locked` (A-23, Q3) | the same tests, on the production path |
| app `view/term.rs:492-493` (ScrollPage chord asserts) | remove with A-25 (Q4) | `tab_keys.rs:182-183` (the Move binding) |
| git-view `live_tests.rs:79` `live_refresh_keeps_the_selection_and_the_scroll_on_the_same_commit` | remove | `refresh_tests.rs:192` (the far-down case) |
| git-view `live_tests.rs:146` `live_refresh_stops_the_watchers_when_the_tab_closes` (watchers on the shared fixture) | remove | the `close()` assert in `live_tests.rs:48`, used by `:54`, `:110` and `refresh_tests.rs:90`; `core/watch.rs:223` |
| git-view `ref_panel/tests.rs:491` `a_right_click_opens_the_refs_menu_and_its_solo_solos_it` | merge into `:271` (`a_solo_walks…`, the same right-click and solo), with its "a pick closes the menu" assert | `:271` |
| git-view `commit/tests.rs:123` `the_wip_rows_menu_stages_stashes_and_discards_everything` (asserts only labels and stage-all) | rename to what it asserts, or fold the labels into `wip/write_tests.rs` | `wip/write_tests.rs:125` (stage all) |
| supervisor `executor.rs:2061` (`session_settings`) | point it at `settings_with` (C-23) | the same test |
| core `process.rs:885` (F-1) | keep it, asserting `< PIPES_SETTLE / 2`; the 10 ms figure moves to the ignored `child_overhead_measured` | itself |

**Move**
- 17 gitman integration binaries into one (T-17).
- git-view test files to one layout (T-30).
- `inbox_card` tests to work-view (T-31).
- `git_tab_tests.rs:30-45` to `saved_tabs` (T-32).
- `live_tests.rs:79,131` to `refresh_tests.rs` (T-33).
- `executor.rs`'s tests to `executor/tests.rs` (T-36).

**Shared helpers**
- `git_view::test_support::{Holder, open, click, press, right_click, painted, toast, asking, menu, row_of}` (T-10 … T-12).
- `Fixture::{at, write_job, undo}` (T-13).
- `supraman_core` test-support `eventually` (T-14).
- `supraman_workman::fake::inbox_rows` (T-15).
- An always-built `supraman_workman::closed_base` (A-30).

**Selftest steps to cut** (each covered by a gpui or unit test; the step keeps only what the test cannot do: the real sidebar icon, real watchers, real fonts, real windows):

| Step | Cut | Covered by |
|---|---|---|
| 7g open: "a second click focuses it (one tab)" (`selftest_git.rs:119-139`) | the one-tab check; keep the sidebar-icon click | app `view/git_tab_tests.rs:12` |
| 7g live: a commit appears by itself (`selftest_git.rs:150-162`) | cut | git-view `live_tests.rs:54` (real watchers there) |
| 7g reopen (`selftest_git_reopen.rs`): hidden ref, narrowed column, panels | cut, keeping the minimum-width part | git-view `tests.rs` `git_tab_reopens_at_its_repos_layout_and_a_new_repo_at_the_last_one`; `git_tab_tests.rs:12` |
| selftest_tab_menu: "Open Git opening and then activating the one Git tab" | cut | `git_tab_tests.rs:12` |
| 7 `every_kind` (`selftest_work.rs:264-288`) | cut | app `inbox_card/tests.rs:85`; work-view `cards.rs:204,231` |
| 7 inbox dismissal survives a reload (`selftest_work.rs:650-677`) | cut | work-view `inbox.rs:501,526` |
| 7 filters and closed rows (`selftest_work.rs:440-458`); row click selects (`:478`) | cut | `work_view/tests.rs:274,291`; `navigator.rs:181` |
| 7n notices × (`selftest_notices.rs:73`); messages dismissal (`selftest_messages.rs`) | keep only the painted ×; the state is tested | `view/notices.rs:76,87`; `dismissed_messages.rs:100` |
| 7k search, 7l mark/star | keep paint and clicks only | `search.rs:212-240`; `stars.rs:61-95` |

These step files say nothing about why a unit test cannot do their checks (C144 §7). Each one gets a line or loses its steps: closed, delete, drop, first_run, font, left, link, mark, messages, notices, refresh, reopen, search, tabs, term, title, updates, zoom, work and manage (T-29).

## Proposed units, in order

Each unit fits one context window. A unit's gate is the CLAUDE.md unit gate (`cargo test --profile unit $(affected)`, `cargo build --profile unit --workspace`, `--selftest`) unless a stronger one is named. **Beh.** = changes behaviour; **Prot.** = touches protected logic: the controller blocks those as questions.

| # | Unit | Findings | Outcome (criteria) | Gate | Size | Risk | Beh. / Prot. |
|---|---|---|---|---|---|---|---|
| U1 | **Incremental sessions that never finalized stop breaking links** | L-1 … L-5, W-6 | (a) After every heavy-lock cargo run, no member crate in `target\<profile>\incremental` has an unlocked `-working` session as its newest. (b) An LNK2019 whose symbols name a workspace crate is followed by one `cargo clean --profile <p> -p <those>` and one retry, said on stderr. (c) DEVELOPING.md: the clean rule, the 35 GB, Defender. (d) Whether `--all-targets` saves the gate's rebuild is measured and written down. | Q1's holder probe (`holder.ps1`, a held `.o` during a build): after the run no hazard state remains. Then two consecutive gates with no LNK2019. Scripts only, so no cargo test is needed beyond one `cargo test --profile unit -p supraman-core`. | S | low | – |
| U2 | **The Git tab tests stop flaking** | F-1 … F-11, T-16, T-34, T-41, T-40 | `copy()` never carries a lock; no test writes through `Fixture::shared()`; `open_in` asserts `Load::Ready` with the problem text; the process timing test asserts against `PIPES_SETTLE`; live tests wake on the refresh and reset their thread-locals; prune retries. | `cargo test --profile unit --workspace --no-fail-fast` 5 times in a row with 0 failures (today: 3 failures in 1 of 2 gate-shaped runs). | M | low | – |
| U3 | **Zero warnings, no dead code** | W-1, W-2, A-24, M-4, M-5 … M-8, U-3 … U-7, G-6, G-7, A-26 … A-28, C-23, C-26, C-33 … C-36, G-8, G-9, P-38 | `cargo build`/`test --profile unit --workspace` report no warning from our crates; `patch.rs` and its tests are gone, and CLAUDE.md no longer lists them; test-only items are gated; five unused `windows` features dropped. A-25 joins once Q4 is answered; A-23 and C-22/C-24/C-25/C-27/C-29/C-31 stay with U12. | The unit gate; plus `grep -c '^warning' ` of the build log is 0 for our crates. | S | low | – |
| U4 | **GitMan's helpers in one place** | M-1 … M-3, M-9 … M-13, G-3, G-4, G-10, G-13 … G-17 | One `Session::{verify, rev, line}`, `WriteError::io`, `GitError::failed`, one `UNDER_WAY`, `Refs::empty`/`Head::sha`, `RemoteBranch::branch`, `text::count`, `JsonStore::load_kept`; settings read once per tab with a failure said (closes followup 14); rebase marker and `.gitignore` through `write_atomic`; fetches through `pool::map`. | The unit gate; gitman's and git-view's tests unchanged and green. | M | low | (M-2 writes `.gitignore`: Del-adjacent, not confinement) |
| U5 | **The test suite reorganised** | T-10 … T-15, T-17, T-30 … T-33, T-36, T-42, T-43 | git-view's tests share `test_support` helpers and one layout; gitman's integration tests are one binary; the moves listed in the test plan done; no test lost (`cargo test -- --list` count before = after, minus the removals named in U2/U3). | The full suite: test count reconciled, and the gitman integration time measured before and after (today 104 s summed). | M | low | – |
| U6 | **Selftest cuts and selftest helpers** | T-20 … T-29, T-44, T-45, A-2, A-20 … A-22, A-29 … A-31, A-33, A-38 (helpers only), T-64 | The steps in the test plan cut, each with its covering test named in the commit; `selftest.rs` back under 1,500 lines (`selftest/wait.rs`, `drawn_in`); one `closed_base`, one shots dir, one hold-secs reader; no fixed sleep in `selftest_left.rs`; every remaining step file says why a unit test cannot do it. | The unit gate with `--selftest` once (controller); the selftest's wall time before and after. | M | low | – |
| U7 | **GitView's state and its write actors** | G-1, G-11, G-12 | The diff state and the paging state on entities of their own; the `WriteGate` owns the tab's one `Writer`; one shared ask/busy/written helper for the six actors. | The unit gate; git-view's 216 tests green 3 runs in a row. | L | med | – |
| U8 | **The app back within C144** | A-1, A-3 … A-19, A-32, A-34, A-36, A-40, U-1, U-2, C-1, C-6, C-9 … C-11, C-16, C-17, C-19, C-37 | `view.rs` shrinks (hand tab and transcripts out); the fns that grew or are new over 80 lines split; `TabKeys` an entity; one live-session handle; no `fs` or process call in a click or render (A-14 … A-18); startup loads off the UI thread (A-19); `SessionEnv` the one reader of the session env; one elevation read, one Toolhelp walk. | The unit gate. | L (split in two if needed: A-items / C-items) | med | A-17 the updater's timing (Beh., small) |
| U9 | **Text or binary, one rule** (followup 15) | C-20 | `supraman_core::text::classify`; UTF-16 with a BOM is shown as text with its encoding; `invisible.rs`'s encoding change reachable and tested from `load`. | The unit gate; a new git-view test opening a UTF-16 file (`MEDIA_FILES` has one). | S | low | **Beh.** (UTF-16 files show as text) |
| U10 | **A platform boundary, Windows its only implementation** | P-1 … P-8, P-13 … P-19, P-21, P-23, P-24, P-27, P-28 (non-Sched), P-32 … P-35, P-39, G-18, G-19, M-14, M-15, A-37, U-9 | `supraman_core::platform::{process, open, locale, fs, paths, desktop, shell}` with the Windows code moved (not rewritten) and a `cfg(not(windows))` stub that refuses with words; no `windows::` import outside `platform` and the vendored term (a grep in the gate); the selftest Win32 harness behind a `selftest` feature. | The unit gate; `rg 'windows::' crates --glob '!**/platform/**' --glob '!supraman-term/**'` empty. | L (two units: process/open/locale/fs/paths, then desktop/shell) | med | the moves touching CLI (P-6, P-20), Del (P-25, P-26) and Sched (P-9, P-22, P-28 part, P-29 … P-31) are left in place for U12 |
| U11 | **Comments state the present** (C57) | A-39, U-11 … U-13, the git-view and gitman blocks (96 + 121, 48 of them 3+ lines), the five worst: `write/guard.rs:5-6` (wrong: the gate asks nothing), `layout.rs:41`, `branch/menu.rs:1-3,15-23`, `remote/dialogs.rs:1-5`, the module essays in `background/maintain.rs`, `background/mod.rs`, `remote/menu.rs`, `journal/mod.rs` | `rlarch:code-comments` over every file touched since refactor3; no comment names a unit or brief; no multi-line block that restates code. | The unit gate (comments only; the diff is reviewed for no code change). | M | low | U-13 Contrast (wording only) |
| U12 | **Questions for René, then the protected leftovers** | A-23, A-35, C-2 … C-5, C-12, C-18, C-21, C-22, C-24, C-25, C-27, C-29, C-31, C-38, P-6, P-9 … P-12, P-20, P-22, P-25, P-26, P-29 … P-31, A-325 leftovers | Each question answered (Q2, Q3, Q5, Q6); the answered ones done in one careful unit, Mint and Sched each with their own tests. | The unit gate; plus `--selftest-supervise` for Mint, and the schedule tests with `FakeRegistrar` for Sched. | M | med | **Prot.** (Mint, CLI, Del, Sched, WS) |

Not proposed as units: C-7, C-8, C-13, C-14, U-8, U-10, P-36, P-37, P-40, T-35, the unchanged over-80 functions. Fold them into the unit that next touches their file.

## Questions for René

1. **Q1 (U1):** May I (or you) add a Windows Defender exclusion for `P:\Projects\LarchSys\AutoWork\target`? It needs admin. It removes the LNK2019 trigger (L-1) and the scan of every object file. `C:\Projects` is the only exclusion today.
2. **Q2 (U12, Mint/WS/Del/Sched):**
   - C-21: a per-file lock on the run records that sm and the app both write.
   - C-22: gate the token-in-file config writers to tests.
   - C-27: remove `Client::backlog` (a GET nobody calls).
   - C-29: put `install::apply`'s test hook behind a feature.
   - C-31: correct CLAUDE.md's `recover_if_dead` line.
   - C-38: what `default_store` should do off Windows until mac/Linux.
3. **Q3 (Mint):** may the ~20 `hand.rs` tests move from the dead `launch_for` to `launch_locked`, and `launch_for` go (A-23, W-3)?
4. **Q4:** remove the unreachable `ctrl+shift+pageup/pagedown → ScrollPage` (A-25), or give the terminal Shift+PageUp/PageDown page scroll instead?
5. **Q5 (CLI, Sched, Del):** should U10's platform moves also cover the claude [+] runner (P-20, A-35), the elevation read (P-6), the install rename (P-25), the reparse checks (P-26) and the Task Scheduler code (P-29 … P-31)? These are moves with no logic change, but protected.
6. **Q6 (Sched, u12-narrowed):** keep `APPROVAL_KEY = "foreman/schedule-approval/v1"` and the `foreman/schedule/…` run key forever? Renaming them voids every approval and live file. If kept, record the reason in Workman so the followup can close.
7. **Q7:** `git_view/layout.rs`'s adopt shim for old saved-tab sizes (G-2): which release removes it?
8. **Q8:** chdir-flake (F-6): no test changes the process cwd at `bfa588a`. Close it, or which test was meant?
