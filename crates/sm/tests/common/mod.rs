/// A pid no process runs under; a fixed one can alias a live process, as Windows ignores a pid's low two bits.
pub fn exited_pid() -> u32 {
    let exe = std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into());
    let mut child = supraman_core::process::command(exe).args(["/c", "exit"]).spawn().unwrap();
    child.wait().unwrap();
    let pid = child.id();
    drop(child);
    assert!(!supraman_core::process::alive(pid), "pid {pid} runs again");
    pid
}
