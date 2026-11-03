//! This machine's shells for a plain terminal tab, and `[terminal] shell` resolved against them.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use crate::paths::{PowerShell, expand_env, powershell, windows_powershell};

/// How long one `vswhere` or `wsl.exe` may run before it is killed and its shells are left out.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// `[terminal] shell`: the shell a plain terminal tab starts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShellSetting {
    /// pwsh, else Windows PowerShell.
    #[default]
    Auto,
    Pwsh,
    PowerShell,
    Cmd,
    Wsl(String),
    /// An exe by path (`%VAR%`s expanded when resolved), started with `[terminal] shell_args`.
    Custom(PathBuf),
}

impl FromStr for ShellSetting {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let word = text.to_ascii_lowercase();
        Ok(match word.as_str() {
            "auto" => Self::Auto,
            "pwsh" => Self::Pwsh,
            "powershell" => Self::PowerShell,
            "cmd" => Self::Cmd,
            _ if word.starts_with("wsl:") => match text[4..].trim() {
                "" => return Err("wsl: names no distro".into()),
                distro => Self::Wsl(distro.to_string()),
            },
            _ if text.contains(['\\', '/']) => Self::Custom(PathBuf::from(text)),
            _ => return Err(format!("{text:?} is not auto, pwsh, powershell, cmd, wsl:<distro> or the path of a shell's exe")),
        })
    }
}

impl fmt::Display for ShellSetting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Pwsh => f.write_str("pwsh"),
            Self::PowerShell => f.write_str("powershell"),
            Self::Cmd => f.write_str("cmd"),
            Self::Wsl(distro) => write!(f, "wsl:{distro}"),
            Self::Custom(path) => write!(f, "{}", path.display()),
        }
    }
}

impl serde::Serialize for ShellSetting {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for ShellSetting {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// A shell a tab can start: `program` (absolute) with `args`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shell {
    pub label: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The `[terminal] shell` value that picks it; `None` for a developer shell, which no setting names.
    pub setting: Option<ShellSetting>,
}

impl Shell {
    /// The name a tab title starts with: `pwsh`, `powershell`, `cmd`, a distro, a custom exe's stem, or a
    /// developer shell's label without its `for <Visual Studio>`.
    pub fn short(&self) -> String {
        match &self.setting {
            Some(ShellSetting::Pwsh) => "pwsh".into(),
            Some(ShellSetting::PowerShell | ShellSetting::Auto) => "powershell".into(),
            Some(ShellSetting::Cmd) => "cmd".into(),
            Some(ShellSetting::Wsl(distro)) => distro.clone(),
            Some(ShellSetting::Custom(_)) => self.label.clone(),
            None => self.label.split(" for ").next().unwrap_or(&self.label).to_string(),
        }
    }
}

/// The environment detection reads: `PATH`, `PATHEXT`, `SystemRoot` and `ProgramFiles(x86)`.
#[derive(Clone, Debug, Default)]
pub struct Machine {
    pub path: OsString,
    pub pathext: String,
    pub system_root: Option<OsString>,
    pub program_files_x86: Option<OsString>,
}

impl Machine {
    pub fn current() -> Self {
        Self {
            path: std::env::var_os("PATH").unwrap_or_default(),
            pathext: std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into()),
            system_root: std::env::var_os("SystemRoot"),
            program_files_x86: std::env::var_os("ProgramFiles(x86)"),
        }
    }

    fn system32(&self) -> PathBuf {
        self.system_root.as_deref().filter(|r| !r.is_empty()).map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from).join("System32")
    }

    fn vswhere(&self) -> Option<PathBuf> {
        let root = self.program_files_x86.as_deref().filter(|r| !r.is_empty())?;
        Some(Path::new(root).join(r"Microsoft Visual Studio\Installer\vswhere.exe"))
    }
}

/// What detection found: every shell in menu order, and the one `auto` starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shells {
    pub list: Vec<Shell>,
    /// pwsh, else Windows PowerShell, listed or not: what a tab starts when nothing else will do.
    pub auto: Shell,
}

