//! `fm probe launch`: the m2/1 launcher probes, end to end, on throwaway `aw-m2-probe-*` sessions run
//! from `<cargo root>/target/aw-m2-probe-<hex>` (a plain subfolder of the trusted worktree).
//! Answers are recorded in `docs/LEARNINGS.md` (m2/1).

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use foreman_claude::{Claude, ClaudeError, Health, LaunchSpec, Session, SessionIdentity, now_ms, workstate_mcp_config};

const WORKSTATE_URL: &str = "http://127.0.0.1:5290/mcp";
const INVALID_TOKEN: &str = "fm-probe-invalid";
pub(crate) const ANSWER_WITHIN: Duration = Duration::from_secs(240);
pub(crate) const ROW_WITHIN: Duration = Duration::from_secs(15);

const PROMPT_A: &str = "This is an automated probe. Do exactly this and nothing else. \
1) Run the shell command that prints the environment variable FM_PROBE: in PowerShell `$env:FM_PROBE`, in Bash `echo \"$FM_PROBE\"`. \
2) Use the Write tool to create the file probe-a.txt in the current directory containing exactly one line: ENV=<the printed value, or EMPTY if nothing was printed>. \
Then stop.";

const PROMPT_B: &str = "This is an automated probe. Do exactly this and nothing else. \
1) List the names of every tool you have whose name starts with mcp__workstate__ (if tools are deferred, use ToolSearch with query \"workstate\" to find them). \
2) If mcp__workstate__work_brief exists, call it once with no arguments. \
3) Use the Write tool to create the file probe-b.txt in the current directory with exactly four lines: \
TOOLS=<comma-separated mcp__workstate__ tool names, or NONE> \
/ TOOL=<present if work_brief exists, else absent> \
/ RESULT=<ok if work_brief returned a brief, error if it failed, none if not called> \
/ TEXT=<the first 200 characters of the result or error text on one line; if work_brief does not exist, any message you were given about the workstate MCP server failing to connect; else none>. \
Then stop.";

const PROMPT_D: &str = "This is an automated probe. Without reading any file, from your memory of this conversation: \
what exact line did you write into probe-a.txt earlier? Use the Write tool to create the file probe-d.txt in the current \
directory containing exactly one line: PREV=<that line, or UNKNOWN if you do not remember writing it>. Then stop.";

const PROMPT_READY: &str = "This is an automated probe. Reply with the word ready and stop.";

pub fn launch() -> bool {
    let claude = Claude::default();
    let hex = hex6();
    let root = cargo_root().join("target").join(format!("aw-m2-probe-{hex}"));
    let mut fleet = Fleet {
        claude: claude.clone(),
        names: Vec::new(),
    };
    println!("scratch {}", root.display());
    let ok = match run(&claude, &mut fleet, &root, &hex) {
        Ok(ok) => ok,
        Err(e) => {
            println!("probe error: {e}");
            false
        }
    };
    drop(fleet);
    ok
}

