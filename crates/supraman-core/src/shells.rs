//! This machine's shells for a plain terminal tab, and `[terminal] shell` resolved against them.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use crate::platform::shell::expand_env;

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

/// What detection found: every shell in menu order, and the one `auto` starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shells {
    pub list: Vec<Shell>,
    /// pwsh, else Windows PowerShell, listed or not: what a tab starts when nothing else will do.
    pub auto: Shell,
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


}
