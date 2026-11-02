# Releasing Supraman

## Build a release

```powershell
scripts\release.ps1                 # -> <target dir>\dist\Supraman + <target dir>\dist\Supraman-<version>.zip
scripts\release.ps1 -OutDir D:\out\Supraman
scripts\release.ps1 -Version 0.1.0  # what the release workflow runs: Supraman-0.1.0.zip
```

`<target dir>` is `CARGO_TARGET_DIR` if set, else `<repo>\target`. The script runs
`cargo build --release --workspace`, assembles the folder below, runs `sm plugin install` into it, writes `VERSION`
and `SOURCE.txt`, zips the folder (the zip's root is the folder, `Supraman\`) and runs `sm release manifest`, which
writes `supraman-update.json` and `SHA256SUMS` beside the zip. It replaces an existing `-OutDir` only if that holds a
`VERSION` (an earlier release). Without `-Version`, `<version>` is `<crate version>-<yyyyMMdd>-<git sha>`, with
`-dirty` if the tree had changes. With `-Version <semver>` it is that semver, which must equal the crates' version
(`crates\supraman-app\Cargo.toml`), and a dirty tree is refused. A local build is never signed.

No installer: the zip is extracted anywhere (`C:\Tools\supraman` here) and `supraman.exe` started from there. The exes
are not code-signed and may get a SmartScreen prompt on first start from a downloaded zip (unblock the zip before
extracting).

## Publish a release

1. Set the crates' `version` to the release's semver (every `crates\*\Cargo.toml`), commit, pass the gate below,
   and push master.
2. Start the release on master: `gh workflow run release.yml -f version=0.1.3` (or Actions → release → Run workflow).
3. `.github/workflows/release.yml` refuses a ref other than master, a version other than the crates', and a tag
   `v<version>` that names another commit. It builds (`windows-latest`, compiled dependencies cached by
   `Swatinem/rust-cache` under master, the one ref every run can restore from; no tests: the commit passed the gate,
   and `.github/workflows/test.yml` runs `cargo test` on GitHub when started by hand) with `release.ps1 -Version
   <version>`, signs with `sm release sign` and the repository secret `UPDATE_SIGNING_KEY`, runs `sm release verify`,
   pushes the tag `v<version>` on that commit, then `gh release create v<version> --draft` with the zip,
   `SHA256SUMS`, `supraman-update.json` and `supraman-update.json.sig`. Its notes name the tag and link the source at
   it (GPL-3.0 §6). It stops before the build while `update.pub` is the placeholder. A failed run is started again
   the same way; a tag it already pushed on the same commit is kept.
4. René installs from the draft, checks it, and publishes it on GitHub. The workflow never publishes.

## Signing

`supraman-update.json` is `{ version, commit, zip, sha256, size, published, notes_url }`; the bytes of the file as
written are what is signed (ed25519), and `supraman-update.json.sig` is the base64 of the 64-byte signature
(`supraman_core::update`). The app verifies with the public keys compiled in from
`crates\supraman-core\src\update.pub` (one base64 key per line, `#` lines ignored), then checks the zip's size and
sha256 against the manifest.

- **The key pair:** René runs `scripts\new-signing-key.ps1` once. It pipes `sm release keygen --public-out
  crates\supraman-core\src\update.pub` into `gh secret set UPDATE_SIGNING_KEY`: the secret key exists only in that
  pipe and in the secret (`keygen` refuses a console stdout), and `update.pub` gets the public key, to be committed.
- `sm release sign <manifest>` reads `UPDATE_SIGNING_KEY` and refuses when it is unset or its public half is not in
  `update.pub`; it never prints the key. `sm release verify <manifest> [--public-key <file>]` checks the signature and
  the zip beside the manifest.
- **Rotation:** a new key needs a release whose app already carries both keys. Run the script (the secret becomes the
  new key), add the old key's line back to `update.pub` under the new one, and release; drop the old line in a later
  release, once installs have updated past the one that carries both.

## Layout

```
Supraman\
  supraman.exe
  supraman.pdb                 symbols and line tables: file:line of each frame of a panic's backtrace in foreman.log
  sm.exe                       the CLI controllers and users run
  smw.exe                      sm with no console window: a schedule's task runs `smw.exe schedule fire …`
  fm.exe, fmw.exe              copies of sm.exe and smw.exe (see "The fm name" below)
  VERSION                      version, commit, build time (UTC), claude tested / minimum
  SOURCE.txt                   where this build's source is: the tag's (or commit's) tree on GitHub
  THIRD_PARTY.md               the bundled assets and every shipped crate with its licence and source
  licenses\                    supraman.LICENSE (GPL-3.0), rust-crates.txt (every crate's licence and notices), the assets' licences
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
`cargo deny check licenses` passes (`deny.toml`: only licences that allow shipping the whole as GPL-3.0-or-later)
and `pwsh -File scripts/third-party.ps1 -Check` says `THIRD_PARTY.md is current`; `release.ps1` runs that check
itself and stops on a stale file. After a dependency change, `pwsh -File scripts/third-party.ps1` regenerates it.
The selftests run on a hidden desktop of their own (their first line names it, their last says no selftest window
had the focus), so typing at the machine meanwhile is fine. `--visible` (or `SUPRAMAN_SELFTEST_VISIBLE=1`) runs one
on your desktop to watch it; it can then take your focus and read your held keys.

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

## How Supraman updates itself

The banner's **Update** (`crate::updates::install_update`) does the whole update; a build with no `VERSION` beside
`supraman.exe` (a dev build) or a release without its zip opens the release page instead.

1. **Ask.** With sessions attached in tabs it asks once: Supraman restarts; the sessions keep running in the background.
2. **Download** (`supraman_workstate::updates::download`, off the UI thread): the release's zip into
   `<home>\updates\<version>\` as `<zip>.part`, the banner showing `Downloading 12 of 48 MB` and Cancel. More bytes
   than the signed manifest's `size` stop it. The zip is trusted only through the verified manifest:
   `Manifest::check_zip` (size and sha256); a mismatch deletes it and the banner says why, and nothing else changes.
   A zip already there that matches is not downloaded again.
3. **Unpack** (`supraman_core::install::unpack`) into `<home>\updates\<version>\Supraman\`; an entry outside
   `Supraman\`, absolute, with a drive, `..` or a symlink refuses the whole zip before anything is written. Its
   `VERSION` must be the manifest's version.
4. **Hand over.** The new `sm.exe` is copied to `<home>\updates\<version>\sm-update.exe` (never run from the install
   folder it replaces) and started detached:
   `sm-update.exe update apply --from <unpacked> --to <install> --log <home>\updates\<version>\apply.log --restart <install>\supraman.exe --wait-pid <pid>`.
   Then Supraman quits normally (attached tabs detach; their sessions keep running).
5. **Apply** (`supraman_core::install::apply`): waits up to 60 s for the pid (still running: nothing changes, nothing
   restarts), takes `<install>\.update.lock`, reads the old release's `FILES`, then for each file of the new release
   stages `<name>.new` and moves it over the target with `MoveFileExW`. A target that refuses (a running exe: sessions'
   hooks run `plugins\supraman\bin\sm.exe` and `fm.exe` even while Supraman is closed) is renamed to
   `<name>.old-<stamp>`, listed in `<install>\.old-files` first, and the new one moved in. If even that rename fails,
   every move so far is undone from the journal and its backups (`<home>\updates\<version>\backup\`), the old
   Supraman restarts, and the next start's banner says why (the last `failed:` line of `apply.log`). Files the old
   `FILES` named that the new release lacks are removed (renamed aside when locked); nothing it never named is touched,
   and there is never a recursive delete. `FILES` and then `VERSION` go last: a killed apply leaves every file whole,
   old or new, and the old version reported, and running the update again finishes it.
6. **Restart** the new `supraman.exe`.
7. **Cleanup** (`supraman_core::install::cleanup`) at every start of Supraman and of `sm`/`smw` (hooks included):
   the `.old-<stamp>` files `.old-files` lists that are no longer locked are removed; one whose file is missing (a
   kill between the rename aside and the move in) is put back. Supraman also removes `<home>\updates\<version>`
   folders older than the running version.

`release.ps1` writes `FILES` (every file of the release, `/`-separated); an install without one gets no removals on its
first update, and the apply writes the new release's list. `std::fs::rename` is not used for these moves: on NTFS it
uses POSIX semantics and replaces even a running exe, which a FAT or network install refuses, so the apply uses one
behaviour everywhere. Tests and the selftest (step 7+u) apply onto temp installs only, never `C:\Tools\supraman` or
`%APPDATA%\Supraman` (C109).
