//! `sm update apply` on temp installs: never René's install or home (C109).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::Duration;

use supraman_core::install::{self, Apply, FILES, Failed, LEFTOVERS};
use supraman_core::testing::{polled, wait_until};

const HOOK_EXE: &str = "plugins/supraman/bin/sm.exe";

fn write(root: &Path, files: &BTreeMap<&str, Vec<u8>>) {
    for (rel, bytes) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}

fn old_files() -> BTreeMap<&'static str, Vec<u8>> {
    BTreeMap::from([
        ("supraman.exe", b"old app".to_vec()),
        ("sm.exe", b"old sm".to_vec()),
        ("plugins/supraman/hooks/hooks.json", b"old hooks".to_vec()),
        (HOOK_EXE, b"old hook sm".to_vec()),
        ("gone.txt", b"only the old release ships me".to_vec()),
        ("VERSION", b"version 0.1.0\n".to_vec()),
    ])
}

fn new_files() -> BTreeMap<&'static str, Vec<u8>> {
    BTreeMap::from([
        ("supraman.exe", b"new app".to_vec()),
        ("sm.exe", b"new sm".to_vec()),
        ("plugins/supraman/hooks/hooks.json", b"new hooks".to_vec()),
        (HOOK_EXE, b"new hook sm".to_vec()),
        ("plugins/supraman/skills/new/SKILL.md", b"a new skill".to_vec()),
        ("VERSION", b"version 0.2.0\n".to_vec()),
    ])
}

struct Case {
    _tmp: tempfile::TempDir,
    install: PathBuf,
    release: PathBuf,
    log: PathBuf,
}

/// An install of the old release (with its `FILES` and a file of the user's it never named) and the new
/// release unpacked beside it.
fn case() -> Case {
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let install = tmp.path().join("install");
    let release = tmp.path().join("updates").join("0.2.0").join("Supraman");
    write(&install, &old_files());
    let listed: String = old_files().keys().map(|k| format!("{k}\n")).collect();
    fs::write(install.join(FILES), listed).unwrap();
    fs::write(install.join("mine.txt"), b"the user's").unwrap();
    write(&release, &new_files());
    let log = tmp.path().join("updates").join("0.2.0").join("apply.log");
    Case { _tmp: tmp, install, release, log }
}

fn job(c: &Case) -> Apply {
    Apply {
        from: c.release.clone(),
        to: c.install.clone(),
        wait_pid: None,
        wait: Duration::from_secs(5),
        restart: None,
        restart_args: Vec::new(),
        log: c.log.clone(),
        stop_after: None,
    }
}

fn read(root: &Path, rel: &str) -> Option<Vec<u8>> {
    fs::read(root.join(rel)).ok()
}

fn leftovers(root: &Path) -> Vec<String> {
    install::file_list(root).unwrap().into_iter().filter(|f| f != LEFTOVERS && (f.contains(".old-") || f.ends_with(".new"))).collect()
}

fn assert_new(c: &Case) {
    for (rel, bytes) in new_files() {
        assert_eq!(read(&c.install, rel), Some(bytes), "{rel}");
    }
    assert!(!c.install.join("gone.txt").exists(), "a file the new release lacks is removed");
    assert_eq!(read(&c.install, "mine.txt").as_deref(), Some(&b"the user's"[..]), "a file FILES never named is kept");
    let listed = fs::read_to_string(c.install.join(FILES)).unwrap();
    let want: String = new_files().keys().map(|k| format!("{k}\n")).collect();
    assert_eq!(listed, want);
}

