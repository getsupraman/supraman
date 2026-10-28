//! Where the refresh token lives: Windows Credential Manager, one generic credential per issuer
//! (`<prefix>:<issuer>`), never a file of Foreman's. [`MemoryStore`] stands in for tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Test hook: the credential target prefix instead of [`DEFAULT_PREFIX`], so a test or the selftest
/// writes only entries it made and deletes afterwards.
pub const PREFIX_VAR: &str = "FOREMAN_CREDENTIAL_PREFIX";
pub const DEFAULT_PREFIX: &str = "Foreman/WorkState";

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

/// Credential Manager under `FOREMAN_CREDENTIAL_PREFIX`, else [`DEFAULT_PREFIX`].
pub fn default_store() -> Arc<dyn SecretStore> {
    #[cfg(windows)]
    {
        let prefix = std::env::var(PREFIX_VAR).ok().filter(|p| !p.is_empty()).unwrap_or_else(|| DEFAULT_PREFIX.into());
        Arc::new(CredentialStore::new(prefix))
    }
    #[cfg(not(windows))]
    {
        Arc::new(MemoryStore::default())
    }
}

/// `CRED_MAX_CREDENTIAL_BLOB_SIZE`: a longer secret is split across `<target>`, `<target>#1`, ….
pub const PART_BYTES: usize = 2560;
/// The comment of a split secret's first credential: `parts=<n>`. An unsplit one has none.
#[cfg(windows)]
const PARTS: &str = "parts=";

/// Generic credentials of the signed-in Windows user, persisted on this machine only.
#[cfg(windows)]
pub struct CredentialStore {
    prefix: String,
}

#[cfg(windows)]
impl CredentialStore {
    pub fn new(prefix: impl Into<String>) -> Self {
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
}
