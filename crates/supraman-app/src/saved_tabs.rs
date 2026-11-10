use std::path::{Path, PathBuf};

use supraman_claude::{Kind, Session, is_session_id};
use supraman_core::FM_PREFIX;
use supraman_core::fs::{load_json_or_notice, write_atomic};
use serde::{Deserialize, Serialize};

use crate::fleet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SavedTab {
    /// `shell` is what the tab ran; without it the tab reopens with the default shell.
    Shell {
        title: String,
        cwd: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<SavedShell>,
    },
    /// A hand tab; `session_id` is the full uuid of the claude it runs.
    Claude { title: String, cwd: PathBuf, session_id: Option<String>, name: String },
    Attach { title: String, cwd: PathBuf, id: String, session_id: Option<String>, name: String },
    Work { title: String, workspace: String },
    Git { title: String, folder: PathBuf },
}

impl SavedTab {
    /// A hand tab's session as the newest of its `continued-in` chain (`newest`), where a resume or attach finds it.
    pub fn followed(self, newest: impl Fn(&str) -> String) -> Self {
        match self {
            SavedTab::Claude { title, cwd, session_id: Some(sid), name } if is_session_id(&sid) => {
                SavedTab::Claude { title, cwd, session_id: Some(newest(&sid)), name }
            }
            other => other,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            SavedTab::Shell { title, .. } | SavedTab::Claude { title, .. } | SavedTab::Attach { title, .. } | SavedTab::Work { title, .. } | SavedTab::Git { title, .. } => title,
        }
    }

    pub fn cwd(&self) -> Option<&Path> {
        match self {
            SavedTab::Shell { cwd, .. } | SavedTab::Claude { cwd, .. } | SavedTab::Attach { cwd, .. } | SavedTab::Git { folder: cwd, .. } => Some(cwd),
            SavedTab::Work { .. } => None,
        }
    }

    /// The session a Resume click resumes: by its full uuid, `claude --bg --resume` for an attached one.
    pub fn session(&self) -> Option<Session> {
        let (id, session_id, name, cwd, kind) = match self {
            SavedTab::Claude { session_id: Some(sid), name, cwd, .. } => (sid.get(..8)?.to_string(), sid, name, cwd, Kind::Interactive),
            SavedTab::Attach { id, session_id: Some(sid), name, cwd, .. } => (id.clone(), sid, name, cwd, Kind::Background),
            _ => return None,
        };
        is_session_id(session_id).then(|| Session {
            id,
            session_id: session_id.clone(),
            name: name.clone(),
            cwd: cwd.clone(),
            kind,
            state: None,
            status: None,
            waiting_for: None,
            pid: None,
            started_at: None,
        })
    }
}

/// A plain tab's program and the arguments it was started with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedShell {
    pub program: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTabs {
    pub tabs: Vec<SavedTab>,
    /// Index into `tabs`.
    pub active: Option<usize>,
}

pub fn path(home: &Path) -> PathBuf {
    home.join("tabs.json")
}

pub fn load(home: &Path) -> (SavedTabs, Option<String>) {
    load_json_or_notice(&path(home), "no tabs are reopened")
}

