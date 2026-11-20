//! `supraman-git.png`: the README's Git tab, on a shop's repo with a few days of work, its sessions as agent rows.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use gpui::{AsyncWindowContext, Entity};
use serde_json::json;
use supraman_core::folders::Place;
use supraman_gitman::{Git, fixture};

use super::{GROUP, capture, home, mv, root, sleep};
use crate::view::FleetView;

const DANA: (&str, &str) = ("Dana Ortiz", "dana.ortiz@example.com");
const SAM: (&str, &str) = ("Sam Lee", "sam.lee@example.com");
const PRIYA: (&str, &str) = ("Priya Shah", "priya.shah@example.com");

/// The sessions on the shop's checkouts: id, name, in the linked worktree, state, status, waiting for, minutes running.
const SESSIONS: [(&str, &str, bool, &str, &str, Option<&str>, i64); 3] = [
    ("e0000001", "checkout-api", false, "working", "busy", None, 34),
    ("e0000002", "review", false, "blocked", "waiting", Some("permission prompt"), 12),
    ("e0000003", "login-fix", true, "working", "idle", None, 51),
];

fn shop() -> PathBuf {
    root().join("shop")
}

fn linked() -> PathBuf {
    root().join("shop.worktrees").join("login-redirect")
}

struct Shop {
    git: Git,
    dir: PathBuf,
}

impl Shop {
    fn git(&self, args: &[&str]) -> String {
        self.as_who(DANA, "2026-10-27 09:00:00 +0100", args)
    }

    fn as_who(&self, who: (&str, &str), at: &str, args: &[&str]) -> String {
        let env = [
            ("GIT_AUTHOR_NAME", who.0.to_string()),
            ("GIT_AUTHOR_EMAIL", who.1.to_string()),
            ("GIT_COMMITTER_NAME", who.0.to_string()),
            ("GIT_COMMITTER_EMAIL", who.1.to_string()),
            ("GIT_AUTHOR_DATE", at.to_string()),
            ("GIT_COMMITTER_DATE", at.to_string()),
        ];
        fixture::git_in(&self.git, &self.dir, args, &env, None)
    }

    fn write(&self, files: &[(&str, &str)]) {
        for (path, text) in files {
            let file = self.dir.join(path);
            std::fs::create_dir_all(file.parent().expect("a file in the repo")).expect("the file's folder");
            std::fs::write(file, text).expect("the file writes");
        }
    }