fn run(claude: &Claude, fleet: &mut Fleet, root: &Path, hex: &str) -> Result<bool, String> {
    let nonce = format!("fmprobe{hex}{}", now_ms() % 1_000_000);

    // (a) env through --bg: a dummy http MCP server on a local port records what the session sends.
    let listener = Listener::start()?;
    let dir_a = mkdir(root, "a")?;
    let config_a = write(
        &dir_a,
        "probe-mcp.json",
        &serde_json::json!({ "mcpServers": { "fmprobe": {
            "type": "http",
            "url": format!("http://127.0.0.1:{}/mcp/${{FM_PROBE}}", listener.port),
            "headers": { "X-Fm-Probe": "${FM_PROBE}" }
        }}})
        .to_string(),
    )?;
    let mut spec_a = LaunchSpec::new(format!("aw-m2-probe-{hex}-a"), &dir_a, PROMPT_A);
    spec_a.mcp_config = Some(config_a);
    spec_a.strict_mcp_config = true;
    spec_a.env.push(("FM_PROBE".into(), nonce.clone()));

    // (b) precedence: a `workstate` server with an invalid token, without and with --strict-mcp-config.
    // b2 is the decided route exactly as Foreman will launch: an owner-only per-role config file.
    let dir_b1 = mkdir(root, "b1")?;
    let mut spec_b1 = LaunchSpec::new(format!("aw-m2-probe-{hex}-b1"), &dir_b1, PROMPT_B);
    spec_b1.mcp_config = Some(write(&dir_b1, "probe-mcp.json", &workstate_mcp_config(WORKSTATE_URL, INVALID_TOKEN, &[]))?);
    let dir_b2 = mkdir(root, "b2")?;
    let spec_b2 = LaunchSpec::new(format!("aw-m2-probe-{hex}-b2"), &dir_b2, PROMPT_B)
        .with_session_token(&dir_b2.join("mcp"), &SessionIdentity { agent: format!("aw-m2-probe-{hex}-b2"), ..SessionIdentity::default() }, WORKSTATE_URL, INVALID_TOKEN)
        .map_err(|e| format!("role config: {e}"))?;

    // (a2) the same, but the variable set through `--settings {"env":…}` instead of the process env.
    let listener2 = Listener::start()?;
    let dir_a2 = mkdir(root, "a2")?;
    let config_a2 = write(
        &dir_a2,
        "probe-mcp.json",
        &serde_json::json!({ "mcpServers": { "fmprobe": {
            "type": "http",
            "url": format!("http://127.0.0.1:{}/mcp/${{FM_PROBE}}", listener2.port),
            "headers": { "X-Fm-Probe": "${FM_PROBE}" }
        }}})
        .to_string(),
    )?;
    let mut spec_a2 = LaunchSpec::new(format!("aw-m2-probe-{hex}-a2"), &dir_a2, PROMPT_A.replace("probe-a.txt", "probe-a2.txt"));
    spec_a2.mcp_config = Some(config_a2);
    spec_a2.strict_mcp_config = true;
    spec_a2.extra_args = vec!["--settings".into(), serde_json::json!({ "env": { "FM_PROBE": nonce } }).to_string()];

    for spec in [&spec_a, &spec_a2, &spec_b1, &spec_b2] {
        fleet.launch(spec)?;
    }

    let a_env = report_a("a (FM_PROBE in the --bg process env)", &fleet, &spec_a, "probe-a.txt", &listener, &nonce);
    report_a("a2 (FM_PROBE via --settings {\"env\":…})", &fleet, &spec_a2, "probe-a2.txt", &listener2, &nonce);
    println!("probe a2: nonce persisted under ~/.claude/daemon: {}", persisted_in_daemon(&nonce));

    let b1 = fleet.answer(&spec_b1, "probe-b.txt");
    let b2 = fleet.answer(&spec_b2, "probe-b.txt");
    let b1_used = used_config(b1.as_deref());
    let b2_used = used_config(b2.as_deref());
    println!("probe b1 (--mcp-config, invalid token, no strict): {b1_used} [{}]", one_line(b1.as_deref()));
    println!("probe b2 (decided route: owner-only workstate-<role>.json + --strict-mcp-config): {b2_used} [{}]", one_line(b2.as_deref()));

    // (d) stop, then bring back with the transcript: `--bg --resume <sessionId>` with no other flag.
    let before = fleet.row(&spec_a.name)?;
    claude.stop(&before.id).map_err(|e| e.to_string())?;
    wait_until_stopped(claude, &spec_a.name)?;
    let stopped = fleet.row(&spec_a.name)?;
    let requests_before = listener.count();
    let mut spec_d = spec_a.clone();
    spec_d.prompt = PROMPT_D.into();
    let d_line = match claude.resume(&before.session_id, &spec_a.cwd, Some(PROMPT_D)) {
        Ok(out) => {
            println!("probe d: `claude --bg --resume <sessionId> -- <prompt>` said {:?}", first_lines(&out, 2));
            fleet.answer(&spec_d, "probe-d.txt")
        }
        Err(e) => {
            println!("probe d: `claude --bg --resume` failed: {e}");
            None
        }
    };
    let rows_after = claude.find_by_name(&spec_a.name, ROW_WITHIN).map_err(|e| e.to_string())?;
    let kept = match (&d_line, &a_env.line) {
        (Some(d), Some(a)) => d.trim() == format!("PREV={}", a.trim()),
        _ => false,
    };
    let same_id = rows_after.len() == 1 && rows_after[0].session_id == before.session_id;
    println!(
        "probe d (stop + resume): same sessionId={} | transcript kept={} | saved --mcp-config reused={} | before {} | stopped {} | after {} [answer {:?}]",
        yes(same_id),
        yes(kept),
        yes(listener.count() > requests_before),
        describe(&before),
        describe(&stopped),
        rows_after.iter().map(describe).collect::<Vec<_>>().join(" + "),
        d_line.as_deref().map(|l| l.replace(&nonce, "<nonce>")).unwrap_or_else(|| "none".into()),
    );

    // Names: Foreman's launch while a row with the name exists, the bare CLI likewise, then after rm.
    let spec_dup = LaunchSpec::new(spec_a.name.clone(), &dir_a, PROMPT_READY);
    let checked = match claude.launch(&spec_dup) {
        Err(ClaudeError::NameTaken { .. }) => "refused (NameTaken)".to_string(),
        other => format!("NOT refused: {other:?}"),
    };
    let cli = outcome(claude.launch_unchecked(&spec_dup));
    fleet.remove(&spec_a.name);
    let reuse = outcome(claude.launch(&spec_dup));
    let reuse_rows = claude.find_by_name(&spec_a.name, ROW_WITHIN).map_err(|e| e.to_string())?;
    println!(
        "probe d (names): while a row exists: Foreman launch {checked}, bare CLI {cli} | after rm: Foreman launch {reuse} [{} row(s), sessionId new={}]",
        reuse_rows.len(),
        yes(!reuse_rows.is_empty() && reuse_rows.iter().all(|r| r.session_id != before.session_id)),
    );

    let route = b2_used.starts_with("foreman's");
    println!(
        "route (owner-only <FOREMAN_HOME>/mcp/workstate-<role>.json, --mcp-config + --strict-mcp-config): {}",
        if route { "WORKS" } else { "FAILS" }
    );
    Ok(route)
}

