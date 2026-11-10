use std::ffi::OsStr;
use std::path::Path;

/// `ShellExecuteW(verb, target, args, cwd)`; `Err` carries its return code (`<= 32`; 5 is a declined UAC prompt).
#[cfg(windows)]
fn shell_execute(verb: &str, target: &OsStr, args: Option<&OsStr>, cwd: Option<&Path>) -> Result<(), isize> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR};
    let (verb, target) = (HSTRING::from(verb), HSTRING::from(target));
    let args = args.map(HSTRING::from);
    let cwd = cwd.map(|c| HSTRING::from(c.as_os_str()));
    let ptr = |h: &Option<HSTRING>| h.as_ref().map_or(PCWSTR::null(), |h| PCWSTR(h.as_ptr()));
    // SAFETY: every string is an HSTRING that outlives the call.
    let code = unsafe { ShellExecuteW(None, &verb, &target, ptr(&args), ptr(&cwd), SW_SHOWNORMAL) }.0 as isize;
    if code > 32 { Ok(()) } else { Err(code) }
}

/// Opens `target` with its handler; blocking (ShellExecuteW pumps its thread's messages).
#[cfg(windows)]
pub fn open(target: &OsStr) -> Result<(), String> {
    shell_execute("open", target, None, None).map_err(|code| format!("ShellExecuteW returned {code}"))
}

#[cfg(not(windows))]
pub fn open(_target: &OsStr) -> Result<(), String> {
    Err(super::refused("opening with its handler"))
}

/// Starts `exe` with `args` in `cwd` elevated, behind a UAC prompt; `Err` when the start is refused or declined.
#[cfg(windows)]
pub fn runas(exe: &Path, args: &OsStr, cwd: &Path) -> Result<(), String> {
    shell_execute("runas", exe.as_os_str(), Some(args), Some(cwd))
        .map_err(|code| format!("the elevated start was refused (ShellExecuteW runas returned {code}; 5 is a declined UAC prompt)"))
}

#[cfg(not(windows))]
pub fn runas(_exe: &Path, _args: &OsStr, _cwd: &Path) -> Result<(), String> {
    Err(super::refused("an elevated start"))
}

/// Extensions of a file that runs, scripts, installs, links or registers something; `PATHEXT`'s are added to them.
const RUNS: [&str; 50] = [
    "exe", "com", "bat", "cmd", "ps1", "psm1", "psd1", "ps1xml", "psc1", "vb", "vbs", "vbe", "js", "jse", "ws", "wsf", "wsh",
    "hta", "msi", "msp", "mst", "msc", "msix", "msixbundle", "appx", "appxbundle", "appinstaller", "cpl", "scr", "pif",
    "lnk", "url", "website", "scf", "reg", "inf", "application", "appref-ms", "settingcontent-ms", "library-ms",
    "searchconnector-ms", "diagcab", "chm", "gadget", "jar", "py", "pyw", "pyz", "sh", "xll",
];

/// The name Windows opens for `name`: an alternate stream's `:` part and trailing dots and spaces dropped.
fn effective_name(name: &str) -> &str {
    name.split(':').next().unwrap_or(name).trim_end_matches(['.', ' '])
}

/// `Some(true)` when `name`'s extension runs something, `Some(false)` when it is a document's, `None` without one.
pub fn runs(name: &str, pathext: &str) -> Option<bool> {
    let (_, ext) = effective_name(name).rsplit_once('.')?;
    let pathext = crate::paths::pathext_list(pathext).map(|e| e.trim_start_matches('.'));
    Some(RUNS.into_iter().chain(pathext).any(|known| known.eq_ignore_ascii_case(ext)))
}
