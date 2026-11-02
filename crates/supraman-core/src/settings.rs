use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::{CoreError, Result};

pub const DEFAULT_MAX_WORKING: usize = 20;
/// `[workstate] url` when unset.
pub const DEFAULT_WORKSTATE_URL: &str = "https://workstate.dikaia.ai";
/// `[terminal] font_size` when unset or invalid, and Ctrl+0's size.
pub const DEFAULT_TERMINAL_FONT_SIZE: u32 = 14;
/// The bounds `[terminal] font_size` is clamped to; `supraman_ui::term_element`'s MIN/MAX_FONT_SIZE.
pub const MIN_TERMINAL_FONT_SIZE: u32 = 6;
pub const MAX_TERMINAL_FONT_SIZE: u32 = 72;
/// The terminal font when `[terminal] font_family` is unset or not installed: the first one installed.
pub const TERMINAL_FONT_PREFERENCE: [&str; 3] = ["Cascadia Code", "Cascadia Mono", "Consolas"];

/// A table of `settings.toml` Foreman owns.
pub trait Section: Serialize + DeserializeOwned + Default {
    const KEY: &'static str;
    const FIELDS: &'static [&'static str];
}

/// `[sessions]`: what the supervisor launches and how many may work at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSettings {
    /// Most of this home's `fm-` sessions mid-turn at once, across its workspaces.
    pub max_working: usize,
    /// More plugin dirs every launch gets, as `--plugin-dir` after Foreman's own (e.g. the rlarch plugin
    /// whose `/rlarch:controller` a repo-controller runs). Edited in the file; the app only shows them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_plugin_dirs: Vec<PathBuf>,
    /// Start elevated (one UAC prompt per start), so Foreman reaches a claude background service an
    /// elevated `claude --bg` started.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub run_as_admin: bool,
    /// How long a workspace's relay stays wanted after its last fallback send. Unset: 10 min.
    /// Edited in the file; the selftest shortens it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_idle_secs: Option<u64>,
    /// Reopen the tabs open at the last close when Foreman starts.
    #[serde(skip_serializing_if = "is_true")]
    pub reopen_tabs: bool,
    /// Reopened claude tabs wait for a click on Resume instead of resuming at once.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resume_tabs_on_click: bool,
}

fn is_true(b: &bool) -> bool {
    *b
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            max_working: DEFAULT_MAX_WORKING,
            extra_plugin_dirs: Vec::new(),
            run_as_admin: false,
            relay_idle_secs: None,
            reopen_tabs: true,
            resume_tabs_on_click: false,
        }
    }
}

impl Section for SessionSettings {
    const KEY: &'static str = "sessions";
    const FIELDS: &'static [&'static str] =
        &["max_working", "extra_plugin_dirs", "run_as_admin", "relay_idle_secs", "reopen_tabs", "resume_tabs_on_click"];
}

impl SessionSettings {
    pub fn relay_idle_ms(&self) -> i64 {
        self.relay_idle_secs.map_or(crate::DEFAULT_RELAY_IDLE_MS, |s| s.saturating_mul(1000).min(i64::MAX as u64) as i64)
    }
}

/// `[workstate]`: the server Foreman signs in to. No token is ever kept here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkStateSettings {
    /// An origin like `https://workstate.example`; any path is dropped. Unset: [`DEFAULT_WORKSTATE_URL`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Section for WorkStateSettings {
    const KEY: &'static str = "workstate";
    const FIELDS: &'static [&'static str] = &["url"];
}

impl WorkStateSettings {
    /// The server's origin: `url` without any path, else [`DEFAULT_WORKSTATE_URL`].
    pub fn origin(&self) -> String {
        crate::url::origin(self.url.as_deref().unwrap_or(DEFAULT_WORKSTATE_URL))
    }

    /// The MCP endpoint a minted session's config names.
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.origin())
    }
}

/// `[appearance]`: the theme.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    /// A bundled theme's name or one in `<home>/themes/*.json`. Unset: VS Code Dark Modern.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// "campbell" paints ANSI 0-15 in Windows Terminal's Campbell. Unset: the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_colors: Option<String>,
}

impl Section for AppearanceSettings {
    const KEY: &'static str = "appearance";
    const FIELDS: &'static [&'static str] = &["theme", "terminal_colors"];
}

/// `[terminal]`: every terminal tab's font.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    /// In px, as Ctrl+= / Ctrl+- / Ctrl+wheel and Settings → Appearance leave it; read through
    /// [`TerminalSettings::font_size`].
    #[serde(deserialize_with = "lenient_size", skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u32>,
    /// Read through [`TerminalSettings::font_family`].
    #[serde(deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// Unset or not a bool: on.
    #[serde(deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub ligatures: Option<bool>,
}