fn outcome(result: Result<String, ClaudeError>) -> String {
    match result {
        Ok(out) => format!("accepted ({})", first_lines(&out, 1)),
        Err(e) => format!("refused ({e})"),
    }
}

pub(crate) fn first_lines(text: &str, n: usize) -> String {
    text.lines().take(n).collect::<Vec<_>>().join(" / ")
}

struct EnvSeen {
    tool: bool,
    header: bool,
    line: Option<String>,
}

fn report_a(label: &str, fleet: &Fleet, spec: &LaunchSpec, file: &str, listener: &Listener, nonce: &str) -> EnvSeen {
    let line = fleet.answer(spec, file);
    let requests = listener.wait_for_request(Duration::from_secs(30));
    let header = requests.iter().find_map(|r| header_value(r, "x-fm-probe"));
    let path = requests.first().and_then(|r| r.lines().next().map(str::to_string));
    let seen = EnvSeen {
        tool: line.as_deref().and_then(|l| l.strip_prefix("ENV=")).map(str::trim) == Some(nonce),
        header: header.as_deref() == Some(nonce),
        line,
    };
    let mask = |s: &str| s.replace(nonce, "<nonce>");
    println!(
        "probe {label}: tool sees it={} | --mcp-config header ${{FM_PROBE}} expanded={} | url expanded={} [{} request(s); first {:?}; header {:?}; tool answer {:?}]",
        yes(seen.tool),
        yes(seen.header),
        yes(path.as_deref().is_some_and(|p| p.contains(nonce))),
        requests.len(),
        path.as_deref().map(mask).unwrap_or_else(|| "none".into()),
        header.as_deref().map(mask).unwrap_or_else(|| "none".into()),
        seen.line.as_deref().map(mask).unwrap_or_else(|| "none".into()),
    );
    seen
}

