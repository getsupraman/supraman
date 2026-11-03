//! Plain terminal tabs: the `[terminal] shell` default, the ⌄ menu of this machine's shells, and the command
//! a tab starts. A tab that starts or attaches claude never comes here: it runs PowerShell (C113).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{AppContext as _, Context};
use supraman_core::Settings;
use supraman_core::shells::{Shell, ShellSetting, Shells};
use supraman_ui::menu::item;

use super::menu::{ContextMenu, MenuAction, MenuItem};
use super::{FleetView, TabKind, powershell_args};
use crate::saved_tabs::SavedShell;

/// Menu keys of the shells that no setting names; a machine with more lists the first of them.
const SLOTS: [&str; 24] = [
    "shell-0", "shell-1", "shell-2", "shell-3", "shell-4", "shell-5", "shell-6", "shell-7", "shell-8", "shell-9", "shell-10", "shell-11", "shell-12",
    "shell-13", "shell-14", "shell-15", "shell-16", "shell-17", "shell-18", "shell-19", "shell-20", "shell-21", "shell-22", "shell-23",
];

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

fn key(shell: &Shell, slot: &mut usize) -> Option<&'static str> {
    Some(match &shell.setting {
        Some(ShellSetting::Pwsh) => "shell-pwsh",
        Some(ShellSetting::PowerShell) => "shell-powershell",
        Some(ShellSetting::Cmd) => "shell-cmd",
        Some(ShellSetting::Custom(_)) => "shell-custom",
        _ => {
            let key = SLOTS.get(*slot)?;
            *slot += 1;
            key
        }
    })
}

/// The ⌄ menu: every shell in `shells`' order under its group's caption, `default` marked (and listed last
/// when it is a custom shell).
pub(crate) fn menu_items(shells: &Shells, default: &Shell) -> Vec<MenuItem> {
    let custom = (!shells.list.contains(default)).then_some(default);
    let (mut slot, mut last) = (0, None);
    let mut items = Vec::new();
    for shell in shells.list.iter().chain(custom) {
        let Some(key) = key(shell, &mut slot) else { continue };
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

impl FleetView {
    pub(super) fn detect_shells(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let shells = cx.background_spawn(async { supraman_core::shells::detect() }).await;
            this.update(cx, |this, cx| {
                this.shells = Some(Arc::new(shells));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The `[terminal] shell` default; its notice joins the tab notices. PowerShell while detection runs.
    fn default_of(&mut self) -> Option<Shell> {
        let shells = self.shells.clone()?;
        let terminal = self.home.as_deref().and_then(|h| Settings::load(h).ok()).unwrap_or_default().terminal;
        let (shell, notice) = shells.for_settings(&terminal);
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
                let ps = supraman_core::paths::terminal_powershell();
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

    pub fn open_shell_menu(&mut self, cx: &mut Context<Self>) {
        let Some(default) = self.default_of() else {
            self.pane_toast.show_why("Still finding this machine's shells.".into(),|v: &mut Self| &mut v.pane_toast, cx);
            return;
        };
        let shells = self.shells.clone().expect("default_of found the shells");
        let position = self.hits.borrow().get("new-shell-menu").map(|h| h.bounds.bottom_left()).unwrap_or_default();
        self.menu = Some(ContextMenu { title: "New shell".into(), position, items: menu_items(&shells, &default) });
        cx.notify();
    }
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
        let keys: Vec<&str> = items.iter().map(|i| i.key).collect();
        assert_eq!(keys, ["shell-pwsh", "shell-powershell", "shell-cmd", "shell-0", "shell-1", "shell-2"]);
        let captions: Vec<(&str, &str)> = items.iter().filter_map(|i| i.section.as_deref().map(|c| (i.key, c))).collect();
        assert_eq!(captions, [("shell-pwsh", "PowerShell"), ("shell-cmd", "Command Prompt"), ("shell-0", "Visual Studio"), ("shell-2", "WSL")]);
        assert_eq!(items[2].label, "Command Prompt  (default)");
        assert!(items.iter().filter(|i| i.label.ends_with("(default)")).count() == 1);
        assert_eq!(items[5].action, MenuAction::NewShell(shells.list[5].clone()));

        let bash = shell("bash", r"C:\Git\bin\bash.exe", &["--login"], Some(ShellSetting::Custom(r"C:\Git\bin\bash.exe".into())));
        let items = menu_items(&shells, &bash);
        let last = items.last().unwrap();
        assert_eq!((last.key, last.label.as_str(), last.section.as_deref()), ("shell-custom", "bash  (default)", Some("[terminal] shell")));
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
}