impl Section for TerminalSettings {
    const KEY: &'static str = "terminal";
    const FIELDS: &'static [&'static str] = &["font_size", "font_family", "ligatures"];
}

impl TerminalSettings {
    /// 14 when unset or not a number, else clamped.
    pub fn font_size(&self) -> u32 {
        self.font_size.map_or(DEFAULT_TERMINAL_FONT_SIZE, clamp_terminal_font_size)
    }

    /// Stores `size` clamped; the default is stored as unset.
    pub fn set_font_size(&mut self, size: u32) {
        let size = clamp_terminal_font_size(size);
        self.font_size = (size != DEFAULT_TERMINAL_FONT_SIZE).then_some(size);
    }

    /// The family to paint with, as `installed` spells it: the one set when it is installed, else the default.
    pub fn font_family(&self, installed: &[String]) -> String {
        self.font_family
            .as_deref()
            .and_then(|want| installed.iter().find(|name| name.eq_ignore_ascii_case(want.trim())))
            .cloned()
            .unwrap_or_else(|| default_terminal_font_family(installed).to_string())
    }

    /// The family set, when it is not installed.
    pub fn missing_font(&self, installed: &[String]) -> Option<&str> {
        self.font_family.as_deref().filter(|want| !installed.iter().any(|name| name.eq_ignore_ascii_case(want.trim())))
    }

    /// Stores `family`; `None` or a blank name is stored as unset.
    pub fn set_font_family(&mut self, family: Option<&str>) {
        self.font_family = family.map(str::trim).filter(|f| !f.is_empty()).map(str::to_string);
    }

    pub fn ligatures(&self) -> bool {
        self.ligatures.unwrap_or(true)
    }

    /// On, the default, is stored as unset.
    pub fn set_ligatures(&mut self, on: bool) {
        self.ligatures = (!on).then_some(false);
    }
}

/// `[window]`: the main window's layout.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSettings {
    /// The fleet panel's width in logical pixels, as the divider was last dragged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_width: Option<u32>,
}

impl Section for WindowSettings {
    const KEY: &'static str = "window";
    const FIELDS: &'static [&'static str] = &["left_width"];
}

/// `[updates]`: whether the app looks for a newer release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    /// At start and daily; off sends no request at all.
    #[serde(skip_serializing_if = "is_true")]
    pub check: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self { check: true }
    }
}

impl Section for UpdateSettings {
    const KEY: &'static str = "updates";
    const FIELDS: &'static [&'static str] = &["check"];
}

/// `<FOREMAN_HOME>/settings.toml`, edited in the app and re-read by every supervisor tick. One table per
/// [`Section`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub sessions: SessionSettings,
    pub workstate: WorkStateSettings,
    pub appearance: AppearanceSettings,
    pub terminal: TerminalSettings,
    pub window: WindowSettings,
    pub updates: UpdateSettings,
    /// Every table and key Foreman does not own, as read and written back: a plugin's `[gitman]`, a newer
    /// build's keys, a key outside any table. Read and written through [`Settings::section`].
    pub other: Table,
}

/// The tables [`Settings`] owns, which [`Settings::section`] refuses.
const OWNED: [&str; 6] =
    [SessionSettings::KEY, WorkStateSettings::KEY, AppearanceSettings::KEY, TerminalSettings::KEY, WindowSettings::KEY, UpdateSettings::KEY];

/// The value, or nothing when it has the wrong type: a bad value must not make the whole file unreadable.
fn lenient<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> std::result::Result<Option<T>, D::Error> {
    Ok(toml::Value::deserialize(d)?.try_into().ok())
}

/// The first of [`TERMINAL_FONT_PREFERENCE`] in `installed` (family names, any case), else the last.
pub fn default_terminal_font_family(installed: &[String]) -> &'static str {
    TERMINAL_FONT_PREFERENCE
        .into_iter()
        .find(|want| installed.iter().any(|name| name.eq_ignore_ascii_case(want)))
        .unwrap_or(TERMINAL_FONT_PREFERENCE[TERMINAL_FONT_PREFERENCE.len() - 1])
}

/// A number (integer or float, rounded) or nothing: a bad value must not make the whole file unreadable.
fn lenient_size<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<u32>, D::Error> {
    Ok(match toml::Value::deserialize(d)? {
        toml::Value::Integer(n) => u32::try_from(n.max(0)).ok(),
        toml::Value::Float(f) if f.is_finite() => Some(f.round().clamp(0., u32::MAX as f64) as u32),
        _ => None,
    })
}