/// This machine's shells; blocks on `vswhere` and `wsl.exe` (each up to [`PROBE_TIMEOUT`]), so call it off
/// the UI thread. Only plain shell tabs start them: a tab that starts or attaches claude stays on PowerShell
/// (C113).
pub fn detect() -> Shells {
    detect_on(&Machine::current(), &mut probe)
}

/// [`detect`] with `run(program, args)` giving a tool's stdout, or `None` when it is missing, failed or timed out.
pub fn detect_on(machine: &Machine, run: &mut dyn FnMut(&Path, &[&str]) -> Option<String>) -> Shells {
    let found = powershell(&machine.path, &machine.pathext, machine.system_root.as_deref());
    let windows = windows_powershell(machine.system_root.as_deref());
    let auto = powershell_shell(&found);
    let mut list = Vec::new();
    if found.name == "pwsh" {
        list.push(auto.clone());
    }
    if windows.path.is_file() {
        list.push(powershell_shell(&windows));
    }
    let cmd = machine.system32().join("cmd.exe");
    if cmd.is_file() {
        list.push(Shell { label: "Command Prompt".into(), program: cmd.clone(), args: Vec::new(), setting: Some(ShellSetting::Cmd) });
    }
    if let Some(vswhere) = machine.vswhere() {
        let json = run(&vswhere, &["-products", "*", "-format", "json", "-utf8", "-nologo"]).unwrap_or_default();
        for vs in parse_vswhere(&json) {
            list.extend(dev_shells(&vs, &found.path, &cmd));
        }
    }
    let wsl = machine.system32().join("wsl.exe");
    for distro in parse_wsl(&run(&wsl, &["-l", "-q"]).unwrap_or_default()) {
        list.push(Shell { label: distro.clone(), program: wsl.clone(), args: vec!["-d".into(), distro.clone()], setting: Some(ShellSetting::Wsl(distro)) });
    }
    unique_labels(&mut list);
    Shells { list, auto }
}

fn powershell_shell(shell: &PowerShell) -> Shell {
    let (label, setting) = if shell.name == "pwsh" { ("PowerShell", ShellSetting::Pwsh) } else { ("Windows PowerShell", ShellSetting::PowerShell) };
    Shell { label: label.into(), program: shell.path.clone(), args: bypass(), setting: Some(setting) }
}

fn bypass() -> Vec<String> {
    vec!["-ExecutionPolicy".into(), "Bypass".into()]
}

fn probe(program: &Path, args: &[&str]) -> Option<String> {
    if !program.is_file() {
        return None;
    }
    let mut cmd = crate::process::command(program);
    cmd.args(args).env("WSL_UTF8", "1");
    crate::process::run(&mut cmd, PROBE_TIMEOUT, crate::process::Kill::Tree).ok().filter(|out| out.success()).map(|out| out.stdout)
}

/// One Visual Studio install as `vswhere -format json` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VsInstance {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
}

/// The installs in `vswhere -format json` output; an entry without an id, a path or a name is skipped, and
/// text that is not that JSON lists none.
pub fn parse_vswhere(json: &str) -> Vec<VsInstance> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str(json) else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let field = |key: &str| item.get(key)?.as_str().map(str::trim).filter(|v| !v.is_empty());
            let id = field("instanceId").filter(|id| id.chars().all(|c| c.is_ascii_alphanumeric()))?;
            let mut name = field("displayName")?.to_string();
            if item.get("isPrerelease").and_then(serde_json::Value::as_bool) == Some(true) && !name.contains("Preview") {
                name.push_str(" Preview");
            }
            Some(VsInstance { id: id.into(), path: field("installationPath")?.into(), name })
        })
        .collect()
}

fn dev_shells(vs: &VsInstance, powershell: &Path, cmd: &Path) -> Vec<Shell> {
    let tools = vs.path.join(r"Common7\Tools");
    let (dll, bat) = (tools.join("Microsoft.VisualStudio.DevShell.dll"), tools.join("VsDevCmd.bat"));
    let mut out = Vec::new();
    if dll.is_file() {
        let module = dll.display().to_string().replace('\'', "''");
        let enter = format!("&{{Import-Module '{module}'; Enter-VsDevShell {} -SkipAutomaticLocation}}", vs.id);
        let args = bypass().into_iter().chain(["-NoExit".into(), "-Command".into(), enter]).collect();
        out.push(Shell { label: format!("Developer PowerShell for {}", vs.name), program: powershell.into(), args, setting: None });
    }
    if bat.is_file() {
        let args = vec!["/k".into(), bat.display().to_string()];
        out.push(Shell { label: format!("Developer Command Prompt for {}", vs.name), program: cmd.into(), args, setting: None });
    }
    out
}

