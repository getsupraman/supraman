//! A release zip unpacked and swapped over an install (docs/RELEASE.md, "How Supraman updates itself").

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The release's file list beside `VERSION`: one `/`-separated relative path per line.
pub const FILES: &str = "FILES";
/// The `.old-<stamp>` files an apply left in the install, for [`cleanup`].
pub const LEFTOVERS: &str = ".old-files";
pub const ROOT: &str = "Supraman";
pub const VERSION: &str = "VERSION";
const LOCK: &str = ".update.lock";
const LOG_CAP: u64 = 1 << 20;

/// `true` for a relative path of plain names: no root, drive, `..`, `.` or empty part.
fn plain(rel: &str) -> bool {
    !rel.is_empty()
        && rel.split(['/', '\\']).all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains(':'))
}

/// The zip's `Supraman\` folder unpacked into `dir`; an entry escaping it refuses the whole zip first.
pub fn unpack(zip: &Path, dir: &Path) -> Result<PathBuf, String> {
    let file = fs::File::open(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("{}: not a zip: {e}", zip.display()))?;
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(|e| format!("{}: entry {i}: {e}", zip.display()))?;
        let raw = entry.name().map_err(|e| format!("{}: entry {i}: {e}", zip.display()))?.to_string();
        let name = raw.replace('\\', "/");
        let Some(rest) = name.strip_prefix(&format!("{ROOT}/")) else {
            return Err(format!("{}: entry '{raw}' is outside {ROOT}/", zip.display()));
        };
        let rest = rest.trim_end_matches('/');
        if entry.is_symlink() || (!rest.is_empty() && !plain(rest)) || entry.enclosed_name().is_none() {
            return Err(format!("{}: entry '{raw}' escapes {ROOT}/", zip.display()));
        }
        if !entry.is_dir() && !rest.is_empty() {
            entries.push((i, rest.to_string()));
        }
    }
    let root = dir.join(ROOT);
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    }
    for (i, rel) in entries {
        let mut entry = archive.by_index(i).map_err(|e| format!("{}: {rel}: {e}", zip.display()))?;
        let out = root.join(&rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut target = fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        io::copy(&mut entry, &mut target).map_err(|e| format!("{}: {rel}: {e}", zip.display()))?;
    }
    Ok(root)
}

/// The folder of the running exe; `supraman.exe`, `sm.exe` and `smw.exe` ship side by side in it.
pub fn exe_dir() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{} has no folder", exe.display())))
}

/// `sm.exe` beside the running exe.
pub fn sm_exe() -> io::Result<PathBuf> {
    Ok(exe_dir()?.join(crate::hook_exe()))
}

/// [`exe_dir`] when `sm.exe` is in it: the folder a [+] tab puts first on its `PATH`.
pub fn sm_dir() -> Option<PathBuf> {
    exe_dir().ok().filter(|d| d.join(crate::hook_exe()).is_file())
}

/// A child's `PATH`: [`exe_dir`] first, then this process's, so the `sm` it runs is this build's.
pub fn sm_first_path() -> Option<String> {
    let dir = exe_dir().ok()?;
    Some(format!("{};{}", dir.display(), crate::paths::process_path().to_string_lossy()))
}

/// The install folder of an exe in `exe_dir`: the folder itself, or the install above `plugins\supraman\bin`.
pub fn install_dir(exe_dir: &Path) -> PathBuf {
    let names: Vec<_> = exe_dir.components().rev().take(3).map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
    if names == ["bin", "supraman", "plugins"] {
        exe_dir.ancestors().nth(3).unwrap_or(exe_dir).to_path_buf()
    } else {
        exe_dir.to_path_buf()
    }
}

pub fn file_list(root: &Path) -> io::Result<BTreeSet<String>> {
    fn walk(dir: &Path, prefix: &str, out: &mut BTreeSet<String>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            if entry.file_type()?.is_dir() {
                walk(&entry.path(), &rel, out)?;
            } else {
                out.insert(rel);
            }
        }
        Ok(())
    }
    let mut out = BTreeSet::new();
    walk(root, "", &mut out)?;
    Ok(out)
}

