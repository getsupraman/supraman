//! The one write Supraman makes under the claude dir: a confirmed session's `<sessionId>.jsonl` and its sibling
//! `<sessionId>/` in `projects/<project dir>/`, nothing else. Every path is checked first (a UUID id, no symlink or
//! junction, the canonical path where it belongs); one refused path deletes nothing.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::transcript::slug;

#[derive(Debug, thiserror::Error)]
pub enum ForgetError {
    #[error("{0:?} is not a session id (a UUID)")]
    NotASessionId(String),
    #[error("refused to delete {}: {why}", .path.display())]
    Refused { path: PathBuf, why: &'static str },
    #[error("cannot delete {}: {source}", .path.display())]
    Io { path: PathBuf, source: io::Error },
}

/// What [`forget_transcript`] deleted.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Forgotten {
    pub transcripts: Vec<PathBuf>,
    pub dirs: Vec<PathBuf>,
}

impl Forgotten {
    pub fn is_empty(&self) -> bool {
        self.transcripts.is_empty() && self.dirs.is_empty()
    }
}

/// `8-4-4-4-12` hex digits, as claude names a session.
pub fn is_session_id(id: &str) -> bool {
    let groups: Vec<&str> = id.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Deletes `session_id`'s transcript and sibling dir from every project dir under `<claude_dir>/projects`
/// that holds them; `cwd`'s project dir is looked at first. Nothing there is not an error.
pub fn forget_transcript(claude_dir: &Path, session_id: &str, cwd: Option<&Path>) -> Result<Forgotten, ForgetError> {
    if !is_session_id(session_id) {
        return Err(ForgetError::NotASessionId(session_id.to_string()));
    }
    let projects = claude_dir.join("projects");
    let root = match fs::canonicalize(&projects) {
        Ok(root) => root,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Forgotten::default()),
        Err(source) => return Err(ForgetError::Io { path: projects, source }),
    };
    let mut names: Vec<String> = cwd.map(slug).into_iter().collect();
    let listed = fs::read_dir(&root).map_err(|source| ForgetError::Io { path: root.clone(), source })?;
    names.extend(listed.flatten().filter_map(|e| e.file_name().to_str().map(str::to_string)));
    let transcript = format!("{session_id}.jsonl");
    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in names {
        if !seen.insert(name.to_ascii_lowercase()) {
            continue;
        }
        let dir = root.join(&name);
        let file = dir.join(&transcript);
        let sibling = dir.join(session_id);
        let has = |p: &Path| fs::symlink_metadata(p).is_ok();
        if !has(&file) && !has(&sibling) {
            continue;
        }
        let meta = fs::symlink_metadata(&dir).map_err(|source| ForgetError::Io { path: dir.clone(), source })?;
        if is_link(&meta) || !meta.is_dir() {
            return Err(ForgetError::Refused { path: dir, why: "the project dir is a link, not a dir under projects" });
        }
        if has(&file) {
            found.push((confined(&root, &name, &transcript, false)?, false));
        }
        if has(&sibling) {
            found.push((confined(&root, &name, session_id, true)?, true));
        }
    }
    let mut out = Forgotten::default();
    for (path, is_dir) in found {
        let result = if is_dir { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
        result.map_err(|source| ForgetError::Io { path: path.clone(), source })?;
        if is_dir { out.dirs.push(path) } else { out.transcripts.push(path) }
    }
    Ok(out)
}

/// The first `<project dir>/<session_id>.jsonl` under `<claude_dir>/projects`, `cwd`'s project dir first.
/// Read only.
pub fn find_transcript(claude_dir: &Path, session_id: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    if !is_session_id(session_id) {
        return None;
    }
    let projects = claude_dir.join("projects");
    let name = format!("{session_id}.jsonl");
    if let Some(cwd) = cwd {
        let path = projects.join(slug(cwd)).join(&name);
        if path.is_file() {
            return Some(path);
        }
    }
    fs::read_dir(&projects).ok()?.flatten().map(|e| e.path().join(&name)).find(|p| p.is_file())
}

/// `<root>/<dir>/<item>` when it is a plain file (`want_dir`: a plain dir) that canonicalises to itself.
fn confined(root: &Path, dir: &str, item: &str, want_dir: bool) -> Result<PathBuf, ForgetError> {
    let path = root.join(dir).join(item);
    let meta = fs::symlink_metadata(&path).map_err(|source| ForgetError::Io { path: path.clone(), source })?;
    if is_link(&meta) {
        return Err(ForgetError::Refused { path, why: "it is a symlink or junction" });
    }
    if meta.is_dir() != want_dir {
        return Err(ForgetError::Refused { path, why: if want_dir { "it is not a dir" } else { "it is not a file" } });
    }
    let canonical = fs::canonicalize(&path).map_err(|source| ForgetError::Io { path: path.clone(), source })?;
    let inside = canonical.parent().is_some_and(|p| p.parent() == Some(root) && p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(dir)))
        && canonical.file_name().is_some_and(|n| n.eq_ignore_ascii_case(item));
    if !inside {
        return Err(ForgetError::Refused { path, why: "it does not resolve to itself inside projects/" });
    }
    Ok(canonical)
}