/// `sm.exe inbox` run from the install's hooks copy: a running exe Windows will not overwrite or delete.
fn hold(install: &Path) -> Holder {
    let exe = install.join(HOOK_EXE);
    fs::copy(env!("CARGO_BIN_EXE_sm"), &exe).unwrap();
    let inbox = install.parent().unwrap().join("inbox");
    fs::create_dir_all(&inbox).unwrap();
    let mut cmd = supraman_core::process::command(&exe);
    cmd.args(["inbox", "--dir"]).arg(&inbox).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let child = Holder(cmd.spawn().unwrap());
    wait_until(Duration::from_secs(10), polled(Duration::from_millis(20)), || supraman_core::process::alive(child.id()));
    assert!(fs::write(&exe, b"x").is_err(), "the running hooks copy cannot be overwritten");
    child
}

/// A folder `sm plugin install` copies into: what the restarted exe leaves behind.
fn marker(c: &Case) -> PathBuf {
    let dir = c.install.parent().unwrap().join("marker");
    fs::create_dir_all(dir.join("hooks")).unwrap();
    fs::write(dir.join("hooks").join("hooks.json"), "{}").unwrap();
    dir
}

/// A test process, killed when dropped: also when an assertion fails first.
struct Holder(Child);

impl Holder {
    fn id(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_full_apply_replaces_adds_and_removes_only_listed_files_and_writes_version_last() {
    let c = case();
    let out = install::apply(&job(&c));
    assert_eq!(out.result, Ok(()));
    assert_new(&c);
    assert_eq!(leftovers(&c.install), Vec::<String>::new());
    assert!(!c.install.join(LEFTOVERS).exists());
    assert!(!c.log.parent().unwrap().join("backup").exists(), "the backups go once applied");
    let log = fs::read_to_string(&c.log).unwrap();
    assert!(log.trim_end().ends_with(" done"), "{log}");
    let order: Vec<&str> = log.lines().filter_map(|l| l.split_once(" replaced ").map(|(_, r)| r)).collect();
    assert_eq!(order.last(), Some(&"VERSION"), "{log}");
}

#[test]
fn a_running_hooks_copy_is_renamed_aside_and_the_next_start_removes_it() {
    let c = case();
    let holder = hold(&c.install);
    let out = install::apply(&job(&c));
    assert_eq!(out.result, Ok(()), "{}", fs::read_to_string(&c.log).unwrap_or_default());
    assert_new(&c);
    let aside = leftovers(&c.install);
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert!(aside[0].starts_with(&format!("{HOOK_EXE}.old-")), "{aside:?}");
    assert!(fs::read_to_string(c.install.join(LEFTOVERS)).unwrap().contains(&aside[0]));

    install::cleanup(&c.install);
    assert_eq!(leftovers(&c.install), aside, "still locked: kept");

    drop(holder);
    wait_until(Duration::from_secs(10), polled(Duration::from_millis(50)), || {
        if leftovers(&c.install).is_empty() {
            return true;
        }
        install::cleanup(&c.install);
        false
    });
    assert_eq!(leftovers(&c.install), Vec::<String>::new());
    assert!(!c.install.join(LEFTOVERS).exists());
    assert_new(&c);
}

/// An apply stopped as a kill would after each number of moves: every file is wholly old or wholly new,
/// `VERSION` is the old one, and the next start plus a second apply finish the update.
#[test]
fn a_killed_apply_leaves_a_startable_install_at_every_step() {
    let (old, new) = (old_files(), new_files());
    let mut k = 0;
    loop {
        let c = case();
        let out = install::apply(&Apply { stop_after: Some(k), ..job(&c) });
        if out.result == Ok(()) {
            assert!(k > new.len(), "every file and the removal take a move each; stopped only {k} times");
            break;
        }
        assert_eq!(out.result, Err(Failed::Stopped));
        assert_eq!(out.restarted, None);
        for rel in old.keys().chain(new.keys()) {
            let now = read(&c.install, rel);
            let was = old.get(rel).cloned();
            let will = new.get(rel).cloned();
            assert!(now == was || now == will, "after {k} moves {rel} is {now:?}: neither old {was:?} nor new {will:?}");
        }
        for must in ["supraman.exe", "sm.exe", HOOK_EXE] {
            assert!(c.install.join(must).is_file(), "after {k} moves {must} is missing");
        }
        assert_eq!(read(&c.install, "VERSION").as_deref(), Some(&b"version 0.1.0\n"[..]), "after {k} moves");

        install::cleanup(&c.install);
        assert_eq!(install::apply(&job(&c)).result, Ok(()), "a second apply after {k} moves");
        assert_new(&c);
        k += 1;
    }
}

#[test]
fn a_kill_between_the_rename_aside_and_the_move_in_is_put_back_at_the_next_start() {
    let c = case();
    let _aside = install::rename_aside_on_this_thread();
    let holder = hold(&c.install);
    let held_moves = new_files().keys().filter(|k| **k != "VERSION").position(|k| *k == HOOK_EXE).unwrap();
    let out = install::apply(&Apply { stop_after: Some(held_moves + 1), ..job(&c) });
    assert_eq!(out.result, Err(Failed::Stopped));
    assert!(!c.install.join(HOOK_EXE).exists(), "stopped with the hooks copy renamed aside and the new one not in");
    drop(holder);
    wait_until(Duration::from_secs(10), polled(Duration::from_millis(50)), || {
        if c.install.join(HOOK_EXE).exists() {
            return true;
        }
        install::cleanup(&c.install);
        false
    });
    assert_eq!(fs::read(c.install.join(HOOK_EXE)).unwrap(), fs::read(env!("CARGO_BIN_EXE_sm")).unwrap(), "the old hooks copy is back");
}

#[cfg(windows)]
#[test]
fn a_file_that_cannot_even_be_renamed_rolls_the_whole_apply_back() {
    use std::os::windows::fs::OpenOptionsExt as _;
    let c = case();
    const FILE_SHARE_READ: u32 = 1;
    let pinned = fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(c.install.join("supraman.exe")).unwrap();
    let out = install::apply(&job(&c));
    drop(pinned);
    let Err(Failed::Refused(why)) = &out.result else { panic!("{:?}", out.result) };
    assert!(why.contains("cannot be renamed aside"), "{why}");
    for (rel, bytes) in old_files() {
        assert_eq!(read(&c.install, rel), Some(bytes), "{rel} is the old one again");
    }
    assert!(!c.install.join("plugins/supraman/skills/new/SKILL.md").exists(), "an added file is taken away again");
    assert_eq!(leftovers(&c.install), Vec::<String>::new());
    let log = fs::read_to_string(&c.log).unwrap();
    assert!(log.contains("failed: ") && log.contains("undid "), "{log}");
}

#[test]
fn a_supraman_that_does_not_exit_changes_nothing_and_starts_nothing() {
    let c = case();
    let mut sleeper = supraman_core::process::command(env!("CARGO_BIN_EXE_sm"));
    let inbox = c.install.parent().unwrap().join("inbox-wait");
    fs::create_dir_all(&inbox).unwrap();
    sleeper.args(["inbox", "--dir"]).arg(&inbox).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let child = Holder(sleeper.spawn().unwrap());
    let marker = marker(&c);
    let out = install::apply(&Apply {
        wait_pid: Some(child.id()),
        wait: Duration::from_millis(500),
        restart: Some(PathBuf::from(env!("CARGO_BIN_EXE_sm"))),
        restart_args: vec!["plugin".into(), "install".into(), marker.display().to_string()],
        ..job(&c)
    });
    drop(child);
    let Err(Failed::Refused(why)) = &out.result else { panic!("{:?}", out.result) };
    assert!(why.contains("did not exit"), "{why}");
    assert_eq!(out.restarted, None);
    for (rel, bytes) in old_files() {
        assert_eq!(read(&c.install, rel), Some(bytes), "{rel}");
    }
    std::thread::sleep(Duration::from_millis(500));
    assert!(!marker.join("bin").exists());
}

#[test]
fn the_cli_waits_for_the_pid_applies_and_starts_the_restart_exe() {
    let c = case();
    let mut quick = supraman_core::process::command(env!("CARGO_BIN_EXE_sm"));
    quick.arg("--version").stdout(Stdio::null());
    let mut gone = quick.spawn().unwrap();
    let pid = gone.id();
    gone.wait().unwrap();
    let marker = marker(&c);
    let out = supraman_core::process::command(env!("CARGO_BIN_EXE_sm"))
        .args(["update", "apply", "--from"])
        .arg(&c.release)
        .arg("--to")
        .arg(&c.install)
        .arg("--log")
        .arg(&c.log)
        .args(["--wait-pid", &pid.to_string(), "--wait-secs", "5", "--restart", env!("CARGO_BIN_EXE_sm")])
        .args(["--restart-arg", "plugin", "--restart-arg", "install", "--restart-arg"])
        .arg(&marker)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_new(&c);
    let started = marker.join("bin").join("sm.exe");
    wait_until(Duration::from_secs(10), polled(Duration::from_millis(50)), || started.exists());
    assert!(started.exists(), "the restart exe ran: {}", fs::read_to_string(&c.log).unwrap_or_default());
}

#[test]
fn an_install_without_version_is_refused() {
    let c = case();
    fs::remove_file(c.install.join("VERSION")).unwrap();
    let Err(Failed::Refused(why)) = install::apply(&job(&c)).result else { panic!() };
    assert!(why.contains("has no VERSION"), "{why}");
    assert_eq!(read(&c.install, "supraman.exe").as_deref(), Some(&b"old app"[..]));
}

/// Polls `path` from another thread until dropped; `missed` counts the polls that found nothing there.
struct Watch {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<(u64, u64)>>,
}

impl Watch {
    fn start(path: PathBuf) -> Self {
        use std::sync::atomic::Ordering;
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            let (mut polls, mut missed) = (0, 0);
            while !flag.load(Ordering::Relaxed) {
                polls += 1;
                if fs::symlink_metadata(&path).is_err() {
                    missed += 1;
                }
            }
            (polls, missed)
        });
        Self { stop, thread: Some(thread) }
    }