/// The distros in `wsl.exe -l -q` output, UTF-8 or UTF-16LE, without Docker Desktop's own `docker-desktop*`.
pub fn parse_wsl(text: &str) -> Vec<String> {
    let text = if text.contains('\0') {
        let bytes: Vec<u8> = text.chars().map(|c| c as u32 as u8).collect();
        String::from_utf16_lossy(&bytes.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect::<Vec<_>>())
    } else {
        text.to_string()
    };
    text.lines()
        .map(|line| line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0'))
        .filter(|name| !name.is_empty() && !name.to_ascii_lowercase().starts_with("docker-desktop"))
        .map(str::to_string)
        .collect()
}

fn unique_labels(list: &mut [Shell]) {
    for i in 0..list.len() {
        if list.iter().enumerate().any(|(j, other)| j != i && other.label == list[i].label) {
            let program = list[i].program.display().to_string();
            list[i].label = format!("{} ({})", list[i].label, list[i].args.last().cloned().unwrap_or(program));
        }
    }
}

impl Shells {
    /// The shell `setting` starts, with a notice when it cannot and `auto` stands in. `args` are
    /// `[terminal] shell_args`, which only a custom shell takes.
    pub fn resolve(&self, setting: &ShellSetting, args: &[String]) -> (Shell, Option<String>) {
        let fallback = |why: String| (self.auto.clone(), Some(format!("terminal.shell {why}; {} is used", self.auto.label)));
        let ignored = |shell: Shell| {
            let notice = (!args.is_empty()).then(|| "terminal.shell_args apply only to a custom shell; they are ignored".to_string());
            (shell, notice)
        };
        match setting {
            ShellSetting::Auto => ignored(self.auto.clone()),
            ShellSetting::Custom(path) => {
                let program = PathBuf::from(expand_env(&path.to_string_lossy()));
                if !program.is_absolute() || !program.is_file() {
                    return fallback(format!("{} is not a file", program.display()));
                }
                let label = program.file_stem().map_or_else(|| program.display().to_string(), |s| s.to_string_lossy().into_owned());
                (Shell { label, program, args: args.to_vec(), setting: Some(setting.clone()) }, None)
            }
            wanted => match self.list.iter().find(|s| s.setting.as_ref().is_some_and(|have| same(have, wanted))) {
                Some(shell) => ignored(shell.clone()),
                None => fallback(match wanted {
                    ShellSetting::Wsl(distro) => format!("wsl:{distro} is not an installed WSL distro"),
                    other => format!("{other} is not installed"),
                }),
            },
        }
    }

    /// [`Shells::resolve`] for the `[terminal]` table.
    pub fn for_settings(&self, terminal: &crate::TerminalSettings) -> (Shell, Option<String>) {
        self.resolve(&terminal.shell(), &terminal.shell_args)
    }
}

fn same(a: &ShellSetting, b: &ShellSetting) -> bool {
    match (a, b) {
        (ShellSetting::Wsl(a), ShellSetting::Wsl(b)) => a.eq_ignore_ascii_case(b),
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Fake {
        _tmp: tempfile::TempDir,
        machine: Machine,
        root: PathBuf,
        vs: PathBuf,
    }

    fn touch(file: &Path) {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, "").unwrap();
    }

    fn fake(pwsh: bool) -> Fake {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Windows");
        touch(&root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
        touch(&root.join(r"System32\cmd.exe"));
        let bin = tmp.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        if pwsh {
            touch(&bin.join("pwsh.exe"));
        }
        let vs = tmp.path().join("VS");
        let machine = Machine {
            path: std::env::join_paths([&bin]).unwrap(),
            pathext: ".COM;.EXE;.BAT;.CMD".into(),
            system_root: Some(root.clone().into()),
            program_files_x86: Some(tmp.path().join("x86").into()),
        };
        Fake { _tmp: tmp, machine, root, vs }
    }

    fn vswhere_json(vs: &Path) -> String {
        let at = |n: &str| vs.join(n).display().to_string().replace('\\', "\\\\");
        format!(
            r#"[{{"instanceId":"95451c93","installationPath":"{}","displayName":"Visual Studio Community 2026","isPrerelease":false}},
                {{"instanceId":"b91054a5","installationPath":"{}","displayName":"Visual Studio Community 2022","isPrerelease":false}},
                {{"instanceId":"cafe0001","installationPath":"{}","displayName":"Visual Studio Build Tools 2026","isPrerelease":true}}]"#,
            at("18"),
            at("2022"),
            at("bt")
        )
    }

    const WSL_UTF16: &str = "U\0b\0u\0n\0t\0u\0\r\0\n\0d\0o\0c\0k\0e\0r\0-\0d\0e\0s\0k\0t\0o\0p\0-\0d\0a\0t\0a\0\r\0\n\0d\0o\0c\0k\0e\0r\0-\0d\0e\0s\0k\0t\0o\0p\0\r\0\n\0";

    fn tools(vswhere: Option<String>, wsl: Option<&str>) -> impl FnMut(&Path, &[&str]) -> Option<String> {
        move |program, _| match program.file_name()?.to_str()? {
            "vswhere.exe" => vswhere.clone(),
            "wsl.exe" => wsl.map(str::to_string),
            other => panic!("ran {other}"),
        }
    }

    fn labels(shells: &Shells) -> Vec<&str> {
        shells.list.iter().map(|s| s.label.as_str()).collect()
    }

    #[test]
    fn shells_lists_pwsh_windows_powershell_cmd_dev_shells_and_wsl_distros() {
        let f = fake(true);
        touch(&f.vs.join(r"18\Common7\Tools\Microsoft.VisualStudio.DevShell.dll"));
        touch(&f.vs.join(r"18\Common7\Tools\VsDevCmd.bat"));
        touch(&f.vs.join(r"2022\Common7\Tools\VsDevCmd.bat"));
        let shells = detect_on(&f.machine, &mut tools(Some(vswhere_json(&f.vs)), Some(WSL_UTF16)));
        assert_eq!(
            labels(&shells),
            [
                "PowerShell",
                "Windows PowerShell",
                "Command Prompt",
                "Developer PowerShell for Visual Studio Community 2026",
                "Developer Command Prompt for Visual Studio Community 2026",
                "Developer Command Prompt for Visual Studio Community 2022",
                "Ubuntu"
            ]
        );
        let pwsh = f.machine.path.clone();
        let pwsh = Path::new(&pwsh).join("pwsh.exe");
        let cmd = f.root.join(r"System32\cmd.exe");
        let by = |label: &str| shells.list.iter().find(|s| s.label == label).unwrap().clone();
        assert_eq!(shells.auto, by("PowerShell"));
        assert_eq!((by("PowerShell").program, by("PowerShell").args), (pwsh.clone(), vec!["-ExecutionPolicy".to_string(), "Bypass".into()]));
        assert_eq!(by("Windows PowerShell").program, f.root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
        assert_eq!((by("Command Prompt").program, by("Command Prompt").args.len()), (cmd.clone(), 0));
        let dev = by("Developer PowerShell for Visual Studio Community 2026");
        let dll = f.vs.join(r"18\Common7\Tools\Microsoft.VisualStudio.DevShell.dll");
        assert_eq!(dev.program, pwsh);
        assert_eq!(
            dev.args[2..],
            ["-NoExit".to_string(), "-Command".into(), format!("&{{Import-Module '{}'; Enter-VsDevShell 95451c93 -SkipAutomaticLocation}}", dll.display())]
        );
        assert_eq!(dev.setting, None);
        let devcmd = by("Developer Command Prompt for Visual Studio Community 2022");
        assert_eq!((devcmd.program, devcmd.args), (cmd, vec!["/k".to_string(), f.vs.join(r"2022\Common7\Tools\VsDevCmd.bat").display().to_string()]));
        let ubuntu = by("Ubuntu");
        assert_eq!((ubuntu.program, ubuntu.args), (f.root.join(r"System32\wsl.exe"), vec!["-d".to_string(), "Ubuntu".into()]));
        assert_eq!(ubuntu.setting, Some(ShellSetting::Wsl("Ubuntu".into())));
        assert!(shells.list.iter().all(|s| s.program.is_absolute()));
    }

    #[test]
    fn shells_without_pwsh_auto_is_windows_powershell() {
        let f = fake(false);
        let shells = detect_on(&f.machine, &mut tools(None, None));
        assert_eq!(labels(&shells), ["Windows PowerShell", "Command Prompt"]);
        assert_eq!(shells.auto.label, "Windows PowerShell");
        assert_eq!(shells.auto.setting, Some(ShellSetting::PowerShell));
    }

    #[test]
    fn shells_with_vswhere_or_wsl_missing_or_failing_lists_the_rest() {
        let f = fake(true);
        touch(&f.vs.join(r"18\Common7\Tools\VsDevCmd.bat"));
        let base = ["PowerShell", "Windows PowerShell", "Command Prompt"];
        for (vswhere, wsl) in [(None, None), (Some("not json".to_string()), Some("")), (Some("{}".to_string()), Some("\r\n"))] {
            assert_eq!(labels(&detect_on(&f.machine, &mut tools(vswhere.clone(), wsl))), base, "{vswhere:?} {wsl:?}");
        }
        let shells = detect_on(&f.machine, &mut tools(Some(vswhere_json(&f.vs)), None));
        assert_eq!(labels(&shells)[3..], ["Developer Command Prompt for Visual Studio Community 2026"], "wsl missing, VS still listed");
        let shells = detect_on(&f.machine, &mut tools(None, Some(WSL_UTF16)));
        assert_eq!(labels(&shells)[3..], ["Ubuntu"], "vswhere missing, WSL still listed");
        let none = Machine { program_files_x86: None, ..f.machine.clone() };
        assert_eq!(labels(&detect_on(&none, &mut tools(None, None))), base);
    }

    #[test]
    fn shells_real_probe_of_a_missing_tool_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(probe(&tmp.path().join("vswhere.exe"), &[]), None);
    }

    #[test]
    fn shells_short_names_title_a_tab() {
        let f = fake(true);
        touch(&f.vs.join(r"18\Common7\Tools\Microsoft.VisualStudio.DevShell.dll"));
        touch(&f.vs.join(r"18\Common7\Tools\VsDevCmd.bat"));
        let shells = detect_on(&f.machine, &mut tools(Some(vswhere_json(&f.vs)), Some(WSL_UTF16)));
        let shorts: Vec<String> = shells.list.iter().map(Shell::short).collect();
        assert_eq!(shorts, ["pwsh", "powershell", "cmd", "Developer PowerShell", "Developer Command Prompt", "Ubuntu"]);
        let custom = Shell { label: "bash".into(), program: r"C:\Git\bin\bash.exe".into(), args: vec![], setting: Some(ShellSetting::Custom(r"C:\Git\bin\bash.exe".into())) };
        assert_eq!(custom.short(), "bash");
    }

    #[test]
    fn shells_wsl_list_hides_docker_desktop_in_either_encoding() {
        assert_eq!(parse_wsl(WSL_UTF16), ["Ubuntu"]);
        assert_eq!(parse_wsl("\u{feff}Ubuntu-22.04\r\ndocker-desktop\r\nDebian\r\n\r\nDocker-Desktop-Data\n"), ["Ubuntu-22.04", "Debian"]);
        assert!(parse_wsl("").is_empty());
    }

    #[test]
    fn shells_vswhere_parses_each_install_and_skips_the_incomplete() {
        let json = r#"[{"instanceId":"95451c93","installationPath":"C:\\VS\\18","displayName":"Visual Studio Community 2026","isPrerelease":false},
            {"instanceId":"abc","displayName":"no path"},
            {"instanceId":"x y; rm","installationPath":"C:\\VS\\bad","displayName":"bad id"},
            {"instanceId":"cafe0001","installationPath":"C:\\VS\\bt","displayName":"Visual Studio Build Tools 2026","isPrerelease":true}]"#;
        assert_eq!(
            parse_vswhere(json),
            [
                VsInstance { id: "95451c93".into(), path: r"C:\VS\18".into(), name: "Visual Studio Community 2026".into() },
                VsInstance { id: "cafe0001".into(), path: r"C:\VS\bt".into(), name: "Visual Studio Build Tools 2026 Preview".into() },
            ]
        );
        assert!(parse_vswhere("").is_empty() && parse_vswhere("{\"a\":1}").is_empty());
    }

    #[test]
    fn shells_dev_powershell_quotes_a_path_with_an_apostrophe() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("René's VS");
        touch(&path.join(r"Common7\Tools\Microsoft.VisualStudio.DevShell.dll"));
        let vs = VsInstance { id: "abc1".into(), path, name: "VS".into() };
        let shells = dev_shells(&vs, Path::new(r"C:\ps.exe"), Path::new(r"C:\cmd.exe"));
        assert_eq!(shells.len(), 1);
        assert!(shells[0].args.last().unwrap().contains("René''s VS"), "{:?}", shells[0].args);
    }

    #[test]
    fn shells_setting_parses_every_form_and_writes_it_back() {
        for (text, want) in [
            ("auto", ShellSetting::Auto),
            (" PWSH ", ShellSetting::Pwsh),
            ("powershell", ShellSetting::PowerShell),
            ("cmd", ShellSetting::Cmd),
            ("wsl:Ubuntu", ShellSetting::Wsl("Ubuntu".into())),
            ("WSL: Ubuntu-22.04", ShellSetting::Wsl("Ubuntu-22.04".into())),
            (r"C:\Program Files\Git\bin\bash.exe", ShellSetting::Custom(r"C:\Program Files\Git\bin\bash.exe".into())),
            (r"%ProgramFiles%\Git\bin\bash.exe", ShellSetting::Custom(r"%ProgramFiles%\Git\bin\bash.exe".into())),
        ] {
            let got: ShellSetting = text.parse().unwrap();
            assert_eq!(got, want, "{text}");
            assert_eq!(got.to_string().parse::<ShellSetting>().unwrap(), want, "{text} round trips");
        }
        for bad in ["", "bash", "zsh.exe", "wsl:", "wsl:  "] {
            assert!(bad.parse::<ShellSetting>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn shells_resolve_falls_back_to_auto_with_a_notice() {
        let f = fake(false);
        let shells = detect_on(&f.machine, &mut tools(None, Some("Ubuntu\n")));
        let resolve = |s: &str, args: &[&str]| shells.resolve(&s.parse().unwrap(), &args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert_eq!(resolve("auto", &[]), (shells.auto.clone(), None));
        assert_eq!(resolve("cmd", &[]).0.label, "Command Prompt");
        assert_eq!(resolve("wsl:ubuntu", &[]), (shells.list.last().unwrap().clone(), None), "a distro in any case");
        for (bad, why) in [("pwsh", "pwsh is not installed"), ("wsl:Arch", "wsl:Arch is not an installed WSL distro"), (r"C:\nowhere\sh.exe", "is not a file")] {
            let (shell, notice) = resolve(bad, &[]);
            assert_eq!(shell, shells.auto, "{bad}");
            let notice = notice.unwrap_or_default();
            assert!(notice.contains(why) && notice.ends_with("Windows PowerShell is used"), "{bad}: {notice}");
        }
        let custom = f.root.join(r"Git\bash.exe");
        touch(&custom);
        let (shell, notice) = resolve(&custom.display().to_string(), &["--login", "-i"]);
        assert_eq!((shell.label.as_str(), shell.program, shell.args, notice), ("bash", custom, vec!["--login".to_string(), "-i".into()], None));
        let (shell, notice) = resolve("cmd", &["-x"]);
        assert_eq!(shell.label, "Command Prompt");
        assert!(notice.is_some_and(|n| n.contains("only to a custom shell")));
    }
}
