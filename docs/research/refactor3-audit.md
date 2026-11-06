# refactor3 audit: findings, measurements and proposed units

Unit refactor3/1, 2026-11-06, worktree `r3-audit` at `76c9c63`. It audits the diff `049f8d6..76c9c63`: 210 commits, 405 files, +34,854 / −9,264 lines. That runs from refactor2's base (release 0.1.4) to today and covers GitMan gv-3 … gv-31 and fix-u5 … fix-u18.

This is a read-only audit. Nothing under `crates/`, `plugins/`, `scripts/` or `Cargo.*` changed. The timing harnesses were uncommitted files under `crates/`, removed after the run; copies are in `target\tmp\r3\` (`r3_timing.rs`, `app_r3_timing.rs`).

**Method.**
- Four read-only reviewers ran in parallel, each briefed on CLAUDE.md: C144 1–8, the shared-components list, and present-tense comments. Their scopes:
  - `supraman-gitman`, `supraman-editor`, supraman-ui `table*`/`tree*`/`segmented*`/`split*`/`themes*`, and supraman-core `watch`/`time`/`text`;
  - `git_view.rs`, `git_view/**` and `selftest_git*.rs`;
  - the rest of the diff (app outside the Git tab, ui, core, supervisor, claude, workman, term, sm);
  - every test added or changed.
- I checked the highest-risk claims against the code myself: A-302, A-301, G-201, G-101 and P-501.
- **Workspace tests.** `cargo test --release --workspace --no-fail-fast` ran once from 08:13 to 08:18. Result: **1184 passed, 0 failed, 4 ignored**, the baseline. Neither known flake fired.
- **Per-test times.** I ran the seven slowest test binaries again with `--report-time` (see Tests).
- **Measurements.** I timed the read layer on `fixture::Generated` repos of 2k, 20k and 100k commits, each with 1,000 branches and 1,000 remote branches added, and one with a 20,000-file commit. I timed a Git tab end to end in a gpui test: 20k commits, 2,000 refs, a 6,000-line file edit and a 3,001-file commit. Git processes were counted with `GIT_TRACE2`. Every number is in "Measurements".
- **Not run:** any `--selftest*`, as the brief asks.

**Columns** are those of the m5/44 and refactor2 audits:
- **Risk** is the risk of the fix.
- **Prot.** marks protected logic: WS, OAuth, Mint, CLI, Del, C15/17/21, Sched, Update.
- **Sev.**: *blocker* means a security hole, a UI freeze or data loss in normal use; then *major*, *minor*, *nit*.

**gv-34.** Findings in `git_view/ref_panel/**`, `git_view/details/**` or the loading/flash paths are marked **gv-34**: that unit may be changing them now. Re-check each against gv-34's merge before you start it.

**Id ranges:**

| ids | area |
|---|---|
| P-5xx | measured performance |
| G-1xx | Git tab (`git_view/**`) |
| G-2xx | `supraman-gitman`, `supraman-editor`, the new supraman-ui and core modules |
| A-3xx | the rest of the diff |
| T-4xx | tests |
| X-6xx | build infrastructure |

## Summary

- **One blocker, A-302.** A Ctrl+click on a path in terminal output now runs the file. Path hyperlinks came in with fix-u8/u14, and `Link::Path` goes to `ShellExecuteW("open")` (`view.rs:818`, `open.rs:28`). A claude session or `cat` that prints `tools\setup.bat`, `x.js`, `y.lnk` or `z.msi` turns one Ctrl+click into running it. That is the code-execution path sched2/11 closed for URL schemes. The fix is small (U1).
- **Correctness, majors:**
  - **A-301 (Sched):** a dead scheduled run's kept session is never retried by the app. `recover()` passes the stem `<id>.dead`, not `<id>`.
  - **A-303 / A-304 (Del):** a multi-edit Save that fails half-way leaves the dialog unable to save again. Rescan's `DropGone` stops sessions without the confirm.
  - **G-201:** the gitman test fixture inherits `GIT_DIR` and `GIT_INDEX_FILE`. A `cargo test` run from a git hook writes into the user's repo (C151).
- **The Git tab is correct and keeps clause 4 everywhere.** No gitman call runs on the UI thread, and the stall watch saw no section over 100 ms in any timed action. Its cost lies in four places:
  1. **P-501:** every git call waits about 23 ms longer than git runs, because `process::run_raw` polls `try_wait` with a 20 ms sleep. Measured: 42.4 ms through `run_bytes` against 19.3 ms bare. Every Git tab action pays this per process: 8 per graph refresh, 3 per selection, 2 per diff.
  2. **G-101:** every graph refresh re-reads every loaded commit and lays them all out again. The trigger is any ref change, the window coming back, an eye or a solo.
  3. **G-203:** each history page re-walks the whole history. Measured on 100k commits: 451 ms per page of 200, the same as the first page of 2,000.
  4. **UI-thread stalls on very large inputs, all unbounded:**
     - G-103, G-104: a commit's `ls-tree -r` sorted and filtered on every selection, and an O(files × changes) lookup per file step.
     - G-106: text diffs have no size cap, and their buffers are built on the UI thread.
     - P-506: `Tree::build` costs 68 ms at 100k paths.
- **Clean code.**
  - In the GitMan crates: no fn over 80 lines, no file over 1,500 lines, no new `env::var` and no new `Command::new`.
  - The rest of the diff:
    - two new fns over 80 lines;
    - seven new `FleetView` fields (C144 §6);
    - three new `std::env::var` reads outside the owners (C144 §3);
    - three UI-thread `Settings` reads or writes (R-105 still open).
  - The other repeats are small: four short-sha helpers, two filter boxes, two path labels.
- **Tests.**
  - The supraman-app binary takes 40–53 s, and its Git tab tests take 14–24 s each when they run in parallel.
  - The causes:
    - about 58 tests each build the full `Fixture` (about 55 git processes, each 42 ms by P-501);
    - the two `live_refresh_*` tests wait on 1 s and 3 s wall-clock deadlines over 200 ms and 2 s debounces;
    - most of the `selftest_git*` steps repeat gpui tests.
  - **X-601:** the shared target dir lets another worktree's build of `supraman` replace this worktree's binary under the same file name. My first harness run ran gv-34's 299-test binary until I touched the sources.

| Area | Findings | blocker | major | minor | nit |
|---|---|---|---|---|---|
| P measured | 8 | – | 4 | 3 | 1 |
| G Git tab | 32 | – | 8 | 12 | 12 |
| G read layer / editor / ui / core | 22 | – | 3 | 10 | 9 |
| A rest of diff | 28 | 1 | 3 | 17 | 7 |
| T tests | 18 | – | 6 | 6 | 6 |
| X infra | 1 | – | 1 | – | – |
| **Total** | **109** | **1** | **25** | **48** | **35** |

Some ids are cross-references to the same defect (P-502 = G-203, P-503 = G-101, P-504 = G-217, P-505 = G-118); counted once: 105 findings, 1 blocker, 23 major, 46 minor, 35 nit.

**The top five:**
1. A-302, the path link that runs the file.
2. P-501, the 20 ms poll on every child process.
3. G-101, the graph refresh that re-reads everything loaded.
4. A-301, the dead run never retried.
5. G-201, the fixture that can write into the user's repo.

## Measurements

All are release builds on René's machine (Windows 11, Git for Windows). A second worker built in parallel during some runs, so where a value was taken twice I give the quieter one and say so.

### Read layer (`supraman_gitman`, direct calls; best of n)

| Call | 2k commits | 20k commits | 100k commits |
|---|---|---|---|
| one git process through `run_bytes` (`locate::check`) | 42.4 ms | – | – |
| one bare `git --version` (PowerShell loop) | 19.3 ms | – | – |
| `refs()`, 2,000 refs, 5 processes | 420 ms | 437 ms | 520 ms |
| `status()`, 2 processes | 87 ms | 86 ms | 86 ms |
| `history(first 2000)` | 66 ms | 126 ms | 471 ms |
| `history(skip 10000, 200)`, one later page | – | 124 ms | 451 ms |
| `history(first 10000)`, a refresh after scrolling | – | 179 ms | 504 ms |
| `history(all)` | – | 230 ms | 974 ms |
| `Layout::new + extend(2000)` | 3.1 ms | 3.2 ms | 3.7 ms |
| `Layout::new + extend(all)` | – | 16 ms | 74 ms |
| `details`, `changed_files`, `patch(Commit)`, `stashes` on a small commit | 42–45 ms each (the spawn floor) | | |

On a commit of 20,000 files (20k-commit repo):

| Call | Time |
|---|---|
| `tree()` | 71 ms |
| `changed_files` | 66 ms |
| `patch(all files)` | 537 ms |
| `patch(one path)` | 43 ms |
| `status()` on a 20,000-file worktree, clean | 228 ms |
| `status()` with 2,000 untracked files | 228 ms |
| `ignored(2,000 paths)` (`check-ignore --stdin`, run per work-dir batch on the watcher thread) | **730 ms** |
| `Tree::build` (supraman-ui), 3k paths | 1.9 ms |
| `Tree::build`, 20k paths | 13.2 ms |
| `Tree::build`, 100k paths | 67.8 ms |

Layout is cheap: 74 ms for 100k rows, linear. A page of 200 laid out costs the same per row as all at once. Time goes to process count, `rev-list --date-order` re-walks, and `refs()`.

### Git tab (gpui test, 20k commits, 2,000 refs; git processes from `GIT_TRACE2`)

gpui's test dispatcher runs background tasks on the test thread, one after another. So these times are end-to-end upper bounds of background plus foreground work, not UI-thread time. The UI-thread share is bounded by `stall`: no `stall::section` reported over 100 ms in any row.

| Action | Time | git processes | Which |
|---|---|---|---|
| open (first page) | 1.9 s | 9 | `--version`, for-each-ref, rev-parse HEAD, symbolic-ref, worktree list, `log -g` stash, status, rev-parse git-dir, rev-list |
| redraw (notify + draw), graph only | 7.9 ms (quiet run), 24 ms (loaded run) | 0 | |
| select a commit | ~330 ms each (10 in 3.3 s) | 3 | rev-list `--no-walk`, diff-tree, `ls-tree -r --full-tree` |
| select a 3,001-file commit | 600 ms | 3 | as above |
| redraw with that commit's files shown | 24 ms (loaded run) | 0 | |
| open a diff of a 6,000-line file (120 changed lines) | 870–970 ms | 2 | `cat-file` ×2 |
| close the diff | 92 ms | 0 | |
| redraw with the diff open | 22 ms (loaded run) | 0 | |
| `refresh_graph` (a ref moved) | 2.4 s | 8 | `load::read` again |
| `refresh_wip` (a work-dir file changed) | 470 ms | 2 | status, rev-parse git-dir |

- `size_of::<GraphRow>()` is 256 B and `Row` is 120 B inline. Edges, ref groups, the body and the parents add heap per row, so a scrolled-out history of 100k rows holds tens of MB (G-124).
- **Not measured: the cost of a refresh after paging to 10k commits.** `Table::reveal` does not trigger `on_near_end` in the headless test, so the harness stayed at 2,000 rows. The read layer's `history(first 10000)` row above prices it instead: 179 ms on 20k commits, 504 ms on 100k, plus layout.

### Process count per user action and per watcher batch (from the code, confirmed by the trace above)

| Event | git processes | Thread |
|---|---|---|
| open a tab | 10: 9 above + `rev-parse` dirs for the watchers | background |
| select a commit or stash | 3 | background; `set_tree` sort on the UI thread (G-103) |
| select the WIP row | 2 | background |
| open a commit file's diff | 1–2 `cat-file` | background; buffers built on the UI thread (G-106) |
| open an unstaged diff | 2 (`cat-file :0`, `rev-parse --show-toplevel`) + fs | background |
| page near the end | 1 `rev-list` (full walk, G-203) | background |
| eye, solo, Show all | 8 | background |
| window activated | 8 per open Git tab, hidden ones too (G-110); +2 with WIP selected; +2 with a work-tree diff open | background |
| watcher batch on refs/HEAD | 8 + 2 (WIP panel) + 2 (diff reload) = up to 12 | background |
| watcher batch on the index | up to 6, or 14 when the WIP row comes or goes | background |
| watcher batch on the work dir | `check-ignore` (730 ms per 2,000 paths) + up to 6 | watcher thread + background |

### Test times (`--report-time`, the slowest binaries run again alone)

| Binary | Tests | Wall | Slowest tests |
|---|---|---|---|
| supraman-app (bin) | 295 | 40.2 s (53.0 s in the workspace run) | every Git tab gpui test takes 14–24 s in parallel: `git_tab_reopens_at_its_repos_layout…` 23.7, `picking_a_ref_selects_its_commit…` 21.1, `a_solo_walks_only…` 20.3, `hiding_a_branch_prunes…` 20.3, `live_refresh_shows_a_new_commit…` 18.9 |
| supraman-core | 228 | 17.3 s | `process::tests::a_timeout_kills_the_tree_or_only_the_child` 17.3 s (the binary's whole wall) |
| supraman-term | 100 | 13.6 s | – |
| gitman `tests/readers.rs` | 13 | 7.5 s | 13 × `Fixture::new()` |
| gitman `tests/read_only.rs` | 1 | 6.9 s | – |
| workman `tests/oauth.rs` | 21 | 6.3 s | – |
| gitman unit | 23 | 4.3 s | – |

A Git tab test runs in under 2 s alone (open is 1.9 s on a 20k repo, about 0.5 s on `Fixture`). The 14–24 s comes from about 50 tests each building a `Fixture`, 55 processes at 42 ms, all at once on the machine's cores (T-401, P-501).

## Findings

### P. Measured performance

| ID | Where | Problem | Fix | Risk | Prot. | Sev. |
|---|---|---|---|---|---|---|
| P-501 | `core/process.rs:127-138` (`run_raw`) | Every child outside the PTY waits in a `try_wait` + `sleep(20 ms)` loop, and pipes settle in a 10 ms loop (:148-151). Git exits in about 19 ms, so the first `try_wait` misses and every call costs one more sleep. Measured 42.4 ms against 19.3 ms bare. That is +180 ms per graph refresh (8 calls), +70 ms per selection (3), and about 1.3 s per `Fixture::new()` (55 calls). Every `sm`, claude and schedule child pays it too. | Wait on the process handle with a timeout (`WaitForSingleObject(handle, remaining)`, or a waiter thread plus `recv_timeout`). Keep the `stop` callback on a short timeout only for `run_watched`. Join the drain threads instead of polling them. | med (every child goes through it; the Kill::Tree and timeout semantics must hold) | C15/17, Sched | major |
| P-502 | `gitman/history.rs:52-63` | Paging re-runs `rev-list --date-order --skip=N`. `--date-order` walks the whole history before it prints, so each page of 200 costs a full walk: 451 ms at 100k commits, and seconds at 1M. Scrolling k pages is O(k × history). | Keep one streaming `rev-list` per walk on a loader thread and read records as pages are asked for. Or page with `^<last page's boundary>` exclusions. (= G-203) | med | none | major |
| P-503 | `git_view/refresh.rs:134` | `refresh_graph` reads `max(commits, FIRST_PAGE)` commits and replaces every row. 2.4 s end to end with 8 processes at 2,000 rows. After scrolling, every refresh re-reads every loaded commit. (= G-101) | see G-101 | med | none | major |
| P-504 | `gitman/refs.rs:94-100, 233-241`, `status.rs:55-58`, `blob.rs:33` | `refs()` takes 5 processes (420–520 ms with 2,000 refs). `head()` alone takes 2, and `status()` and `worktree_file()` each add a `rev-parse` the tab already knows (`Dirs`). | Read HEAD in one `rev-parse --symbolic-full-name HEAD HEAD^{commit}` or from for-each-ref's `%(HEAD)`. Pass `&Dirs` to `status` and `worktree_file`. Together with P-501, a graph refresh drops from 8 processes to 5. | low | none | major |
| P-505 | `git_view/live.rs:115-122`, `gitman/dirs.rs:25` | `check-ignore --stdin` costs 730 ms per 2,000 paths, and runs on every work-dir batch. A cargo build sends thousands of `target/` paths per 2 s batch. It is off the UI thread, but it occupies the watcher thread and a core. | Drop paths under a directory `check-ignore` already reported ignored (cache ignored prefixes per tab), and send each directory once rather than every file under it. | low | none | minor |
| P-506 | `supraman-ui/tree.rs:58-108, 124-152, 307` | `Tree::build` costs 13 ms at 20k paths and 68 ms at 100k. `set_tree` re-sorts (`sort_all` clones each folder's children, :141-152) and re-filters on the UI thread. At 100k files every commit selection stalls a frame (G-103). | Build and sort in the background and hand over a sorted tree. `std::mem::take` the children in `sort_all`. | low | none | minor |
| P-507 | `git_view/diff/mod.rs:153-156`; `supraman-editor/diff.rs:40-45, 206-211` | Opening a 6,000-line diff took 870–970 ms end to end. The ropes are built in `cx` and the hunks walked in `cx.update` on the UI thread. That stayed under the 100 ms stall bar here, but it grows with file size and has no cap (G-106). | see G-106 | low | none | minor |
| P-508 | `git_view.rs:526`, `cells.rs:33-47,151`, `graph.rs:56`, `colours.rs:22`, `time.rs:144-162` | Per frame and visible row: `wip.counts()` is formatted for every message cell, `time::stamp` re-parses the locale pattern, `ref_cell` clones `RefGroup`s, `graph::cell` collects an edges `Vec`, and `Palette::of` locks a mutex. Redraw measured 7.9 ms, so this is not urgent. | Format the date and WIP counts once per row in `load` (background), pass `&RefGroup`, parse the locale pattern once in `user_locale`, and cache the `Palette` per theme id. | low | none | nit |

### G-1xx. Git tab (`git_view.rs`, `git_view/**`, `selftest_git*.rs`)

| ID | Where | Problem | Fix | Risk | Prot. | Sev. |
|---|---|---|---|---|---|---|
| G-101 | `refresh.rs:127-139, 144-173` | Every graph refresh (a ref change, window activation, an eye, a solo, Show all) re-reads every loaded commit with full bodies and lays out all of them again. At 50k loaded commits that is seconds of CPU and a `rows` swap per agent commit. | Read the first page only. Keep loaded rows below the first row whose sha, lanes and refs are unchanged, or reset to the first page when the walk changed. | med | none | major |
| G-102 | `git_view.rs:292-300, 508-510` | **gv-34.** `reveal_sha` pages until the sha appears or `more` is false. A ref outside the walk (a hidden branch, a tag while soloing, a hidden stash) pages through the whole repo, and `measure_lanes` rescans every row per page on the UI thread (O(n²)). | Check `Entry::walked` before seeking, cap the seek, and say "not in the graph". | low | none | major |
| G-103 | `details/load.rs:71`, `details/mod.rs:189` | **gv-34.** Each selection runs `ls-tree -r --full-tree` even with View all files off, and `set_tree` sorts it on the UI thread (P-506). | Run `ls-tree` when View all files turns on. Build `files` from `changes` otherwise, and sort off-thread. | med | none | major |
| G-104 | `details/mod.rs:295-318`, `wip/mod.rs:237-246` | **gv-34.** `file_order` runs `changes.iter().find` for each tree path, so it is O(files × changes) per Up/Down in the viewer: 100k × 2k = 2·10⁸ compares per key. | Index the changes by path in `Shown`. | low | none | major |
| G-105 | `diff/mod.rs:166-170`, `refresh.rs:236-244` | A reload of the open work-tree diff builds new editors, so the scroll position and current hunk reset. An agent editing the viewed file throws the view back to the top every 2 s. | Keep the scroll anchor and hunk across `show`, or diff into the existing buffers. | med | none | major |
| G-106 | `diff/load.rs:110-133`, `diff/mod.rs:153-156`, `gitman/blob.rs:18` | No size cap: a 50–200 MB blob is read whole and decoded, and its buffers are built on the UI thread, which freezes it. Binaries are also read whole just to show their sizes. | `cat-file -s` first. Above a cap (5–8 MB), show the size only. Probe binaries from the first 8 KB. | low | none | major |
| G-110 | `refresh.rs:103-119` | Window activation runs an 8-process full refresh in every Git tab, hidden ones too, though the watchers already cover changes. | Refresh only the visible tab, and only when its watcher is missing or failed. | low | none | major |
| G-121 | `ref_panel/render.rs:31-34,78,198`, `entries.rs:126-152` | **gv-34.** Each section header calls `counts()`, about 6 per frame, each over every ref with `matches(filter)` and a `String` per entry. At 10k tags that is about 60k allocations per frame. | Count once per `set_refs`, `set_filter` or walk change. | low | none | major |
| G-124 | `git_view.rs:131, 498` | `rows` keeps every page scrolled. With 100k rows that is tens of MB per tab, and G-101 re-reads all of it on each event. | Cap the loaded rows, or drop the tail when far off-screen. | med | none | minor |
| G-107 | `diff/image.rs:44` | No pixel cap: a 20000×20000 PNG decodes to 1.6 GB of RGBA. | Read the header first and refuse anything over about 64 Mpx. | low | none | minor |
| G-108 | `diff/image.rs:72-83` | The checkerboard paints about 6k quads per repaint, twice that in Split. | One cached tiled image. | low | none | minor |
| G-109 | `refresh.rs:187, 211, 229` | With the WIP row selected, each batch runs `status` (2 processes) twice: once in `load::wip` and once in `wip_panel.refresh`. | Pass the read `Status` to the panel. | low | none | minor |
| G-111 | `refresh.rs:144-173` vs `git_view.rs:443, 499` | `apply_refresh` does not call `measure_lanes`, so a refresh that adds lanes clips them, and the column cannot widen. | Call it. | low | none | minor |
| G-112 | `git_view.rs:270, 401-402`; `keys.rs:130-136` | A parent click, H and Shift+H do nothing when the target's page is not loaded. | `reveal_sha` (after G-102). | low | none | minor |
| G-114 | `refresh.rs:85-87, 148, 223` | A failed refresh or watcher start is dropped or only logged. The tab shows stale data without a sign. | Set `page_error` or a banner. | low | none | minor |
| G-118 | `live.rs:115-122` | = P-505. | | | | minor |
| G-120 | `git_view.rs:471-473, 499` | `measure_lanes` scans every row per page. | Keep a running max. | low | none | nit |
| G-122 | `ref_panel/mod.rs:105-115`, `entries.rs:172-178` | **gv-34.** `entries()` sorts on the UI thread on every refresh, with `by_parts` lowercasing two strings per comparison. | Build the entries in `load::read` and pre-lower the keys. | low | none | minor |
| G-128 | `cells.rs:155`, `details/render.rs:26`, `ref_panel/render.rs:108`, `wip/load.rs:99` | Four copies of the 7-character short sha (C144 §1). | One `short_sha` beside `fonts::figures`, or in gitman. | low | none | minor |
| G-129 | `diff/render.rs:19-31` vs `details/files.rs:97-113` | **gv-34.** Two "folder muted, name bright" path painters. | One `path_label`. | low | none | minor |
| G-130 | `details/mod.rs:261-274` vs `ref_panel/mod.rs:239-252` | **gv-34.** Two copies of the filter box (a `TextInput`, a subscription to `Edited`, `set_filter`). | A shared `text_input::filter_box`. | low | none | minor |
| G-131 | `git_view.rs:617-631`, `details/render.rs:48-60`, `diff/render.rs:104-120` | Three centred "title + detail" placeholders. | One shared placeholder. | low | none | nit |
| G-132 | `colours.rs:24` vs `view/left.rs:75` | The secondary text colour is defined twice. | `Palette::of` uses `secondary`. | low | none | nit |
| G-115 | `git_view.rs:646-650` | A load `Problem` such as "no git" has no Retry. | Retry calls `reload`. | low | none | nit |
| G-116 | `load.rs:139` | `Settings::load(..).ok().unwrap_or_default()` drops a bad `[gitman] git` without a notice (C144 §5). | `load_with_notices`, and the notice in `Problem`. | low | none | nit |
| G-117 | `refresh.rs:130` | **gv-34.** An eye or solo set before the first load ends is lost. | Re-read the walk in `apply_first`. | low | none | nit |
| G-123 | `details/mod.rs:220-229`, `wip/mod.rs:184-195` | **gv-34.** A sort-order change clones the whole `Shown`, including the file `HashSet`. | Keep the changes in their own `Rc`. | low | none | nit |
| G-126 | `load.rs:139-140` | `Settings::load` and `git --version` run on every tab open. | Cache the located `Git` per setting. | low | none | nit |
| G-133 | `details/mod.rs:36` | **gv-34.** The `PanelEvent::File` doc says "against the first parent", but the WIP panel sends work-tree targets. | Delete the comment. | low | none | nit |
| G-134 | `refresh.rs:1-2` | The module doc says refreshes keep "the user's place", which is false for an open diff (G-105). | Fix G-105; the comment then holds. | – | none | nit |
| G-135 | `git_view.rs:59-60`; `details/load.rs:67` | **gv-34.** The `GitSizes` doc lists its fields; `&stash.sha.clone()` is a needless clone. | Drop both. | low | none | nit |
| G-137 | `selftest_git.rs:395-397, 414` | `themed` writes and removes a file on the window's foreground executor. | Background spawn. | low | none | nit |

### G-2xx. Read layer, editor, shared ui and core modules

| ID | Where | Problem | Fix | Risk | Prot. | Sev. |
|---|---|---|---|---|---|---|
| G-201 | `gitman/fixture.rs:35-44` | **Verified.** `run_git` does not strip `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE` and the rest, which `Repo::run_with` strips (`repo.rs:27, 76`). `cargo test` run from a pre-commit or pre-push hook makes the fixture's `commit`, `tag`, `stash` and `worktree` act on the user's repo (C151). It is also a near copy of `run_with` (C144 §1). | Make `REPO_ENV` `pub(crate)` and strip it in `run_git`, or build both on one helper. | low | none | major |
| G-202 | `supraman-editor/diff.rs:206-211` | Both texts are copied (`Arc::from`) and two `Buffer::local` ropes are built in `cx`, with no size cap upstream (G-106). | A cap in `diff/load.rs`; build the ropes off-thread (`Rope::from` in `background_spawn`). | med | none | major |
| G-203 | `gitman/history.rs:52-63` | = P-502. | | | | major |
| G-204 | `supraman-editor/split.rs:138-161, 198-211` | `ruler()` takes 3–4 display snapshots and maps every hunk on every render, and `copy_scroll` notifies on every scroll event. A file with thousands of hunks janks while scrolling. | Cache the marks keyed on the display-map versions; recompute only the viewport box on scroll. | low | none | minor |
| G-205 | `supraman-ui/tree.rs:258, 542-546` | `TreeList::render` finds the selected row by a linear path search every frame, twice per arrow key. | Keep `selected_row` in `Inner`. | low | none | minor |
| G-206 | `supraman-ui/tree.rs:403-406`; app `ref_panel/overlay.rs:81` | `visible().len()` clones every visible path each frame to count them. | `TreeState::row_count()`. | low | none | nit |
| G-207 | `supraman-ui/tree.rs:67-108, 258, 491-494` | A deleted file `x` and an added `x/y` in one change set share the path `x` between a file and a folder, so hit keys, `open` and selection collide. | Key folders with a trailing `/`. | low | none | minor |
| G-208 | `supraman-ui/tree.rs:186-189` | The filter calls `find_all` (a Vec of ranges) per file just to test for a match. | Add `matcher::contains`. | low | none | nit |
| G-209 | `supraman-ui/tree.rs:141-152` | `sort_all` clones every folder's children (P-506). | `mem::take`. | low | none | nit |
| G-210 | `supraman-ui/split.rs:173` | `on_end` (persist the width, the table's `on_resize`) hangs on the grip's `on_mouse_up`. A release past a clamped drag may land off the grip, and the width is not saved. This is unconfirmed in gpui. | Add `on_mouse_up_out`, and a test that releases past the clamp. | low | none | minor |
| G-211 | `gitman/layout/lanes.rs:213-221` | Every row stores a pass-through `Edge` per open lane: 100k rows × 40 lanes ≈ 160 MB. | Store a carried-lanes bitset per row and derive the pass-throughs when painting. | med | none | minor |
| G-212 | `supraman-ui/themes/lanes.rs:31, 86, 102-110` | `FITTED` gains an entry on every theme rebuild and never drops one. | Keep the current id only. | low | none | nit |
| G-213 | `themes/syntax.rs:110-117` vs `themes/lanes.rs:90-100` | Syntax fitting searches one direction by `appearance`; lanes search both. A theme whose `appearance` disagrees with its background misses the 4.5:1 floor. Two contrast searches (C144 §1). | One shared `nearest_holding`. | low | none | minor |
| G-214 | `themes/syntax.rs:114-121` | An alpha-only change is reported as "adjusted". | Compare contrast, not hex. | low | none | nit |
| G-215 | `supraman-editor/languages.rs:255-261` | Extensions match case-sensitively, so `Build.PS1`, `X.CS` and `README.MD` get no highlighting (common on Windows). | `eq_ignore_ascii_case` for extensions. | low | none | minor |
| G-216 | `gitman/refs.rs:249-270` | A tag of a tag peels one level only, so it gets no pill and no tip. | Resolve `^{commit}` for `peeled_kind == "tag"`. | low | none | nit |
| G-217 | `status.rs:253`, `blob.rs:33`, `refs.rs:237-241` | = P-504. | | | | minor |
| G-218 | `gitman/patch.rs:79-80, 113`; `history.rs:99-115` | Two Strings per patch line, and a String per record before splitting. | Use `Cow` or slices. | low | none | nit |
| G-220 | `core/time.rs:172-179` | `pattern` prints `''` as nothing and `MMM`/`MMMM` as `11`. Locales with `dd-MMM-yy` show a number for the month. | Handle runs of 3+ `M` with `%b`/`%B`, and `''` as a literal quote. | low | none | minor |
| G-222 | `core/text.rs` (`decode`, `chunks_exact(2)`) | Odd-length UTF-16 drops its last byte without a U+FFFD. | Append U+FFFD. | low | none | nit |
| G-223 | `gitman/locate.rs:256-258` | A bad `[gitman]` section falls back to the PATH git without a notice. | Return it in `Located::Unusable`, or as a notice. | low | none | minor |
| G-224 | `gitman/lib.rs:37` ("research §2"), `supraman-editor/lib.rs:1` ("rev 03efd49") | One comment cites a doc without its file; another repeats the Cargo pin, which goes stale on the next bump. | Name the file; drop the rev. | low | none | nit |

### A-3xx. The rest of the diff

| ID | Where | Problem | Fix | Risk | Prot. | Sev. |
|---|---|---|---|---|---|---|
| A-302 | `view.rs:816-818, 2305`; `view/links.rs:39-66`; `supraman-ui/open.rs:5, 28` | **Verified.** Path hyperlinks came in with fix-u8/u14. `Link::Path` goes to `open_off_thread` → `ShellExecuteW("open")`, which runs `.exe`, `.bat`, `.cmd`, `.ps1` (by association), `.js`, `.vbs`, `.hta`, `.lnk`, `.msi`, `.reg` and the like. `resolve` also picks the longest existing prefix of a spaced path. Terminal output from claude, `cat` or `git log` decides the target, and a Ctrl+click runs it. `file:` in `LINK_SCHEMES` was already there at 049f8d6 and opens the same way. | In `supraman_ui::open`, refuse a path or `file:` target whose extension is in `PATHEXT` or a fixed list (`.lnk .url .hta .js .jse .vbs .vbe .wsf .wsh .scr .msi .msp .ps1 .reg .cpl .application .appref-ms .jar`). For those, open the folder (`explorer /select,`) or show `refused`. Add a unit test per extension. | low | C15/17 (sched2/11 allowlist) | **blocker** |
| A-301 | `app/schedules.rs:186-200`; `sm/fire.rs:577-583, 488` | **Verified.** `recover` takes each `*.json` stem as a run id. For `<id>.dead.json` that gives `<id>.dead`, whose `Paths` look for `<id>.dead.dead.json` and parse `<id>.dead.json` as `Live`, which fails. `retry_dead_session` then runs only while a new `<id>.json` exists, that is while the next fire holds the lock, so `try_lock` fails. A dead run whose stop failed keeps its claude session running and spending tokens. The unit tests call `recover_if_dead_with(id)` directly. | Strip `.dead` from the stem (or list ids from both suffixes, once each), plus a test through `recover()`. | low | Sched | major |
| A-303 | `view/folders.rs:56-92, 246` | A multi-edit Save that is refused part way has already written and released the earlier edits, but returns `Err`. The dialog keeps the original `Workspace`, so every later Save repeats the applied `RemoveFolder` and is refused. | Return the partial result plus the refusal, refresh the dialog's original, and say "applied X; refused Y". | low | Del | major |
| A-304 | `view/folders.rs:104-116`, `core/edit.rs:141-146` | `rescan_offer` runs `Edit::DropGone` before the dialog, outside `request_edits`, so a briefly unreachable or newly Hidden subfolder is dropped and its wanted session stopped without the confirm. | Show the gone folders in the dialog, applied by Save through `request_edits`. | med | Del | major |
| A-305 | `supervise.rs:24-41, 290, 405` | A queued sign-out is ended on a later tick. A sign-out and sign-in to the same origin in between makes `end_sessions` remove the records and tokens minted on the new grant. | Tag the queue with a sign-in generation and end only older records; drain it at the top of `tick`. | med | OAuth, Mint | minor |
| A-306 | `view.rs:1931`, `hand.rs:336-417` | `launch_for` holds the global `Hands` lock across up to four 15 s mints. Other tabs' `ended`/`release`, sign-out and quit's `release_all_within` wait or time out. | Pick the name under the lock and mint outside it. | med | Mint | minor |
| A-307 | `settings_window.rs:315-333` | `change` runs `Settings::update` (`lock_file` + `write_atomic`) in the click (C144 §4). | Background, then apply, as `set_max_working` does. | low | none | minor |
| A-308 | `settings_window.rs` `render_shell` → `core/shells.rs:263`; `view/shells.rs:117` | `Shells::for_settings` does `expand_env` + `is_file` per render and per + click (C144 §4). | Resolve at detection and cache it on `ShellDefault`. | low | none | minor |
| A-309 | `view.rs` `FleetView` | Seven new fields: `shells`, `shell_default`, `sends`, `max_writes`, `continued`, `blink_nudge`, `blink_parked` (C144 §6). Ten were removed, so the struct shrank overall. | `ShellsState` and writes entities, and a `CursorBlink` struct. | low | none | minor |
| A-310 | `view.rs:2006, 1216, 1264, 1279` | R-105 is still open: `hand_extras` (`Settings::load` + `is_file` per [+]), `set_workman_url` and `save_workman_url` run on the UI thread. | Into the spawned tasks. | low | OAuth | minor |
| A-311 | `supervisor/executor.rs:162-166`, `view.rs:2008`, `hand.rs:40` | Two builders of "sm's folder first on PATH", and `executor.rs:164` reads `PATH` itself (C144 §1, §3). | `install::sm_first_path()`. | low | CLI | minor |
| A-312 | `core/shells.rs:109-116`, `app/view/start.rs:99` | New `env::var` reads of `PATH`, `SystemRoot` and `ProgramFiles(x86)`, duplicating the private `paths::process_env` (§3). | Make `paths::process_env` public and use it. | low | none | minor |
| A-313 | `core/shells.rs:188-195, 238-252` | WSL output is decoded lossily, then re-encoded with `c as u8` and BOM-stripped by hand (§1 against `text::decode`). Non-ASCII distro names are mangled. | `run_bytes` + `text::decode`. | low | none | nit |
| A-314 | `core/paths.rs:275-284` | `powershell` repeats `first_on_path`'s PATHEXT loop. `first_on_path` and `pwsh_command` accept relative PATH entries; `powershell` does not. | One `first_on_path(absolute_only)`; `executable_on_path` (git) should skip relative entries too. | low | none | minor |
| A-315 | `sm/fire.rs:715-724` | Approved schedule scripts lost `FOREMAN_STATE_DIR`, `RUN` and the rest with no shim and no release note. The fingerprint does not change, so nothing re-asks. | Set both names until a named release, or note it in the release notes and the schedule's Details. | low | Sched | minor |
| A-316 | `supervisor/mint.rs:567-572` | `revoke_deleted_runs` deletes the whole run file when any name is gone. Safe while it holds one name. | Remove only the gone names. | low | Mint | nit |
| A-317 | `supervisor/executor.rs:602-645` | A record minted for a name that never bound to a session (unwanted before bind) stays forever with its secret and config. | Drop unbound records whose name has left the desired state for N ticks (stop, not revoke). | low | Mint | minor |
| A-318 | `sm/lib.rs:198-208`, `workman/health.rs:15-25` | `sm mcp-header` pings Workman (5 s timeout) before every header. A slow server stalls every session's MCP connect. | Skip the ping while the kept token is fresh, or ping only after a 401. | low | WS, Mint | minor |
| A-319 | `view.rs:1625-1634` | A Send failure that lands after compose closed falls into `_ => {}` and is never shown. | Into `action_error`. | low | CLI | minor |
| A-320 | `view/start.rs:107-114`, `core/prune.rs` | Prompt files in `<home>/prompts` left by a closed or crashed start are never pruned, so user text stays on disk. | Add `prompts/` to `prune::prune`. | low | none | minor |
| A-321 | `view/start.rs:202` | `fs::remove_file` inside `this.update` (§4). | Background executor. | low | none | nit |
| A-322 | `view/right.rs:26` | The tab hover card runs `branch_at`, a `.git` walk with file reads, while building on the UI thread. | Use the branch the poll already reads. | low | none | minor |
| A-323 | `workman/updates.rs:323-333` | After Cancel, the reader thread blocks until the agent's 30-min timeout, holding the socket. | A read timeout of `DOWNLOAD_STALL`. | low | Update | nit |
| A-324 | `core/process.rs:117-122` | The job is created after `spawn`, so a fast grandchild escapes it. If `limit()` fails, `Job::adopt` returns `None` silently. | `CREATE_SUSPENDED`, assign, resume; log the failure. Do it together with P-501. | med | Sched | minor |
| A-325 | `executor.rs:831`, `hand.rs:19` | `launched_by = "foreman/…"` is left from the rename. | Rename together with Workman, or record why it stays (a Workman ref). | med | WS | nit |
| A-326 | `view.rs:2293` `start_terminal_env` (83 lines), `hand.rs:336` `launch_for` (82) | Two new fns over 80 lines (§6). Eight more over 80 grew by 2–12 lines: `render_right` 138, `path_match` 166, `fire_inner` 129, `Supervision::tick` 126, `session_stage` 106, `stream::connect` 100, `run_action` 96, `render_appearance` 167. | Split when next touched; refuse growth at review. | low | none | minor |
| A-327 | `view/shells.rs:17-20, 170` | `SLOTS` is a hand-made array of 24 `'static` keys, so a 23rd unnamed shell drops silently. One line is not rustfmt'd. | A `SharedString` key on `MenuItem`; run rustfmt. | low | none | nit |
| A-328 | `claude/launch.rs` (`mcp_header_command`), `mint.rs:528-544` | A home with `%VAR%` or `$` expands in the headersHelper line. `revoke_deleted` trusts that `claude agents --json --all` lists every session. | Pass the home by env; confirm with `find_transcript` before revoking. | low | Mint, CLI | nit |

**Mechanical, rest of diff:**
- Files over 1,500 non-test lines: `view.rs` 2,904 (was 2,933) and `view/left.rs` 1,532 (was 1,966). Neither grew.
- `Command::new` outside `supraman_core::process`: 0, production and tests.
- refactor2 findings fixed at HEAD: R-101, R-102, R-109, R-112, R-113, R-116, R-117, R-118, R-120, R-207, R-209, R-210 (partly, see A-324), R-212 … R-218, R-222.
- Still open: R-105 (A-310).

### T-4xx. Tests

| ID | Where | Problem | Fix | Risk | Sev. |
|---|---|---|---|---|---|
| T-401 | about 58 `Fixture::new()` calls: `gitman/tests/readers.rs` (13), `git_view/tests.rs` (10), `ref_panel/tests.rs` (6), `details/tests.rs` (5), `wip/tests.rs` (5), `keys_tests.rs` (4), `media_tests.rs` (4), `live_tests.rs` (4) and more | Each builds about 55 git processes (2–5 s on Windows with P-501). Most callers only read. This is the main reason a Git tab test takes 14–24 s in the parallel run. | `Fixture::shared()` (a `OnceLock` in `test-support`) for read-only tests; `read_only.rs` already proves the readers leave it unchanged. Keep `new()` for writers (live, conflict, the worktree merge, the solo branch). | med | major |
| T-402 | `git_view/live_tests.rs:59, 71, 117` | **Known flake.** A 1 s deadline over a 200 ms debounce + `load::read` (8 processes); 3 s over a 2 s debounce + `check-ignore` + status. A failed read is dropped with no retry (`refresh.rs:148, 223`), and `until` busy-spins with `yield_now` (:26). | A debounce seam (`live::start_with(repo, git, work)`, 20/50 ms in tests), deadlines about 10 s, and a short sleep in `until`. | low | major |
| T-403 | `core/schedule/tests.rs:640-683` (asserts :670, :676) | **Known flake.** `prune` must return no errors right after a `remove_dir_all` of fresh files and a `git worktree remove` of a fresh checkout, which Defender or the indexer often hold briefly. The test's git (:634) is not isolated from global or system config. | `GIT_CONFIG_NOSYSTEM`/`GLOBAL` (or `fixture::git_in`); split out a pure "is spent" test; keep one integration path with a short retry. | low | major |
| T-404 | `selftest_git_refs.rs:1-5` | The header says a unit test cannot click rows, eyes or solos or send Ctrl+J. `ref_panel/tests.rs:78, 138, 179, 202, 245, 268, 307` do all of that (C144 §7). | Delete the step (or keep `counts` as a smoke check) and fix the header. | low | major |
| T-405 | `selftest_git.rs:156, 202, 214, 234, 260, 281, 308, 326, 394` | `refs`, `select`, `files`, `diff`, `stash`, `keys`, `wip`, `live` and `themed` repeat gpui tests: `tests.rs:199, 335`; `details/tests.rs:43, 115, 206, 231`; `diff/tests.rs:115-317`; `keys_tests.rs:103, 149`; `wip/tests.rs:61, 143`; `live_tests.rs:52`. The header justifies only the sidebar icon, real watchers and real fonts. | Keep `open`, `narrow`, `panel_min` and `reopen`, plus one real-watcher check; cut the rest. That means less selftest time and less disturbance of René's typing. | low | major |
| T-406 | `core/process.rs` `a_timeout_kills_the_tree_or_only_the_child` | 17.3 s, the whole supraman-core binary's wall time. | Shorter timeouts (the test proves the kill, not the duration), or run its two cases in parallel. | low | major |
| T-407 | `selftest_git_media.rs:1-2`, `selftest_shells.rs:1-2` | No "why not a unit test" line (§7). The media step's bom part repeats `media_tests.rs:108`. | Add the reason (real atlas, real PTY), or move the step down; drop bom. | low | minor |
| T-408 | `gitman/tests/paging.rs:9`, `git_view/tests.rs:252`, `ref_panel/tests.rs:213`, `keys_tests.rs:104, 121`, `layout/tests.rs:160` | `Generated` repos of 20k, 4k, 1k, 700 and 200 commits are rebuilt per test (1.3 s for 20k, measured). | `Generated::shared(n)` per size; live tests keep their own. | low | minor |
| T-409 | `git_view/tests.rs:251-272`, `diff/tests.rs:246-261` | "No stall" asserts on wall time (over 100 ms), at risk under parallel load. | Assert that the work runs off the UI thread; keep latency in the perf selftest. | low | minor |
| T-410 | `gitman/layout/tests.rs:313-322`; `gitman/tests/paging.rs:12, 32` | In debug, the 10k-row layout test asserts only the length and one `max_lane`. `paging` asserts `< 20 s` and `branches.len() > 10` on a deterministic repo. | Exact counts and known rows; make the timing a log line. | low | minor |
| T-411 | `supraman-editor/diff.rs:114, 122, 131` vs `words.rs:121, 128, 153, 159`; `languages/tests.rs:106` vs `:348` | Duplicate word-diff cases (the gpui copies also init fonts), and Razor tested twice. | Keep the `words.rs` unit tests and the gpui injection test. | low | minor |
| T-412 | `core/watch.rs:192`; `core/tests/watch_drop.rs:49` | One-batch assumption (`got.len() == 1` within 150 ms); a busy-spin wait. | Accept the paths across batches; sleep in the loop. | low | minor |
| T-413 | `details/tests.rs:198` | `assert!(late.is_ok())` hides the error. | `.expect`. | low | nit |
| T-414 | `supraman-ui/table.rs:687`, `tree.rs:798` | Painted rows asserted as `<= 12/14`, though the window is fixed. | Assert the exact range. | low | nit |
| T-415 | `selftest_launch.rs:22-37` | Mostly re-reads an argv constant; a `temp_dir()` + `hex6` dir leaks on panic. | `tempfile`; keep the `--mcp-config` adjacency check. | low | nit |
| T-416 | `gitman/tests/locate.rs:15` | A pure `judge` parse in an integration binary. | Move it to a unit module. | low | nit |
| T-417 | `selftest_supervise.rs:608` (3 s), `selftest_manage.rs:339` (800 ms) | Fixed negative waits ("nothing happens"). | gpui tests with the clock advanced by hand. | low | nit |
| T-418 | test counts across the range | Tests went down: sm skills 9→4, hook 4→2, schedule 14→12; term hyperlinks 40→21; core home 5→1; `view/manage` 3→0; fake_claude 6→3. | Confirm each was a deliberate merge (units fix-u13, fix-u14), not lost coverage. | low | nit |

**Checked and clean:**
- No `Command::new`, hand-rolled server or `TcpListener` in the new tests.
- The new GitMan tests assert exact fixture SHAs, hit-key geometry and clocks advanced by hand.

### X-6xx. Infrastructure

| ID | Where | Problem | Fix | Risk | Sev. |
|---|---|---|---|---|---|
| X-601 | the shared `CARGO_TARGET_DIR` (`P:\Projects\LarchSys\AutoWork\target`) across worktrees | Cargo hashes a workspace member by its path relative to the workspace root, so every worktree's `supraman` test binary has the same name (`deps\supraman-a93bc72db5c07c1f.exe`) and the same fingerprint files. A build in one worktree replaces another's binary. A later `cargo test` there can see "fresh" and run the other worktree's code. **Observed here:** my harness ran gv-34's 299-test binary, with 0 tests matching, until I touched the sources. Units that merge on "cargo test passed" may have tested another branch. | A target dir per worktree (`<worktree>\target`, or `target\wt-<name>`) with shared registry and git caches, or `CARGO_TARGET_DIR` per unit in the controller's brief. At least `touch` the crate roots before a gate's test run. | low | major |

## Proposed units, in the order I would run them

| # | Unit | Findings | Why this order | Size |
|---|---|---|---|---|
| U1 | **Path links never run a file** | A-302 (+ a unit test per refused extension) | The blocker; small, and in `supraman_ui::open` only. | S |
| U2 | **Target dir per worktree** | X-601 | Every later gate should test its own code. A controller/brief change plus `scripts/heavy-lock.ps1`; no product code. | S |
| U3 | **Schedules and folders correctness** | A-301, A-303, A-304, A-315, A-320 | Sched and Del protected paths with live consequences: a dead run keeps spending tokens, and Save locks up. | M |
| U4 | **Child processes wait on the handle** | P-501, A-324 | Saves about 23 ms per child everywhere (git, sm, claude). It also shortens every test that spawns (T-401, T-406). It is protected (C15/17, Sched), so it gets its own unit with the process tests. | M |
| U5 | **Git tab refresh churn** | G-101, G-110, G-109, G-111, G-114, P-504, G-126 | Refresh only the first page and splice; only the visible tab on activation; one status per batch; HEAD in one process; `Dirs` passed in. A graph refresh goes from 8 processes and all loaded rows to 5 processes and one page. | M |
| U6 | **Big repos and big files** (after gv-34 merges) | G-106, G-202, G-107, G-103, G-104, P-506, G-121, G-122, G-102, G-112, G-124, G-205 | Size and pixel caps, lazy `ls-tree`, off-thread tree builds, indexed file order, cached ref counts, bounded seek. Every unbounded UI-thread cost. | L |
| U7 | **History streaming** | P-502/G-203, G-211 | One `rev-list` per walk read as pages are asked for; edges stored compactly. A design change in gitman, so it goes after U5's refresh contract. | M |
| U8 | **Test speed and flakes** | T-401, T-402, T-403, T-406, T-408, T-409, T-410, G-201 | Shared fixtures, a debounce seam, the prune test isolated, the fixture env stripped. After U4, the app binary should drop from 40–53 s to well under 20 s. | M |
| U9 | **Selftest cuts** | T-404, T-405, T-407, T-417 | Pure deletion against existing gpui tests. The controller runs the selftest once to confirm. | S |
| U10 | **Clean code, rest of diff** | A-307, A-308, A-309, A-310, A-311, A-312, A-313, A-314, A-321, A-322, A-326, A-327 | C144 §1/§3/§4/§6 items; no behaviour change. | M |
| U11 | **Shared GitMan helpers and small fixes** (after gv-34) | G-128, G-129, G-130, G-131, G-132, G-105, G-108, G-115, G-116, G-117, G-120, G-123, G-213, G-215, G-220, G-223, P-508, G-133 … G-137, G-224 | Extract `short_sha`, `path_label`, `filter_box` and the placeholder; the diff keeps its place on reload; locale `MMM`; case-insensitive extensions. | M |
| U12 | **Mint and session leftovers** | A-305, A-306, A-317, A-318, A-319, A-316, A-325, A-328 | Mint/OAuth-protected, each low-impact today; one careful unit. | M |

Not proposed as units: G-204, G-206 … G-210, G-212, G-214, G-216, G-218, G-222, A-323, T-411 … T-416, T-418. Fold them into the unit that next touches their file.