/// The paths the install's `FILES` names; none when it has no list.
fn listed(install: &Path) -> BTreeSet<String> {
    fs::read_to_string(install.join(FILES))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim_start_matches('\u{feff}').trim().replace('\\', "/"))
        .filter(|l| plain(l))
        .collect()
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn os_error(e: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(e.code().0 & 0xffff)
}

/// `MoveFileExW`, which refuses to replace a running exe; [`rename_over`] is the one that does not.
#[cfg(windows)]
fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW};
    use windows::core::PCWSTR;
    let (from, to) = (wide(from), wide(to));
    unsafe { MoveFileExW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) }.map_err(os_error)
}

#[cfg(not(windows))]
fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// A POSIX-semantics rename (`FileRenameInfoEx`) that replaces even a running exe; FAT and shares refuse it.
#[cfg(windows)]
fn rename_over(from: &Path, to: &Path) -> io::Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, DELETE, FILE_FLAGS_AND_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FileRenameInfoEx, OPEN_EXISTING, SetFileInformationByHandle,
    };
    use windows::core::PCWSTR;
    const REPLACE_IF_EXISTS: u32 = 0x1;
    const POSIX_SEMANTICS: u32 = 0x2;
    let from = wide(from);
    let to = wide(to);
    let name = &to[..to.len() - 1];
    let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let size = offset + std::mem::size_of_val(to.as_slice());
    let mut buf = vec![0u64; size.div_ceil(8)];
    let info = buf.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: `buf` is zeroed, 8-aligned and holds the header and the name with its nul.
    unsafe {
        (*info).Anonymous.Flags = REPLACE_IF_EXISTS | POSIX_SEMANTICS;
        (*info).FileNameLength = std::mem::size_of_val(name) as u32;
        std::ptr::copy_nonoverlapping(name.as_ptr(), info.cast::<u8>().add(offset).cast::<u16>(), name.len());
    }
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    let handle = unsafe { CreateFileW(PCWSTR(from.as_ptr()), DELETE.0, share, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }
        .map_err(os_error)?;
    let renamed = unsafe { SetFileInformationByHandle(handle, FileRenameInfoEx, info.cast(), size as u32) };
    let _ = unsafe { CloseHandle(handle) };
    renamed.map_err(os_error)
}

#[cfg(not(windows))]
fn rename_over(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// `DeleteFileW`, which refuses a running exe, for the same reason as [`move_file`].
#[cfg(windows)]
fn delete_file(path: &Path) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::DeleteFileW;
    use windows::core::PCWSTR;
    let wide = wide(path);
    unsafe { DeleteFileW(PCWSTR(wide.as_ptr())) }.map_err(os_error)
}

#[cfg(not(windows))]
fn delete_file(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

#[derive(Debug, Clone)]
pub struct Apply {
    /// The unpacked release folder.
    pub from: PathBuf,
    pub to: PathBuf,
    pub wait_pid: Option<u32>,
    pub wait: Duration,
    pub restart: Option<PathBuf>,
    pub restart_args: Vec<String>,
    /// `apply.log`; its folder also holds the journal and the backups.
    pub log: PathBuf,
    /// Stops as a kill would, before the move after this many: what a test of an interrupted apply sets.
    pub stop_after: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// Nothing in the install changed, or what changed was put back.
    Refused(String),
    /// [`Apply::stop_after`] was reached: the install is left as a kill there would leave it.
    Stopped,
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(why) => f.write_str(why),
            Self::Stopped => f.write_str("stopped before the next move"),
        }
    }
}