    /// (polls, missed)
    fn finish(mut self) -> (u64, u64) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap()
    }
}

fn hook_copy_is_old_or_new(c: &Case, when: &str) {
    let now = fs::read(c.install.join(HOOK_EXE)).unwrap_or_else(|e| panic!("{when}: {HOOK_EXE} is missing: {e}"));
    let old = fs::read(env!("CARGO_BIN_EXE_sm")).unwrap();
    assert!(now == old || now == new_files()[HOOK_EXE], "{when}: {HOOK_EXE} is neither the old nor the new one");
}

#[test]
fn a_running_hooks_copy_is_replaced_in_one_rename_and_its_path_never_goes_missing() {
    let c = case();
    let holder = hold(&c.install);
    let watch = Watch::start(c.install.join(HOOK_EXE));
    let out = install::apply(&job(&c));
    let (polls, missed) = watch.finish();
    let log = fs::read_to_string(&c.log).unwrap_or_default();
    assert_eq!(out.result, Ok(()), "{log}");
    assert!(log.contains(&format!("replaced {HOOK_EXE} in one rename")), "{log}");
    assert!(!log.contains("renamed aside"), "{log}");
    assert!(polls > 0);
    assert_eq!(missed, 0, "{HOOK_EXE} was missing in {missed} of {polls} polls");
    assert!(supraman_core::process::alive(holder.id()), "the old copy ran on while it was replaced");
    assert_new(&c);

    let linked = leftovers(&c.install);
    assert_eq!(linked.len(), 1, "{linked:?}");
    assert!(linked[0].starts_with(&format!("{HOOK_EXE}.old-")), "{linked:?}");
    assert_eq!(fs::read(c.install.join(&linked[0])).unwrap(), fs::read(env!("CARGO_BIN_EXE_sm")).unwrap(), "the old copy stays recoverable");
    assert!(fs::read_to_string(c.install.join(LEFTOVERS)).unwrap().contains(&linked[0]));
    install::cleanup(&c.install);
    assert_eq!(leftovers(&c.install), linked, "still running: kept");

    drop(holder);
    wait_until(Duration::from_secs(10), polled(Duration::from_millis(50)), || {
        if leftovers(&c.install).is_empty() {
            return true;
        }
        install::cleanup(&c.install);
        false
    });
    assert_eq!(leftovers(&c.install), Vec::<String>::new());
    assert!(!c.install.join(LEFTOVERS).exists());
    assert_new(&c);
}

