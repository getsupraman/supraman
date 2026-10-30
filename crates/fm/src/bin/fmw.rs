//! The windowless `fm` a schedule's task runs (`docs/research/scheduler.md`, "No console window").
#![windows_subsystem = "windows"]

fn main() -> std::process::ExitCode {
    fm::fmw(std::env::args().skip(1).collect())
}