thread_local! {
    static RENAME_ASIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Makes [`apply`] on this thread take the rename-aside path for a locked file until the guard drops.
#[doc(hidden)]
#[must_use]
pub fn rename_aside_on_this_thread() -> RenameAside {
    RENAME_ASIDE.with(|c| c.set(true));
    RenameAside(())
}

#[doc(hidden)]
pub struct RenameAside(());

impl Drop for RenameAside {
    fn drop(&mut self) {
        RENAME_ASIDE.with(|c| c.set(false));
    }
}

#[derive(Debug)]
enum Step {
    Added(PathBuf),
    Replaced { target: PathBuf, backup: PathBuf },
    /// Replaced in one rename; `old` is a hard link to the file it replaced.
    Swapped { target: PathBuf, old: PathBuf, backup: PathBuf },
    Aside { target: PathBuf, old: PathBuf },
    Removed { target: PathBuf, backup: PathBuf },
}

/// How the apply ended and the pid of the Supraman it started, if any.
#[derive(Debug)]
pub struct Outcome {
    pub result: Result<(), Failed>,
    pub restarted: Option<u32>,
}

/// Swaps the release at `from` over the install at `to`, then starts `restart`; see docs/RELEASE.md.
pub fn apply(job: &Apply) -> Outcome {
    let mut run = Run::new(job);
    run.log(&format!("apply {} over {}", job.from.display(), job.to.display()));
    let result = run.run();
    let restarted = match &result {
        Err(Failed::Stopped) => None,
        Err(Failed::Refused(why)) => {
            run.log(&format!("failed: {why}"));
            run.undo();
            if run.waited { run.restart() } else { None }
        }
        Ok(()) => {
            run.finish();
            run.log("done");
            run.restart()
        }
    };
    Outcome { result, restarted }
}

struct Run<'a> {
    job: &'a Apply,
    dir: PathBuf,
    stamp: String,
    moves: usize,
    steps: Vec<Step>,
    /// The old Supraman has exited: a failed apply starts it again.
    waited: bool,
    _lock: Option<fs::File>,
}

impl<'a> Run<'a> {
    fn new(job: &'a Apply) -> Self {
        let dir = job.log.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let stamp = crate::time::now_ms().to_string();
        Self { job, dir, stamp, moves: 0, steps: Vec::new(), waited: false, _lock: None }
    }

    fn log(&self, line: &str) {
        let at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let _ = fs::create_dir_all(&self.dir);
        let _ = crate::fs::append_capped(&self.job.log, format!("{at} {line}\n").as_bytes(), LOG_CAP);
    }

    fn run(&mut self) -> Result<(), Failed> {
        let refused = |why: String| Err(Failed::Refused(why));
        let (from, to) = (&self.job.from, &self.job.to);
        if let Some(pid) = self.job.wait_pid
            && crate::process::alive(pid)
            && !crate::platform::process::wait_any_exit(&[pid], self.job.wait)
            && crate::process::alive(pid)
        {
            return refused(format!("Supraman (pid {pid}) did not exit within {} s: nothing changed", self.job.wait.as_secs()));
        }
        self.waited = true;
        for (dir, what) in [(from, "the release"), (to, "the install")] {
            if !dir.join(VERSION).is_file() {
                return refused(format!("{what} {} has no {VERSION}: nothing changed", dir.display()));
            }
        }
        self._lock = Some(crate::fs::lock_file(&to.join(LOCK)).map_err(|e| Failed::Refused(format!("{}: {e}", to.join(LOCK).display())))?);
        let old = listed(to);
        let new = file_list(from).map_err(|e| Failed::Refused(format!("{}: {e}", from.display())))?;
        let last = [FILES, VERSION];
        for rel in new.iter().filter(|r| !last.contains(&r.as_str())) {
            self.place(&from.join(rel), rel)?;
        }
        for rel in old.difference(&new).filter(|r| !last.contains(&r.as_str()) && r.as_str() != LEFTOVERS) {
            self.remove(rel)?;
        }
        let mut list: BTreeSet<String> = new.clone();
        list.remove(FILES);
        let staged = self.dir.join(FILES);
        let text: String = list.iter().map(|r| format!("{r}\n")).collect();
        crate::fs::write_atomic(&staged, text).map_err(|e| Failed::Refused(format!("{}: {e}", staged.display())))?;
        self.place(&staged, FILES)?;
        self.place(&from.join(VERSION), VERSION)
    }

