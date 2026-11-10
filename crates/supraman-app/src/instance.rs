use std::path::Path;

/// FNV-1a, stable across builds unlike `DefaultHasher`, so an old and a new build on one home see each other.
pub fn key(home: &Path) -> String {
    let path = canonical(home);
    let path = path.to_string_lossy();
    let path = match path.strip_prefix(r"\\?\") {
        Some(rest) => rest.strip_prefix(r"UNC\").map_or_else(|| rest.to_string(), |share| format!(r"\\{share}")),
        None => path.into_owned(),
    };
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The nearest existing ancestor canonicalized (8.3 names expanded) and the rest appended, so a home hashes the
/// same before and after it is created.
fn canonical(home: &Path) -> std::path::PathBuf {
    let absolute = std::path::absolute(home).unwrap_or_else(|_| home.to_path_buf());
    let mut base = absolute.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(found) = std::fs::canonicalize(base) {
            return rest.iter().rev().fold(found, |path, part| path.join(part));
        }
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                base = parent;
            }
            _ => return absolute,
        }
    }
}

const NAME: &str = "Supraman";

pub use supraman_core::platform::desktop::{Claimed as Acquired, Instance};

pub fn acquire(home: &Path) -> anyhow::Result<Acquired> {
    supraman_core::platform::desktop::claim(&format!("{NAME}-{}", key(home))).map_err(anyhow::Error::msg)
}

#[cfg(all(test, windows))]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::{Acquired, acquire, key};

    #[test]
    fn a_second_acquire_on_the_same_home_finds_the_first_and_asks_it_to_open() {
        let home = tempfile::tempdir().unwrap();
        let Acquired::First(first) = acquire(home.path()).unwrap() else {
            panic!("the first acquire must win");
        };
        let (tx, rx) = mpsc::channel();
        first.on_open(move || { tx.send(()).ok(); }).unwrap();
        assert!(matches!(acquire(home.path()).unwrap(), Acquired::Running));
        rx.recv_timeout(Duration::from_secs(5)).expect("the first instance was not asked to open");
        let upper = std::path::PathBuf::from(home.path().to_string_lossy().to_uppercase());
        assert!(matches!(acquire(&upper).unwrap(), Acquired::Running));
        drop(first);
        assert!(matches!(acquire(home.path()).unwrap(), Acquired::First(_)));
    }

    #[test]
    fn a_home_hashes_the_same_before_and_after_it_exists() {
        let parent = tempfile::tempdir().unwrap();
        let home = parent.path().join("home");
        let before = key(&home);
        std::fs::create_dir(&home).unwrap();
        assert_eq!(before, key(&home));
    }

    #[test]
    fn another_home_takes_its_own_lock() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        assert_ne!(key(a.path()), key(b.path()));
        let first = acquire(a.path()).unwrap();
        assert!(matches!(first, Acquired::First(_)));
        assert!(matches!(acquire(b.path()).unwrap(), Acquired::First(_)));
    }
}
