//! A stand-in native claude for hand.rs's launch tests: appends its argv as one JSON line to `$SUPRAMAN_ARGV_OUT`.

use std::io::Write as _;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let out = std::env::var_os("SUPRAMAN_ARGV_OUT").expect("SUPRAMAN_ARGV_OUT");
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(out).expect("open SUPRAMAN_ARGV_OUT");
    writeln!(file, "{}", serde_json::to_string(&argv).unwrap()).unwrap();
    println!("fake claude ran");
    std::process::exit(3);
}
