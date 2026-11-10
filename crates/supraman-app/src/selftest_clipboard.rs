//! gpui turns a CF_DIB into a PNG-only entry that claude's Alt+V does not see as an image, so steps that borrow the clipboard use this.

pub(crate) use crate::selftest::win::clipboard::{Held, Snapshot, sequence};

use std::time::{Duration, Instant};

use gpui::{AsyncWindowContext, ClipboardItem};

const HELD_WITHIN: Duration = Duration::from_secs(60);
const MUTEX: &[&str] = &[r"Local\Supraman-selftest-clipboard"];
const MACHINE: &[&str] = &[r"Local\Supraman-selftest"];
const MACHINE_WITHIN: Duration = Duration::from_secs(20 * 60);

/// Held for a whole `--selftest`, `--selftest-supervise` or `--selftest-detach-shutdown`: two runs at once break each other's windows.
pub(crate) fn hold_machine() -> anyhow::Result<Held> {
    let pid_file = std::env::temp_dir().join("Supraman-selftest.pid");
    let tell = |pid: Option<u32>, waited: Duration| {
        let pid = pid.map_or_else(|| "unknown".to_string(), |p| p.to_string());
        println!("setup: waiting for another selftest (pid {pid}), {} s so far", waited.as_secs());
    };
    match wait_for(MACHINE, &pid_file, MACHINE_WITHIN, Duration::from_secs(30), tell)? {
        Some(held) => Ok(held),
        None => anyhow::bail!("another selftest held the machine for {} min", MACHINE_WITHIN.as_secs() / 60),
    }
}

/// `None` once `within` ran out; `tell` gets the last holder's pid every `every`.
fn wait_for(
    names: &[&str],
    pid_file: &std::path::Path,
    within: Duration,
    every: Duration,
    mut tell: impl FnMut(Option<u32>, Duration),
) -> anyhow::Result<Option<Held>> {
    let held = Held::open(names)?;
    let started = Instant::now();
    let mut told: Option<Instant> = None;
    loop {
        if held.try_take()? {
            supraman_core::fs::write_atomic(pid_file, std::process::id().to_string()).ok();
            return Ok(Some(held));
        }
        let waited = started.elapsed();
        if waited >= within {
            return Ok(None);
        }
        if told.is_none_or(|t| t.elapsed() >= every) {
            tell(std::fs::read_to_string(pid_file).ok().and_then(|p| p.trim().parse().ok()), waited);
            told = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_millis(250).min(within - waited));
    }
}

/// Every `--selftest` on the machine borrows it through the named mutexes; released on drop, on the UI thread that owns it.
pub(crate) async fn borrow(step: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<Held> {
    let held = Held::open(MUTEX)?;
    let started = Instant::now();
    let mut told = false;
    while !held.try_take()? {
        if !told {
            println!("step {step}: waiting up to {} s for the clipboard, which another selftest holds", HELD_WITHIN.as_secs());
            told = true;
        }
        if started.elapsed() > HELD_WITHIN {
            anyhow::bail!("another selftest held the clipboard for {} s", HELD_WITHIN.as_secs());
        }
        crate::selftest::sleep(cx, Duration::from_millis(100)).await;
    }
    if told {
        println!("step {step}: the clipboard is this run's after {} ms", started.elapsed().as_millis());
    }
    Ok(held)
}

pub(crate) fn text(cx: &mut AsyncWindowContext) -> anyhow::Result<Option<String>> {
    cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()))
}

pub(crate) fn put_text(text: &str, cx: &mut AsyncWindowContext) -> anyhow::Result<()> {
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.into())))
}

pub(crate) const CLIPBOARD_TRIES: usize = 5;

pub(crate) async fn read_own(cx: &mut AsyncWindowContext, own: impl Fn(&str) -> bool) -> anyhow::Result<Option<String>> {
    let started = Instant::now();
    loop {
        let text = text(cx)?;
        if text.as_deref().is_some_and(&own) || started.elapsed() > Duration::from_secs(1) {
            return Ok(text);
        }
        crate::selftest::sleep(cx, Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn the_clipboard_is_one_holders_until_dropped() {
        let name = format!(r"Local\Supraman-selftest-clipboard-test-{}", crate::selftest::hex6());
        let take = |name: String| std::thread::spawn(move || super::Held::open(&[&name]).unwrap().try_take().unwrap()).join().unwrap();
        let first = super::Held::open(&[&name]).unwrap();
        assert!(first.try_take().unwrap());
        let second = take(name.clone());
        assert!(!second, "a second holder took the clipboard while the first held it");
        drop(first);
        let third = take(name);
        assert!(third, "the clipboard stayed held after its holder dropped");
    }

    #[cfg(windows)]
    #[test]
    fn a_selftest_waits_for_the_machine_and_names_the_holder() {
        use std::time::Duration;
        let hex = crate::selftest::hex6();
        let name = format!(r"Local\Supraman-selftest-machine-test-{hex}");
        let pid_file = std::env::temp_dir().join(format!("Supraman-selftest-test-{hex}.pid"));
        let (taken_tx, taken) = std::sync::mpsc::channel();
        let (release_tx, release) = std::sync::mpsc::channel::<()>();
        let (n, f) = (name.clone(), pid_file.clone());
        let holder = std::thread::spawn(move || {
            let held = super::wait_for(&[&n], &f, Duration::from_secs(5), Duration::from_secs(5), |_, _| {}).unwrap();
            assert!(held.is_some());
            taken_tx.send(()).unwrap();
            release.recv().unwrap();
        });
        taken.recv().unwrap();
        let mut told = Vec::new();
        let waited = super::wait_for(&[&name], &pid_file, Duration::from_millis(600), Duration::from_millis(200), |pid, _| told.push(pid)).unwrap();
        assert!(waited.is_none(), "took the machine while another run held it");
        assert!(told.len() >= 2 && told.iter().all(|p| *p == Some(std::process::id())), "{told:?}");
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        let after = super::wait_for(&[&name], &pid_file, Duration::from_secs(2), Duration::from_secs(5), |_, _| {}).unwrap();
        assert!(after.is_some(), "the machine stayed held after its holder ended");
        std::fs::remove_file(&pid_file).ok();
    }

}
