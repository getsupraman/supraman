//! Alone in its binary: the process's thread and handle counts are only this test's.

#![cfg(windows)]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use supraman_core::watch::{Watch, Watcher};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetProcessHandleCount};

const WITHIN: Duration = Duration::from_secs(5);

fn threads() -> usize {
    let pid = unsafe { GetCurrentProcessId() };
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.expect("snapshot");
    let mut entry = THREADENTRY32 { dwSize: size_of::<THREADENTRY32>() as u32, ..Default::default() };
    let mut n = 0;
    let mut more = unsafe { Thread32First(snap, &mut entry) }.is_ok();
    while more {
        n += usize::from(entry.th32OwnerProcessID == pid);
        more = unsafe { Thread32Next(snap, &mut entry) }.is_ok();
    }
    unsafe { CloseHandle(snap) }.ok();
    n
}

fn handles() -> u32 {
    let mut n = 0;
    unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut n) }.expect("handle count");
    n
}

/// One watcher seen working, then dropped and seen stopped.
fn cycle(dir: &std::path::Path, file: &str) {
    let (tx, rx) = mpsc::channel();
    let w = Watcher::new(&[Watch::tree(dir)], Duration::from_millis(20), move |b| {
        tx.send(b).ok();
    })
    .unwrap();
    let stopped = w.stopped();
    std::fs::write(dir.join(file), b"x").unwrap();
    rx.recv_timeout(WITHIN).expect("a batch");
    drop(w);
    assert!(stopped.wait(WITHIN), "stopped after the drop");
}

/// The counts back at `want` within `WITHIN`: a thread that has let its state go still takes a moment to exit.
fn settles_at(want: (usize, u32)) -> (usize, u32) {
    let deadline = Instant::now() + WITHIN;
    loop {
        let now = (threads(), handles());
        if now == want || Instant::now() > deadline {
            return now;
        }
        std::thread::yield_now();
    }
}

#[test]
fn watch_leaves_no_thread_or_handle_after_drop() {
    let dir = tempfile::tempdir().unwrap();
    let threads_before = threads();
    cycle(dir.path(), "warm.txt");
    let before = settles_at((threads_before, handles()));
    for i in 0..3 {
        cycle(dir.path(), &format!("{i}.txt"));
    }
    assert_eq!(settles_at(before), before, "threads and handles after three watchers came and went");
}
