# Releasing Foreman

## Build a release

```powershell
scripts\release.ps1                 # -> <target dir>\dist\Foreman + <target dir>\dist\Foreman-<version>.zip
scripts\release.ps1 -OutDir D:\out\Foreman
```

`<target dir>` is `CARGO_TARGET_DIR` if set, else `<repo>\target`. The script runs
`cargo build --release --workspace`, assembles the folder below, runs `fm plugin install` into it, writes `VERSION`
and zips the folder (the zip's root is the folder, `Foreman\`). It replaces an existing `-OutDir` only if that holds a
`VERSION` (an earlier release). `<version>` is `<crate version>-<yyyyMMdd>-<git sha>`, with `-dirty` if the tree had
changes.

No signing and no installer in v1: the zip is extracted anywhere and `foreman.exe` started from there. Unsigned
exes may get a SmartScreen prompt on first start from a downloaded zip (unblock the zip before extracting).

## Layout

```
Foreman\
  foreman.exe
  foreman.pdb                  symbols and line tables: file:line of each frame of a panic's backtrace in foreman.log
  fm.exe                       the CLI controllers and users run
  fmw.exe                      fm with no console window: a schedule's task runs `fmw.exe schedule fire …`
  VERSION                      version, commit, build time (UTC), claude tested / minimum
  plugins\foreman\
    hooks\hooks.json           the exec-form hooks, run from ${CLAUDE_PLUGIN_ROOT}\bin\fm.exe
    roles\*.md                 role templates
    skills\**
    bin\fm.exe                 the hooks' copy (fm plugin install; the app refreshes it at start)
```

The folder is self-contained: it runs from any path, no junction to a repo. Foreman's state stays in
`%APPDATA%\Foreman` (or `FOREMAN_HOME`), not in this folder.

## How the exes find the plugin

`foreman_core::find_plugin_dir`, shared by `foreman.exe` (start, the `--plugin-dir` of every session it launches)
and `fm.exe` (the repo role template when the app has not written one), looks in this order:

1. `<exe dir>\plugins\foreman` — the release layout;
2. the plugin the exe is inside — `fm.exe` run as `<plugin>\bin\fm.exe` by a hook;
3. `plugins\foreman` in every folder above the exe — a dev build at `<repo>\target\<profile>\` (also with
   `CARGO_TARGET_DIR` elsewhere only if a `plugins\foreman` is above it).

A plugin is a folder with `hooks\hooks.json` and `roles\`. If none is found the app shows
`No Foreman plugin for <exe>: looked next to it at <exe dir>\plugins\foreman (a release) and for plugins\foreman in
every folder above it (a dev build): sessions cannot be started from this build.`

## claude CLI version policy

`foreman_claude::{MIN_VERSION, TESTED_VERSION}` in `crates/foreman-claude/src/cli.rs`; `check_version` applies it.

| claude CLI                   | Foreman                                                        |
|------------------------------|----------------------------------------------------------------|
| below `MIN_VERSION` (2.1.308) | refuses, naming the minimum and why: "update claude"          |
| 2.1.308 … `TESTED_VERSION` (2.1.310) | runs                                                    |
| a newer patch of the tested minor (2.1.311) | runs, says nothing                                     |
| a newer minor or major (2.2.0, 3.0.0) | runs; the toolbar ⚠ (its tooltip says why), `fm` stderr, selftest line; no Inbox notice |

Never block a newer claude (decision 2026-10-26): the CLI auto-updates, and blocking would stop every user on the
day of a release. The minimum is the oldest version every CLI fact Foreman relies on was measured on
(docs/LEARNINGS.md, docs/peer-messaging.md):

- `claude --bg` (`--name`, `--permission-mode`, `--mcp-config`, `--plugin-dir`, `--add-dir`, `--resume`) — m0/2, m2;
- the fields of `claude agents --json --all` — m0/2, m1;
- `--plugin-dir` loading the hooks and skills — m0/3;
- the peer registry's `peerProtocol 1` for direct messages — m5/4 (an unknown protocol falls back to the relay, so
  a change there degrades, it does not break).

When a release passes the gate on a newer claude, raise `TESTED_VERSION` in the same commit. Raise `MIN_VERSION`
only when Foreman starts depending on something an older CLI lacks, and say what in its doc comment.

## Release gate

From the release folder **outside the repo** (e.g. copied to `C:\…\Foreman`): `foreman.exe --selftest` and
`foreman.exe --selftest-supervise` both PASS, and `cargo test --workspace` passes in the repo.
