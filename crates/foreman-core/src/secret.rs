//! Where the refresh token lives: one DPAPI-encrypted file per key under `<home>/oauth/secrets`
//! ([`FileStore`]), written only under `refresh.lock`. [`MemoryStore`] stands in for tests.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The Credential Manager target prefix of tokens stored before [`FileStore`]; read once to migrate them.
pub const LEGACY_PREFIX: &str = "Foreman/WorkState";

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

/// `home`'s [`FileStore`], migrating from Credential Manager under [`LEGACY_PREFIX`].
pub fn default_store(home: &Path) -> Arc<dyn SecretStore> {
    #[cfg(windows)]
    {
        Arc::new(FileStore::new(home, Arc::new(CredentialStore::new(LEGACY_PREFIX))))
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
/// user. `set` and `delete` rely on the caller holding `refresh.lock` (`foreman_workstate::auth`); the migration in `get`
/// never replaces a file, so it is safe without it.
#[cfg(windows)]
pub struct FileStore {
    dir: std::path::PathBuf,
    legacy: Arc<dyn SecretStore>,
}

#[cfg(windows)]
impl FileStore {
    pub fn new(home: &Path, legacy: Arc<dyn SecretStore>) -> Self {
        Self { dir: home.join("oauth").join("secrets"), legacy }
    }

    pub fn path(&self, key: &str) -> std::path::PathBuf {
        use sha2::{Digest, Sha256};
        self.dir.join(format!("{:x}.bin", Sha256::digest(key.as_bytes())))
    }

    fn read(&self, key: &str) -> Result<Option<String>, String> {
        let path = self.path(key);
        let sealed = match retrying(|| std::fs::read(&path)) {
            Ok(sealed) => sealed,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        let plain = dpapi(&sealed, false).map_err(|e| format!("cannot decrypt {}: {}", path.display(), e.message()))?;
        String::from_utf8(plain).map(Some).map_err(|_| format!("{} does not hold text", path.display()))
    }

    /// False when `replace` is off and a file was already there.
    fn write(&self, key: &str, secret: &str, replace: bool) -> Result<bool, String> {
        use std::io::Write;
        let path = self.path(key);
        let sealed = dpapi(secret.as_bytes(), true).map_err(|e| format!("cannot encrypt the secret: {}", e.message()))?;
        if replace {
            return crate::fs::write_atomic(&path, &sealed).map(|()| true).map_err(|e| format!("cannot write {}: {e}", path.display()));
        }
        // write_atomic always replaces; a migration must never overwrite a token rotated meanwhile.
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).map_err(|e| e.to_string())?;
        let tmp = path.with_extension(format!("{}.tmp", r.iter().map(|b| format!("{b:02x}")).collect::<String>()));
        let staged = std::fs::create_dir_all(&self.dir).and_then(|()| {
            let mut file = std::fs::File::create_new(&tmp)?;
            file.write_all(&sealed)?;
            file.sync_all()
        });
        let placed = staged.and_then(|()| retrying(|| move_new(&tmp, &path)));
        match placed {
            Ok(()) => Ok(true),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(false)
                } else {
                    Err(format!("cannot write {}: {e}", path.display()))
                }
            }
        }
    }
}

#[cfg(windows)]
impl SecretStore for FileStore {
    /// Without a file, a credential under the legacy target moves into one and is deleted.
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        if let Some(secret) = self.read(key)? {
            return Ok(Some(secret));
        }
        let Some(secret) = self.legacy.get(key)? else { return Ok(None) };
        let placed = self.write(key, &secret, false)?;
        // the file wins over a leftover credential, and `delete` tries again
        let _ = self.legacy.delete(key);
        if placed { Ok(Some(secret)) } else { self.read(key) }
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.write(key, secret, true).map(|_| ())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        let path = self.path(key);
        match retrying(|| std::fs::remove_file(&path)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot delete {}: {e}", path.display())),
        }
        self.legacy.delete(key)
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

/// A rename that fails with AlreadyExists instead of replacing `to`.
#[cfg(windows)]
fn move_new(from: &Path, to: &Path) -> std::io::Result<()> {
    use windows::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    use windows::core::HSTRING;
    // SAFETY: both paths are HSTRINGs that outlive the call.
    unsafe { MoveFileExW(&HSTRING::from(from), &HSTRING::from(to), MOVEFILE_WRITE_THROUGH) }
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))
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

