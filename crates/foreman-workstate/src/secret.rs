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

    fn target(&self, key: &str) -> Vec<u16> {
        format!("{}:{key}", self.prefix).encode_utf16().chain(Some(0)).collect()
    }
}

#[cfg(windows)]
impl SecretStore for CredentialStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        use windows::Win32::Foundation::ERROR_NOT_FOUND;
        use windows::Win32::Security::Credentials::{CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW};
        use windows::core::PCWSTR;
        let target = self.target(key);
        let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: `target` is NUL-terminated and outlives the call; `cred` is freed with CredFree below.
        let read = unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut cred) };
        if let Err(e) = read {
            return if e.code() == ERROR_NOT_FOUND.to_hresult() { Ok(None) } else { Err(format!("Credential Manager: {}", e.message())) };
        }
        // SAFETY: CredReadW succeeded, so `cred` points at a CREDENTIALW whose blob has the given size.
        let secret = unsafe {
            let c = &*cred;
            let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
            let text = String::from_utf8(blob.to_vec());
            CredFree(cred as *const _);
            text
        };
        secret.map(Some).map_err(|_| "Credential Manager: the stored value is not text".into())
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        use windows::Win32::Security::Credentials::{CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredWriteW};
        use windows::core::PWSTR;
        let mut target = self.target(key);
        let mut user: Vec<u16> = "Foreman".encode_utf16().chain(Some(0)).collect();
        let mut blob = secret.as_bytes().to_vec();
        let cred = CREDENTIALW {
            Flags: CRED_FLAGS(0),
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_mut_ptr()),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: PWSTR(user.as_mut_ptr()),
            ..Default::default()
        };
        // SAFETY: every pointer in `cred` points into a buffer that outlives the call.
        unsafe { CredWriteW(&cred, 0) }.map_err(|e| format!("Credential Manager: {}", e.message()))
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        use windows::Win32::Foundation::ERROR_NOT_FOUND;
        use windows::Win32::Security::Credentials::{CRED_TYPE_GENERIC, CredDeleteW};
        use windows::core::PCWSTR;
        let target = self.target(key);
        // SAFETY: `target` is NUL-terminated and outlives the call.
        match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Err(e) if e.code() != ERROR_NOT_FOUND.to_hresult() => Err(format!("Credential Manager: {}", e.message())),
            _ => Ok(()),
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Only an entry under a random prefix of its own, deleted at the end.
    #[test]
    fn credential_manager_round_trip_under_a_test_prefix() {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let prefix = format!("fm-test-{}", r.iter().map(|b| format!("{b:02x}")).collect::<String>());
        let store = CredentialStore::new(&prefix);
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
}
