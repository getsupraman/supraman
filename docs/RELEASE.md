# Releasing Supraman

## Build a release

```powershell
scripts\release.ps1                 # -> <target dir>\dist\Supraman + <target dir>\dist\Supraman-<version>.zip
scripts\release.ps1 -OutDir D:\out\Supraman
```

`<target dir>` is `CARGO_TARGET_DIR` if set, else `<repo>\target`. The script runs
`cargo build --release --workspace`, assembles the folder below, runs `sm plugin install` into it, writes `VERSION`
and zips the folder (the zip's root is the folder, `Supraman\`). It replaces an existing `-OutDir` only if that holds a
`VERSION` (an earlier release). `<version>` is `<crate version>-<yyyyMMdd>-<git sha>`, with `-dirty` if the tree had
changes.

No signing and no installer in v1: the zip is extracted anywhere (`C:\Tools\supraman` here) and `supraman.exe`
started from there. Unsigned
exes may get a SmartScreen prompt on first start from a downloaded zip (unblock the zip before extracting).

## Layout

```
Supraman\
  supraman.exe
  supraman.pdb                 symbols and line tables: file:line of each frame of a panic's backtrace in foreman.log
  sm.exe                       the CLI controllers and users run
  smw.exe                      sm with no console window: a schedule's task runs `smw.exe schedule fire …`
  fm.exe, fmw.exe              copies of sm.exe and smw.exe (see "The fm name" below)
  VERSION                      version, commit, build time (UTC), claude tested / minimum
  plugins\supraman\
    hooks\hooks.json           the exec-form hooks, run from ${CLAUDE_PLUGIN_ROOT}\bin\sm.exe
    roles\*.md                 role templates
    skills\**
    bin\sm.exe                 the hooks' copy (sm plugin install; the app refreshes it at start)
    bin\fm.exe                 the same copy under the old name
```

The folder is self-contained: it runs from any path, no junction to a repo. Supraman's state stays in
`%APPDATA%\Supraman` (or `SUPRAMAN_HOME`, else `FOREMAN_HOME`), not in this folder.

## How the exes find the plugin

`supraman_core::find_plugin_dir`, shared by `supraman.exe` (start, the `--plugin-dir` of every session it launches)
and `sm.exe` (the repo role template when the app has not written one), looks in this order:

1. `<exe dir>\plugins\supraman` — the release layout;
2. the plugin the exe is inside — `sm.exe` run as `<plugin>\bin\sm.exe` by a hook;
3. `plugins\supraman` in every folder above the exe — a dev build at `<repo>\target\<profile>\` (also with
   `CARGO_TARGET_DIR` elsewhere only if a `plugins\supraman` is above it).

A plugin is a folder with `hooks\hooks.json` and `roles\`. If none is found the app shows
`No Supraman plugin for <exe>: looked next to it at <exe dir>\plugins\supraman (a release) and for plugins\supraman in
every folder above it (a dev build): sessions cannot be started from this build.`

## claude CLI version policy

`supraman_claude::{MIN_VERSION, TESTED_VERSION}` in `crates/supraman-claude/src/cli.rs`; `check_version` applies it.

| claude CLI                   | Supraman                                                       |
|------------------------------|----------------------------------------------------------------|
| below `MIN_VERSION` (2.1.308) | refuses, naming the minimum and why: "update claude"          |
| 2.1.308 … `TESTED_VERSION` (2.1.310) | runs                                                    |
| a newer patch of the tested minor (2.1.311) | runs, says nothing                                     |
| a newer minor or major (2.2.0, 3.0.0) | runs; the toolbar ⚠ (its tooltip says why), `sm` stderr, selftest line; no Inbox notice |

Never block a newer claude (decision 2026-10-26): the CLI auto-updates, and blocking would stop every user on the
day of a release. The minimum is the oldest version every CLI fact Supraman relies on was measured on
(docs/LEARNINGS.md, docs/peer-messaging.md):

- `claude --bg` (`--name`, `--permission-mode`, `--mcp-config`, `--plugin-dir`, `--add-dir`, `--resume`) — m0/2, m2;
- the fields of `claude agents --json --all` — m0/2, m1;
- `--plugin-dir` loading the hooks and skills — m0/3;
- the peer registry's `peerProtocol 1` for direct messages — m5/4 (an unknown protocol falls back to the relay, so
  a change there degrades, it does not break).

When a release passes the gate on a newer claude, raise `TESTED_VERSION` in the same commit. Raise `MIN_VERSION`
only when Supraman starts depending on something an older CLI lacks, and say what in its doc comment.

## Release gate

From the release folder **outside the repo** (e.g. copied to `C:\…\Supraman`): `supraman.exe --selftest` and
`supraman.exe --selftest-supervise` both PASS, and `cargo test --workspace` passes in the repo.

## The fm name

The CLI is `sm` (`smw` without a console). For one release the old name still works, and every copy runs the same code:

- `plugins\supraman\bin\fm.exe` beside `bin\sm.exe` (`install_hook_exe`): a session started from the previous
  release runs its hooks through `bin\fm.exe` until it restarts.
- `sm_allowed_tools` allows `Bash(fm:*)` and the plugin's `bin/fm.exe` next to the `sm` rules, so a session whose
  prompt, role or skill still says `fm …` runs it without a permission prompt.
- `fm.exe` and `fmw.exe` beside `sm.exe` and `smw.exe`: a scheduled task registered by the previous release calls
  `fmw.exe`. The task's definition names `smw.exe` now, so its digest differs and the workspace's next `sync`
  (Approve, Pause, Resume, Remove) registers it again with `smw.exe`.

All of this is removed in the release after the first one that ships `sm.exe` (its `VERSION` line names it).

## Updating from a Foreman install

A Foreman install (`C:\Tools\foreman`, home `%APPDATA%\Foreman`) stays where it is: sessions it started keep running
its `plugins\foreman\bin\fm.exe` until they restart. Quit Foreman (tray, Quit) first: Supraman does not start while
a Foreman runs on its home or on `%APPDATA%\Foreman`, and brings that Foreman's window forward instead. Then extract
the zip to `C:\Tools\supraman` and start `supraman.exe`.

- **Home.** With neither `SUPRAMAN_HOME` nor `FOREMAN_HOME` set, the first start finds no `%APPDATA%\Supraman` and
  copies the whole of `%APPDATA%\Foreman` there (settings, registry, supervisor state, minted records, `oauth` with
  its DPAPI secrets, stars, dismissals, tabs, schedules with their runs and state, logs), writes
  `migrated-from.toml`, and says so once. The old folder is only read, never moved or deleted. Until that copy
  exists, `sm` refuses to run on the default home rather than start an empty one.
- **Tasks.** A workspace's next `sync` (Approve, Pause, Resume, Remove) registers its tasks under `\Supraman\<ws>`
  (another home: `\Supraman-<hex8>`) and deletes that home's old ones under `\Foreman\<ws>`, then that folder and
  `\Foreman` itself when they are left empty.
- **Env.** `SUPRAMAN_HOME` wins over `FOREMAN_HOME`; sessions and schedule scripts get both. The other session
  variables (`FOREMAN_ROLE`, `FOREMAN_WORKSPACE`, `FOREMAN_SESSION`, `FOREMAN_REPO`, `FOREMAN_SCHEDULE`, `FOREMAN_RUN`,
  a script's `FOREMAN_*`) keep their names: running sessions and scripts written against them keep working.
- **Sessions.** A session Foreman started keeps its saved `--settings`, so `FOREMAN_HOME=%APPDATA%\Foreman` stays with it
  through every resume: its hook events, relay messages and `fm tell` go to the old home, which Supraman does not read.
  After switching, stop each such session and start it fresh from Supraman (a new conversation, not a resume); until
  then it shows no status or messages in Supraman.