/// `size` within [`MIN_TERMINAL_FONT_SIZE`]..=[`MAX_TERMINAL_FONT_SIZE`].
pub fn clamp_terminal_font_size(size: u32) -> u32 {
    size.clamp(MIN_TERMINAL_FONT_SIZE, MAX_TERMINAL_FONT_SIZE)
}

/// A server URL typed into Settings as the origin to keep: `http(s)://host[:port]`, any path dropped.
/// Plain `http` only for this machine (a token must not cross a network in the clear).
pub fn validate_workstate_url(text: &str) -> std::result::Result<String, String> {
    let text = text.trim();
    let (scheme, rest) = text.split_once("://").ok_or("a URL starts with https:// (or http:// for this machine)")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(format!("{scheme}:// is not http or https"));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.chars().any(char::is_whitespace) {
        return Err("the URL names no host".into());
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        let (h, port) = v6.split_once(']').ok_or("an IPv6 host is closed with ]")?;
        if !port.is_empty() && port.strip_prefix(':').is_none_or(|p| p.parse::<u16>().is_err()) {
            return Err(format!("{port} is not a port"));
        }
        h.to_string()
    } else {
        let (h, port) = authority.rsplit_once(':').map_or((authority, None), |(h, p)| (h, Some(p)));
        if let Some(p) = port
            && p.parse::<u16>().is_err()
        {
            return Err(format!("{p} is not a port"));
        }
        if h.is_empty() || !h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            return Err(format!("{h} is not a host name"));
        }
        h.to_ascii_lowercase()
    };
    let local = matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1");
    if scheme == "http" && !local {
        return Err("use https:// for a server that is not on this machine".into());
    }
    Ok(format!("{scheme}://{}", authority.to_ascii_lowercase()))
}

impl Settings {
    pub fn path(home: &Path) -> PathBuf {
        home.join("settings.toml")
    }

    pub fn load(home: &Path) -> Result<Self> {
        Self::load_with_notices(home).map(|(settings, _)| settings)
    }

    /// The defaults when the file does not exist. A key with a value of the wrong type keeps its
    /// default, and a section that is not a table all of its defaults, each named in a notice; only a file
    /// that is not TOML at all is an error.
    pub fn load_with_notices(home: &Path) -> Result<(Self, Vec<String>)> {
        let path = Self::path(home);
        match read(&path)? {
            Some(text) => parse(&path, &text),
            None => Ok((Self::default(), Vec::new())),
        }
    }

    pub fn save(&self, home: &Path) -> Result<PathBuf> {
        let path = Self::path(home);
        let serialize = |source| CoreError::Serialize { name: "settings".into(), source };
        let text = toml::to_string(&self.to_table().map_err(serialize)?).map_err(serialize)?;
        crate::fs::write_atomic(&path, text).map_err(|e| CoreError::io(&path, e))?;
        Ok(path)
    }

    /// Load, `edit`, save, against other writers as `supraman_supervisor::DesiredState::update` does:
    /// a file changed between the load and the save starts it over (up to 10 times). `edit` may run more than once.
    pub fn update<T>(home: &Path, mut edit: impl FnMut(&mut Self) -> T) -> Result<(Self, T)> {
        let path = Self::path(home);
        let mut attempt = 0;
        loop {
            let before = read(&path)?;
            let mut settings = match &before {
                Some(text) => parse(&path, text)?.0,
                None => Self::default(),
            };
            let out = edit(&mut settings);
            attempt += 1;
            if read(&path)? != before && attempt < 10 {
                continue;
            }
            settings.save(home)?;
            return Ok((settings, out));
        }
    }

    /// The table `key` of another owner, e.g. a plugin's `[gitman]`: `None` when it is absent or not `T`.
    pub fn section<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        assert!(!OWNED.contains(&key), "[{key}] is a field of Settings");
        self.other.get(key)?.clone().try_into().ok()
    }

    pub fn set_section<T: Serialize>(&mut self, key: &str, value: &T) -> std::result::Result<(), toml::ser::Error> {
        assert!(!OWNED.contains(&key), "[{key}] is a field of Settings");
        self.other.insert(key.into(), Value::try_from(value)?);
        Ok(())
    }

    fn to_table(&self) -> std::result::Result<Table, toml::ser::Error> {
        let mut out = self.other.clone();
        put(&mut out, &self.sessions)?;
        put(&mut out, &self.workstate)?;
        put(&mut out, &self.appearance)?;
        put(&mut out, &self.terminal)?;
        put(&mut out, &self.window)?;
        put(&mut out, &self.updates)?;
        Ok(out)
    }
}

