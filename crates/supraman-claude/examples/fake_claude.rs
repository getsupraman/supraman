//! A stand-in `claude` for `tests/fake_claude.rs`. Appends `cwd`, every argument and `FAKE_SEEN`
//! to `$FAKE_CLAUDE_LOG` (one per line), then answers like the real CLI per `$FAKE_CLAUDE_MODE`.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(log) = std::env::var_os("FAKE_CLAUDE_LOG") {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log).unwrap();
        writeln!(f, "cwd={}", std::env::current_dir().unwrap().display()).unwrap();
        for a in &args {
            writeln!(f, "arg={a}").unwrap();
        }
        writeln!(f, "FAKE_SEEN={}", std::env::var("FAKE_SEEN").unwrap_or_default()).unwrap();
        writeln!(f, "--").unwrap();
    }
    let mode = std::env::var("FAKE_CLAUDE_MODE").unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("agents") => {
            print!("{}", std::env::var("FAKE_CLAUDE_ROWS").unwrap_or_else(|_| "[]".into()));
            ExitCode::SUCCESS
        }
        Some("stop") => {
            println!("stopped {}", args[1]);
            ExitCode::SUCCESS
        }
        Some("rm") => {
            println!("removed {}", args[1]);
            ExitCode::SUCCESS
        }
        _ if mode == "trust" => {
            let cwd = std::env::current_dir().unwrap();
            eprintln!("Workspace not trusted. Run `claude` in {} once and accept the trust prompt, then retry.", cwd.display());
            ExitCode::from(1)
        }
        _ if mode == "fail" => {
            eprintln!("boom");
            ExitCode::from(7)
        }
        _ => {
            let name = args.iter().skip_while(|a| *a != "--name").nth(1).cloned().unwrap_or_default();
            println!("backgrounded · abcd1234 · {name}");
            ExitCode::SUCCESS
        }
    }
}
