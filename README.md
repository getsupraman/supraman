<picture>
  <source media="(prefers-color-scheme: dark)" srcset="crates/supraman-app/assets/brand/lockups/supraman-lockup-on-dark.svg">
  <img alt="Supraman" src="crates/supraman-app/assets/brand/lockups/supraman-lockup-on-light.svg" height="48">
</picture>

**Supraman** is a desktop app that supervises workspaces of Claude Code sessions: it launches, resumes and
watches them, and shows you which ones need you.

It is one of a family of tools:

- **Supraman**: the supervisor (this repository).
- **Workman**: the work tracker your sessions plan and report in; Supraman shows its board and inbox.
- **Gitman** and **Fileman**: planned.

## What it does

- Keeps the sessions a workspace wants running: launches them, resumes a stopped one, and reports one that fails.
- Lists every Claude Code session on the machine by workspace and folder, with its state, and opens any of them in
  a terminal tab.
- Puts what needs you in one Inbox: a session waiting on a permission, a question from a session, a blocked task in
  Workman.
- Signs in to Workman once and gives each session it launches its own token.

Supraman runs on Windows. It is a native Rust app built on [gpui](https://github.com/zed-industries/zed).

## Build

```powershell
cargo build --release --workspace
target\release\supraman.exe
```

The crates are `supraman-*` and the CLI is `sm.exe` (`smw.exe` without a console). The app keeps its state in
`%APPDATA%\Supraman`, or in `SUPRAMAN_HOME` when set (`FOREMAN_HOME` is still read); a first start copies an
earlier `%APPDATA%\Foreman` there. `docs/RELEASE.md` describes a release folder, and
`docs/ARCHITECTURE.md` the crates and how they fit together.

## Brand

The marks under `crates/supraman-app/assets/brand` are copied from the Supraman brand kit; see its `NOTICE.md`.