    fn commit(&self, who: (&str, &str), at: &str, message: &str, files: &[(&str, &str)]) -> String {
        self.write(files);
        self.git(&["add", "-A"]);
        self.as_who(who, at, &["commit", "-q", "--no-verify", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn merge(&self, who: (&str, &str), at: &str, branch: &str) -> String {
        self.as_who(who, at, &["merge", "-q", "--no-ff", "--no-verify", "-m", &format!("Merge branch '{branch}'"), branch]);
        self.git(&["branch", "-q", "-d", branch]);
        self.git(&["rev-parse", "HEAD"])
    }
}

/// The shop's repo, its `origin` and its linked worktree; the merge of the search filters, the commit the shot selects.
fn build() -> String {
    let git = fixture::system_git();
    let dir = shop();
    std::fs::create_dir_all(&dir).expect("the shop's folder");
    fixture::git_in(&git, &dir, &["init", "-q", "-b", "main"], &[], None);
    let s = Shop { git, dir };
    for (key, value) in [("core.autocrlf", "false"), ("commit.gpgsign", "false"), ("tag.gpgsign", "false")] {
        s.git(&["config", key, value]);
    }
    let merged = history(&s);
    remote(&s);
    s.git(&["worktree", "add", "-q", &linked().to_string_lossy(), "fix/login-redirect"]);
    std::fs::write(linked().join("src/auth/redirect.ts"), REDIRECT_WIP).expect("the worktree's change");
    s.write(&[("README.md", README_WIP), ("src/payments/webhooks.ts", WEBHOOKS_WIP), ("src/payments/retry.ts", "export const MAX_RETRIES = 1;\n")]);
    merged
}

fn history(s: &Shop) -> String {
    s.commit(DANA, "2026-10-27 09:12:00 +0100", "Initial storefront skeleton", &[("README.md", "# Shop\n\nThe storefront and its API.\n"), ("package.json", "{ \"name\": \"shop\", \"version\": \"1.3.0\" }\n"), ("src/app.ts", "export const app = {};\n")]);
    s.commit(SAM, "2026-10-27 14:30:00 +0100", "Add product catalogue page", &[("src/catalogue/page.ts", "export function page() {}\n")]);
    s.commit(DANA, "2026-10-28 10:05:00 +0100", "Add cart store with quantity limits", &[("src/cart/store.ts", "export const MAX_QUANTITY = 10;\n")]);
    s.git(&["checkout", "-q", "-b", "feature/cart-totals"]);
    s.commit(DANA, "2026-10-28 15:20:00 +0100", "Compute cart totals with tax", &[("src/cart/totals.ts", "export function totals() {}\n")]);
    s.commit(DANA, "2026-10-29 09:40:00 +0100", "Show totals in the cart drawer", &[("src/cart/drawer.ts", "export function drawer() {}\n")]);
    s.git(&["checkout", "-q", "main"]);
    s.commit(SAM, "2026-10-29 11:15:00 +0100", "Fix price rounding in the catalogue", &[("src/catalogue/price.ts", "export const round = (n: number) => Math.round(n * 100) / 100;\n")]);
    s.merge(DANA, "2026-10-29 16:02:00 +0100", "feature/cart-totals");
    s.commit(SAM, "2026-10-30 10:00:00 +0100", "Bump version to 1.4.0", &[("package.json", "{ \"name\": \"shop\", \"version\": \"1.4.0\" }\n")]);
    s.as_who(SAM, "2026-10-30 10:01:00 +0100", &["tag", "-a", "v1.4.0", "-m", "Release 1.4.0"]);
    s.git(&["checkout", "-q", "-b", "feature/search-filters"]);
    s.commit(SAM, "2026-10-30 13:45:00 +0100", "Add price and brand filters", &[("src/search/filters.ts", "export type Filter = { price?: [number, number]; brand?: string };\n"), ("tests/filters.test.ts", "test('filters', () => {});\n")]);
    s.commit(SAM, "2026-10-30 17:10:00 +0100", "Keep filters in the URL", &[("src/search/url.ts", "export function toQuery() {}\n")]);
    s.git(&["checkout", "-q", "main"]);
    s.commit(DANA, "2026-10-31 09:30:00 +0100", "Update payment SDK to 3.2", &[("src/payments/sdk.ts", "export const SDK_VERSION = '3.2';\n")]);
    let merged = s.merge(SAM, "2026-10-31 11:20:00 +0100", "feature/search-filters");
    s.git(&["checkout", "-q", "-b", "fix/login-redirect"]);
    s.commit(SAM, "2026-11-01 14:10:00 +0100", "Validate redirect URL after login", &[("src/auth/redirect.ts", "export function safeRedirect(url: string) { return url.startsWith('/') ? url : '/'; }\n")]);
    s.commit(SAM, "2026-11-02 09:15:00 +0100", "Add tests for open redirects", &[("tests/redirect.test.ts", "test('rejects other hosts', () => {});\n")]);
    s.git(&["checkout", "-q", "main"]);
    s.git(&["checkout", "-q", "-b", "feature/checkout"]);
    s.commit(DANA, "2026-10-31 15:00:00 +0100", "Add checkout route and empty form", &[("src/checkout/route.ts", "export const route = '/checkout';\n")]);
    s.commit(DANA, "2026-11-01 10:20:00 +0100", "Add card payment form", &[("src/checkout/card.ts", "export function cardForm() {}\n")]);
    s.write(&[("src/checkout/address.ts", "export function autocomplete() {}\n")]);
    s.git(&["add", "-A"]);
    s.as_who(DANA, "2026-11-01 12:40:00 +0100", &["stash", "push", "-q", "-m", "Address autocomplete, half done"]);
    s.commit(DANA, "2026-11-02 08:50:00 +0100", "Validate card number and expiry", &[("src/checkout/validate.ts", "export function luhn(n: string) { return n.length > 0; }\n")]);
    s.git(&["checkout", "-q", "main"]);
    s.commit(SAM, "2026-11-01 16:30:00 +0100", "Log failed payment webhooks", &[("src/payments/webhooks.ts", "export function onFailed() { console.warn('payment failed'); }\n")]);
    s.write(&[("src/theme.css", ":root { --header: #f4f1ea; }\n")]);
    s.git(&["add", "-A"]);
    s.as_who(SAM, "2026-11-01 17:05:00 +0100", &["stash", "push", "-q", "-m", "Try a lighter header colour"]);
    s.commit(DANA, "2026-11-02 09:35:00 +0100", "Document the release process", &[("docs/RELEASING.md", "# Releasing\n\nTag `vX.Y.Z` on main, then publish.\n")]);
    merged
}

/// `origin`: a bare clone whose `main` holds a commit this checkout lacks and is one behind it, and `feature/checkout` one commit behind.
fn remote(s: &Shop) {
    let at_main = s.git(&["rev-parse", "main~1"]);
    s.git(&["checkout", "-q", "--detach", &at_main]);
    let theirs = s.commit(PRIYA, "2026-11-02 08:20:00 +0100", "Raise API timeout to 30 s", &[("src/api/timeout.ts", "export const TIMEOUT_MS = 30_000;\n")]);
    s.git(&["checkout", "-q", "main"]);
    let bare = root().join("origin").join("shop.git");
    std::fs::create_dir_all(bare.parent().expect("origin's folder")).expect("origin's folder");
    s.git(&["clone", "-q", "--bare", ".", &bare.to_string_lossy()]);
    let checkout = s.git(&["rev-parse", "feature/checkout~1"]);
    let origin = Shop { git: s.git.clone(), dir: bare.clone() };
    origin.git(&["update-ref", "refs/heads/main", &theirs]);
    origin.git(&["update-ref", "refs/heads/feature/checkout", &checkout]);
    origin.git(&["update-ref", "-d", "refs/heads/fix/login-redirect"]);
    s.git(&["remote", "add", "origin", &bare.to_string_lossy()]);
    s.git(&["update-ref", "refs/remotes/origin/main", &theirs]);
    s.git(&["update-ref", "refs/remotes/origin/feature/checkout", &checkout]);
    s.git(&["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    s.git(&["branch", "-q", "-u", "origin/main", "main"]);
    s.git(&["branch", "-q", "-u", "origin/feature/checkout", "feature/checkout"]);
}

const README_WIP: &str = "# Shop\n\nThe storefront and its API.\n\nFailed card payments are retried once.\n";
const WEBHOOKS_WIP: &str = "import { MAX_RETRIES } from './retry';\n\nexport function onFailed(attempt: number) {\n  if (attempt < MAX_RETRIES) return 'retry';\n  console.warn('payment failed');\n}\n";
const REDIRECT_WIP: &str = "export function safeRedirect(url: string) {\n  return url.startsWith('/') && !url.startsWith('//') ? url : '/';\n}\n";

fn sessions() -> anyhow::Result<Vec<supraman_claude::Session>> {
    let now = supraman_claude::now_ms();
    let rows: Vec<_> = SESSIONS
        .iter()
        .map(|(id, name, side, state, status, waiting, age)| {
            let mut row = json!({ "id": id, "sessionId": format!("{id}-0000-4000-8000-00000000de00"), "name": name, "cwd": if *side { linked() } else { shop() },
                                  "kind": "background", "startedAt": now - age * 60_000, "state": state, "status": status, "pid": std::process::id() });
            if let Some(w) = waiting {
                row["waitingFor"] = json!(w);
            }
            row
        })
        .collect();
    Ok(supraman_claude::parse(&serde_json::Value::Array(rows).to_string())?)
}

/// The shop in the demo group, its sessions on the WIP row and the worktree's row, the ref panel shut so the messages show, the search filters' merge selected.
pub(super) async fn shot(view: &Entity<FleetView>, out: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    let merged = cx.background_executor().spawn(async { build() }).await;
    let dir = shop();
    let folder = dir.clone();
    cx.background_executor()
        .spawn(async move { supraman_core::folders::edit(&home(), |e| e.add_folder(&folder, &Place::Group { name: GROUP.into(), index: usize::MAX })) })
        .await
        .context("adding the shop to the demo group")?;
    let injected = sessions()?;
    view.read_with(cx, |v, _| v.polls.injected.lock().unwrap_or_else(|e| e.into_inner()).extend(injected));
    let tab = view.update(cx, |v, cx| v.open_git(dir.clone(), crate::view::git_tab::title(&dir), cx));
    let git = crate::selftest::until_app(view, cx, Duration::from_secs(20), "the shop's Git tab", |v, cx| crate::selftest_git::git_tabs(v, cx, &dir).first().map(|(_, g)| g.clone())).await?;
    crate::selftest::until(view, cx, Duration::from_secs(20), "the three sessions' agent rows and the worktree's row", |v| {
        let hits = v.hits.borrow();
        let agents = hits.keys().filter(|k| k.strip_prefix("git-agent-").is_some_and(|n| n.parse::<usize>().is_ok())).count();
        (agents >= SESSIONS.len() && hits.keys().any(|k| k.starts_with("git-worktree-ref-"))).then_some(())
    })
    .await?;
    crate::selftest::click(view, "refpanel-collapse", cx).await?;
    crate::selftest::until(view, cx, Duration::from_secs(10), "the ref panel shut to its strip", |v| v.hits.borrow().contains_key("refpanel-expand").then_some(())).await?;
    let row = crate::selftest_git::until_probe(view, &git, "the merge's row", cx, |p, _| crate::selftest_git::row_key(p, &merged).ok()).await?;
    crate::selftest_git::tap(view, &row, cx).await?;
    crate::selftest_git::until_probe(view, &git, "the merge in the commit panel", cx, |p, v| {
        (p.commit.as_ref().is_some_and(|(sha, _)| *sha == merged) && v.hits.borrow().contains_key("commit-sha")).then_some(())
    })
    .await?;
    let away = cx.update(|window, _| window.viewport_size())?;
    mv(gpui::point(away.width - gpui::px(4.), away.height - gpui::px(4.)), cx)?;
    sleep(cx, Duration::from_millis(800)).await;
    let shot = capture(out.join("supraman-git.png"), None, cx).await?;
    println!("shot: {}", shot.display());
    leave(view, tab, &dir, cx).await
}

/// The tab closed, the sessions gone and the shop out of the group, so the later shots show the tree as before.
async fn leave(view: &Entity<FleetView>, tab: u64, dir: &Path, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    crate::selftest::click(view, "refpanel-expand", cx).await?;
    crate::selftest::until(view, cx, Duration::from_secs(10), "the ref panel open again", |v| v.hits.borrow().contains_key("refpanel-collapse").then_some(())).await?;
    view.update(cx, |v, cx| {
        v.close_tab(tab, cx);
        v.hits.borrow_mut().retain(|k, _| !k.starts_with("git-"));
    });
    let ids: Vec<String> = SESSIONS.iter().map(|(id, ..)| format!("{id}-0000-4000-8000-00000000de00")).collect();
    view.read_with(cx, |v, _| v.polls.injected.lock().unwrap_or_else(|e| e.into_inner()).retain(|s| !ids.contains(&s.session_id)));
    let pos = view.read_with(cx, |v, _| v.tree().folders().find(|(_, f)| f.path == dir && f.listed()).and_then(|(_, f)| f.pos)).ok_or_else(|| anyhow!("no shop row"))?;
    cx.background_executor().spawn(async move { supraman_core::folders::edit(&home(), |e| e.remove_folder(pos)) }).await.context("removing the shop from the demo group")?;
    view.update(cx, |v, _| v.polls.poll_nudge.wake());
    crate::selftest::until(view, cx, Duration::from_secs(15), "the shop's row gone", |v| v.tree().folders().all(|(_, f)| f.path != shop()).then_some(())).await
}