/// `section`'s keys into its table in `out`, beside any keys of it Foreman does not know; no table when
/// both are empty.
fn put<S: Section>(out: &mut Table, section: &S) -> std::result::Result<(), toml::ser::Error> {
    let Value::Table(fields) = Value::try_from(section)? else { unreachable!("a section serializes to a table") };
    let mut table = match out.remove(S::KEY) {
        Some(Value::Table(kept)) => kept,
        _ => Table::new(),
    };
    table.extend(fields);
    if !table.is_empty() {
        out.insert(S::KEY.into(), Value::Table(table));
    }
    Ok(())
}

fn read(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::io(path, e)),
    }
}

fn parse(path: &Path, text: &str) -> Result<(Settings, Vec<String>)> {
    let mut table: Table = text.parse().map_err(|source| CoreError::Parse { path: path.to_path_buf(), source })?;
    let mut notices = Vec::new();
    let sessions = take(&mut table, path, &mut notices);
    let workstate = take(&mut table, path, &mut notices);
    let appearance = take(&mut table, path, &mut notices);
    let terminal = take(&mut table, path, &mut notices);
    let window = take(&mut table, path, &mut notices);
    let updates = take(&mut table, path, &mut notices);
    for (key, value) in &table {
        if !value.is_table() {
            notices.push(format!("{}: {key} is not in a table; it is ignored", path.display()));
        }
    }
    Ok((Settings { sessions, workstate, appearance, terminal, window, updates, other: table }, notices))
}

