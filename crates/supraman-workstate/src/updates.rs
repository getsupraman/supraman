//! Whether a newer signed release than this build is published: `GET <api>/repos/<repo>/releases/latest`,
//! its `supraman-update.json` verified against the release keys (docs/RELEASE.md, "Signing"), and what
//! the last check found, kept in `<home>/updates/`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use supraman_core::Settings;
use supraman_core::update::{MANIFEST_FILE, Manifest, Version, VerifyingKey, release_keys, verify};

use crate::http::{agent_following, read, server_reason};

pub const GITHUB_API: &str = "https://api.github.com";
pub const REPO: &str = "getsupraman/supraman";
/// Another API base than [`GITHUB_API`]: the selftest's fake releases endpoint.
pub const API_VAR: &str = "SUPRAMAN_UPDATE_API";
pub const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

const DIR: &str = "updates";
const STATE_FILE: &str = "state.json";
const DISMISSED_FILE: &str = "dismissed";
const TIMEOUT: Duration = Duration::from_secs(20);
const REDIRECTS: u32 = 5;

/// Where a check looks and what it trusts.
#[derive(Debug, Clone)]
pub struct Source {
    pub api: String,
    pub repo: String,
    /// `Err` is why there is none: no release can be trusted, so no request is sent.
    pub keys: Result<Vec<VerifyingKey>, String>,
    pub running: Version,
}

