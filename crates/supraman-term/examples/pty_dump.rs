// Spawns a program in a ConPTY and prints every byte it writes with escape sequences made
// visible (which terminal modes it and conhost request), and the rendered screen after each input.
// usage: pty_dump <secs> <program> [args...]
//   PTY_DUMP_SEND  `|`-separated inputs (\e \r \n \xNN escapes), written after <secs>, 1.5 s apart
//   PTY_DUMP_RAW   set to also print the raw output stream
use alacritty_terminal::event::{VoidListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty::{self, EventedReadWrite, Options, Shell};
use alacritty_terminal::vte::ansi::Processor;
use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant};

const LINES: usize = 30;
const COLS: usize = 100;

struct Size;
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        LINES
    }
    fn screen_lines(&self) -> usize {
        LINES
    }
    fn columns(&self) -> usize {
        COLS
    }
}

fn visible(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        match b {
            0x1b => out.push_str("\\e"),
            b'\r' => out.push_str("\\r"),
            b'\n' => out.push_str("\\n\n"),
            0x07 => out.push_str("\\a"),
            0x20..=0x7e => out.push(b as char),
            _ => out.push_str(&format!("\\x{b:02x}")),
        }
    }
    out
}

fn unescape(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            match b[i + 1] {
                b'e' => out.push(0x1b),
                b'r' => out.push(b'\r'),
                b'n' => out.push(b'\n'),
                b'x' => {
                    out.push(u8::from_str_radix(&s[i + 2..i + 4], 16).unwrap());
                    i += 2;
                }
                c => out.push(c),
            }
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

struct Probe {
    pty: tty::Pty,
    term: Term<VoidListener>,
    parser: Processor,
    raw: bool,
}

impl Probe {
    fn pump(&mut self, for_: Duration) {
        let until = Instant::now() + for_;
        let mut buf = [0u8; 65536];
        while Instant::now() < until {
            match self.pty.reader().read(&mut buf) {
                Ok(0) => std::thread::sleep(Duration::from_millis(20)),
                Ok(n) => {
                    if self.raw {
                        print!("{}", visible(&buf[..n]));
                    }
                    self.parser.advance(&mut self.term, &buf[..n]);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(e) => {
                    println!("\n[read error {e}]");
                    return;
                }
            }
        }
    }

    fn screen(&self, from: usize) {
        println!("\n[screen, cursor {:?}]", self.term.grid().cursor.point);
        for line in from..LINES {
            let text = self.term.bounds_to_string(
                Point::new(Line(line as i32), Column(0)),
                Point::new(Line(line as i32), Column(COLS - 1)),
            );
            let text = text.trim_end();
            if !text.is_empty() {
                println!("{line:2}| {text}");
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let secs: u64 = args[0].parse().unwrap();
    let options = Options {
        shell: Some(Shell::new(args[1].clone(), args[2..].to_vec())),
        working_directory: std::env::current_dir().ok(),
        ..Default::default()
    };
    let size = WindowSize { num_lines: LINES as u16, num_cols: COLS as u16, cell_width: 8, cell_height: 18 };
    let pty = tty::new(&options, size, 0).expect("spawn");
    println!("[pid {:?}]", pty.child_watcher().pid());
    let mut probe = Probe {
        pty,
        term: Term::new(Config::default(), &Size, VoidListener),
        parser: Processor::new(),
        raw: std::env::var("PTY_DUMP_RAW").is_ok(),
    };
    probe.pump(Duration::from_secs(secs));
    probe.screen(LINES - 8);
    if let Ok(send) = std::env::var("PTY_DUMP_SEND") {
        for item in send.split('|') {
            let bytes = unescape(item);
            println!("\n[send {}]", visible(&bytes));
            probe.pty.writer().write_all(&bytes).unwrap();
            probe.pty.writer().flush().unwrap();
            probe.pump(Duration::from_millis(1500));
            probe.screen(LINES - 8);
        }
    }
    probe.pump(Duration::from_secs(2));
    println!("\n[end]");
}
