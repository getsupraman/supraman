//! What the `fm probe` subcommands share: a [`Fleet`] of throwaway `aw-m2-probe-*` sessions run from
//! `<cargo root>/target/aw-m2-probe-<hex>` (a plain subfolder of the trusted worktree), and its cleanup.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use foreman_claude::{Claude, LaunchSpec, Removal, Session};

pub(crate) const ROW_WITHIN: Duration = Duration::from_secs(15);

/// Every name launched here; dropping it stops and removes each row with one of those names.
pub(crate) struct Fleet {
    pub(crate) claude: Claude,
    pub(crate) names: Vec<String>,
}

impl Fleet {
    pub(crate) fn launch(&mut self, spec: &LaunchSpec) -> Result<String, String> {
        assert!(spec.name.starts_with("aw-m2-probe-"));
        if !self.names.contains(&spec.name) {
            self.names.push(spec.name.clone());
        }
        self.claude.launch(spec).map_err(|e| format!("launch {}: {e}", spec.name))
    }

    pub(crate) fn row(&self, name: &str) -> Result<Session, String> {
        self.claude
            .find_by_name(name, ROW_WITHIN)
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or_else(|| format!("no row named {name}"))
    }

    /// Stops and removes every row named `name`. A `--bg` session started in a main checkout moves into
    /// `<repo>/.claude/worktrees/<w>` before its first write and commits there; `claude rm` then keeps the
    /// row and the worktree and names the way out (`--discard-unpushed <sha>@<token>`), which is taken:
    /// the row is ours, so is its worktree. A worktree rm keeps for another reason (dirty, locked) is
    /// removed with git, its `worktree-*` branch deleted, and the row removed again (m2/2).
    pub(crate) fn remove(&mut self, name: &str) {
        let rows = self.claude.find_by_name(name, Duration::ZERO).unwrap_or_default();
        for row in rows {
            let _ = self.claude.stop(&row.id);
            let _ = wait_until_stopped(&self.claude, name);
            let refusal = match self.claude.remove(&row.id, None) {
                Ok(Removal::Removed(out)) => {
                    println!("cleanup: rm {} ({name}){}", row.id, one_line_tail(&out));
                    continue;
                }
                Ok(Removal::Kept(refusal)) => refusal,
                Err(e) => {
                    println!("cleanup: rm {} ({name}) failed{}", row.id, one_line_tail(&e.to_string()));
                    continue;
                }
            };
            println!("cleanup: rm {} ({name}) refused{}", row.id, one_line_tail(&refusal.text));
            if let Some(discard) = &refusal.discard {
                match self.claude.remove(&row.id, Some(discard)) {
                    Ok(Removal::Removed(out)) => {
                        println!("cleanup: rm {} --discard-unpushed{}", row.id, one_line_tail(&out));
                        prune_empty(refusal.worktree.as_deref());
                        continue;
                    }
                    Ok(Removal::Kept(again)) => println!("cleanup: rm --discard-unpushed refused{}", one_line_tail(&again.text)),
                    Err(e) => println!("cleanup: rm --discard-unpushed failed: {e}"),
                }
            }
            let Some((repo, worktree)) = refusal.worktree.as_deref().and_then(claude_worktree) else { continue };
            let branch = git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
            let wt = worktree.display().to_string();
            let _ = git(&repo, &["worktree", "unlock", &wt]);
            let removed = git(&repo, &["worktree", "remove", "--force", "--force", &wt]);
            let branch_gone = match branch.strip_prefix("worktree-") {
                Some(_) => git(&repo, &["branch", "-D", &branch]).map(|_| format!("branch {branch} deleted")),
                None => Ok(format!("branch {branch} kept (not worktree-*)")),
            };
            println!("cleanup: git worktree remove {wt}: {removed:?}; {branch_gone:?}");
            prune_empty(Some(&worktree));
            match self.claude.rm(&row.id) {
                Ok(_) => println!("cleanup: rm {} ({name}) after the worktree went", row.id),
                Err(e) => println!("cleanup: rm {} ({name}) failed again: {e}", row.id),
            }
        }
    }
}

