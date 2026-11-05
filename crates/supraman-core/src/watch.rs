//! The one file watcher: a set of paths watched with the OS's notifications (Zed's `notify`), their changes
//! delivered in batches to the owner's callback on the watcher's own thread, and everything stopped when
//! the [`Watcher`] drops.

use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

/// A path to watch: a folder with everything below it, or a folder or file alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watch {
    pub path: PathBuf,
    pub recursive: bool,
}

impl Watch {
    pub fn tree(path: impl Into<PathBuf>) -> Watch {
        Watch { path: path.into(), recursive: true }
    }

    pub fn only(path: impl Into<PathBuf>) -> Watch {
        Watch { path: path.into(), recursive: false }
    }
}

/// One batch of changes: each changed path once, sorted; `rescan` when the OS lost changes and the owner
/// should read everything again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    pub paths: Vec<PathBuf>,
    pub rescan: bool,
}

enum Msg {
    Event(notify::Result<notify::Event>),
    Stop,
}

/// Watching while it lives; dropping it stops the OS watch and the thread without waiting for them.
pub struct Watcher {
    stop: Sender<Msg>,
    stopped: Stopped,
}

/// Whether a dropped watcher's thread and OS watch are gone.
#[derive(Debug, Clone, Default)]
pub struct Stopped(Arc<(Mutex<bool>, Condvar)>);

impl Stopped {
    pub fn stopped(&self) -> bool {
        *self.0.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Until stopped or `timeout`; whether it stopped.
    pub fn wait(&self, timeout: Duration) -> bool {
        let (flag, cv) = &*self.0;
        let guard = flag.lock().unwrap_or_else(|e| e.into_inner());
        let (guard, _) = cv.wait_timeout_while(guard, timeout, |stopped| !*stopped).unwrap_or_else(|e| e.into_inner());
        *guard
    }
}

/// Marks its [`Stopped`] when the watcher's thread lets it go, panicking or not.
struct Exit(Stopped);

impl Drop for Exit {
    fn drop(&mut self) {
        let (flag, cv) = &*self.0.0;
        *flag.lock().unwrap_or_else(|e| e.into_inner()) = true;
        cv.notify_all();
    }
}

impl Watcher {
    /// Watches `paths`; a batch closes `debounce` after its first change and goes to `on_batch` on the
    /// watcher's thread. A path that cannot be watched is an error and nothing is watched.
    pub fn new(paths: &[Watch], debounce: Duration, on_batch: impl FnMut(Batch) + Send + 'static) -> io::Result<Watcher> {
        let (tx, rx) = mpsc::channel();
        let events = tx.clone();
        let mut os = notify::recommended_watcher(move |e| {
            events.send(Msg::Event(e)).ok();
        })
        .map_err(io::Error::other)?;
        for w in paths {
            let mode = if w.recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            os.watch(&w.path, mode).map_err(|e| io::Error::other(format!("watch {}: {e}", w.path.display())))?;
        }
        let stopped = Stopped::default();
        let exit = Exit(stopped.clone());
        thread::Builder::new().name("supraman watch".into()).spawn(move || {
            run(os, rx, debounce, on_batch);
            drop(exit);
        })?;
        Ok(Watcher { stop: tx, stopped })
    }

    pub fn stopped(&self) -> Stopped {
        self.stopped.clone()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.send(Msg::Stop).ok();
    }
}

/// Until the OS watcher's handler is gone after a stop: the handler holds the last sender, so the channel
/// closes only once notify's thread has closed its handles.
fn run(os: RecommendedWatcher, rx: mpsc::Receiver<Msg>, debounce: Duration, mut on_batch: impl FnMut(Batch)) {
    let mut os = Some(os);
    let mut batch = Batch::default();
    let mut paths = BTreeSet::new();
    let mut due: Option<Instant> = None;
    loop {
        let msg = match due {
            Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(Msg::Event(e)) if os.is_some() => {
                match e {
                    Ok(e) if e.kind.is_access() => continue,
                    Ok(e) => {
                        batch.rescan |= e.need_rescan();
                        paths.extend(e.paths);
                    }
                    Err(_) => batch.rescan = true,
                }
                due.get_or_insert_with(|| Instant::now() + debounce);
            }
            Ok(Msg::Event(_)) => {}
            Ok(Msg::Stop) => {
                os = None;
                due = None;
            }
            Err(RecvTimeoutError::Timeout) => {
                due = None;
                batch.paths = std::mem::take(&mut paths).into_iter().collect();
                on_batch(std::mem::take(&mut batch));
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    const WITHIN: Duration = Duration::from_secs(5);

    /// The batches until every path of `want` was in one, or `WITHIN`.
    fn until_seen(rx: &mpsc::Receiver<(Instant, Batch)>, want: &[&Path]) -> Vec<(Instant, Batch)> {
        let deadline = Instant::now() + WITHIN;
        let mut got: Vec<(Instant, Batch)> = Vec::new();
        while !want.iter().all(|w| got.iter().any(|(_, b)| b.paths.iter().any(|p| p == w))) {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(b) => got.push(b),
                Err(_) => panic!("{want:?} not seen within {WITHIN:?}; got {got:?}"),
            }
        }
        got
    }

    fn batches() -> (impl FnMut(Batch) + Send + 'static, mpsc::Receiver<(Instant, Batch)>) {
        let (tx, rx) = mpsc::channel();
        (move |b| tx.send((Instant::now(), b)).unwrap(), rx)
    }

    #[test]
    fn watch_delivers_a_folders_changes_in_one_batch_after_the_debounce() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        let (on_batch, rx) = batches();
        let debounce = Duration::from_millis(150);
        let w = Watcher::new(&[Watch::tree(&root)], debounce, on_batch).unwrap();
        let start = Instant::now();
        let (a, b) = (root.join("a.txt"), root.join("sub").join("b.txt"));
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        let got = until_seen(&rx, &[&a, &b]);
        assert_eq!(got.len(), 1, "both changes in one batch: {got:?}");
        assert!(got[0].0 - start >= debounce, "the batch waits out the debounce");
        let paths = &got[0].1.paths;
        assert!(paths.windows(2).all(|p| p[0] < p[1]), "each path once, sorted: {paths:?}");
        drop(w);
    }

    #[test]
    fn watch_of_one_folder_alone_misses_its_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        let (on_batch, rx) = batches();
        let _w = Watcher::new(&[Watch::only(&root)], Duration::from_millis(50), on_batch).unwrap();
        let (deep, top) = (root.join("sub").join("deep.txt"), root.join("top.txt"));
        fs::write(&deep, b"x").unwrap();
        fs::write(&top, b"x").unwrap();
        let got = until_seen(&rx, &[&top]);
        assert!(got.iter().all(|(_, b)| !b.paths.contains(&deep)), "{got:?}");
    }

    #[test]
    fn watch_refuses_a_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let err = Watcher::new(&[Watch::tree(dir.path().join("gone"))], Duration::from_millis(50), |_| {}).err().expect("refused");
        assert!(err.to_string().contains("gone"), "{err}");
    }

    #[test]
    fn watch_stops_its_thread_and_drops_the_callback_when_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let (on_batch, rx) = batches();
        let w = Watcher::new(&[Watch::tree(dir.path())], Duration::from_millis(50), on_batch).unwrap();
        let stopped = w.stopped();
        assert!(!stopped.wait(Duration::from_millis(1)), "running while it lives");
        drop(w);
        assert!(stopped.wait(WITHIN), "the thread ends after the drop");
        assert!(matches!(rx.recv_timeout(WITHIN), Err(mpsc::RecvTimeoutError::Disconnected)), "the callback is gone with it");
    }
}
