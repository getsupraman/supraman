//! Where the refresh token lives: one DPAPI-encrypted file per key under `<home>/oauth/secrets`
//! ([`FileStore`]), written only under `refresh.lock`. [`MemoryStore`] stands in for tests.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Errors never carry the secret.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, secret: &str) -> Result<(), String>;
    /// Deleting what is not there is fine.
    fn delete(&self, key: &str) -> Result<(), String>;
}

#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).get(key).cloned())
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(key.into(), secret.into());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
        Ok(())
    }
}

/// `home`'s [`FileStore`].
pub fn default_store(home: &Path) -> Arc<dyn SecretStore> {
    #[cfg(windows)]
    {
        Arc::new(FileStore::new(home))
    }
    #[cfg(not(windows))]
    {
        let _ = home;
        Arc::new(MemoryStore::default())
    }
}

/// DPAPI's optional entropy: another program of the same user cannot unprotect the files without it.
#[cfg(windows)]
const ENTROPY: &[u8] = b"Foreman/WorkState refresh token";
/// Tries of a file operation that a scanner or a concurrent reader holds up.
#[cfg(windows)]
const TRIES: u32 = 8;

/// `<home>/oauth/secrets/<sha256 of key>.bin`, each the key's secret sealed with DPAPI for the current
/// user. `set` and `delete` rely on the caller holding `refresh.lock` (`foreman_workstate::auth`).
#[cfg(windows)]
pub struct FileStore {
    dir: std::path::PathBuf,
}

#[cfg(windows)]
impl FileStore {
    pub fn new(home: &Path) -> Self {
        Self { dir: home.join("oauth").join("secrets") }
    }

    pub fn path(&self, key: &str) -> std::path::PathBuf {
        use sha2::{Digest, Sha256};
        self.dir.join(format!("{:x}.bin", Sha256::digest(key.as_bytes())))
    }
}

#[cfg(windows)]
impl SecretStore for FileStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let path = self.path(key);
        let sealed = match retrying(|| std::fs::read(&path)) {
            Ok(sealed) => sealed,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        let plain = dpapi(&sealed, false).map_err(|e| format!("cannot decrypt {}: {}", path.display(), e.message()))?;
        String::from_utf8(plain).map(Some).map_err(|_| format!("{} does not hold text", path.display()))
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        let path = self.path(key);
        let sealed = dpapi(secret.as_bytes(), true).map_err(|e| format!("cannot encrypt the secret: {}", e.message()))?;
        crate::fs::write_atomic(&path, &sealed).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        let path = self.path(key);
        match retrying(|| std::fs::remove_file(&path)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot delete {}: {e}", path.display())),
        }
        Ok(())
    }
}

/// `op` again while it fails with PermissionDenied or a sharing violation, up to [`TRIES`] times.
#[cfg(windows)]
fn retrying<T>(mut op: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    let mut tried = 1;
    loop {
        match op() {
            Err(e) if tried < TRIES && (e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(ERROR_SHARING_VIOLATION)) => {
                std::thread::sleep(std::time::Duration::from_millis(10 << tried));
                tried += 1;
            }
            done => return done,
        }
    }
}

/// CryptProtectData (`seal`) or CryptUnprotectData, current-user scope, with [`ENTROPY`].
#[cfg(windows)]
fn dpapi(data: &[u8], seal: bool) -> windows::core::Result<Vec<u8>> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};
    use windows::core::PCWSTR;
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr().cast_mut() };
    let entropy = CRYPT_INTEGER_BLOB { cbData: ENTROPY.len() as u32, pbData: ENTROPY.as_ptr().cast_mut() };
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` and `entropy` point at buffers that outlive the call and are only read; on success
    // `out` holds a LocalAlloc'd buffer of `cbData` bytes, freed once copied.
    unsafe {
        if seal {
            CryptProtectData(&input, PCWSTR::null(), Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)?;
        } else {
            CryptUnprotectData(&input, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)?;
        }
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(Some(HLOCAL(out.pbData.cast())));
        Ok(bytes)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    const KEY: &str = "https://issuer.invalid/";
    const FILE: &str = "65aac1b76166f0a56db102dc44547dfb5331cf8792548734d2113ebdd5eeeaa7.bin";

    #[test]
    fn a_file_store_keeps_one_encrypted_file_per_key_under_the_home() {
        let home = tempfile::tempdir().unwrap();
        let store = FileStore::new(home.path());
        assert_eq!(store.get(KEY).unwrap(), None);
        store.set(KEY, "fake-rt-1").unwrap();
        store.set(KEY, "fake-rt-2").unwrap();
        assert_eq!(store.get(KEY).unwrap().as_deref(), Some("fake-rt-2"), "a rotation replaces the file");
        let path = store.path(KEY);
        assert!(path.starts_with(home.path().join("oauth").join("secrets")), "{}", path.display());
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, [path.file_name().unwrap()], "no temp file is left");
        let sealed = std::fs::read(&path).unwrap();
        assert!(!sealed.windows(9).any(|w| w == b"fake-rt-2"), "never plain text on disk");
        assert_eq!(FileStore::new(home.path()).get(KEY).unwrap().as_deref(), Some("fake-rt-2"), "another process reads it");
        store.delete(KEY).unwrap();
        assert_eq!((store.get(KEY).unwrap(), path.exists()), (None, false));
        store.delete(KEY).unwrap();
    }

    /// A home as `<home>/oauth/secrets/<hash>.bin` is today, built by hand: found under the same name and
    /// opened with the same entropy.
    #[test]
    fn a_token_file_in_todays_layout_loads() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("oauth").join("secrets");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), dpapi(b"fake-rt-today", true).unwrap()).unwrap();
        assert_eq!(ENTROPY, b"Foreman/WorkState refresh token");
        let store = FileStore::new(home.path());
        assert_eq!(store.path(KEY), dir.join(FILE));
        assert_eq!(store.get(KEY).unwrap().as_deref(), Some("fake-rt-today"));
        assert_eq!(default_store(home.path()).get(KEY).unwrap().as_deref(), Some("fake-rt-today"));
    }

    #[test]
    fn a_file_another_program_sealed_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let store = FileStore::new(home.path());
        std::fs::create_dir_all(store.path(KEY).parent().unwrap()).unwrap();
        std::fs::write(store.path(KEY), b"fake-rt-plain").unwrap();
        let err = store.get(KEY).unwrap_err();
        assert!(err.starts_with("cannot decrypt") && !err.contains("fake-rt"), "{err}");
    }
}