pub fn save(home: &Path, tabs: &SavedTabs) -> Result<(), String> {
    let path = path(home);
    let text = serde_json::to_string_pretty(tabs).map_err(|e| e.to_string())?;
    write_atomic(&path, text).map_err(|e| format!("Cannot save the open tabs to {}: {e}", path.display()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reopen {
    Shell { title: String, cwd: PathBuf, shell: Option<SavedShell> },
    Attach { title: String, session: Session },
    Resume { title: String, session: Session },
    /// A tab with a Resume button; nothing runs until it is clicked.
    Placeholder(SavedTab),
    Work { workspace: String },
    Git { title: String, folder: PathBuf },
}

pub struct World<'a> {
    pub sessions: &'a [Session],
    pub now: i64,
    pub workspaces: &'a [String],
    pub resume_on_click: bool,
    pub is_dir: &'a dyn Fn(&Path) -> bool,
    pub is_file: &'a dyn Fn(&Path) -> bool,
    pub has_transcript: &'a dyn Fn(&str, &Path) -> bool,
}

/// How `saved` comes back, or why it does not.
pub fn plan(saved: &SavedTab, world: &World) -> Result<Reopen, String> {
    if let Some(cwd) = saved.cwd()
        && !(world.is_dir)(cwd)
    {
        return Err(format!("its folder {} is gone", cwd.display()));
    }
    let resumable = |session: Option<Session>| {
        session.filter(|s| (world.has_transcript)(&s.session_id, &s.cwd)).ok_or_else(|| "it has no conversation to resume".to_string())
    };
    match saved {
        SavedTab::Shell { title, cwd, shell } => Ok(Reopen::Shell { title: title.clone(), cwd: cwd.clone(), shell: shell.clone().filter(|s| (world.is_file)(&s.program)) }),
        SavedTab::Work { workspace, .. } if world.workspaces.contains(workspace) => Ok(Reopen::Work { workspace: workspace.clone() }),
        SavedTab::Work { workspace, .. } => Err(format!("workspace {workspace} is gone")),
        SavedTab::Git { title, folder } => Ok(Reopen::Git { title: title.clone(), folder: folder.clone() }),
        SavedTab::Claude { title, .. } => {
            let session = saved.session().ok_or_else(|| "it has no conversation to resume".to_string())?;
            let live = world.sessions.iter().find(|s| s.session_id == session.session_id && fleet::is_alive(&s.health(world.now)));
            if let Some(live) = live.filter(|s| s.kind == Kind::Background) {
                return Ok(Reopen::Attach { title: title.clone(), session: live.clone() });
            }
            let session = resumable(Some(session))?;
            if live.is_some() {
                return Err("it runs in another terminal".into());
            }
            Ok(if world.resume_on_click { Reopen::Placeholder(saved.clone()) } else { Reopen::Resume { title: title.clone(), session } })
        }
        SavedTab::Attach { title, id, name, .. } => {
            let listed = world.sessions.iter().find(|s| &s.id == id);
            if let Some(live) = listed.filter(|s| fleet::is_alive(&s.health(world.now))) {
                return Ok(Reopen::Attach { title: title.clone(), session: live.clone() });
            }
            if name.starts_with(FM_PREFIX) {
                return Err("it is stopped and Supraman's supervisor starts it".into());
            }
            resumable(saved.session())?;
            Ok(Reopen::Placeholder(saved.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use supraman_claude::SessionState;

    use super::*;

    const SID: &str = "0a0b0c0d-0000-4000-8000-000000000001";

    fn world<'a>(sessions: &'a [Session], workspaces: &'a [String], on_click: bool, has: &'a dyn Fn(&str, &Path) -> bool) -> World<'a> {
        World { sessions, now: 1_000_000, workspaces, resume_on_click: on_click, is_dir: &|p| !p.ends_with("gone"), is_file: &|p| !p.ends_with("gone.exe"), has_transcript: has }
    }

    fn attach(name: &str) -> SavedTab {
        SavedTab::Attach { title: "a".into(), cwd: "C:/demo".into(), id: SID[..8].into(), session_id: Some(SID.into()), name: name.into() }
    }

    fn row(state: SessionState, pid: Option<u32>) -> Session {
        Session {
            id: SID[..8].into(),
            session_id: SID.into(),
            name: "worker".into(),
            cwd: "C:/demo".into(),
            kind: Kind::Background,
            state: Some(state),
            status: None,
            waiting_for: None,
            pid,
            started_at: None,
        }
    }

    #[test]
    fn the_file_round_trips_in_order_with_the_active_tab() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(load(home.path()), (SavedTabs::default(), None), "no file, nothing to reopen and no notice");
        let tabs = SavedTabs {
            tabs: vec![
                SavedTab::Shell { title: "pwsh 1".into(), cwd: "C:/a".into(), shell: None },
                SavedTab::Claude { title: "c".into(), cwd: "C:/b".into(), session_id: Some(SID.into()), name: "hand".into() },
                attach("worker"),
                SavedTab::Work { title: "Work · w".into(), workspace: "w".into() },
            ],
            active: Some(2),
        };
        save(home.path(), &tabs).unwrap();
        assert_eq!(load(home.path()), (tabs, None));
        std::fs::write(path(home.path()), "{").unwrap();
        let (bad, notice) = load(home.path());
        assert_eq!(bad, SavedTabs::default());
        assert!(notice.is_some_and(|n| n.contains("tabs.json") && n.ends_with("no tabs are reopened")));
    }

    #[test]
    fn a_git_tab_comes_back_with_its_title_and_folder() {
        let folder = PathBuf::from("C:/demo/repo");
        let saved = SavedTab::Git { title: "Git · repo".into(), folder: folder.clone() };
        let home = tempfile::tempdir().unwrap();
        save(home.path(), &SavedTabs { tabs: vec![saved.clone()], active: Some(0) }).unwrap();
        let (back, notice) = load(home.path());
        assert_eq!((back.tabs.as_slice(), notice), (std::slice::from_ref(&saved), None));
        let has = |_: &str, _: &Path| false;
        let world = world(&[], &[], false, &has);
        assert_eq!(plan(&back.tabs[0], &world), Ok(Reopen::Git { title: "Git · repo".into(), folder: folder.clone() }));
        let gone = SavedTab::Git { title: "Git · x".into(), folder: PathBuf::from("C:/demo/gone") };
        assert!(plan(&gone, &world).unwrap_err().contains("is gone"));

        let old = serde_json::json!({ "kind": "git", "title": "Git · repo", "folder": folder, "widths": [200, 90, 300, 130, 70], "panel": 360 });
        assert_eq!(serde_json::from_value::<SavedTab>(old).unwrap(), saved, "a tab saved with its sizes loads without them");
    }

    #[test]
    fn each_kind_comes_back_as_its_setting_and_its_session_say() {
        let has: &dyn Fn(&str, &Path) -> bool = &|sid, _| sid == SID;
        let none: &dyn Fn(&str, &Path) -> bool = &|_, _| false;
        let ws = ["w".to_string()];
        let shell = SavedTab::Shell { title: "s".into(), cwd: "C:/a".into(), shell: None };
        assert_eq!(plan(&shell, &world(&[], &ws, false, has)), Ok(Reopen::Shell { title: "s".into(), cwd: "C:/a".into(), shell: None }));
        let gone = SavedTab::Shell { title: "s".into(), cwd: "C:/gone".into(), shell: None };
        assert!(plan(&gone, &world(&[], &ws, false, has)).unwrap_err().contains("folder"));

        let work = SavedTab::Work { title: "t".into(), workspace: "w".into() };
        assert_eq!(plan(&work, &world(&[], &ws, false, has)), Ok(Reopen::Work { workspace: "w".into() }));
        assert!(plan(&work, &world(&[], &[], false, has)).is_err(), "an unregistered workspace");

        let claude = SavedTab::Claude { title: "c".into(), cwd: "C:/b".into(), session_id: Some(SID.into()), name: "hand".into() };
        let Ok(Reopen::Resume { session, .. }) = plan(&claude, &world(&[], &ws, false, has)) else { panic!("resumes at once by default") };
        assert_eq!((session.session_id.as_str(), session.name.as_str(), session.kind), (SID, "hand", Kind::Interactive));
        assert_eq!(plan(&claude, &world(&[], &ws, true, has)), Ok(Reopen::Placeholder(claude.clone())));
        assert!(plan(&claude, &world(&[], &ws, false, none)).is_err(), "a transcript deleted since");
        let elsewhere = [Session { kind: Kind::Interactive, ..row(SessionState::Working, Some(9)) }];
        assert!(plan(&claude, &world(&elsewhere, &ws, false, has)).unwrap_err().contains("another terminal"), "never a second process");
        let short = SavedTab::Claude { title: "c".into(), cwd: "C:/b".into(), session_id: Some(SID[..8].into()), name: String::new() };
        assert!(plan(&short, &world(&[], &ws, false, &|_, _| true)).is_err(), "never a short id: it would start a new session");
        let fresh = SavedTab::Claude { title: "c".into(), cwd: "C:/b".into(), session_id: None, name: String::new() };
        assert!(plan(&fresh, &world(&[], &ws, false, has)).is_err());
    }

    #[test]
    fn a_hand_tab_follows_its_sessions_chain_and_attaches_to_a_running_continuation() {
        const NEXT: &str = "0a0b0c0d-0000-4000-8000-000000000002";
        let ws = ["w".to_string()];
        let saved = SavedTab::Claude { title: "c".into(), cwd: "C:/b".into(), session_id: Some(SID.into()), name: "workman".into() };
        let newest = |sid: &str| if sid == SID { NEXT.to_string() } else { sid.to_string() };
        let followed = saved.clone().followed(newest);
        assert_eq!(followed, SavedTab::Claude { title: "c".into(), cwd: "C:/b".into(), session_id: Some(NEXT.into()), name: "workman".into() });
        assert_eq!(followed.clone().followed(newest), followed);
        let shell = SavedTab::Shell { title: "s".into(), cwd: "C:/a".into(), shell: None };
        assert_eq!(shell.clone().followed(|_| NEXT.to_string()), shell, "only a hand tab's session follows");

        let continuation = Session { id: NEXT[..8].into(), session_id: NEXT.into(), ..row(SessionState::Blocked, Some(9)) };
        let none: &dyn Fn(&str, &Path) -> bool = &|_, _| false;
        assert_eq!(
            plan(&followed, &world(std::slice::from_ref(&continuation), &ws, false, none)),
            Ok(Reopen::Attach { title: "c".into(), session: continuation.clone() }),
            "the running background continuation is attached, never resumed a second time"
        );
        let has: &dyn Fn(&str, &Path) -> bool = &|sid, _| sid == NEXT;
        let Ok(Reopen::Resume { session, .. }) = plan(&followed, &world(&[], &ws, false, has)) else { panic!("a stopped continuation resumes") };
        assert_eq!(session.session_id, NEXT);
    }

    #[test]
    fn a_plain_tab_reopens_with_the_shell_it_had_and_an_old_file_with_the_default() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(path(home.path()), r#"{"tabs":[{"kind":"shell","title":"pwsh 1","cwd":"C:/a"}],"active":0}"#).unwrap();
        let (old, notice) = load(home.path());
        assert_eq!((old.tabs.as_slice(), notice), (&[SavedTab::Shell { title: "pwsh 1".into(), cwd: "C:/a".into(), shell: None }][..], None), "a file without the shell loads");
        let ws = ["w".to_string()];
        let has: &dyn Fn(&str, &Path) -> bool = &|_, _| false;
        assert_eq!(plan(&old.tabs[0], &world(&[], &ws, false, has)), Ok(Reopen::Shell { title: "pwsh 1".into(), cwd: "C:/a".into(), shell: None }));

        let cmd = SavedShell { program: r"C:\Windows\System32\cmd.exe".into(), args: vec!["/k".into(), r#""C:\VS 18\VsDevCmd.bat""#.into()] };
        let tabs = SavedTabs { tabs: vec![SavedTab::Shell { title: "cmd 4".into(), cwd: "C:/a".into(), shell: Some(cmd.clone()) }], active: None };
        save(home.path(), &tabs).unwrap();
        let (back, _) = load(home.path());
        assert_eq!(back, tabs);
        assert_eq!(plan(&back.tabs[0], &world(&[], &ws, false, has)), Ok(Reopen::Shell { title: "cmd 4".into(), cwd: "C:/a".into(), shell: Some(cmd) }));
        let uninstalled = SavedTab::Shell { title: "x 5".into(), cwd: "C:/a".into(), shell: Some(SavedShell { program: r"C:\x\gone.exe".into(), args: vec![] }) };
        assert_eq!(plan(&uninstalled, &world(&[], &ws, false, has)), Ok(Reopen::Shell { title: "x 5".into(), cwd: "C:/a".into(), shell: None }), "a shell no longer installed: the default");
    }

    #[test]
    fn an_attached_session_reattaches_only_while_it_runs_and_never_restarts_unasked() {
        let has: &dyn Fn(&str, &Path) -> bool = &|_, _| true;
        let live = [row(SessionState::Working, Some(7))];
        let Ok(Reopen::Attach { session, .. }) = plan(&attach("worker"), &world(&live, &[], false, has)) else { panic!("live: attach") };
        assert_eq!(session.id, &SID[..8]);
        let stopped = [row(SessionState::Stopped, None)];
        for sessions in [&stopped[..], &[]] {
            for on_click in [false, true] {
                assert_eq!(plan(&attach("worker"), &world(sessions, &[], on_click, has)), Ok(Reopen::Placeholder(attach("worker"))));
            }
        }
        let Reopen::Placeholder(saved) = plan(&attach("worker"), &world(&stopped, &[], false, has)).unwrap() else { unreachable!() };
        let session = saved.session().unwrap();
        assert_eq!((session.id.as_str(), session.session_id.as_str(), session.kind), (&SID[..8], SID, Kind::Background));
        assert!(plan(&attach("fm-alpha-workspace"), &world(&stopped, &[], false, has)).is_err(), "the supervisor's");
        assert!(plan(&attach("worker"), &world(&[], &[], false, &|_, _| false)).is_err(), "deleted");
    }
}