/// The apply stopped as a kill would at every step while the hooks copy runs: the copy is always there, old
/// or new, and the next start plus a second apply finish the update.
#[test]
fn a_kill_at_any_step_leaves_a_running_hooks_copy_in_place() {
    let mut k = 0;
    loop {
        let c = case();
        let holder = hold(&c.install);
        let out = install::apply(&Apply { stop_after: Some(k), ..job(&c) });
        hook_copy_is_old_or_new(&c, &format!("after {k} moves"));
        if out.result == Ok(()) {
            assert!(k > new_files().len(), "stopped only {k} times");
            break;
        }
        assert_eq!(out.result, Err(Failed::Stopped));
        install::cleanup(&c.install);
        hook_copy_is_old_or_new(&c, &format!("cleanup after {k} moves"));
        assert_eq!(install::apply(&job(&c)).result, Ok(()), "a second apply after {k} moves");
        assert_new(&c);
        drop(holder);
        wait_until(Duration::from_secs(10), polled(Duration::from_millis(50)), || {
            if leftovers(&c.install).is_empty() {
                return true;
            }
            install::cleanup(&c.install);
            false
        });
        assert_eq!(leftovers(&c.install), Vec::<String>::new(), "after {k} moves");
        k += 1;
    }
}

#[cfg(windows)]
#[test]
fn a_rolled_back_apply_puts_the_running_hooks_copy_back_from_its_link() {
    use std::os::windows::fs::OpenOptionsExt as _;
    let c = case();
    let holder = hold(&c.install);
    const FILE_SHARE_READ: u32 = 1;
    let pinned = fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(c.install.join("supraman.exe")).unwrap();
    let out = install::apply(&job(&c));
    drop(pinned);
    let log = fs::read_to_string(&c.log).unwrap_or_default();
    let Err(Failed::Refused(_)) = &out.result else { panic!("{:?}", out.result) };
    assert!(log.contains(&format!("replaced {HOOK_EXE} in one rename")) && log.contains("undid Swapped"), "{log}");
    assert_eq!(fs::read(c.install.join(HOOK_EXE)).unwrap(), fs::read(env!("CARGO_BIN_EXE_sm")).unwrap(), "the old hooks copy is back");
    assert!(supraman_core::process::alive(holder.id()));
    assert_eq!(leftovers(&c.install), Vec::<String>::new());
    assert!(!c.install.join(LEFTOVERS).exists());
}

#[test]
fn the_rename_aside_path_journals_the_rename_and_lists_the_old_copy() {
    let c = case();
    let _aside = install::rename_aside_on_this_thread();
    let _holder = hold(&c.install);
    let out = install::apply(&job(&c));
    let log = fs::read_to_string(&c.log).unwrap_or_default();
    assert_eq!(out.result, Ok(()), "{log}");
    assert!(log.contains(&format!("renamed aside {HOOK_EXE} -> {HOOK_EXE}.old-")), "{log}");
    assert!(!log.contains("in one rename"), "{log}");
    assert_new(&c);
    let aside = leftovers(&c.install);
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert_eq!(fs::read_to_string(c.install.join(LEFTOVERS)).unwrap(), format!("aside {}
", aside[0]));
}
