//! `--selftest-shots`: the README's screenshots, from fixture sessions only (no `claude agents` listing).

use std::{
    path::{Path, PathBuf},
    sync::{OnceLock, atomic::{AtomicBool, Ordering}},
    time::Duration,
};

use anyhow::{Context as _, anyhow, bail};
use supraman_core::{Folder, FolderKind, Registry, Settings, Workspace};
use supraman_workman::fake::{Answer, FakeWorkman};
use gpui::{AsyncWindowContext, Entity};
use serde_json::{Value, json};

use crate::{
    selftest::{home, remove_dir_retrying, sleep},
    selftest_manage::capture,
    view::{FleetView, MenuTarget},
};

pub const WORKSPACE: &str = "demo";
const ROOT: &str = r"C:\src\demo";
const FOLDERS: [(&str, &str); 4] = [("api", "main"), ("web", "feature/login"), ("docs", "main"), ("infra", "main")];

const PROJECT: &str = "de000000-0000-4000-8000-000000000001";
const RUN: &str = "de000000-0000-4000-8000-0000000000a1";

static FIXTURES_ONLY: AtomicBool = AtomicBool::new(false);
static FAKE: OnceLock<FakeWorkman> = OnceLock::new();
static OAUTH: OnceLock<std::sync::Arc<supraman_workman::fake::FakeAuth>> = OnceLock::new();

/// The session list holds the injected rows alone.
pub(crate) fn fixtures_only() -> bool {
    FIXTURES_ONLY.load(Ordering::SeqCst)
}

fn out_dir() -> anyhow::Result<PathBuf> {
    std::env::var_os("SUPRAMAN_SHOTS").map(PathBuf::from).ok_or_else(|| anyhow!("--selftest-shots writes to SUPRAMAN_SHOTS, which is unset"))
}

pub fn prepare() -> anyhow::Result<()> {
    std::fs::create_dir_all(out_dir()?)?;
    FIXTURES_ONLY.store(true, Ordering::SeqCst);
    crate::selftest::install_guards(crate::Mode::SelftestShots);
    let home = home();
    remove_dir_retrying(&home).with_context(|| format!("cannot clear {}", home.display()))?;
    std::fs::create_dir_all(&home)?;
    unsafe { std::env::set_var(supraman_core::HOME_VAR, &home) };
    let folders = FOLDERS
        .iter()
        .map(|(name, branch)| Folder {
            name: (*name).to_string(),
            path: Path::new(ROOT).join(name),
            kind: FolderKind::Git,
            has_claude_md: true,
            has_agents_md: false,
            branch: Some((*branch).to_string()),
            enabled: true,
            added: true,
        })
        .collect();
    Registry::from_env()?.save(&Workspace::new(WORKSPACE, ROOT, folders), true)?;
    let fake = workman();
    OAUTH.set(fake.enable_oauth()).map_err(|_| anyhow!("the fake OAuth server was started twice"))?;
    let mut settings = Settings::load(&home)?;
    settings.workman.url = Some(fake.base().to_string());
    settings.save(&home)?;
    crate::signin::sign_in(&home, fake.base()).map_err(|e| anyhow!("signing in to the fake Workman: {e}"))?;
    println!(
        "shots: SUPRAMAN_HOME={} ; workspace {WORKSPACE} at {ROOT} (fixture folders, not on disk); fake Workman at {}",
        home.display(),
        fake.base()
    );
    FAKE.set(fake).map_err(|_| anyhow!("the fake Workman was started twice"))
}

fn item(id: &str, qref: &str, kind: &str, title: &str, status: &str) -> Value {
    let ref_ = qref.rsplit('/').next().unwrap_or(qref);
    json!({ "id": id, "ref": ref_, "qualifiedRef": qref, "kind": kind, "title": title, "status": status })
}

