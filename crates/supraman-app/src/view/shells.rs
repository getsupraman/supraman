//! Plain terminal tabs: the `[terminal] shell` default, the ⌄ menu of this machine's shells, and the command
//! a tab starts. A tab that starts or attaches claude never comes here: it runs PowerShell (C113).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{AppContext as _, Context, SharedString};
use supraman_core::shells::{Shell, ShellSetting, Shells};
use supraman_core::{Settings, TerminalSettings};
use supraman_ui::menu::item;

use super::menu::{ContextMenu, MenuAction, MenuItem};
use super::{FleetView, TabKind};
use crate::saved_tabs::SavedShell;

/// What a tab of `shell` starts; a plain PowerShell takes `powershell` (the view's `shell_args`).
pub(crate) fn command(shell: &Shell, powershell: &[String]) -> SavedShell {
    let args = match shell.setting {
        Some(ShellSetting::Pwsh | ShellSetting::PowerShell) => powershell_args(powershell.to_vec()),
        _ => cmd_quoted(&shell.program, &shell.args),
    };
    SavedShell { program: shell.program.clone(), args }
}

/// The PTY passes cmd's arguments unescaped (`ShellKind::Cmd`), so an argument with a space is quoted here.
pub(crate) fn cmd_quoted(program: &Path, args: &[String]) -> Vec<String> {
    let cmd = program.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("cmd"));
    args.iter().map(|a| if cmd && a.contains(char::is_whitespace) && !a.starts_with('"') { format!("\"{a}\"") } else { a.clone() }).collect()
}

pub(crate) fn pty(shell: &SavedShell) -> util::shell::Shell {
    util::shell::Shell::WithArguments { program: shell.program.to_string_lossy().into_owned(), args: shell.args.clone(), title_override: None }
}

fn group(shell: &Shell) -> &'static str {
    match &shell.setting {
        Some(ShellSetting::Pwsh | ShellSetting::PowerShell | ShellSetting::Auto) => "PowerShell",
        Some(ShellSetting::Cmd) => "Command Prompt",
        Some(ShellSetting::Wsl(_)) => "WSL",
        Some(ShellSetting::Custom(_)) => "[terminal] shell",
        None => "Visual Studio",
    }
}

fn key(shell: &Shell, slot: &mut usize) -> SharedString {
    match &shell.setting {
        Some(ShellSetting::Pwsh) => "shell-pwsh".into(),
        Some(ShellSetting::PowerShell) => "shell-powershell".into(),
        Some(ShellSetting::Cmd) => "shell-cmd".into(),
        Some(ShellSetting::Custom(_)) => "shell-custom".into(),
        _ => {
            *slot += 1;
            format!("shell-{}", *slot - 1).into()
        }
    }
}

/// The ⌄ menu: every shell in `shells`' order under its group's caption, `default` marked (and listed last
/// when it is a custom shell).
pub(crate) fn menu_items(shells: &Shells, default: &Shell) -> Vec<MenuItem> {
    let custom = (!shells.list.contains(default)).then_some(default);
    let (mut slot, mut last) = (0, None);
    let mut items = Vec::new();
    for shell in shells.list.iter().chain(custom) {
        let key = key(shell, &mut slot);
        let label = if shell == default { format!("{}  (default)", shell.label) } else { shell.label.clone() };
        let mut entry = item(key, label, MenuAction::NewShell(shell.clone()));
        if last != Some(group(shell)) {
            last = Some(group(shell));
            entry = entry.under(group(shell));
        }
        items.push(entry);
    }
    items
}

/// This machine's shells once detected, and the `[terminal]` table + and the ⌄ menu start from, read off the UI
/// thread: at start, by the ⌄ menu, from the Settings window, and from each supervisor tick, whose value counts only
/// when it changed since the last tick's. The default is resolved again only when either changes.
#[derive(Debug, Default)]
pub(crate) struct ShellDefault {
    found: Option<Arc<Shells>>,
    terminal: TerminalSettings,
    ticked: Option<TerminalSettings>,
    resolved: Option<(Shell, Option<String>)>,
}

impl ShellDefault {
    pub(crate) fn new(terminal: TerminalSettings) -> Self {
        Self { terminal, ..Self::default() }
    }

    pub(crate) fn found(&self) -> Option<&Arc<Shells>> {
        self.found.as_ref()
    }

    /// The default shell and the notice when `auto` stands in for it; `None` while detection runs.
    pub(crate) fn resolved(&self) -> Option<&(Shell, Option<String>)> {
        self.resolved.as_ref()
    }

