# Roadmap

After the public release, the next apps are **Gitman** and **Fileman**, built as plugin apps next to
Supraman (roadmap/3). Then come macOS and Linux. Supraman runs on Windows today. This page lists every piece that
is Windows-only, where it is, and what takes its place on each platform. It is the starting point for the porting
units roadmap/macos-port and roadmap/linux-port.

Each row is marked:

- **verified**: read in the code at master 78691df, or checked with the compiler.
- **expected**: the planned counterpart, not built or tried yet.
- **unknown**: the code does not tell, and it has to be measured on that platform first.

## Where things stand

- Every Windows-only piece sits behind `#[cfg(windows)]`. Most have a `#[cfg(not(windows))]` stub that does
  nothing, returns `None` or refuses, so the code compiles off Windows but does not work there. **verified**
- `supraman-core` and `supraman-claude` pass `cargo check --target x86_64-unknown-linux-musl`. **verified**
- The other crates were not checked from Windows. `cargo check` stops in C build scripts that need a cross
  toolchain: `openssl-sys` (through `ureq`'s `native-tls`, in `supraman-workstate`, `supraman-supervisor` and `sm`)
  and `zstd-sys` (through zed's `util` → `rust-embed`, in `supraman-term` and `supraman-ui`). On a Linux or macOS
  host they build natively. **verified** (the failure); whether the Rust code then compiles: **unknown**
- `tray-icon`, `image` and `windows` are `cfg(windows)` dependencies of `supraman-app`; the `windows` crate is
  also a `cfg(windows)` dependency of `supraman-core`, `supraman-claude`, `supraman-term` and `supraman-ui`.
  **verified**
- gpui runs on macOS (Metal) and Linux (Wayland and X11). Supraman uses it with `default-features = false`, so the
  port has to turn on gpui's macOS or Linux platform features. **expected**

## Shared by both ports

These pieces are Windows-only in the same way for macOS and Linux, and the counterpart is the same Unix call.

| Windows-only piece | Where it lives | Unix counterpart | State |
|---|---|---|---|
| ConPTY for terminal tabs | `supraman-term/src/builder.rs`, `alacritty.rs`, `pty_info.rs`, `events.rs`, `terminal.rs` | `alacritty_terminal`'s Unix PTY (`forkpty`); `builder.rs` already passes the child signal mask, `pty_info.rs` already kills the group with `killpg` | **verified**: the `cfg(unix)` branches exist; never run |
| Job object, so a timeout kills the whole tree (`Kill::Tree`) | `supraman-core/src/process.rs` (`Job`) | The child in its own process group (`process_group(0)`), killed with `killpg`; `Kill::Child` stays a plain kill | **expected**; off Windows `Job` kills the child alone today (**verified**) |
| No console window (`CREATE_NO_WINDOW`) | `supraman-core/src/process.rs` (`no_window`) | Nothing: a Unix child gets no window | **verified**: already a no-op off Windows |
| Process start time, so a pid that was reused reads as gone | `supraman-core/src/process.rs` (`started`, `alive`, `wait_any_exit`) | Linux: field 22 of `/proc/<pid>/stat`. macOS: `proc_pidinfo` (`PROC_PIDTBSDINFO`, `pbi_start_tvsec`). Waiting: `pidfd` on Linux, `kqueue` `EVFILT_PROC` on macOS | **verified**: off Windows `started` checks only that `/proc/<pid>` exists and returns 0, so on macOS, which has no `/proc`, every pid reads as dead |
| Process snapshot (pid → parent) | `supraman-claude/src/proc_tree.rs` | Linux: `/proc/*/stat`. macOS: `proc_listallpids` and `proc_pidinfo` | **verified**: returns an empty list off Windows |
| ShellExecute (open a folder, a file, a URL) | `supraman-core/src/process.rs` (`shell_execute`), `supraman-app/src/signin.rs` (`open_system_browser`) | `open` on macOS, `xdg-open` on Linux, through `process::spawn_detached` | **verified**: off Windows `shell_execute` fails and the sign-in refuses to open the browser, so sign-in does not work there today |
| Owner-only ACL on a session's config file (`icacls`) | `supraman-claude/src/launch.rs` (`restrict_to_owner`) | Mode `0600` | **verified**: the `cfg(unix)` branch exists, and `hand.rs` tests it |
| Single instance (named mutex plus an event that asks the first instance to show itself) | `supraman-app/src/instance.rs` | `fs::try_lock_file` on a file in the home, plus a Unix socket in the home that a second start writes "open" to | **expected**; off Windows every start is the first (**verified**) |
| Elevation: the app's integrity level, a session's, and the relaunch that crosses it | `supraman-claude/src/daemon.rs` (`is_elevated`, `pid_elevated`), `supraman-app/src/elevate.rs` | None. Unix has no integrity levels; a process of the same uid reaches the daemon | **verified**: off Windows `is_elevated` is `true`, `pid_elevated` is `None` and the relaunch refuses |
| Junctions and reparse points in a transcript's folder | `supraman-claude/src/forget.rs` | `symlink_metadata` and `is_symlink`; Unix has no junctions | **expected**; the Windows tests use `mklink /J` (**verified**) |
| Path rules: `\` folded to `/`, the `\\?\` prefix, case folded in `path_key` | `supraman-core/src/paths.rs` (`normalize`, `path_key`, `within`) | No case folding on Linux (`path_key` already does that off Windows). macOS volumes are usually case-insensitive but case-preserving, so `path_key` there should fold case too | Linux **verified**; macOS **expected** (today `path_key` treats macOS as case-sensitive) |
| A command's file (`.ps1`, then `PATHEXT`) for a [+] tab | `supraman-core/src/paths.rs` (`pwsh_command`), `supraman-app/src/hand.rs` | Looked up on `PATH` with the executable bit; no extensions | **expected** |
| The [+] tab runs pwsh with a launch script | `supraman-app/src/hand.rs` | The user's login shell (`$SHELL`, `-l -c`) and a POSIX `sh` script with the same steps | **expected** |
| The scheduler's raw argument line | `sm/src/fire.rs` (`raw`) | `Command::arg`, which the stub already does | **verified** |
| Claude's peer protocol (message pipes) | `supraman-claude/src/peer.rs` | A Unix domain socket under `$XDG_RUNTIME_DIR/cc-socks/` or `/tmp/cc-socks-<uid>/`, the same JSON lines, the auth line optional (`docs/peer-messaging.md`, read from claude's code). `peer.rs` already has a `cfg(unix)` `write_pipe` on a `UnixStream` | **verified** in code on both sides; never run against a real session; the `procStart` format claude writes there: **unknown** |
| The process memory figure in the status bar | `supraman-ui/src/lib.rs` (`memory`) | Linux: `/proc/self/statm`. macOS: `task_info` (`MACH_TASK_BASIC_INFO`) | **verified**: 0 off Windows |
| Icon font (`Segoe Fluent Icons` or `Segoe MDL2 Assets`) | `supraman-ui/src/fonts.rs` (`icons`) | A bundled icon font, the same on every platform | **expected**; off Windows it names Segoe Fluent Icons, which is not installed there (**verified**) |
| Clipboard lock and save/restore around selftests | `supraman-app/src/selftest_clipboard.rs` | Not needed on a private display (Xvfb, or a CI machine's own session); a lock file stays for machines that share one | **expected** |
| Windows resources: icon, version info, manifest | `supraman-app/assets/foreman.rc`, `foreman.manifest`, `build.rs` | macOS: `Info.plist` and `.icns` in the app bundle. Linux: a `.desktop` file and PNG icons | **expected** |
| Release script | `scripts/release.ps1` | `cargo xtask release`, the same steps for every target, run in CI on each platform | **expected** |

## macOS

| Windows-only piece | Where it lives | macOS counterpart | State |
|---|---|---|---|
| Task Scheduler over COM, a task folder per workspace (`WindowsRegistrar`, `sync`) | `supraman-core/src/schedule/task_service.rs`, `task_xml.rs`, `registrar.rs`, `supraman-claude` `task_run_level` | A launchd agent per schedule in `~/Library/LaunchAgents`, its label under one prefix per home (`ai.supraman.<hex8>.<ws>.<id>`), loaded with `launchctl bootstrap gui/<uid>`; `Registrar` stays the seam, `FakeRegistrar` the tests' | **expected**. `StartCalendarInterval` has no time zone and no "every N minutes from a start", so `next_fire` decides and each run arms the next |
| Other tools' tasks that run claude (`foreign`) | `supraman-core/src/schedule/foreign.rs` | The user's launchd agents and `crontab -l`, read only | **expected** |
| DPAPI secret store (a file per key under `<home>/oauth/secrets`) | `supraman-core/src/secret.rs` (`FileStore`, `default_store`), `supraman-workstate/src/secret.rs` | Keychain (`Security.framework`, a generic password per key, service `ai.supraman.<hex8>`) behind `SecretStore` | **expected**; off Windows `default_store` is a `MemoryStore`, so sign-in does not survive a restart (**verified**) |
| Tray icon and its menu, light and dark taskbar icons | `supraman-app/src/tray.rs`, `main.rs` (`tray_state`) | A menu bar item (`NSStatusItem`, through `tray-icon`'s macOS backend) with a template image | **expected**; no tray off Windows today (**verified**) |
| Console release when started from Explorer, the start-up message box | `supraman-app/src/main.rs` (`release_own_console`, `alert`) | Nothing for the console; the alert through `NSAlert` (or gpui's prompt once a window exists) | **expected** |
| Hidden desktop for selftests (`HiddenDesktop`, `spawn_on`) | `supraman-core/src/desktop.rs`, `supraman-app/src/selftest_desktop.rs` | No counterpart. macOS has no private desktop in a session, and a second login session cannot be started headless. Selftests run in a CI machine's own logged-in session, or on a dedicated test account, never on the user's | **verified** that nothing like it exists; the CI setup is **expected** |
| Selftest input: `PostMessageW` to the app's own window | `supraman-app/src/selftest.rs`, `selftest_left.rs`, `selftest_manage.rs` | Events handed to the app's own `NSApplication` (`sendEvent`) in process; never `CGEventPost`, which moves the user's pointer | **expected** |
| Selftest window capture: `PrintWindow` | `supraman-app/src/selftest_capture.rs` | ScreenCaptureKit (`SCScreenshotManager`) for the app's own window; it needs the Screen Recording permission once per machine | **expected** |
| Selftest timing: thread priority, `FILETIME` frame clock | `supraman-app/src/selftest/frames.rs` | `pthread_set_qos_class_self_np` and `clock_gettime` | **expected** |
| Packaging: a zip of the release folder | `docs/RELEASE.md`, `scripts/release.ps1` | A signed and notarized `Supraman.app` in a `.dmg`, universal (arm64 and x86_64); `sm` inside the bundle | **expected** |
| Updater (public/8, public/9): replaces the release folder | public/8, public/9 | Replaces the whole `.app` bundle, then relaunches; the new bundle has to keep its signature, or Gatekeeper refuses it. The plugin's `bin/sm` is copied out of the bundle so running hooks keep a stable path | **expected** |

## Linux

| Windows-only piece | Where it lives | Linux counterpart | State |
|---|---|---|---|
| Task Scheduler over COM (`WindowsRegistrar`, `sync`) | `supraman-core/src/schedule/task_service.rs`, `task_xml.rs`, `registrar.rs` | systemd user units: a `.service` and a `.timer` per schedule in `~/.config/systemd/user`, under one prefix per home, `OnCalendar=` with its time zone, enabled with `systemctl --user`; `loginctl enable-linger` only when the user asks for runs while logged out | **expected**. A machine without systemd gets no schedules, with a notice |
| Other tools' tasks that run claude (`foreign`) | `supraman-core/src/schedule/foreign.rs` | The user's systemd timers and `crontab -l`, read only | **expected** |
| DPAPI secret store | `supraman-core/src/secret.rs`, `supraman-workstate/src/secret.rs` | The Secret Service (libsecret, over D-Bus) behind `SecretStore`; with no Secret Service running, a notice that sign-in lasts until the app quits | **expected**; `MemoryStore` today (**verified**) |
| Tray icon and its menu | `supraman-app/src/tray.rs`, `main.rs` | A StatusNotifierItem (`tray-icon`'s Linux backend, or `ksni`); GNOME shows it only with the AppIndicator extension, so the app keeps working without one | **expected** |
| Console release, the start-up message box | `supraman-app/src/main.rs` | Nothing for the console; the alert on stderr and through `notify-send` | **expected** |
| Hidden desktop for selftests | `supraman-core/src/desktop.rs`, `supraman-app/src/selftest_desktop.rs` | A private display: Xvfb for X11, or a headless Wayland compositor (`weston --backend=headless`, `sway` headless) on its own socket; the app starts with `DISPLAY` or `WAYLAND_DISPLAY` pointing at it | **expected** |
| Selftest input: `PostMessageW` | `supraman-app/src/selftest.rs`, `selftest_left.rs`, `selftest_manage.rs` | XTest on the private Xvfb display (it reaches no other display), or the headless compositor's input | **expected** |
| Selftest window capture: `PrintWindow` | `supraman-app/src/selftest_capture.rs` | `XGetImage` on the Xvfb display, or the headless compositor's screenshot | **expected** |
| Selftest timing | `supraman-app/src/selftest/frames.rs` | `setpriority` or `sched_setattr`, and `clock_gettime` | **expected** |
| Packaging: a zip of the release folder | `docs/RELEASE.md`, `scripts/release.ps1` | A tarball of the release folder (x86_64 and aarch64), and an AppImage; distribution packages later | **expected** |
| Updater (public/8, public/9) | public/8, public/9 | The tarball install updates in place like the Windows folder (a Unix file can be replaced while it runs, so the swap is a rename). An AppImage replaces its one file. A distribution package is never self-updated; the app only says a release is out | **expected** |

## Porting order

1. **Build on the platform.** gpui's platform features, the C dependencies (`openssl-sys` on Linux; macOS uses
   `Security.framework` through `native-tls`), and a CI job that runs `cargo check` and the unit tests. Nothing
   else can be tried before this.
2. **Processes and paths** (`process.rs`, `proc_tree.rs`, `paths.rs`). `started` and `alive` decide which
   sessions run, so the supervisor, `recover_if_dead` and the peer registry all depend on them; on macOS they are
   wrong today.
3. **The terminal** (`supraman-term`). The Unix PTY branches exist; this makes the app usable for a session by
   hand, with the [+] tab on the user's shell.
4. **Secrets and sign-in** (`secret.rs`, `shell_execute`, `signin.rs`). Without them a launched session gets no
   token that survives a restart.
5. **Single instance, the tray and the start-up alert** (`instance.rs`, `tray.rs`, `main.rs`). The app is then
   usable day to day.
6. **Peer messaging** (`peer.rs`). Measure claude's socket registry and `procStart` on the platform first.
7. **Schedules** (`schedule`: launchd or systemd behind `Registrar`, and `foreign`). They need 2 and 4.
8. **Selftests** (private display, input, capture). Until then the port is checked by unit tests and by hand.
9. **Packaging and the updater** (`cargo xtask release`, `.dmg`, tarball and AppImage).

Linux goes first: its selftests can run headless on a private display, and gpui and `alacritty_terminal` are used
there by Zed every day. macOS follows with the same steps, plus signing and notarization.

## Size

104 `#[cfg(windows)]` sites in 38 files, 45 `#[cfg(not(windows))]` stubs, and 9 `target_os = "windows"` conditions
in `supraman-term` (`alacritty.rs`, `alacritty/hyperlinks.rs`, `pty_info.rs`), counted at master 78691df with
`grep -o` over `crates/**/*.rs`. The files with the most `cfg(windows)`:

| File | `cfg(windows)` |
|---|---|
| `supraman-app/src/tray.rs` | 16 |
| `supraman-core/src/secret.rs` | 8 |
| `supraman-core/src/process.rs` | 7 |
| `supraman-claude/src/forget.rs` | 7 |
| `supraman-term/src/builder.rs` | 5 |
| `supraman-core/src/desktop.rs` | 5 |
| `supraman-claude/src/peer.rs` | 5 |
| `supraman-claude/src/daemon.rs` | 4 |
| `supraman-app/src/selftest_clipboard.rs` | 4 |
| `supraman-term/src/alacritty.rs` | 3 |
| `supraman-claude/src/launch.rs` | 3 |
| `supraman-app/src/main.rs` | 3 |
| 26 more files | 1 or 2 each |

`supraman-core/src/schedule/task_service.rs` has no `cfg` of its own: the whole module is `cfg(windows)` in
`schedule/mod.rs`, so one site there stands for the Task Scheduler client.