/// `S` out of its own table in `table`, each key checked on its own. What `S` does not know stays in
/// `table` for [`Settings::other`].
fn take<S: Section>(table: &mut Table, path: &Path, notices: &mut Vec<String>) -> S {
    let at = path.display();
    let mut own = match table.remove(S::KEY) {
        None => Table::new(),
        Some(Value::Table(own)) => own,
        Some(_) => {
            notices.push(format!("{at}: [{}] is not a table; its defaults are used", S::KEY));
            Table::new()
        }
    };
    let mut known = Table::new();
    for key in S::FIELDS {
        let Some(value) = own.remove(*key) else { continue };
        let one = Table::from_iter([(key.to_string(), value.clone())]);
        match Value::Table(one).try_into::<S>() {
            Ok(_) => {
                known.insert(key.to_string(), value);
            }
            Err(e) => {
                let why = e.message().lines().next().unwrap_or_default().to_string();
                notices.push(format!("{at}: {}.{key} is not valid ({why}); its default is used", S::KEY));
            }
        }
    }
    if !own.is_empty() {
        table.insert(S::KEY.into(), Value::Table(own));
    }
    Value::Table(known).try_into().unwrap_or_else(|e: toml::de::Error| {
        notices.push(format!("{at}: [{}] is not valid ({}); its defaults are used", S::KEY, e.message().lines().next().unwrap_or_default()));
        S::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_key_set() -> Settings {
        Settings {
            sessions: SessionSettings {
                max_working: 1,
                extra_plugin_dirs: vec![r"C:\x\plugins\rlarch".into()],
                run_as_admin: true,
                relay_idle_secs: Some(20),
                reopen_tabs: false,
                resume_tabs_on_click: true,
            },
            workstate: WorkStateSettings { url: Some("https://workstate.example".into()) },
            appearance: AppearanceSettings { theme: Some("Foreman Dark".into()), terminal_colors: Some("campbell".into()) },
            terminal: TerminalSettings { font_size: Some(20), font_family: Some("Lucida Console".into()), ligatures: Some(false) },
            window: WindowSettings { left_width: Some(512) },
            updates: UpdateSettings { check: false },
            other: Table::new(),
        }
    }

    fn load_text(home: &Path, text: &str) -> (Settings, Vec<String>) {
        fs::write(Settings::path(home), text).unwrap();
        Settings::load_with_notices(home).unwrap()
    }

    fn saved(home: &Path) -> Table {
        fs::read_to_string(Settings::path(home)).unwrap().parse().unwrap()
    }

    #[test]
    fn absent_is_default_and_round_trips_as_tables() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(home.path()).unwrap(), Settings::default());
        let s = every_key_set();
        s.save(home.path()).unwrap();
        assert_eq!(Settings::load(home.path()).unwrap(), s);
        let file = saved(home.path());
        assert_eq!(file.keys().collect::<Vec<_>>(), ["appearance", "sessions", "terminal", "updates", "window", "workstate"]);
        assert_eq!(file["terminal"]["font_size"].as_integer(), Some(20));
        assert_eq!(file["workstate"]["url"].as_str(), Some("https://workstate.example"));
    }

    #[test]
    fn tabs_reopen_and_claude_tabs_resume_at_once_unless_set() {
        let home = tempfile::tempdir().unwrap();
        let s = Settings::default();
        assert!(s.sessions.reopen_tabs && !s.sessions.resume_tabs_on_click);
        s.save(home.path()).unwrap();
        let file = saved(home.path());
        let sessions = file.get("sessions").and_then(Value::as_table);
        assert!(sessions.is_none_or(|t| !t.contains_key("reopen_tabs") && !t.contains_key("resume_tabs_on_click")), "{file:?}");
        let (s, notices) = load_text(home.path(), "[sessions]\nreopen_tabs = false\nresume_tabs_on_click = true\n");
        assert!(!s.sessions.reopen_tabs && s.sessions.resume_tabs_on_click && notices.is_empty(), "{notices:?}");
    }

    #[test]
    fn every_section_lists_every_field_it_writes() {
        fn check<S: Section>(all: &S) {
            let Value::Table(written) = Value::try_from(all).unwrap() else { panic!() };
            let mut written: Vec<_> = written.keys().cloned().collect();
            let mut listed: Vec<_> = S::FIELDS.iter().map(|key| key.to_string()).collect();
            written.sort();
            listed.sort();
            assert_eq!(written, listed, "[{}]", S::KEY);
        }
        let s = every_key_set();
        check(&s.sessions);
        check(&s.workstate);
        check(&s.appearance);
        check(&s.terminal);
        check(&s.window);
        check(&s.updates);
    }

    #[test]
    fn updates_are_checked_unless_turned_off() {
        let home = tempfile::tempdir().unwrap();
        assert!(Settings::default().updates.check);
        Settings::default().save(home.path()).unwrap();
        assert!(!saved(home.path()).contains_key("updates"));
        let (s, notices) = load_text(home.path(), "[updates]
check = false
");
        assert!(!s.updates.check && notices.is_empty(), "{notices:?}");
    }

    /// The shape of a home's `settings.toml` today, with made-up values: loads with no notice, and a save
    /// writes the same tables back.
    #[test]
    fn a_tables_file_as_written_today_loads_and_saves_unchanged() {
        let home = tempfile::tempdir().unwrap();
        let today = r#"[appearance]
theme = "VS Code Dark Modern"

[sessions]
max_working = 20

[window]
left_width = 531

[workstate]
url = "https://workstate.example"
"#;
        let (s, notices) = load_text(home.path(), today);
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(s.appearance.theme.as_deref(), Some("VS Code Dark Modern"));
        assert_eq!((s.sessions.max_working, s.window.left_width), (20, Some(531)));
        assert_eq!(s.workstate.origin(), "https://workstate.example");
        assert!(s.other.is_empty(), "{:?}", s.other);
        s.save(home.path()).unwrap();
        assert_eq!(saved(home.path()), today.parse::<Table>().unwrap());
        assert_eq!(Settings::load(home.path()).unwrap(), s);
    }

    /// A key outside any table, a former flat name or not, is named in a notice and keeps no value; the
    /// save keeps it where it is.
    #[test]
    fn a_key_outside_a_table_is_a_notice_and_the_rest_of_the_file_stands() {
        let home = tempfile::tempdir().unwrap();
        let (s, notices) = load_text(home.path(), "max_working = 9\nworkstate_env = 'C:/x/.env'\n\n[sessions]\nmax_working = 4\n\n[gitman]\nfetch = true\n");
        assert_eq!(s.sessions.max_working, 4);
        assert_eq!(notices.len(), 2, "{notices:?}");
        assert!(notices.iter().any(|n| n.ends_with("max_working is not in a table; it is ignored")), "{notices:?}");
        assert!(notices.iter().any(|n| n.ends_with("workstate_env is not in a table; it is ignored")), "{notices:?}");
        assert!(notices.iter().all(|n| n.contains("settings.toml")), "{notices:?}");
        let (s, _) = load_text(home.path(), "theme = \"Foreman Dark\"\nleft_width = 300\n");
        assert_eq!((s.appearance.theme.clone(), s.window.left_width), (None, None), "a flat file is all defaults");
        s.save(home.path()).unwrap();
        assert_eq!(saved(home.path())["left_width"].as_integer(), Some(300), "nothing is dropped");
    }

    #[test]
    fn tables_and_keys_of_other_owners_survive_a_save_and_an_update() {
        let home = tempfile::tempdir().unwrap();
        let text = "[future]\nkey = 1\n\n[gitman]\nfetch = true\n\n[gitman.remotes]\norigin = \"x\"\n\n[terminal]\nfont_size = 18\ncursor = \"bar\"\n";
        let (s, notices) = load_text(home.path(), text);
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(s.terminal.font_size(), 18);
        Settings::update(home.path(), |s| s.terminal.set_font_size(19)).unwrap();
        let file = saved(home.path());
        assert_eq!(file["gitman"]["remotes"]["origin"].as_str(), Some("x"));
        assert_eq!(file["future"]["key"].as_integer(), Some(1));
        assert_eq!((file["terminal"]["cursor"].as_str(), file["terminal"]["font_size"].as_integer()), (Some("bar"), Some(19)));
    }

    #[test]
    fn a_plugin_reads_and_writes_its_own_table() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct GitMan {
            fetch_every_secs: u64,
        }
        let home = tempfile::tempdir().unwrap();
        Settings::update(home.path(), |s| s.set_section("gitman", &GitMan { fetch_every_secs: 60 }).unwrap()).unwrap();
        let s = Settings::load(home.path()).unwrap();
        assert_eq!(s.section::<GitMan>("gitman"), Some(GitMan { fetch_every_secs: 60 }));
        assert_eq!(s.section::<GitMan>("fileman"), None);
        assert_eq!(s, Settings { other: s.other.clone(), ..Settings::default() });
    }

    #[test]
    #[should_panic(expected = "[terminal] is a field of Settings")]
    fn an_owned_table_is_not_a_plugins() {
        Settings::default().section::<Table>("terminal");
    }

    #[test]
    fn a_bad_section_falls_back_alone() {
        let home = tempfile::tempdir().unwrap();
        let (s, notices) = load_text(home.path(), "terminal = 5\nwindow = \"wide\"\n\n[sessions]\nmax_working = 3\n\n[appearance]\ntheme = \"Foreman Dark\"\n");
        assert_eq!((s.sessions.max_working, s.appearance.theme.as_deref()), (3, Some("Foreman Dark")));
        assert_eq!((s.terminal.clone(), s.window.clone()), (TerminalSettings::default(), WindowSettings::default()));
        assert_eq!(notices.len(), 2, "{notices:?}");
        assert!(notices.iter().any(|n| n.ends_with("[terminal] is not a table; its defaults are used")), "{notices:?}");
        assert!(notices.iter().any(|n| n.contains("[window] is not a table")), "{notices:?}");
        s.save(home.path()).unwrap();
        let (again, quiet) = Settings::load_with_notices(home.path()).unwrap();
        assert_eq!((again, quiet), (s, Vec::new()), "the save writes the defaults in their place");
    }

    #[test]
    fn partial_file_keeps_defaults() {
        let home = tempfile::tempdir().unwrap();
        let (s, _) = load_text(home.path(), "[workstate]\nurl = \"http://x/mcp\"\n");
        assert_eq!(s.sessions.max_working, DEFAULT_MAX_WORKING);
        assert_eq!(s.workstate.url.as_deref(), Some("http://x/mcp"));
        assert!(s.sessions.extra_plugin_dirs.is_empty());
        assert_eq!(s.sessions.relay_idle_ms(), crate::DEFAULT_RELAY_IDLE_MS, "10 min unless set");
        assert_eq!(load_text(home.path(), "[sessions]\nrelay_idle_secs = 20\n").0.sessions.relay_idle_ms(), 20_000);
        fs::write(Settings::path(home.path()), "[sessions]\nmax_working = = 3").unwrap();
        assert!(matches!(Settings::load(home.path()), Err(CoreError::Parse { .. })), "not TOML at all");
    }

    #[test]
    fn workstate_url_is_an_origin() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(Settings::default().workstate.origin(), "https://workstate.dikaia.ai");
        assert_eq!(Settings::default().workstate.mcp_url(), "https://workstate.dikaia.ai/mcp");
        let (s, _) = load_text(home.path(), "[workstate]\nurl = \"http://127.0.0.1:5290/mcp\"\n");
        assert_eq!(s.workstate.origin(), "http://127.0.0.1:5290");
        assert_eq!(s.workstate.mcp_url(), "http://127.0.0.1:5290/mcp");
    }

    #[test]
    fn validate_workstate_url_keeps_an_origin_and_https_off_this_machine() {
        assert_eq!(validate_workstate_url(" https://WorkState.dikaia.ai/ ").as_deref(), Ok("https://workstate.dikaia.ai"));
        assert_eq!(validate_workstate_url("https://workstate.dikaia.ai/mcp").as_deref(), Ok("https://workstate.dikaia.ai"));
        assert_eq!(validate_workstate_url("http://127.0.0.1:5290").as_deref(), Ok("http://127.0.0.1:5290"));
        assert_eq!(validate_workstate_url("http://localhost:9/api").as_deref(), Ok("http://localhost:9"));
        assert_eq!(validate_workstate_url("http://[::1]:9").as_deref(), Ok("http://[::1]:9"));
        for bad in ["workstate.dikaia.ai", "ftp://x", "http://workstate.dikaia.ai", "https://", "https://a b", "https://x:port", "https://u@x"] {
            assert!(validate_workstate_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn terminal_font_size_is_14_unless_set_and_clamped() {
        let home = tempfile::tempdir().unwrap();
        let load = |text: &str| load_text(home.path(), text).0;
        assert_eq!(Settings::default().terminal.font_size(), DEFAULT_TERMINAL_FONT_SIZE);
        assert_eq!(load("[sessions]\nmax_working = 2\n").terminal.font_size(), 14, "missing");
        assert_eq!(load("[terminal]\nfont_size = 18\n").terminal.font_size(), 18);
        assert_eq!(load("[terminal]\nfont_size = 17.6\n").terminal.font_size(), 18, "a float is rounded");
        assert_eq!(load("[terminal]\nfont_size = 1\n").terminal.font_size(), MIN_TERMINAL_FONT_SIZE);
        assert_eq!(load("[terminal]\nfont_size = -3\n").terminal.font_size(), MIN_TERMINAL_FONT_SIZE);
        assert_eq!(load("[terminal]\nfont_size = 500\n").terminal.font_size(), MAX_TERMINAL_FONT_SIZE);
        for bad in ["\"big\"", "true", "[14]", "nan"] {
            let s = load(&format!("[sessions]\nmax_working = 2\n\n[terminal]\nfont_size = {bad}\n"));
            assert_eq!(s.terminal.font_size(), 14, "{bad} is invalid");
            assert_eq!(s.sessions.max_working, 2, "{bad} does not cost the rest of the file");
        }
    }

    #[test]
    fn terminal_font_size_saves_clamped_and_the_default_as_unset() {
        let home = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        s.terminal.set_font_size(19);
        s.save(home.path()).unwrap();
        assert_eq!(saved(home.path())["terminal"]["font_size"].as_integer(), Some(19));
        assert_eq!(Settings::load(home.path()).unwrap().terminal.font_size(), 19);
        s.terminal.set_font_size(999);
        assert_eq!(s.terminal.font_size, Some(MAX_TERMINAL_FONT_SIZE));
        s.terminal.set_font_size(0);
        assert_eq!(s.terminal.font_size, Some(MIN_TERMINAL_FONT_SIZE));
        s.terminal.set_font_size(DEFAULT_TERMINAL_FONT_SIZE);
        s.save(home.path()).unwrap();
        assert!(!fs::read_to_string(Settings::path(home.path())).unwrap().contains("font_size"));
        assert_eq!(Settings::load(home.path()).unwrap().terminal.font_size(), 14);
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_default_terminal_font_is_the_first_installed_of_cascadia_code_cascadia_mono_consolas() {
        assert_eq!(default_terminal_font_family(&names(&["Arial", "Cascadia Mono", "Cascadia Code", "Consolas"])), "Cascadia Code");
        assert_eq!(default_terminal_font_family(&names(&["Arial", "cascadia mono", "Consolas"])), "Cascadia Mono");
        assert_eq!(default_terminal_font_family(&names(&["Arial", "Consolas"])), "Consolas");
        assert_eq!(default_terminal_font_family(&names(&["Arial"])), "Consolas", "nothing installed: Consolas");
        assert_eq!(default_terminal_font_family(&[]), "Consolas");
    }

    #[test]
    fn terminal_font_family_is_the_one_set_when_installed_else_the_default() {
        let installed = names(&["Cascadia Code", "Consolas", "Lucida Console"]);
        let mut t = TerminalSettings::default();
        assert_eq!(t.font_family(&installed), "Cascadia Code");
        assert_eq!(t.missing_font(&installed), None);
        t.set_font_family(Some(" lucida console "));
        assert_eq!(t.font_family(&installed), "Lucida Console", "matched in any case, spelt as installed");
        t.set_font_family(Some("Fira Code"));
        assert_eq!(t.font_family(&installed), "Cascadia Code");
        assert_eq!(t.missing_font(&installed), Some("Fira Code"));
        assert_eq!(t.font_family(&names(&["Consolas"])), "Consolas");
        t.set_font_family(Some("  "));
        assert_eq!(t.font_family, None);
    }

    #[test]
    fn terminal_font_family_and_ligatures_load_save_and_survive_bad_values() {
        let home = tempfile::tempdir().unwrap();
        let load = |text: &str| load_text(home.path(), text).0;
        assert!(Settings::default().terminal.ligatures(), "on unless set");
        let s = load("[terminal]\nfont_family = \"Consolas\"\nligatures = false\n");
        assert_eq!(s.terminal.font_family.as_deref(), Some("Consolas"));
        assert!(!s.terminal.ligatures());
        for bad in ["font_family = 12\nligatures = \"yes\"", "font_family = [\"x\"]\nligatures = 0"] {
            let s = load(&format!("[sessions]\nmax_working = 3\n\n[terminal]\n{bad}\n"));
            assert_eq!((s.terminal.font_family.as_deref(), s.terminal.ligatures()), (None, true), "{bad}");
            assert_eq!(s.sessions.max_working, 3, "{bad} does not cost the rest of the file");
        }

        let mut s = Settings::default();
        s.terminal.set_font_family(Some("Cascadia Mono"));
        s.terminal.set_ligatures(false);
        s.save(home.path()).unwrap();
        let file = saved(home.path());
        assert_eq!((file["terminal"]["font_family"].as_str(), file["terminal"]["ligatures"].as_bool()), (Some("Cascadia Mono"), Some(false)));
        assert_eq!(Settings::load(home.path()).unwrap(), s);
        s.terminal.set_font_family(None);
        s.terminal.set_ligatures(true);
        s.save(home.path()).unwrap();
        assert!(!saved(home.path()).contains_key("terminal"), "the defaults are saved as unset");
    }

    #[test]
    fn a_bad_key_keeps_its_default_and_is_named_while_the_rest_of_the_file_stands() {
        let home = tempfile::tempdir().unwrap();
        let (s, notices) = load_text(
            home.path(),
            "[sessions]\nmax_working = \"many\"\nrun_as_admin = \"yes\"\n\n[workstate]\nurl = \"https://ws.example\"\n\n[window]\nleft_width = 300\n",
        );
        assert_eq!(s.sessions.max_working, DEFAULT_MAX_WORKING);
        assert!(!s.sessions.run_as_admin);
        assert_eq!((s.workstate.url.as_deref(), s.window.left_width), (Some("https://ws.example"), Some(300)));
        assert_eq!(notices.len(), 2, "{notices:?}");
        assert!(notices.iter().any(|n| n.contains("sessions.max_working is not valid")), "{notices:?}");
        assert!(notices.iter().any(|n| n.contains("sessions.run_as_admin is not valid")), "{notices:?}");
        assert!(notices.iter().all(|n| n.contains("settings.toml") && !n.contains('\n')), "{notices:?}");
        assert_eq!(Settings::load(home.path()).unwrap(), s);
        let (_, quiet) = Settings::load_with_notices(tempfile::tempdir().unwrap().path()).unwrap();
        assert!(quiet.is_empty(), "no file, no notice");
    }

    #[test]
    fn update_edits_what_is_on_disk_and_keeps_the_rest() {
        let home = tempfile::tempdir().unwrap();
        let (s, out) = Settings::update(home.path(), |s| {
            s.window.left_width = Some(400);
            7
        })
        .unwrap();
        assert_eq!((s.window.left_width, out), (Some(400), 7));
        Settings::update(home.path(), |s| s.appearance.theme = Some("Foreman Dark".into())).unwrap();
        let back = Settings::load(home.path()).unwrap();
        assert_eq!((back.window.left_width, back.appearance.theme.as_deref()), (Some(400), Some("Foreman Dark")));
        fs::write(Settings::path(home.path()), "left_width = = 1").unwrap();
        assert!(Settings::update(home.path(), |s| s.window.left_width = Some(1)).is_err(), "a file that is not TOML is not overwritten");
        assert_eq!(fs::read_to_string(Settings::path(home.path())).unwrap(), "left_width = = 1");
    }

    #[test]
    fn update_starts_over_when_another_writer_saved_meanwhile() {
        let home = tempfile::tempdir().unwrap();
        Settings::default().save(home.path()).unwrap();
        let mut calls = 0;
        let (s, ()) = Settings::update(home.path(), |s| {
            calls += 1;
            if calls == 1 {
                let other = Settings { appearance: AppearanceSettings { theme: Some("Other".into()), ..Default::default() }, ..Settings::default() };
                other.save(home.path()).unwrap();
            }
            s.window.left_width = Some(333);
        })
        .unwrap();
        assert_eq!(calls, 2, "the edit ran again on the other writer's file");
        assert_eq!((s.appearance.theme.as_deref(), s.window.left_width), (Some("Other"), Some(333)));
        assert_eq!(Settings::load(home.path()).unwrap(), s);
    }
}
