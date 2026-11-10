# Developing Supraman

How a unit is built and checked on the development machine: where to work, which cargo command to run when, and
what each costs. The release itself is docs/RELEASE.md.

## Where

- **One target dir for every checkout:** `P:\Projects\LarchSys\AutoWork\target` (the main checkout's). Zed's crates
  take ~15 min cold and the disk holds one copy. `scripts/heavy-lock.ps1` sets `CARGO_TARGET_DIR` to it when the
  shell has none; a target under a `*.worktrees` path builds everything afresh and fails sm's
  `dry_run_prints_the_prompt_the_argv_the_permissions_and_three_fires`.
- **One unit worktree: `P:\Projects\LarchSys\AutoWork.worktrees\unit`.** Every unit runs in it on a branch of its own
  (`git switch -c <unit> master`), and the controller gates and merges from it too. Cargo names a workspace member's
  artifacts by its path below the workspace root, so two checkouts write the same files: a build in another checkout
  either rebuilds every member (~7 min) or, when its sources are older than the artifacts, links the
  other checkout's code. One path for all unit work keeps the members built; a branch switch rebuilds only the crates
  whose files changed.
- **The main checkout builds releases only** (`release` profile). It never cleans the unit worktree's members: see
  the clean rule below.
- **Every cargo command through heavy-lock**, at BelowNormal so the machine stays usable:
  `powershell -File scripts/heavy-lock.ps1 cargo …` (Windows PowerShell 5.1 runs it; no pwsh needed). It waits for a
  running selftest or build (`supraman_core::fs::heavy_job_lock`) and applies the clean rule.

## Which command when

| When | Command (each through `scripts/heavy-lock.ps1`) |
|---|---|
| A unit's inner loop | `cargo test --profile unit -p <crate> <filter>` |
| A unit's gate | `cargo test --profile unit $(powershell -File scripts/affected.ps1)`, then `cargo build --profile unit --workspace` and `target\unit\supraman.exe --selftest` |
| The controller's merge gate | the unit's gate again in the unit worktree, on the unit's branch with master merged in |
| Per batch of merged units, and before a release | `cargo test --profile unit --workspace` |
| A release | `cargo test --release --workspace`, `scripts/release.ps1`, the release gate (docs/RELEASE.md) |

- `scripts/affected.ps1 [-Base <rev>] [-Head <rev>]` prints `-p <crate>` for every workspace crate the diff touches
  (by default the working tree against its merge base with master, untracked files included) and every member that
  depends on one of them; a change to `Cargo.toml`, `Cargo.lock`, `.cargo\` or `rust-toolchain*` means all. It prints
  nothing (and says so on stderr) when no crate changed: a bare `cargo test` would then test the whole workspace.
- Where a test lives and the shared test helpers: docs/ARCHITECTURE.md "Testing". GitMan's integration tests are one
  binary: `cargo test --profile unit -p supraman-gitman --test it <filter>` runs them without its lib's tests.
- The `unit` profile (`Cargo.toml`) is `release` with the workspace members incremental in 256 codegen units; the
  dependencies stay as in `release`. Tests run as fast as under `release`; only the members' compile changes. Its artifacts live in `target\unit`: 48 GB on 2026-11-10 with its
  three dependency variants, 23 GB of it `incremental\` (one `<crate>-<hash>` folder per member, target kind and
  feature set, never collected); 40 GB once heavy-lock had removed the 116 members' unfinalized sessions (below).
- `-p <crate>`, `cargo test --workspace` and `cargo build --workspace` unify the dependencies' features differently
  (dev-dependencies count for tests), so the first run of each kind builds its own variant of ~70 dependencies (7 to
  11 min, once per profile); every variant stays, so later runs build only what changed.
- So a gate builds the changed members twice: its `test -p …` and its `build --workspace` are two feature sets, and
  no order or flag shares them. Measured 2026-11-10 after touching `supraman-work-view/src/work_view/state.rs`, two
  rounds each, cargo's `Finished` (the test step `--no-run`, `-p supraman-work-view -p supraman-app`):

  | Order | first step | second step | the second recompiles |
  |---|---|---|---|
  | `test -p …`, then `build --workspace` (the gate) | 29.5 s, 22.7 s | 23.9 s, 23.3 s | supraman-work-view, supraman-app |
  | `test -p …`, then `build --workspace --all-targets` | 23.9 s, 24.7 s | 27.8 s, 23.5 s | those, sm and term-spike |
  | `build --workspace`, then `test -p …` | 24.1 s, 22.0 s | 24.9 s, 22.5 s | supraman-work-view, supraman-app |

  `--all-targets` saves nothing and builds two more members, so the gate stays as it is: the second build costs about
  one member edit's compile (~23 s here), and it is the one that writes the `target\unit\supraman.exe` the selftest runs.
- The controller creates the unit worktree once (`git worktree add P:\Projects\LarchSys\AutoWork.worktrees\unit
  -b <first unit> master`); each later unit starts there with `git switch -c <unit> master` once the last one merged.

## The clean rule

`scripts/heavy-lock.ps1` cleans the workspace members (never the dependencies) when the profile it builds last built
another checkout. `debug` and `release` share the marker `target\.last-workspace` (older copies of the script on
other branches read it); any other profile keeps its own, `target\unit\.last-workspace`. So the main checkout's release
builds leave the unit worktree's `target\unit` alone, and the unit worktree never cleans while every unit runs in it.

After every cargo run it wraps, heavy-lock also removes `target\<profile>\incremental\<crate>-<hash>\` of each
workspace member whose newest session folder is still `s-…-working`, and says so on stderr, one line per crate. Rustc
ends a session by renaming that folder; Windows Defender scanning a file in it makes the rename fail ("did not
finalize incremental compilation session directory …: Access is denied", about a third of the members' sessions), and
the crate's next build then starts from an older session: the LNK2019s on a member's `.llvm.*` and `drop_in_place`
symbols (docs/research/refactor4-audit.md L-1, L-2). With the folder gone, that next build compiles the crate in full
(seconds to a minute), never the whole target. A folder some process still holds a file in is kept (it says so) and
removed by a later run. Count `-working` folders, not those notes: cargo replays a fresh crate's old notes on every
build (L-3).

When a run fails with an LNK2019 or LNK2001 whose symbols name a member's crate (`_ZN15supraman_gitman…`,
`supraman_ui..table..`), heavy-lock runs `cargo clean --profile <p> -p <those members>` and the same command once more,
and says so; it never retries twice. Dot-sourced (`. scripts/heavy-lock.ps1`), the script only defines its functions:
`$run = Get-CargoRun cargo @('build')` then `Find-LinkMembers (Get-Content saved.log) $run.linked` names the members a
saved log's link errors hold.

A Defender exclusion for `P:\Projects\LarchSys\AutoWork\target` would stop the failed renames at their source and
spare every object file a scan; it needs an administrator (the audit's Q1).

## The one known linker warning

Linking supraman-editor's lib test and `theme_switch` test and supraman-app's bin and bin test prints LNK4217
(`wasm_*`/`wasmtime_*` imported as dllimport but defined in `wasmtime-c-api`, linked statically), counted by cargo as
"1 warning". tree-sitter's `wasm` feature brings it, and Zed's `language`, `migrator` and `multi_buffer` turn that feature
on themselves, so it cannot go without patching Zed. It is harmless and the only warning a build or test of ours prints.

## rust-lld is not used

With `linker = "rust-lld.exe"` for `x86_64-pc-windows-msvc`, supraman-editor's tests, its `theme_switch` test and
supraman-app's bin tests die with STATUS_ACCESS_VIOLATION at the image base. The C# grammar's scanner, built by `cc`
with `/MD`, calls the CRT's allocator through an `__imp_` pointer (`call [rip+…]`); link.exe binds it in the import
table, while lld-link leaves a pointer of its own in `.rdata`, outside the import table, holding the image base (a
run of such pointers, all the same). It also saved only ~4 s of the ~60 s edit cycle: compiling supraman-app, not
linking, is the cost.

## Timings

Measured 2026-11-09 on the 24-core development machine at BelowNormal, cargo's own `Finished … in` (compile only)
and the time holding the lock (compile and tests); each step run twice, the second number shown.

| Step | `release` | `unit` |
|---|---|---|
| One-line edit in `crates/supraman-app/src/git_view`, then `cargo test -p supraman-app git_view::columns` | 2 m 28 s compile, 2 m 33 s in all | 1 m 03 s compile, 1 m 07 s in all |
| One-line edit in `crates/supraman-git-view/src/git_view` (its own crate since dev-speed/2), then `cargo test -p supraman-git-view git_view::columns` | — | 21 s compile, 28 s in all |
| The same edit, then `cargo build -p supraman-app` (the crate, then the app compiled against it) | — | 51 s compile, 52 s in all |
| `cargo test --workspace` after that edit | 2 m 13 s compile, 7 m 08 s in all | 1 m 02 s compile, 4 m 58 s in all |
| One-line edit in `crates/supraman-app/src/work_view` (`state.rs`), then `cargo test -p supraman-app work_view::state` | — | 45 s compile, 46 s in all |
| One-line edit in `crates/supraman-work-view/src/work_view` (its own crate since dev-speed/3), then `cargo test -p supraman-work-view work_view::state` | — | 7 s compile, 8 s in all |
| First `cargo test --workspace --no-run` after moving between two worktree paths | 6 m 41 s | 6 m 54 s |
| The same in one unit worktree after a branch switch that changed supraman-gitman and supraman-app | — | 34 s |
| Cold build of every dependency (once per profile) | ~15 min (LEARNINGS refactor3/3) | 11 m 01 s |

The suite's tests take ~2.5 min summed (`unit`, 2026-11-10: 1,617 tests, 148 s of `finished in` over 48 binaries,
155 s holding the lock with nothing to compile). With rust-lld the unit edit cycle was 59 s.
