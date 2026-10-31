//! The windowless `sm` a schedule's task runs (`docs/research/scheduler.md`, "No console window").
#![windows_subsystem = "windows"]

fn main() -> std::process::ExitCode {
    sm::smw(std::env::args().skip(1).collect())
}