    fn check_stop(&self) -> Result<(), Failed> {
        if self.job.stop_after == Some(self.moves) { Err(Failed::Stopped) } else { Ok(()) }
    }

    /// One rename that changes the install, retried briefly while a scanner holds the file.
    fn mv(&mut self, from: &Path, to: &Path) -> Result<Result<(), io::Error>, Failed> {
        self.retried(from, to, move_file)
    }

    /// `staged` renamed over `target` in one move, after a hard link `old` to the file it replaces.
    fn swap(&mut self, target: &Path, old: &Path, staged: &Path) -> Result<Result<(), io::Error>, Failed> {
        self.check_stop()?;
        if let Err(e) = fs::hard_link(target, old) {
            return Ok(Err(e));
        }
        self.moves += 1;
        let renamed = self.retried(staged, target, rename_over)?;
        if renamed.is_err() {
            let _ = delete_file(old);
        }
        Ok(renamed)
    }

    fn retried(&mut self, from: &Path, to: &Path, rename: fn(&Path, &Path) -> io::Result<()>) -> Result<Result<(), io::Error>, Failed> {
        self.check_stop()?;
        let mut tries = 0;
        loop {
            match rename(from, to) {
                Ok(()) => {
                    self.moves += 1;
                    return Ok(Ok(()));
                }
                Err(_) if tries < 3 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(40));
                }
                Err(e) => return Ok(Err(e)),
            }
        }
    }

    fn backup(&self, target: &Path, rel: &str) -> Result<PathBuf, Failed> {
        let backup = self.dir.join("backup").join(rel);
        backup.parent().map(fs::create_dir_all).transpose().and_then(|_| fs::copy(target, &backup)).map_err(|e| {
            Failed::Refused(format!("cannot back up {}: {e}", target.display()))
        })?;
        Ok(backup)
    }

    /// Appends to [`LEFTOVERS`] before the rename it names, so a kill after the rename still finds it.
    fn note_leftover(&mut self, kind: &str, rel: &str) -> Result<(), Failed> {
        let line = format!("{kind} {rel}");
        let path = self.job.to.join(LEFTOVERS);
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| {
                f.write_all(format!("{line}\n").as_bytes())?;
                f.sync_all()
            })
            .map_err(|e| Failed::Refused(format!("{}: {e}", path.display())))?;
        Ok(())
    }

    fn place(&mut self, src: &Path, rel: &str) -> Result<(), Failed> {
        let refused = |what: String| move |e: io::Error| Failed::Refused(format!("{what}: {e}"));
        let target = self.job.to.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(refused(parent.display().to_string()))?;
        }
        let staged = with_suffix(&target, ".new");
        fs::copy(src, &staged)
            .and_then(|_| fs::File::options().write(true).open(&staged)?.sync_all())
            .map_err(refused(format!("cannot stage {}", staged.display())))?;
        if !target.exists() {
            self.mv(&staged, &target)?.map_err(refused(format!("cannot add {}", target.display())))?;
            self.steps.push(Step::Added(target));
            return Ok(());
        }
        let backup = self.backup(&target, rel)?;
        if self.mv(&staged, &target)?.is_ok() {
            self.log(&format!("replaced {rel}"));
            self.steps.push(Step::Replaced { target, backup });
            return Ok(());
        }
        let mut old_rel = format!("{rel}.old-{}", self.stamp);
        if !RENAME_ASIDE.with(std::cell::Cell::get) {
            let old = self.job.to.join(&old_rel);
            self.note_leftover("aside", &old_rel)?;
            match self.swap(&target, &old, &staged)? {
                Ok(()) => {
                    self.log(&format!("replaced {rel} in one rename, the old one linked as {old_rel}"));
                    self.steps.push(Step::Swapped { target, old, backup });
                    return Ok(());
                }
                Err(e) => self.log(&format!("one rename refused for {rel} ({e}): renaming aside")),
            }
            if old.exists() {
                old_rel.push_str("-aside");
            }
        }
        let old = self.job.to.join(&old_rel);
        self.note_leftover("aside", &old_rel)?;
        if let Err(e) = self.mv(&target, &old)? {
            let _ = fs::remove_file(&staged);
            return Err(Failed::Refused(format!("{rel} is locked and cannot be renamed aside: {e}")));
        }
        self.log(&format!("renamed aside {rel} -> {old_rel}"));
        self.steps.push(Step::Aside { target: target.clone(), old });
        self.mv(&staged, &target)?.map_err(refused(format!("cannot move the new {rel} in")))?;
        Ok(())
    }

    fn remove(&mut self, rel: &str) -> Result<(), Failed> {
        let target = self.job.to.join(rel);
        if !target.is_file() {
            return Ok(());
        }
        let backup = self.backup(&target, rel)?;
        self.check_stop()?;
        if delete_file(&target).is_ok() {
            self.moves += 1;
            self.log(&format!("removed {rel}"));
            self.steps.push(Step::Removed { target, backup });
            return Ok(());
        }
        let old = with_suffix(&target, &format!(".old-{}", self.stamp));
        let old_rel = format!("{rel}.old-{}", self.stamp);
        self.note_leftover("removed", &old_rel)?;
        if let Err(e) = self.mv(&target, &old)? {
            return Err(Failed::Refused(format!("{rel} is locked and cannot be removed or renamed aside: {e}")));
        }
        self.log(&format!("removed {rel} (renamed aside, locked)"));
        self.steps.push(Step::Aside { target, old });
        Ok(())
    }

    /// Puts back what the journal moved, newest first.
    fn undo(&mut self) {
        while let Some(step) = self.steps.pop() {
            let undone = match &step {
                Step::Added(target) => delete_file(target),
                Step::Replaced { target, backup } | Step::Removed { target, backup } => {
                    let staged = with_suffix(target, ".new");
                    fs::copy(backup, &staged).and_then(|_| move_file(&staged, target))
                }
                Step::Swapped { target, old, backup } => rename_over(old, target).or_else(|_| {
                    let staged = with_suffix(target, ".new");
                    fs::copy(backup, &staged).and_then(|_| rename_over(&staged, target))
                }),
                Step::Aside { target, old } => {
                    let _ = delete_file(target);
                    move_file(old, target)
                }
            };
            match undone {
                Ok(()) => self.log(&format!("undid {step:?}")),
                Err(e) => self.log(&format!("cannot undo {step:?}: {e}")),
            }
        }
        let _ = fs::remove_dir_all(self.dir.join("backup"));
        self._lock = None;
        cleanup(&self.job.to);
    }

    fn finish(&mut self) {
        let _ = fs::remove_dir_all(self.dir.join("backup"));
        let _ = fs::remove_file(self.dir.join(FILES));
        self._lock = None;
        cleanup(&self.job.to);
    }

    fn restart(&self) -> Option<u32> {
        let exe = self.job.restart.as_ref()?;
        let mut cmd = crate::process::command(exe);
        cmd.args(&self.job.restart_args).current_dir(&self.job.to);
        match crate::process::spawn_detached(&mut cmd) {
            Ok(pid) => {
                self.log(&format!("started {} (pid {pid})", exe.display()));
                Some(pid)
            }
            Err(e) => {
                self.log(&format!("cannot start {}: {e}", exe.display()));
                None
            }
        }
    }
}