/// File names (not contents) under `~/.claude/daemon` that contain `needle`.
fn persisted_in_daemon(needle: &str) -> String {
    let Some(home) = foreman_core::paths::user_home() else {
        return "unknown (no home dir)".into();
    };
    let mut hits = Vec::new();
    let mut stack = vec![home.join(".claude").join("daemon")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if fs::read(&path).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes())) {
                hits.push(path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default());
            }
        }
    }
    if hits.is_empty() { "no".into() } else { format!("yes, in {}", hits.join(", ")) }
}

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

    /// The first line of `file` in the session's cwd, once the session wrote it.
    pub(crate) fn answer(&self, spec: &LaunchSpec, file: &str) -> Option<String> {
        let path = spec.cwd.join(file);
        let deadline = Instant::now() + ANSWER_WITHIN;
        let mut checked = Instant::now();
        while Instant::now() < deadline {
            if let Ok(text) = fs::read_to_string(&path)
                && !text.trim().is_empty()
            {
                thread::sleep(Duration::from_millis(500));
                return fs::read_to_string(&path).ok().map(|t| t.trim().to_string());
            }
            if checked.elapsed() > Duration::from_secs(5) {
                checked = Instant::now();
                if let Ok(row) = self.row(&spec.name)
                    && let Health::Blocked { waiting_for } = row.health(now_ms())
                {
                    println!("  {} is blocked ({waiting_for:?}) — no answer", spec.name);
                    return None;
                }
            }
            thread::sleep(Duration::from_millis(500));
        }
        println!("  {} wrote no {file} within {}s", spec.name, ANSWER_WITHIN.as_secs());
        None
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
            let refusal = match self.claude.rm(&row.id) {
                Ok(out) => {
                    println!("cleanup: rm {} ({name}){}", row.id, one_line_tail(&out));
                    continue;
                }
                Err(e) => e.to_string(),
            };
            println!("cleanup: rm {} ({name}) refused{}", row.id, one_line_tail(&refusal));
            if let Some(token) = discard_token(&refusal) {
                let out = std::process::Command::new(&self.claude.program)
                    .args(["rm", &row.id, "--discard-unpushed", token])
                    .stdin(std::process::Stdio::null())
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        println!("cleanup: rm {} --discard-unpushed{}", row.id, one_line_tail(&String::from_utf8_lossy(&o.stdout)));
                        prune_empty(kept_worktree(&refusal).as_deref());
                        continue;
                    }
                    Ok(o) => println!("cleanup: rm --discard-unpushed failed{}", one_line_tail(&String::from_utf8_lossy(&o.stderr))),
                    Err(e) => println!("cleanup: rm --discard-unpushed failed: {e}"),
                }
            }
            let Some((repo, worktree)) = kept_worktree(&refusal).as_deref().and_then(claude_worktree) else { continue };
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
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
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

/// `<sha>@<token>` from rm's "… claude rm <id> --discard-unpushed <sha>@<token>".
fn discard_token(refusal: &str) -> Option<&str> {
    let rest = refusal.split_once("--discard-unpushed ")?.1;
    rest.split_whitespace().next().filter(|t| t.contains('@'))
}

/// The path in rm's "kept <id> — its worktree is still at “<path>”".
fn kept_worktree(refusal: &str) -> Option<PathBuf> {
    let rest = refusal.split_once("worktree is still at ")?.1;
    let rest = rest.trim_start_matches(['“', '"']);
    let end = rest.find(['”', '"'])?;
    Some(PathBuf::from(&rest[..end]))
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

/// The probe-b answer read as which config the session's `workstate` server came from.
fn used_config(answer: Option<&str>) -> String {
    let Some(text) = answer else {
        return "no answer".into();
    };
    let field = |k: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().to_string()))
            .unwrap_or_default()
    };
    match (field("TOOL=").as_str(), field("RESULT=").as_str()) {
        ("present", "ok") => "user scope (valid token: work_brief succeeded)".into(),
        ("present", "error") => "foreman's (invalid token: work_brief failed)".into(),
        ("absent", _) => "foreman's (invalid token: workstate server did not connect, no work_brief)".into(),
        (tool, result) => format!("unclear (TOOL={tool} RESULT={result})"),
    }
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