impl Drop for Fleet {
    fn drop(&mut self) {
        for name in self.names.clone() {
            self.remove(&name);
        }
        let left: Vec<String> = self
            .claude
            .list_sessions()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| self.names.contains(&s.name))
            .map(|s| s.id)
            .collect();
        println!("cleanup: {} aw-m2-probe row(s) of this run left {left:?}", left.len());
    }
}

/// `(<repo>, <repo>/.claude/worktrees/<w>)` when `cwd` is in such a worktree.
pub(crate) fn claude_worktree(cwd: &Path) -> Option<(PathBuf, PathBuf)> {
    let parts: Vec<_> = cwd.components().collect();
    let at = parts.windows(2).position(|w| w[0].as_os_str() == ".claude" && w[1].as_os_str() == "worktrees")?;
    parts.get(at + 2)?;
    let repo: PathBuf = parts[..at].iter().collect();
    let worktree: PathBuf = parts[..at + 3].iter().collect();
    Some((repo, worktree))
}

/// `git -C <dir> <args>`: trimmed stdout, or stderr as the error.
pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = foreman_core::process::git(dir, args).map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if out.success() { Ok(out.stdout.trim().to_string()) } else { Err(out.stderr.trim().to_string()) }
}

/// Removes `<repo>/.claude/worktrees` and `<repo>/.claude` when a removal left them empty.
fn prune_empty(worktree: Option<&Path>) {
    let Some(worktrees) = worktree.and_then(Path::parent) else { return };
    if fs::remove_dir(worktrees).is_ok()
        && let Some(dot_claude) = worktrees.parent()
    {
        let _ = fs::remove_dir(dot_claude);
    }
}

fn one_line_tail(out: &str) -> String {
    let text = out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" / ");
    if text.is_empty() { String::new() } else { format!(": {text}") }
}

pub(crate) fn wait_until_stopped(claude: &Claude, name: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let rows = claude.find_by_name(name, Duration::ZERO).map_err(|e| e.to_string())?;
        if rows.iter().all(|r| r.pid.is_none()) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(500));
    }
    Err(format!("{name} still has a pid 20 s after stop"))
}

pub(crate) fn describe(s: &Session) -> String {
    format!(
        "id={} sessionId={} name={} pid={} state={}",
        s.id,
        s.session_id,
        s.name,
        s.pid.map_or("-".into(), |p| p.to_string()),
        s.state.as_ref().map_or("-", |st| st.as_str()),
    )
}

pub(crate) fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// Without the `\\?\` prefix `canonicalize` adds on Windows: Claude Code appends `/bin/fm.exe` to the
/// plugin dir, and a verbatim path does not accept `/` as a separator.
pub(crate) fn cargo_root() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").canonicalize().unwrap_or_else(|_| PathBuf::from("."));
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(plain) if !plain.starts_with("UNC") => PathBuf::from(plain),
        _ => path,
    }
}

pub(crate) fn mkdir(root: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = root.join(name);
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    Ok(dir)
}

pub(crate) fn write(dir: &Path, name: &str, text: &str) -> Result<PathBuf, String> {
    let path = dir.join(name);
    fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

pub(crate) use foreman_core::time::hex6;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_claude_worktree_a_cwd_is_in() {
        let (repo, wt) = claude_worktree(Path::new(r"P:\r\AutoWork\.claude\worktrees\role-tags\target\x")).unwrap();
        assert_eq!(repo, PathBuf::from(r"P:\r\AutoWork"));
        assert_eq!(wt, PathBuf::from(r"P:\r\AutoWork\.claude\worktrees\role-tags"));
        assert_eq!(claude_worktree(Path::new(r"P:\r\AutoWork\target\x")), None);
        assert_eq!(claude_worktree(Path::new(r"P:\r\AutoWork\.claude\worktrees")), None);
    }
}