fn workman() -> FakeWorkman {
    let project = item(PROJECT, WORKSPACE, "project", "Demo shop", "in_progress");
    let run = item(RUN, "checkout", "run", "Checkout flow", "in_progress");
    let task = |n: &str, title: &str, status: &str| item(&format!("de000000-0000-4000-8000-00000000020{n}"), &format!("checkout/{n}"), "task", title, status);
    let rows = vec![
        json!({ "key": "question:t1", "kind": "question", "item": task("1", "Payment provider adapter", "in_progress"), "run": run,
                "what": "Card or invoice first?", "since": "2026-11-02T09:40:00+00:00", "call": "" }),
        json!({ "key": "blocked:t2", "kind": "blocked", "item": task("2", "Login form validation", "blocked"), "run": run,
                "what": "Blocked: waits for the session API", "since": "2026-11-02T09:10:00+00:00", "call": "" }),
    ];
    let resolved = json!({ "asOf": "2026-11-02T10:00:00+00:00", "seq": 10, "matches": [project] }).to_string();
    let inbox = json!({ "asOf": "2026-11-02T10:00:00+00:00", "seq": 10, "rows": rows }).to_string();
    FakeWorkman::start(vec![
        ("/health".into(), Answer::text("Healthy")),
        (format!("/api/v1/resolve?ref={WORKSPACE}"), Answer::json(resolved)),
        (format!("/api/v1/inbox?project={PROJECT}"), Answer::json(inbox)),
    ])
}

fn rows() -> anyhow::Result<Vec<supraman_claude::Session>> {
    let now = supraman_claude::now_ms();
    let pid = std::process::id();
    let row = |id: &str, name: &str, folder: Option<&str>, kind: &str, state: &str, status: &str, waiting: Option<&str>, age_min: i64| {
        let cwd = folder.map_or_else(|| PathBuf::from(ROOT), |f| Path::new(ROOT).join(f));
        let mut v = serde_json::json!({
            "id": id,
            "sessionId": format!("{id}-0000-4000-8000-00000000demo"),
            "name": name,
            "cwd": cwd,
            "kind": kind,
            "startedAt": now - age_min * 60_000,
            "state": state,
            "status": status,
            "pid": pid,
        });
        if let Some(w) = waiting {
            v["waitingFor"] = w.into();
        }
        v
    };
    let json = serde_json::Value::Array(vec![
        row("d0000001", "fm-demo", None, "background", "working", "busy", None, 95),
        row("d0000002", "fm-demo-api", Some("api"), "background", "working", "busy", None, 42),
        row("d0000003", "fm-demo-web", Some("web"), "background", "blocked", "waiting", Some("permission prompt"), 17),
        row("d0000004", "fm-demo-docs", Some("docs"), "background", "working", "idle", None, 63),
        row("d0000005", "refactor-auth", Some("api"), "interactive", "working", "busy", None, 8),
        row("d0000006", "fm-demo-infra", Some("infra"), "background", "stopped", "idle", None, 240),
    ]);
    Ok(supraman_claude::parse(&json.to_string())?)
}

pub async fn run(view: &Entity<FleetView>, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let out = out_dir()?;
    let fixtures = rows()?;
    let count = fixtures.len();
    view.read_with(cx, |v, _| v.injected.lock().unwrap_or_else(|e| e.into_inner()).extend(fixtures));
    crate::selftest::until(view, cx, Duration::from_secs(20), "the fixture sessions in the tree", |v| {
        let tree = v.tree();
        let node = tree.workspaces.iter().find(|n| n.name == WORKSPACE)?;
        (node.rows().count() >= count).then_some(())
    })
    .await?;
    sleep(cx, Duration::from_millis(1500)).await;
    let main = capture(out.join("supraman-main.png"), None, cx).await?;
    println!("shot: {}", main.display());

    let target = MenuTarget::Session {
        workspace: WORKSPACE.to_string(),
        repo: "api".to_string(),
        session_id: "d0000002-0000-4000-8000-00000000demo".to_string(),
    };
    let tab = view.update(cx, |v, cx| v.open_details(target, cx));
    if tab.is_none() {
        bail!("no Details tab for fm-demo-api");
    }
    sleep(cx, Duration::from_millis(1500)).await;
    let details = capture(out.join("supraman-details.png"), None, cx).await?;
    println!("shot: {}", details.display());
    Ok(())
}