fn one_line(text: Option<&str>) -> String {
    text.map(|t| t.lines().map(str::trim).collect::<Vec<_>>().join(" / ")).unwrap_or_else(|| "none".into())
}

pub(crate) fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn header_value(request: &str, name: &str) -> Option<String> {
    request.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
    })
}

/// Records request heads; answers every request 404 so the client gives up quickly.
struct Listener {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Listener {
    fn start() -> Result<Self, String> {
        let socket = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind: {e}"))?;
        let port = socket.local_addr().map_err(|e| e.to_string())?.port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        thread::spawn(move || {
            for mut stream in socket.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                seen.lock().unwrap_or_else(|e| e.into_inner()).push(String::from_utf8_lossy(&buf).into_owned());
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
        Ok(Self { port, requests })
    }

    fn count(&self) -> usize {
        self.requests.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    fn wait_for_request(&self, within: Duration) -> Vec<String> {
        let deadline = Instant::now() + within;
        loop {
            let seen = self.requests.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if !seen.is_empty() || Instant::now() >= deadline {
                return seen;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
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
    fn reads_which_config_was_used() {
        assert!(used_config(Some("TOOLS=NONE\nTOOL=absent\nRESULT=none\nTEXT=none")).starts_with("foreman's"));
        assert!(used_config(Some("TOOLS=a\nTOOL=present\nRESULT=error\nTEXT=401")).starts_with("foreman's"));
        assert!(used_config(Some("TOOLS=a\nTOOL=present\nRESULT=ok\nTEXT=# brief")).starts_with("user scope"));
        assert_eq!(used_config(None), "no answer");
    }

    #[test]
    fn finds_the_claude_worktree_a_cwd_is_in() {
        let (repo, wt) = claude_worktree(Path::new(r"P:\r\AutoWork\.claude\worktrees\role-tags\target\x")).unwrap();
        assert_eq!(repo, PathBuf::from(r"P:\r\AutoWork"));
        assert_eq!(wt, PathBuf::from(r"P:\r\AutoWork\.claude\worktrees\role-tags"));
        assert_eq!(claude_worktree(Path::new(r"P:\r\AutoWork\target\x")), None);
        assert_eq!(claude_worktree(Path::new(r"P:\r\AutoWork\.claude\worktrees")), None);
    }

    /// `claude rm` on a session whose worktree has commits (m2/2, CLI 2.1.309), verbatim.
    const KEPT: &str = "claude failed (exit 1): kept 202c9998 — its worktree is still at “P:\\Projects\\LarchSys\\AutoWork\\.claude\\worktrees\\iso-probe”\n  39 unpushed commits on “worktree-iso-probe”: 3ccd48a “probe: add iso.txt” and 38 more. They exist on no remote, so deleting the worktree would lose them.\n  push them and run 'claude rm 202c9998' again, or discard the worktree and its commits: claude rm 202c9998 --discard-unpushed 3ccd48a1bb9d660d38c0fabd2f5e63bd64be737a@d71f54d85da4468315a1efd780272116";

    #[test]
    fn reads_rm_refusals() {
        assert_eq!(
            discard_token(KEPT),
            Some("3ccd48a1bb9d660d38c0fabd2f5e63bd64be737a@d71f54d85da4468315a1efd780272116")
        );
        assert_eq!(
            kept_worktree(KEPT),
            Some(PathBuf::from(r"P:\Projects\LarchSys\AutoWork\.claude\worktrees\iso-probe"))
        );
        assert_eq!(discard_token("removed 202c9998"), None);
        assert_eq!(kept_worktree("removed 202c9998"), None);
    }

    #[test]
    fn header_lookup_ignores_case() {
        let req = "POST /mcp/x HTTP/1.1\r\nHost: 127.0.0.1\r\nx-FM-probe: abc\r\n\r\n";
        assert_eq!(header_value(req, "x-fm-probe").as_deref(), Some("abc"));
        assert_eq!(header_value(req, "authorization"), None);
    }
}