impl Source {
    /// GitHub (or [`API_VAR`]'s base) and the compiled-in release keys.
    pub fn github(running: Version) -> Self {
        let api = std::env::var(API_VAR).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| GITHUB_API.into());
        Self { api, repo: REPO.into(), keys: release_keys().map_err(|e| e.to_string()), running }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Available {
    pub manifest: Manifest,
    /// The release page.
    pub page_url: String,
    /// The manifest's `notes_url` when it is http(s), else the release page.
    pub notes_url: String,
    /// The release asset named by `manifest.zip`.
    pub zip_url: Option<String>,
}

impl Available {
    pub fn version(&self) -> &str {
        &self.manifest.version
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    UpToDate,
    Available(Available),
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    pub at_ms: i64,
    pub outcome: Outcome,
}

/// The latest release as last answered with a 200, kept for a 304 to that answer's ETag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Release {
    Verified(Available),
    Refused { reason: String },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    etag: Option<String>,
    release: Option<Release>,
    last: Option<Checked>,
}

fn dir(home: &Path) -> PathBuf {
    home.join(DIR)
}

fn load(home: &Path) -> State {
    std::fs::read(dir(home).join(STATE_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(home: &Path, state: &State) -> Result<(), String> {
    let path = dir(home).join(STATE_FILE);
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir(home)).and_then(|()| supraman_core::fs::write_atomic(&path, bytes)).map_err(|e| format!("{}: {e}", path.display()))
}

/// What the last check found, as kept in the home.
pub fn last(home: &Path) -> Option<Checked> {
    load(home).last
}

/// The version whose banner was dismissed.
pub fn dismissed(home: &Path) -> Option<Version> {
    Version::parse(std::fs::read_to_string(dir(home).join(DISMISSED_FILE)).ok()?.trim()).ok()
}

pub fn dismiss(home: &Path, version: &str) -> Result<(), String> {
    let path = dir(home).join(DISMISSED_FILE);
    std::fs::create_dir_all(dir(home)).and_then(|()| supraman_core::fs::write_atomic(&path, version)).map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether the banner for `available` shows: not when that version or a newer one was dismissed.
pub fn shows(available: &Available, dismissed: Option<&Version>) -> bool {
    dismissed.is_none_or(|d| available.manifest.semver() > *d)
}

/// The version an install runs: `version <semver>` from the `VERSION` file in `exe_dir`, else `crate_version`.
/// A local build's `<crate>-<yyyymmdd>-<sha>` counts as its crate version.
pub fn running_version(crate_version: &str, exe_dir: Option<&Path>) -> Version {
    let from_file = exe_dir
        .and_then(|d| std::fs::read_to_string(d.join("VERSION")).ok())
        .and_then(|text| text.trim_start_matches('\u{feff}').lines().find_map(|l| l.strip_prefix("version ").map(|v| v.trim().to_string())))
        .and_then(|v| Version::parse(&v).ok());
    let version = from_file.or_else(|| Version::parse(crate_version).ok()).unwrap_or_else(|| Version::new(0, 0, 0));
    let pre = version.pre.as_str().as_bytes();
    let local = pre.len() > 9 && pre[..8].iter().all(u8::is_ascii_digit) && pre[8] == b'-';
    if local { Version::new(version.major, version.minor, version.patch) } else { version }
}

/// [`check`], unless `[updates] check` is off: then nothing is sent and `None` comes back.
pub fn check_if_on(home: &Path, source: &Source) -> Option<Checked> {
    if !Settings::load(home).map_or(true, |s| s.updates.check) {
        return None;
    }
    Some(check(home, source))
}

/// One check against `source`, kept in the home with the answer's ETag.
pub fn check(home: &Path, source: &Source) -> Checked {
    let mut state = load(home);
    let outcome = match &source.keys {
        Err(why) => Outcome::Failed { reason: format!("no release key to verify a release with: {why}") },
        Ok(keys) => match latest(&mut state, source, keys) {
            Ok(release) => judge(release, &source.running),
            Err(reason) => Outcome::Failed { reason },
        },
    };
    let checked = Checked { at_ms: supraman_core::time::now_ms(), outcome };
    state.last = Some(checked.clone());
    if let Err(e) = save(home, &state) {
        return Checked { outcome: Outcome::Failed { reason: format!("cannot keep the update check: {e}") }, ..checked };
    }
    checked
}

fn judge(release: &Release, running: &Version) -> Outcome {
    match release {
        Release::Verified(a) if a.manifest.semver() > *running => Outcome::Available(a.clone()),
        Release::Verified(_) => Outcome::UpToDate,
        Release::Refused { reason } => Outcome::Failed { reason: reason.clone() },
    }
}

/// The latest release, from `state` on a 304; `Err` is a failure that keeps nothing.
fn latest<'s>(state: &'s mut State, source: &Source, keys: &[VerifyingKey]) -> Result<&'s Release, String> {
    let agent = agent_following(TIMEOUT, REDIRECTS);
    let url = format!("{}/repos/{}/releases/latest", source.api.trim_end_matches('/'), source.repo);
    let mut req = agent
        .get(&url)
        .header("User-Agent", format!("Supraman/{}", source.running))
        .header("Accept", "application/vnd.github+json");
    if let (Some(etag), Some(_)) = (&state.etag, &state.release) {
        req = req.header("If-None-Match", etag);
    }
    let reply = read(req.call()).map_err(|e| format!("cannot reach {url}: {e}"))?;
    match reply.status {
        304 => state.release.as_ref().ok_or_else(|| "an unchanged answer to a check that kept nothing".to_string()),
        200 => {
            let etag = reply.headers.get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
            let release = verified(&agent, &reply.body, source, keys)?;
            state.etag = etag;
            Ok(state.release.insert(release))
        }
        404 => Err("no release is published yet".into()),
        status => Err(format!("{url} answered {status}: {}", server_reason(&reply.body))),
    }
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

fn web(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// `Err` is a download that failed; a release that cannot be trusted is `Refused`.
fn verified(agent: &ureq::Agent, body: &str, source: &Source, keys: &[VerifyingKey]) -> Result<Release, String> {
    let refused = |reason: String| Ok(Release::Refused { reason });
    let gh: GhRelease = match serde_json::from_str(body) {
        Ok(gh) => gh,
        Err(e) => return refused(format!("the latest release is not a release: {e}")),
    };
    let tag = &gh.tag_name;
    if gh.draft || gh.prerelease {
        return refused(format!("release {tag} is a draft or a pre-release"));
    }
    let Some(version) = tag.strip_prefix('v').and_then(|v| Version::parse(v).ok()) else {
        return refused(format!("release {tag} is not tagged v<semver>"));
    };
    if !web(&gh.html_url) {
        return refused(format!("release {tag} has no web page"));
    }
    let asset = |name: &str| gh.assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.as_str()).filter(|u| web(u));
    let sig_name = format!("{MANIFEST_FILE}.sig");
    let (Some(manifest_url), Some(sig_url)) = (asset(MANIFEST_FILE), asset(&sig_name)) else {
        return refused(format!("release {tag} has no {MANIFEST_FILE} and {sig_name}"));
    };
    let download = |url: &str| -> Result<String, String> {
        let req = agent.get(url).header("User-Agent", format!("Supraman/{}", source.running)).header("Accept", "application/octet-stream");
        let reply = read(req.call()).map_err(|e| format!("cannot download {url}: {e}"))?;
        if reply.status != 200 {
            return Err(format!("{url} answered {}", reply.status));
        }
        Ok(reply.body)
    };
    let manifest_bytes = download(manifest_url)?;
    let sig_bytes = download(sig_url)?;
    let manifest = match verify(manifest_bytes.as_bytes(), sig_bytes.as_bytes(), keys) {
        Ok(m) => m,
        Err(e) => return refused(format!("release {tag} is not offered: {e}")),
    };
    if manifest.semver() != version {
        return refused(format!("release {tag} is not offered: its manifest is version {}", manifest.version));
    }
    let notes_url = manifest.notes_url.clone().filter(|u| web(u)).unwrap_or_else(|| gh.html_url.clone());
    let zip_url = asset(&manifest.zip).map(str::to_string);
    Ok(Release::Verified(Available { manifest, page_url: gh.html_url, notes_url, zip_url }))
}

#[cfg(test)]
mod tests {
    use supraman_core::update::{SigningKey, sign};

    use super::*;
    use crate::fake::{Answer, FakeWorkState};

    const LATEST: &str = "/repos/getsupraman/supraman/releases/latest";
    const MANIFEST_AT: &str = "/download/supraman-update.json";
    const SIG_AT: &str = "/download/supraman-update.json.sig";

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn manifest(version: &str) -> Manifest {
        Manifest {
            version: version.into(),
            commit: "c52996b".into(),
            zip: format!("Supraman-{version}.zip"),
            sha256: "ab".repeat(32),
            size: 9,
            published: "2026-11-02T10:00:00Z".into(),
            notes_url: Some("https://example.test/notes".into()),
        }
    }

    fn release(base: &str, tag: &str) -> String {
        serde_json::json!({
            "tag_name": tag,
            "html_url": format!("{base}/releases/{tag}"),
            "body": "notes",
            "draft": false,
            "prerelease": false,
            "assets": [
                {"name": "supraman-update.json", "browser_download_url": format!("{base}{MANIFEST_AT}")},
                {"name": "supraman-update.json.sig", "browser_download_url": format!("{base}{SIG_AT}")},
                {"name": format!("Supraman-{}.zip", tag.trim_start_matches('v')), "browser_download_url": format!("{base}/download/zip")},
            ],
        })
        .to_string()
    }

    /// A fake serving release `tag` with a manifest of `version` signed by `signer`.
    fn serve(tag: &str, version: &str, signer: &SigningKey) -> FakeWorkState {
        let fake = FakeWorkState::start(Vec::new());
        let base = fake.base().to_string();
        let bytes = manifest(version).to_bytes();
        fake.set(LATEST, Answer::json(release(&base, tag)).api(None).header("ETag", "\"e1\""));
        fake.set(MANIFEST_AT, Answer::text(String::from_utf8(bytes.clone()).unwrap()).api(None));
        fake.set(SIG_AT, Answer::text(String::from_utf8(sign(&bytes, signer)).unwrap()).api(None));
        fake
    }

    fn source(fake: &FakeWorkState, running: &str) -> Source {
        Source { api: fake.base().into(), repo: REPO.into(), keys: Ok(vec![key(1).verifying_key()]), running: Version::parse(running).unwrap() }
    }

    fn failed(outcome: &Outcome) -> &str {
        match outcome {
            Outcome::Failed { reason } => reason,
            other => panic!("not a failure: {other:?}"),
        }
    }

    #[test]
    fn a_newer_signed_release_is_available() {
        let home = tempfile::tempdir().unwrap();
        let fake = serve("v0.2.0", "0.2.0", &key(1));
        let checked = check(home.path(), &source(&fake, "0.1.0"));
        let Outcome::Available(a) = &checked.outcome else { panic!("{checked:?}") };
        assert_eq!(a.version(), "0.2.0");
        assert_eq!(a.notes_url, "https://example.test/notes");
        assert_eq!(a.page_url, format!("{}/releases/v0.2.0", fake.base()));
        assert_eq!(a.zip_url.as_deref(), Some(format!("{}/download/zip", fake.base()).as_str()));
        assert_eq!(last(home.path()), Some(checked));

        let seen = fake.seen();
        assert_eq!(seen.iter().map(|s| s.target.as_str()).collect::<Vec<_>>(), [LATEST, MANIFEST_AT, SIG_AT]);
        assert_eq!(seen[0].header("user-agent"), Some("Supraman/0.1.0"));
        assert_eq!(seen[0].header("accept"), Some("application/vnd.github+json"));
        assert_eq!(seen[0].header("if-none-match"), None);
    }

    #[test]
    fn an_asset_behind_a_redirect_is_followed() {
        let home = tempfile::tempdir().unwrap();
        let fake = serve("v0.2.0", "0.2.0", &key(1));
        let bytes = manifest("0.2.0").to_bytes();
        fake.set("/cdn/manifest", Answer::text(String::from_utf8(bytes).unwrap()).api(None));
        fake.set(MANIFEST_AT, Answer::status(302, "").api(None).header("Location", format!("{}/cdn/manifest", fake.base())));
        assert!(matches!(check(home.path(), &source(&fake, "0.1.0")).outcome, Outcome::Available(_)));
        assert!(fake.seen().iter().any(|s| s.target == "/cdn/manifest"));
    }

    #[test]
    fn the_same_or_an_older_release_is_up_to_date() {
        for running in ["0.2.0", "0.3.1"] {
            let home = tempfile::tempdir().unwrap();
            let fake = serve("v0.2.0", "0.2.0", &key(1));
            assert_eq!(check(home.path(), &source(&fake, running)).outcome, Outcome::UpToDate, "running {running}");
        }
    }

    #[test]
    fn a_manifest_of_another_version_than_the_tag_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let fake = serve("v0.2.0", "0.3.0", &key(1));
        let checked = check(home.path(), &source(&fake, "0.1.0"));
        assert!(failed(&checked.outcome).contains("its manifest is version 0.3.0"), "{checked:?}");
    }

    #[test]
    fn a_release_signed_by_another_key_is_not_offered() {
        let home = tempfile::tempdir().unwrap();
        let fake = serve("v0.2.0", "0.2.0", &key(2));
        let checked = check(home.path(), &source(&fake, "0.1.0"));
        assert!(failed(&checked.outcome).contains("not offered"), "{checked:?}");
    }

    #[test]
    fn a_draft_or_a_release_without_its_manifest_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let fake = serve("v0.2.0", "0.2.0", &key(1));
        let mut draft: serde_json::Value = serde_json::from_str(&release(fake.base(), "v0.2.0")).unwrap();
        draft["prerelease"] = true.into();
        fake.set(LATEST, Answer::json(draft.to_string()).api(None));
        assert!(failed(&check(home.path(), &source(&fake, "0.1.0")).outcome).contains("pre-release"));

        draft["prerelease"] = false.into();
        draft["assets"] = serde_json::json!([]);
        fake.set(LATEST, Answer::json(draft.to_string()).api(None));
        assert!(failed(&check(home.path(), &source(&fake, "0.1.0")).outcome).contains("has no supraman-update.json"));
    }

    #[test]
    fn an_unchanged_answer_reuses_the_kept_release_without_a_download() {
        let home = tempfile::tempdir().unwrap();
        let fake = serve("v0.2.0", "0.2.0", &key(1));
        check(home.path(), &source(&fake, "0.2.0"));
        fake.set(LATEST, Answer::status(304, "").api(None));
        let before = fake.seen().len();

        assert_eq!(check(home.path(), &source(&fake, "0.2.0")).outcome, Outcome::UpToDate);
        let Outcome::Available(a) = check(home.path(), &source(&fake, "0.1.0")).outcome else { panic!("the kept release is newer than 0.1.0") };
        assert_eq!(a.version(), "0.2.0");

        let seen = &fake.seen()[before..];
        assert_eq!(seen.iter().map(|s| s.target.as_str()).collect::<Vec<_>>(), [LATEST, LATEST]);
        assert!(seen.iter().all(|s| s.header("if-none-match") == Some("\"e1\"")), "{seen:?}");
    }

    #[test]
    fn checking_off_sends_no_request() {
        let home = tempfile::tempdir().unwrap();
        let fake = serve("v0.2.0", "0.2.0", &key(1));
        Settings::update(home.path(), |s| s.updates.check = false).unwrap();
        assert_eq!(check_if_on(home.path(), &source(&fake, "0.1.0")), None);
        assert_eq!(fake.seen().len(), 0);

        Settings::update(home.path(), |s| s.updates.check = true).unwrap();
        assert!(matches!(check_if_on(home.path(), &source(&fake, "0.1.0")).map(|c| c.outcome), Some(Outcome::Available(_))));
        assert_eq!(fake.seen().len(), 3);
    }

    #[test]
    fn no_release_key_sends_no_request() {
        let home = tempfile::tempdir().unwrap();
        let fake = serve("v0.2.0", "0.2.0", &key(1));
        let source = Source { keys: Err("the placeholder".into()), ..source(&fake, "0.1.0") };
        assert!(failed(&check(home.path(), &source).outcome).contains("the placeholder"));
        assert_eq!(fake.seen().len(), 0);
    }

    #[test]
    fn an_unreachable_server_is_a_failure() {
        let home = tempfile::tempdir().unwrap();
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let source = Source {
            api: format!("http://127.0.0.1:{closed}"),
            repo: REPO.into(),
            keys: Ok(vec![key(1).verifying_key()]),
            running: Version::new(0, 1, 0),
        };
        let checked = check(home.path(), &source);
        assert!(failed(&checked.outcome).starts_with("cannot reach"), "{checked:?}");
        assert_eq!(last(home.path()), Some(checked));
    }

    #[test]
    fn a_dismissed_version_hides_until_a_newer_one() {
        let home = tempfile::tempdir().unwrap();
        let fake = serve("v0.2.0", "0.2.0", &key(1));
        let Outcome::Available(a) = check(home.path(), &source(&fake, "0.1.0")).outcome else { panic!() };
        assert!(shows(&a, dismissed(home.path()).as_ref()));
        dismiss(home.path(), "0.2.0").unwrap();
        assert!(!shows(&a, dismissed(home.path()).as_ref()));
        let newer = Available { manifest: manifest("0.2.1"), ..a };
        assert!(shows(&newer, dismissed(home.path()).as_ref()));
    }

    #[test]
    fn the_running_version_is_the_install_s_else_the_crate_s() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(running_version("0.1.0", Some(dir.path())), Version::new(0, 1, 0));
        std::fs::write(dir.path().join("VERSION"), "\u{feff}version 0.4.2\r\ncommit abc\r\n").unwrap();
        assert_eq!(running_version("0.1.0", Some(dir.path())), Version::new(0, 4, 2));
        std::fs::write(dir.path().join("VERSION"), "version 0.1.0-20261102-c52996bdeadb-dirty\n").unwrap();
        assert_eq!(running_version("0.1.0", Some(dir.path())), Version::new(0, 1, 0));
    }
}