/// Removes the unlocked `.old-<stamp>` files [`LEFTOVERS`] lists and puts back one a killed apply left aside.
pub fn cleanup(install: &Path) {
    let list = install.join(LEFTOVERS);
    let Ok(text) = fs::read_to_string(&list) else { return };
    let Ok(Some(_lock)) = crate::fs::try_lock_file(&install.join(LOCK)) else { return };
    let text = fs::read_to_string(&list).unwrap_or(text);
    let mut kept = Vec::new();
    for line in text.lines() {
        let Some((kind, rel)) = line.trim().split_once(' ') else { continue };
        let Some((base, _)) = rel.rsplit_once(".old-") else { continue };
        if !plain(rel) || !plain(base) {
            continue;
        }
        let (old, original) = (install.join(rel), install.join(base));
        if !old.exists() {
            continue;
        }
        let done = if kind == "aside" && !original.exists() { move_file(&old, &original) } else { delete_file(&old) };
        if done.is_err() {
            kept.push(line.trim().to_string());
        }
    }
    if kept.is_empty() {
        let _ = fs::remove_file(&list);
    } else {
        let _ = crate::fs::write_atomic(&list, kept.iter().map(|l| format!("{l}\n")).collect::<String>());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_of(entries: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("r.zip");
        let mut w = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            if name.ends_with('/') {
                w.add_directory(*name, options).unwrap();
            } else {
                w.start_file(*name, options).unwrap();
                w.write_all(bytes).unwrap();
            }
        }
        w.finish().unwrap();
        (tmp, path)
    }

    #[test]
    fn a_release_zip_unpacks_into_its_root_folder() {
        let (tmp, zip) = zip_of(&[("Supraman/", b""), ("Supraman/VERSION", b"version 0.2.0\n"), (r"Supraman\plugins\supraman\bin\sm.exe", b"sm")]);
        let out = tmp.path().join("updates").join("0.2.0");
        fs::create_dir_all(out.join(ROOT)).unwrap();
        fs::write(out.join(ROOT).join("stale.txt"), "from an earlier unpack").unwrap();
        let root = unpack(&zip, &out).unwrap();
        assert_eq!(root, out.join(ROOT));
        assert_eq!(file_list(&root).unwrap().into_iter().collect::<Vec<_>>(), ["VERSION", "plugins/supraman/bin/sm.exe"]);
        assert_eq!(fs::read(root.join("plugins/supraman/bin/sm.exe")).unwrap(), b"sm");
    }

    #[test]
    fn an_entry_that_escapes_or_is_outside_the_root_refuses_the_zip() {
        for bad in ["Supraman/../evil.txt", "Supraman/a/../../evil.txt", "/Supraman/abs.txt", "C:/Supraman/x", "Other/x.txt", "evil.txt", "Supraman/C:x"] {
            let (tmp, zip) = zip_of(&[("Supraman/VERSION", b"v"), (bad, b"x")]);
            let out = tmp.path().join("out");
            let err = unpack(&zip, &out).expect_err(bad);
            assert!(err.contains("escapes") || err.contains("outside"), "{bad}: {err}");
            assert!(!out.exists() && !tmp.path().join("evil.txt").exists(), "{bad}: nothing is written");
        }
    }

    #[test]
    fn plain_paths_only() {
        for ok in ["a", "plugins/supraman/bin/sm.exe", "x\\y.txt"] {
            assert!(plain(ok), "{ok}");
        }
        for bad in ["", "/a", "\\a", "a/../b", "..", "./a", "C:/x", "a//b", "a/b:stream"] {
            assert!(!plain(bad), "{bad}");
        }
    }

    #[test]
    fn the_install_of_the_hooks_copy_is_three_folders_up() {
        let root = Path::new(r"C:\x\Supraman");
        assert_eq!(install_dir(&root.join(r"plugins\supraman\bin")), root);
        assert_eq!(install_dir(root), root);
        assert_eq!(install_dir(&root.join("bin")), root.join("bin"));
    }

    #[test]
    fn exe_dir_is_the_running_exes_folder_and_sm_exe_sits_in_it() {
        let exe = std::env::current_exe().unwrap();
        assert_eq!(exe_dir().unwrap(), exe.parent().unwrap());
        assert_eq!(sm_exe().unwrap(), exe.with_file_name("sm.exe"));
    }
}