    pub(crate) fn detected(&mut self, shells: Shells) {
        self.found = Some(Arc::new(shells));
        self.resolve();
    }

    pub(crate) fn read(&mut self, terminal: TerminalSettings) {
        if self.terminal != terminal {
            self.terminal = terminal;
            self.resolve();
        }
    }

    pub(crate) fn ticked(&mut self, terminal: TerminalSettings) {
        if self.ticked.as_ref() != Some(&terminal) {
            self.read(terminal.clone());
        }
        self.ticked = Some(terminal);
    }

    fn resolve(&mut self) {
        self.resolved = self.found.as_ref().map(|shells| shells.for_settings(&self.terminal));
    }
}

impl FleetView {
    pub(super) fn detect_shells(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let shells = cx.background_spawn(async { supraman_core::platform::shell::detect() }).await;
            this.update(cx, |this, cx| {
                this.shell_default.detected(shells);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The `[terminal] shell` default; its notice joins the tab notices. PowerShell while detection runs.
    fn default_of(&mut self) -> Option<Shell> {
        let (shell, notice) = self.shell_default.resolved()?.clone();
        if let Some(notice) = notice
            && !self.tab_notices.contains(&notice)
        {
            self.tab_notices.push(notice);
        }
        Some(shell)
    }

    /// The default shell's short name and command.
    pub(crate) fn default_shell(&mut self) -> (String, SavedShell) {
        match self.default_of() {
            Some(shell) => (shell.short(), command(&shell, &self.shell_args)),
            None => {
                let ps = supraman_core::platform::shell::terminal_powershell();
                (ps.name.to_string(), SavedShell { program: ps.path.clone(), args: powershell_args(self.shell_args.clone()) })
            }
        }
    }

    pub(crate) fn open_plain(&mut self, title: String, cwd: Option<PathBuf>, shell: SavedShell, cx: &mut Context<Self>) -> u64 {
        let id = self.push_tab(title, TabKind::Shell, cwd.clone(), cx);
        self.start_terminal(id, pty(&shell), cwd, cx);
        id
    }

    /// A tab of `shell` where + opens one, titled like + titles it.
    pub fn new_shell_of(&mut self, shell: &Shell, cx: &mut Context<Self>) -> u64 {
        let title = format!("{} {}", shell.short(), self.next_tab_id);
        let cwd = self.new_shell_cwd();
        self.open_plain(title, cwd, command(shell, &self.shell_args), cx)
    }

    /// Shows the ⌄ menu once `settings.toml` is read again, so its default is the file's.
    pub fn open_shell_menu(&mut self, cx: &mut Context<Self>) {
        let home = self.home.clone();
        let read = cx.background_spawn(async move { home.and_then(|h| Settings::load(&h).ok()).unwrap_or_default().terminal });
        cx.spawn(async move |this, cx| {
            let terminal = read.await;
            this.update(cx, |this, cx| {
                this.shell_default.read(terminal);
                this.show_shell_menu(cx);
            })
            .ok();
        })
        .detach();
    }

    fn show_shell_menu(&mut self, cx: &mut Context<Self>) {
        let Some(default) = self.default_of() else {
            self.pane_toast.show_why("Still finding this machine's shells.".into(), |v: &mut Self| &mut v.pane_toast, cx);
            return;
        };
        let shells = self.shell_default.found().cloned().expect("default_of found the shells");
        let position = self.hits.borrow().get("new-shell-menu").map(|h| h.bounds.bottom_left()).unwrap_or_default();
        self.menu = Some(ContextMenu { title: "New shell".into(), position, items: menu_items(&shells, &default) });
        cx.notify();
    }
}

pub(super) fn powershell(args: Vec<String>) -> util::shell::Shell {
    util::shell::Shell::WithArguments { program: supraman_core::platform::shell::terminal_powershell().path.to_string_lossy().into_owned(), args: powershell_args(args), title_override: None }
}

/// The policy is the process's own: a `claude.ps1` first on PATH runs under a Restricted machine policy.
pub(crate) fn powershell_args(args: Vec<String>) -> Vec<String> {
    ["-ExecutionPolicy".to_string(), "Bypass".into()].into_iter().chain(args).collect()
}

/// The file the launch's pwsh runs for `command`: the path of `& '<path>' …`, else a bare name looked up as pwsh does.
pub(super) fn launched_file(command: &str, path: &std::ffi::OsStr, pathext: &str) -> Option<PathBuf> {
    let command = command.trim();
    if let Some(quoted) = command.strip_prefix("& '") {
        return quoted.split_once('\'').map(|(p, _)| PathBuf::from(p));
    }
    let bare = !command.is_empty() && command.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    bare.then(|| supraman_core::paths::pwsh_command(command, path, pathext)).flatten()
}

/// Why `prompt` cannot go to claude as `file`: cmd.exe ends a `.cmd`'s command line at a new line and re-reads its quotes.
pub(super) fn cmd_refusal(file: Option<&Path>, prompt: &str) -> Option<String> {
    let file = file?;
    let ext = file.extension()?.to_string_lossy().to_ascii_lowercase();
    if !matches!(ext.as_str(), "cmd" | "bat") || !prompt.contains(['\n', '"']) {
        return None;
    }
    Some(format!(
        "Not sent: claude here is {}, which runs through cmd.exe and cuts the text at a new line and mangles double quotes. \
         Install the native claude.exe (claude.ai/install), which carries any text, or write the prompt on one line without \".",
        file.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(label: &str, program: &str, args: &[&str], setting: Option<ShellSetting>) -> Shell {
        Shell { label: label.into(), program: program.into(), args: args.iter().map(|a| a.to_string()).collect(), setting }
    }

    fn machine() -> Shells {
        let pwsh = shell("PowerShell", r"C:\pwsh\pwsh.exe", &["-ExecutionPolicy", "Bypass"], Some(ShellSetting::Pwsh));
        let list = vec![
            pwsh.clone(),
            shell("Windows PowerShell", r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe", &["-ExecutionPolicy", "Bypass"], Some(ShellSetting::PowerShell)),
            shell("Command Prompt", r"C:\Windows\System32\cmd.exe", &[], Some(ShellSetting::Cmd)),
            shell("Developer PowerShell for VS 18", r"C:\pwsh\pwsh.exe", &["-NoExit"], None),
            shell("Developer Command Prompt for VS 18", r"C:\Windows\System32\cmd.exe", &["/k", r"C:\Program Files\VS 18\Common7\Tools\VsDevCmd.bat"], None),
            shell("Ubuntu", r"C:\Windows\System32\wsl.exe", &["-d", "Ubuntu"], Some(ShellSetting::Wsl("Ubuntu".into()))),
        ];
        Shells { list, auto: pwsh }
    }

    #[test]
    fn the_shell_menu_lists_every_shell_in_order_under_captions_with_the_default_marked() {
        let shells = machine();
        let items = menu_items(&shells, &shells.list[2]);
        let keys: Vec<&str> = items.iter().map(|i| &*i.key).collect();
        assert_eq!(keys, ["shell-pwsh", "shell-powershell", "shell-cmd", "shell-0", "shell-1", "shell-2"]);
        let captions: Vec<(&str, &str)> = items.iter().filter_map(|i| i.section.as_deref().map(|c| (&*i.key, c))).collect();
        assert_eq!(captions, [("shell-pwsh", "PowerShell"), ("shell-cmd", "Command Prompt"), ("shell-0", "Visual Studio"), ("shell-2", "WSL")]);
        assert_eq!(items[2].label, "Command Prompt  (default)");
        assert!(items.iter().filter(|i| i.label.ends_with("(default)")).count() == 1);
        assert_eq!(items[5].action, MenuAction::NewShell(shells.list[5].clone()));

        let bash = shell("bash", r"C:\Git\bin\bash.exe", &["--login"], Some(ShellSetting::Custom(r"C:\Git\bin\bash.exe".into())));
        let items = menu_items(&shells, &bash);
        let last = items.last().unwrap();
        assert_eq!((&*last.key, last.label.as_str(), last.section.as_deref()), ("shell-custom", "bash  (default)", Some("[terminal] shell")));
    }

    #[test]
    fn the_shell_menu_lists_more_unnamed_shells_than_any_fixed_count() {
        let distros: Vec<Shell> = (0..40).map(|n| shell(&format!("Distro {n}"), r"C:\Windows\System32\wsl.exe", &["-d"], Some(ShellSetting::Wsl(format!("d{n}"))))).collect();
        let shells = Shells { list: distros, auto: machine().auto };
        let items = menu_items(&shells, &shells.auto);
        let keys: std::collections::HashSet<String> = items.iter().map(|i| i.key.to_string()).collect();
        assert_eq!((items.len(), keys.len()), (41, 41), "every distro and the default, each under its own key");
        assert_eq!(items[39].label, "Distro 39");
    }

    #[test]
    fn a_ticks_stale_terminal_table_never_overrides_a_fresher_read() {
        let cmd = TerminalSettings { shell: Some(ShellSetting::Cmd), ..Default::default() };
        let mut default = ShellDefault::new(TerminalSettings::default());
        default.ticked(TerminalSettings::default());
        default.read(cmd.clone());
        default.ticked(TerminalSettings::default());
        assert_eq!(default.terminal, cmd, "the tick read the file before the change");
        default.ticked(cmd.clone());
        default.ticked(TerminalSettings::default());
        assert_eq!(default.terminal, TerminalSettings::default(), "a change by hand reaches + with the next tick");
    }

    #[test]
    fn a_plain_powershell_takes_the_views_args_and_cmd_gets_a_spaced_path_whole() {
        let shells = machine();
        let ps = command(&shells.list[0], &["-NoLogo".to_string()]);
        assert_eq!((ps.program, ps.args), (PathBuf::from(r"C:\pwsh\pwsh.exe"), vec!["-ExecutionPolicy".to_string(), "Bypass".into(), "-NoLogo".into()]));
        let dev = command(&shells.list[4], &["-NoLogo".to_string()]);
        assert_eq!(dev.args, ["/k", r#""C:\Program Files\VS 18\Common7\Tools\VsDevCmd.bat""#]);
        assert_eq!(command(&shells.list[5], &[]).args, ["-d", "Ubuntu"]);
        assert_eq!(cmd_quoted(Path::new(r"C:\x\CMD.EXE"), &[r#""C:\a b\x.bat""#.into()]), [r#""C:\a b\x.bat""#], "already quoted");
        assert_eq!(cmd_quoted(Path::new(r"C:\x\bash.exe"), &["a b".into()]), ["a b"], "the PTY escapes every other shell's");
    }

    /// cmd reads the line as the PTY writes it for cmd: the program, then each argument as is.
    #[test]
    fn the_developer_command_prompts_path_with_spaces_reaches_cmd_whole() {
        use std::os::windows::process::CommandExt as _;
        let dir = tempfile::tempdir().unwrap();
        let bat = dir.path().join(r"Program Files\VS 18\Common7\Tools\VsDevCmd.bat");
        std::fs::create_dir_all(bat.parent().unwrap()).unwrap();
        std::fs::write(&bat, "@echo fm-devcmd-ran\r\n").unwrap();
        let cmd_exe = Path::new(&std::env::var_os("SystemRoot").unwrap()).join(r"System32\cmd.exe");
        let dev = shell("Developer Command Prompt for VS 18", &cmd_exe.display().to_string(), &["/k", &bat.display().to_string()], None);
        let started = command(&dev, &[]);
        let mut child = supraman_core::process::command(&started.program);
        child.raw_arg(started.args.join(" "));
        let out = supraman_core::process::run(&mut child, std::time::Duration::from_secs(20), supraman_core::process::Kill::Tree).unwrap();
        assert!(out.stdout.contains("fm-devcmd-ran"), "stdout {:?} stderr {:?}", out.stdout, out.stderr);
    }

    #[test]
    fn a_claude_cmd_refuses_a_new_line_or_a_double_quote_and_names_claude_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let (npm, native) = (tmp.path().join("npm"), tmp.path().join("native"));
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(npm.join("claude.cmd"), "").unwrap();
        std::fs::write(native.join("claude.exe"), "").unwrap();
        let ext = ".COM;.EXE;.BAT;.CMD";
        let only_cmd = std::env::join_paths([&npm]).unwrap();
        let cmd = launched_file("claude", &only_cmd, ext);
        assert_eq!(cmd, Some(npm.join("claude.cmd")));
        let why = cmd_refusal(cmd.as_deref(), "two\nlines").unwrap();
        assert!(why.contains("claude.cmd") && why.contains("the native claude.exe") && why.contains("carries any text"), "{why}");
        assert!(cmd_refusal(cmd.as_deref(), "say \"hi\"").is_some());
        assert_eq!(cmd_refusal(cmd.as_deref(), "one line, no quotes"), None);
        let exe = launched_file("claude", &std::env::join_paths([&native, &npm]).unwrap(), ext);
        assert_eq!(cmd_refusal(exe.as_deref(), "two\n\"lines\""), None);
        assert_eq!(cmd_refusal(launched_file("claude", &std::ffi::OsString::new(), ext).as_deref(), "a\nb"), None, "no claude: the launch says so");
        assert_eq!(launched_file(r"& 'C:\x\claude.cmd' --selftest-argv 'out'", &only_cmd, ext), Some(PathBuf::from(r"C:\x\claude.cmd")));
    }
}