fn is_link(meta: &fs::Metadata) -> bool {
    supraman_core::platform::fs::is_reparse_point(meta) || meta.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    const OTHER: &str = "ffffffff-0000-4000-8000-000000000001";

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "{}\n").unwrap();
    }

    /// A fake claude dir: two project dirs, the session in A, another in A and B, files beside projects/.
    fn home() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let a = claude.join("projects").join("C--work-a");
        touch(&a.join(format!("{SID}.jsonl")));
        touch(&a.join(SID).join("subagents").join("agent-1.jsonl"));
        touch(&a.join(SID).join("tool-results").join("r.txt"));
        touch(&a.join(format!("{OTHER}.jsonl")));
        touch(&a.join(OTHER).join("subagents").join("agent-2.jsonl"));
        touch(&claude.join("projects").join("C--work-b").join(format!("{OTHER}.jsonl")));
        touch(&claude.join("settings.json"));
        touch(&claude.join("sessions").join("123.json"));
        (tmp, claude)
    }

    /// Every file under `dir`, relative.
    fn files(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                let meta = fs::symlink_metadata(e.path()).unwrap();
                if meta.is_dir() && !is_link(&meta) {
                    stack.push(e.path());
                } else {
                    out.push(e.path().strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        out.sort();
        out
    }

    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let out = supraman_core::process::command("cmd").arg("/C").arg("mklink").arg("/J").arg(link).arg(target).output().unwrap();
        assert!(out.status.success(), "mklink /J: {out:?}");
    }

    #[test]
    fn only_the_sessions_transcript_and_its_dir_go() {
        let (_tmp, claude) = home();
        let before = files(&claude);
        let gone = forget_transcript(&claude, SID, None).unwrap();
        assert_eq!(gone.transcripts.len(), 1);
        assert_eq!(gone.dirs.len(), 1);
        assert!(gone.transcripts[0].ends_with(format!("C--work-a/{SID}.jsonl")) || gone.transcripts[0].ends_with(format!("C--work-a\\{SID}.jsonl")));
        let after = files(&claude);
        let expected: Vec<String> = before.into_iter().filter(|f| !f.contains(SID)).collect();
        assert_eq!(after, expected, "nothing else under the claude dir changed");
        assert!(claude.join("projects").join("C--work-a").is_dir(), "the project dir stays");
        assert_eq!(forget_transcript(&claude, SID, None).unwrap(), Forgotten::default(), "a second delete finds nothing");
    }

    #[test]
    fn the_cwds_project_dir_and_every_other_holding_it() {
        let (_tmp, claude) = home();
        let gone = forget_transcript(&claude, OTHER, Some(Path::new(r"C:\work\b"))).unwrap();
        assert_eq!(gone.transcripts.len(), 2, "{gone:?}");
        assert!(gone.transcripts[0].parent().unwrap().ends_with("C--work-b"), "the cwd's dir first");
        assert_eq!(gone.dirs.len(), 1);
        assert!(claude.join("projects").join("C--work-a").join(format!("{SID}.jsonl")).exists());
    }

    #[test]
    fn anything_but_a_uuid_is_refused_before_a_path_is_built() {
        let (_tmp, claude) = home();
        for id in ["..", "../C--work-a", "..\\settings", "", "0a1b2c3d", &format!("{SID}/.."), &format!("{SID}\\x"), &format!("../{SID}"), "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4z"] {
            assert!(matches!(forget_transcript(&claude, id, None), Err(ForgetError::NotASessionId(_))), "{id:?}");
        }
        assert!(is_session_id(SID) && is_session_id(&SID.to_uppercase()));
        assert_eq!(files(&claude).len(), 8, "nothing deleted");
    }

    #[test]
    fn a_claude_dir_given_with_dot_dot_is_canonicalised_and_stays_confined() {
        let (_tmp, claude) = home();
        let roundabout = claude.join("projects").join("..").join("sessions").join("..");
        let gone = forget_transcript(&roundabout, SID, None).unwrap();
        assert_eq!(gone.transcripts.len(), 1);
        assert!(claude.join("sessions").join("123.json").exists() && claude.join("settings.json").exists());
    }

    #[test]
    fn find_transcript_looks_in_the_cwds_project_dir_first_and_reads_only_files() {
        let (_tmp, claude) = home();
        let projects = claude.join("projects");
        assert_eq!(find_transcript(&claude, SID, None), Some(projects.join("C--work-a").join(format!("{SID}.jsonl"))));
        assert_eq!(
            find_transcript(&claude, OTHER, Some(Path::new(r"C:\work\b"))),
            Some(projects.join("C--work-b").join(format!("{OTHER}.jsonl")))
        );
        assert_eq!(find_transcript(&claude, SID, Some(Path::new(r"C:\work\b"))).unwrap().parent(), Some(projects.join("C--work-a").as_path()));
        let absent = "11111111-2222-4333-8444-555555555555";
        fs::create_dir_all(projects.join("C--work-b").join(format!("{absent}.jsonl"))).unwrap();
        assert_eq!(find_transcript(&claude, absent, None), None, "a dir of that name is no transcript");
        assert_eq!(find_transcript(&claude, "../C--work-a", None), None);
        assert_eq!(find_transcript(&claude.join("missing"), SID, None), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_sibling_dir_that_cannot_go_fails_after_the_transcript_went_and_a_second_call_finishes() {
        use std::os::windows::fs::OpenOptionsExt as _;
        let (_tmp, claude) = home();
        let a = claude.join("projects").join("C--work-a");
        let held = a.join(SID).join("tool-results").join("r.txt");
        let open = fs::OpenOptions::new().read(true).share_mode(0).open(&held).unwrap();
        let err = forget_transcript(&claude, SID, None).unwrap_err();
        assert!(matches!(&err, ForgetError::Io { path, .. } if path.ends_with(SID)), "{err}");
        assert!(!a.join(format!("{SID}.jsonl")).exists(), "the transcript went first");
        assert!(held.exists());
        assert!(a.join(format!("{OTHER}.jsonl")).exists() && a.join(OTHER).is_dir(), "nothing else");
        drop(open);
        let gone = forget_transcript(&claude, SID, None).unwrap();
        assert_eq!((gone.transcripts.len(), gone.dirs.len()), (0, 1));
        assert!(!a.join(SID).exists());
    }

    #[test]
    fn no_projects_dir_is_nothing_to_delete() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(forget_transcript(tmp.path(), SID, None).unwrap(), Forgotten::default());
    }

    #[cfg(windows)]
    #[test]
    fn a_junctioned_sibling_dir_is_refused_and_nothing_goes() {
        let (tmp, claude) = home();
        let outside = tmp.path().join("outside");
        touch(&outside.join("precious.txt"));
        let a = claude.join("projects").join("C--work-a");
        fs::remove_dir_all(a.join(SID)).unwrap();
        junction(&a.join(SID), &outside);
        let err = forget_transcript(&claude, SID, None).unwrap_err();
        assert!(matches!(err, ForgetError::Refused { why: "it is a symlink or junction", .. }), "{err}");
        assert!(outside.join("precious.txt").exists());
        assert!(a.join(format!("{SID}.jsonl")).exists(), "one refused path deletes nothing");
    }

    #[cfg(windows)]
    #[test]
    fn a_junctioned_project_dir_is_refused() {
        let (tmp, claude) = home();
        let outside = tmp.path().join("elsewhere");
        touch(&outside.join(format!("{SID}.jsonl")));
        junction(&claude.join("projects").join("C--linked"), &outside);
        fs::remove_file(claude.join("projects").join("C--work-a").join(format!("{SID}.jsonl"))).unwrap();
        fs::remove_dir_all(claude.join("projects").join("C--work-a").join(SID)).unwrap();
        let err = forget_transcript(&claude, SID, None).unwrap_err();
        assert!(matches!(err, ForgetError::Refused { .. }), "{err}");
        assert!(outside.join(format!("{SID}.jsonl")).exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_inside_the_sibling_dir_goes_without_its_target() {
        let (tmp, claude) = home();
        let outside = tmp.path().join("target-dir");
        touch(&outside.join("precious.txt"));
        junction(&claude.join("projects").join("C--work-a").join(SID).join("link"), &outside);
        let gone = forget_transcript(&claude, SID, None).unwrap();
        assert_eq!(gone.dirs.len(), 1);
        assert!(!claude.join("projects").join("C--work-a").join(SID).exists());
        assert!(outside.join("precious.txt").exists(), "remove_dir_all removes the link, not what it points at");
    }

    #[cfg(windows)]
    #[test]
    fn a_symlinked_transcript_is_refused_when_symlinks_can_be_made() {
        let (tmp, claude) = home();
        let outside = tmp.path().join("secret.jsonl");
        touch(&outside);
        let file = claude.join("projects").join("C--work-a").join(format!("{SID}.jsonl"));
        fs::remove_file(&file).unwrap();
        if std::os::windows::fs::symlink_file(&outside, &file).is_err() {
            eprintln!("skipped: this process may not create symlinks");
            return;
        }
        let err = forget_transcript(&claude, SID, None).unwrap_err();
        assert!(matches!(err, ForgetError::Refused { why: "it is a symlink or junction", .. }), "{err}");
        assert!(outside.exists());
        assert!(claude.join("projects").join("C--work-a").join(SID).exists(), "nothing deleted");
    }
}