/// `CRED_MAX_CREDENTIAL_BLOB_SIZE`: a longer secret is split across `<target>`, `<target>#1`, ….
#[cfg(windows)]
const PART_BYTES: usize = 2560;
/// The comment of a split secret's first credential: `parts=<n>`. An unsplit one has none.
#[cfg(windows)]
const PARTS: &str = "parts=";

/// Generic credentials of the signed-in Windows user: only the legacy store [`FileStore`] migrates from.
#[cfg(windows)]
struct CredentialStore {
    prefix: String,
}

#[cfg(windows)]
impl CredentialStore {
    fn new(prefix: impl Into<String>) -> Self {
        Self { prefix: prefix.into() }
    }

    fn target(&self, key: &str, part: usize) -> Vec<u16> {
        let name = if part == 0 { format!("{}:{key}", self.prefix) } else { format!("{}:{key}#{part}", self.prefix) };
        name.encode_utf16().chain(Some(0)).collect()
    }

    /// One credential's blob and comment.
    fn read(&self, key: &str, part: usize) -> Result<Option<(Vec<u8>, Option<String>)>, String> {
        use windows::Win32::Foundation::ERROR_NOT_FOUND;
        use windows::Win32::Security::Credentials::{CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW};
        use windows::core::PCWSTR;
        let target = self.target(key, part);
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `target` is NUL-terminated and outlives the call; `cred` is freed with CredFree below.
        let read = unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut cred) };
        if let Err(e) = read {
            return if e.code() == ERROR_NOT_FOUND.to_hresult() { Ok(None) } else { Err(format!("Credential Manager: {}", e.message())) };
        }
        // SAFETY: CredReadW succeeded, so `cred` points at a CREDENTIALW whose blob has the given size and
        // whose comment is null or NUL-terminated.
        let found = unsafe {
            let c = &*cred;
            let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec();
            let comment = if c.Comment.is_null() { None } else { c.Comment.to_string().ok() };
            CredFree(cred as *const _);
            (blob, comment)
        };
        Ok(Some(found))
    }

    fn write(&self, key: &str, part: usize, blob: &[u8], comment: Option<&str>) -> Result<(), String> {
        use windows::Win32::Security::Credentials::{CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredWriteW};
        use windows::core::PWSTR;
        let mut target = self.target(key, part);
        let mut user: Vec<u16> = "Foreman".encode_utf16().chain(Some(0)).collect();
        let mut comment: Option<Vec<u16>> = comment.map(|c| c.encode_utf16().chain(Some(0)).collect());
        let mut blob = blob.to_vec();
        let cred = CREDENTIALW {
            Flags: CRED_FLAGS(0),
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_mut_ptr()),
            Comment: comment.as_mut().map_or(PWSTR::null(), |c| PWSTR(c.as_mut_ptr())),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: PWSTR(user.as_mut_ptr()),
            ..Default::default()
        };
        // SAFETY: every pointer in `cred` points into a buffer that outlives the call.
        unsafe { CredWriteW(&cred, 0) }.map_err(|e| format!("Credential Manager: {}", e.message()))
    }

    /// Whether there was one to delete.
    fn remove(&self, key: &str, part: usize) -> Result<bool, String> {
        use windows::Win32::Foundation::ERROR_NOT_FOUND;
        use windows::Win32::Security::Credentials::{CRED_TYPE_GENERIC, CredDeleteW};
        use windows::core::PCWSTR;
        let target = self.target(key, part);
        // SAFETY: `target` is NUL-terminated and outlives the call.
        match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(true),
            Err(e) if e.code() == ERROR_NOT_FOUND.to_hresult() => Ok(false),
            Err(e) => Err(format!("Credential Manager: {}", e.message())),
        }
    }

    /// Parts `from`, `from + 1`, … up to the first that is not there.
    fn remove_parts_from(&self, key: &str, from: usize) -> Result<(), String> {
        let mut part = from;
        while self.remove(key, part)? {
            part += 1;
        }
        Ok(())
    }
}

