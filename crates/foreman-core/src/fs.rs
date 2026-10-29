use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde::de::DeserializeOwned;

static TMP_SEQ: AtomicU32 = AtomicU32::new(0);

/// Replaces `path` with `bytes`, creating its folder: a temp file unique to this process and call, renamed
/// over `path` (retried while another process holds it open), so a reader never sees half a file.
pub fn write_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let tmp = tmp_path(path);
    if let Err(e) = fs::write(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    let mut tries = 0;
    loop {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) if tries < 20 && e.kind() == ErrorKind::PermissionDenied => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
        }
    }
}

/// `<name>.<pid>-<seq>.tmp` beside `path`: ends in `.tmp`, so a watcher of `*.json` never takes it.
fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(format!(".{}-{}.tmp", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
    path.with_file_name(name)
}

/// `path` parsed as JSON. Missing: the default, no notice. Unreadable or not the expected JSON: the
/// default and a notice naming the file, the error and `fallback` (what the default means to the user).
pub fn load_json_or_notice<T: DeserializeOwned + Default>(path: &Path, fallback: &str) -> (T, Option<String>) {
    let notice = |e: &dyn std::fmt::Display| Some(format!("Cannot read {} ({e}): {fallback}", path.display()));
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(v) => (v, None),
            Err(e) => (T::default(), notice(&e)),
        },
        Err(e) if e.kind() == ErrorKind::NotFound => (T::default(), None),
        Err(e) => (T::default(), notice(&e)),
    }
}

/// Appends `bytes` to `path`, first moving a file of `cap` bytes or more to `<path>.1` (replacing an
/// older one). A rotation that fails (the file held open elsewhere) does not cost the append.
pub fn append_capped(path: &Path, bytes: &[u8], cap: u64) -> io::Result<()> {
    if fs::metadata(path).is_ok_and(|m| m.len() >= cap) {
        let _ = fs::rename(path, rotated(path));
    }
    fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(bytes)
}

/// `<path>.1`, where [`append_capped`] moves a full file.
pub fn rotated(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_files(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn write_atomic_creates_the_dir_replaces_the_file_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b.json");
        write_atomic(&path, "one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        assert!(tmp_files(path.parent().unwrap()).is_empty());
    }

    #[test]
    fn write_atomic_temp_names_differ_per_call_and_end_in_tmp() {
        let path = Path::new(r"C:\x\inbox\1.json");
        let (a, b) = (tmp_path(path), tmp_path(path));
        assert_ne!(a, b);
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(&format!("1.json.{}-", std::process::id())) && name.ends_with(".tmp"), "{name}");
        assert_eq!(a.parent(), path.parent());
    }

    #[test]
    fn concurrent_writers_never_clobber_each_others_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared.toml");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        write_atomic(&path, format!("writer = {i}\n")).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("writer = ") && text.ends_with('\n') && text.lines().count() == 1, "{text:?}");
        assert!(tmp_files(dir.path()).is_empty());
    }

    #[test]
    fn write_atomic_into_a_file_as_dir_fails_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        fs::write(&blocker, "x").unwrap();
        assert!(write_atomic(&blocker.join("x.json"), "y").is_err());
        assert_eq!(fs::read_to_string(&blocker).unwrap(), "x");
    }

    #[test]
    fn load_json_or_notice_is_quiet_when_missing_and_names_the_file_when_broken() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("starred.json");
        let (v, notice): (Vec<String>, _) = load_json_or_notice(&path, "no session is starred");
        assert!(v.is_empty() && notice.is_none());
        fs::write(&path, r#"["a","b"]"#).unwrap();
        let (v, notice): (Vec<String>, _) = load_json_or_notice(&path, "no session is starred");
        assert_eq!((v, notice), (vec!["a".to_string(), "b".to_string()], None));
        fs::write(&path, "not json").unwrap();
        let (v, notice): (Vec<String>, _) = load_json_or_notice(&path, "no session is starred");
        let notice = notice.unwrap();
        assert!(v.is_empty());
        assert!(notice.starts_with(&format!("Cannot read {} (", path.display())) && notice.ends_with("): no session is starred"), "{notice}");
    }

    #[test]
    fn append_capped_rotates_at_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.log");
        append_capped(&path, b"12345", 5).unwrap();
        append_capped(&path, b"67", 5).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "67");
        assert_eq!(fs::read_to_string(rotated(&path)).unwrap(), "12345");
        append_capped(&path, b"8", 5).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "678");
        for n in 0..25 {
            append_capped(&path, format!("line {n:02}\n").as_bytes(), 5).unwrap();
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2, "never a third file");
        assert!(append_capped(&dir.path().join("missing").join("x.log"), b"y", 5).is_err());
    }
}