#[cfg(windows)]
impl SecretStore for CredentialStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let Some((mut bytes, comment)) = self.read(key, 0)? else { return Ok(None) };
        let parts = match comment.as_deref().and_then(|c| c.strip_prefix(PARTS)) {
            Some(n) => n.parse::<usize>().map_err(|_| format!("Credential Manager: the part count '{n}' is not a number"))?,
            None => 1,
        };
        for part in 1..parts {
            let Some((blob, _)) = self.read(key, part)? else {
                return Err(format!("Credential Manager: part {part} of {parts} is missing"));
            };
            bytes.extend(blob);
        }
        String::from_utf8(bytes).map(Some).map_err(|_| "Credential Manager: the stored value is not text".into())
    }

    /// The parts first and the first credential last, so a reader never finds a count with parts missing.
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        let chunks: Vec<&[u8]> = secret.as_bytes().chunks(PART_BYTES).collect();
        let parts = chunks.len().max(1);
        for (part, chunk) in chunks.iter().enumerate().skip(1) {
            self.write(key, part, chunk, None)?;
        }
        let comment = (parts > 1).then(|| format!("{PARTS}{parts}"));
        self.write(key, 0, chunks.first().copied().unwrap_or_default(), comment.as_deref())?;
        self.remove_parts_from(key, parts)
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.remove(key, 0)?;
        self.remove_parts_from(key, 1)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Concurrent Credential Manager writers can lose an update (F-05): one test writes at a time.
    fn one_writer() -> std::sync::MutexGuard<'static, ()> {
        static WRITER: Mutex<()> = Mutex::new(());
        WRITER.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn test_store() -> CredentialStore {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        CredentialStore::new(format!("fm-test-{}", r.iter().map(|b| format!("{b:02x}")).collect::<String>()))
    }

    /// Deletes every credential under the store's prefix when dropped, even after a failed assert.
    struct Cleanup<'a>(&'a CredentialStore, &'a str);

    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.0.delete(self.1);
            let _ = self.0.remove_parts_from(self.1, 1);
        }
    }

    /// Only an entry under a random prefix of its own, deleted at the end.
    #[test]
    fn credential_manager_round_trip_under_a_test_prefix() {
        let _one = one_writer();
        let store = test_store();
        let key = "https://issuer.invalid/";
        assert_eq!(store.get(key).unwrap(), None);
        store.set(key, "fake-rt-1").unwrap();
        assert_eq!(store.get(key).unwrap().as_deref(), Some("fake-rt-1"));
        store.set(key, "fake-rt-2").unwrap();
        assert_eq!(store.get(key).unwrap().as_deref(), Some("fake-rt-2"), "a rotation overwrites");
        store.delete(key).unwrap();
        assert_eq!(store.get(key).unwrap(), None);
        store.delete(key).unwrap();
    }

    #[test]
    fn a_secret_over_the_blob_limit_is_split_and_a_short_one_stays_one_credential() {
        let _one = one_writer();
        let store = test_store();
        let key = "https://issuer.invalid/";
        let _cleanup = Cleanup(&store, key);
        let long: String = (0..6 * 1024).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        store.set(key, &long).unwrap();
        assert_eq!(store.get(key).unwrap().as_deref(), Some(long.as_str()));
        let (first, comment) = store.read(key, 0).unwrap().unwrap();
        assert_eq!((first.len(), comment.as_deref()), (PART_BYTES, Some("parts=3")));
        assert_eq!(store.read(key, 2).unwrap().unwrap().0.len(), 6 * 1024 - 2 * PART_BYTES);

        let exact = "x".repeat(PART_BYTES);
        store.set(key, &exact).unwrap();
        assert_eq!(store.read(key, 0).unwrap(), Some((exact.clone().into_bytes(), None)), "as today: one blob, no comment");
        assert_eq!(store.read(key, 1).unwrap(), None, "the longer secret's parts are gone");
        assert_eq!(store.get(key).unwrap(), Some(exact));

        store.set(key, &long).unwrap();
        store.delete(key).unwrap();
        assert_eq!(store.get(key).unwrap(), None);
        assert_eq!((store.read(key, 1).unwrap(), store.read(key, 2).unwrap()), (None, None), "every part deleted");
    }

    const KEY: &str = "https://issuer.invalid/";

    fn file_store(legacy: Arc<dyn SecretStore>) -> (tempfile::TempDir, FileStore) {
        let home = tempfile::tempdir().unwrap();
        let store = FileStore::new(home.path(), legacy);
        (home, store)
    }

    #[test]
    fn a_file_store_keeps_one_encrypted_file_per_key_under_the_home() {
        let (home, store) = file_store(Arc::new(MemoryStore::default()));
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
        let other = FileStore::new(home.path(), Arc::new(MemoryStore::default()));
        assert_eq!(other.get(KEY).unwrap().as_deref(), Some("fake-rt-2"), "another process reads it");
        store.delete(KEY).unwrap();
        assert_eq!((store.get(KEY).unwrap(), path.exists()), (None, false));
        store.delete(KEY).unwrap();
    }

    /// A file already on disk is found under the same name and opened with the same entropy.
    #[test]
    fn the_file_name_and_entropy_stay_as_written() {
        let (home, store) = file_store(Arc::new(MemoryStore::default()));
        let name = "65aac1b76166f0a56db102dc44547dfb5331cf8792548734d2113ebdd5eeeaa7.bin";
        assert_eq!(store.path(KEY), home.path().join("oauth").join("secrets").join(name));
        assert_eq!(ENTROPY, b"Foreman/WorkState refresh token");
        assert_eq!(LEGACY_PREFIX, "Foreman/WorkState");
    }

    #[test]
    fn a_file_another_program_sealed_is_refused() {
        let (_home, store) = file_store(Arc::new(MemoryStore::default()));
        std::fs::create_dir_all(store.path(KEY).parent().unwrap()).unwrap();
        std::fs::write(store.path(KEY), b"fake-rt-plain").unwrap();
        let err = store.get(KEY).unwrap_err();
        assert!(err.starts_with("cannot decrypt") && !err.contains("fake-rt"), "{err}");
    }

    #[test]
    fn a_legacy_credential_moves_into_the_file_once_and_is_deleted() {
        let legacy = Arc::new(MemoryStore::default());
        legacy.set(KEY, "fake-rt-legacy").unwrap();
        let (_home, store) = file_store(legacy.clone());
        assert_eq!(store.get(KEY).unwrap().as_deref(), Some("fake-rt-legacy"));
        assert_eq!(legacy.get(KEY).unwrap(), None, "the credential is gone");
        assert!(store.path(KEY).exists());
        legacy.set(KEY, "fake-rt-stale").unwrap();
        assert_eq!(store.get(KEY).unwrap().as_deref(), Some("fake-rt-legacy"), "the file wins");
        store.delete(KEY).unwrap();
        assert_eq!((store.get(KEY).unwrap(), legacy.get(KEY).unwrap()), (None, None), "delete removes a leftover credential too");
    }

    #[test]
    fn a_migration_never_replaces_a_file_already_written() {
        let (_home, store) = file_store(Arc::new(MemoryStore::default()));
        store.set(KEY, "fake-rt-rotated").unwrap();
        assert!(!store.write(KEY, "fake-rt-legacy", false).unwrap());
        assert_eq!(store.get(KEY).unwrap().as_deref(), Some("fake-rt-rotated"));
        assert_eq!(std::fs::read_dir(store.path(KEY).parent().unwrap()).unwrap().count(), 1, "no temp file is left");
    }

    /// The legacy store under a random prefix of the test's own, never [`LEGACY_PREFIX`].
    #[test]
    fn a_split_legacy_credential_migrates_and_every_part_is_deleted() {
        let _one = one_writer();
        let legacy = Arc::new(test_store());
        let _cleanup = Cleanup(&legacy, KEY);
        let long: String = (0..6 * 1024).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        legacy.set(KEY, &long).unwrap();
        let (home, store) = file_store(legacy.clone());
        assert_eq!(store.get(KEY).unwrap().as_deref(), Some(long.as_str()));
        assert_eq!((legacy.read(KEY, 0).unwrap(), legacy.read(KEY, 1).unwrap(), legacy.read(KEY, 2).unwrap()), (None, None, None));
        let fresh = FileStore::new(home.path(), legacy.clone());
        assert_eq!(fresh.get(KEY).unwrap().as_deref(), Some(long.as_str()));
    }
}
