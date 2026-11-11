//! A secret sealed for the current user at rest (DPAPI), and a file only its owner can open.

use std::path::Path;

/// DPAPI's optional entropy: another program of the same user cannot unprotect the files without it.
#[cfg(windows)]
pub(crate) const ENTROPY: &[u8] = b"Foreman/WorkState refresh token";

/// `data` encrypted for the current user; the error is the OS's words, never the data.
#[cfg(windows)]
pub fn seal(data: &[u8]) -> Result<Vec<u8>, String> {
    dpapi(data, true).map_err(|e| e.message())
}

/// What [`seal`] encrypted; refused for bytes another user or program sealed.
#[cfg(windows)]
pub fn unseal(sealed: &[u8]) -> Result<Vec<u8>, String> {
    dpapi(sealed, false).map_err(|e| e.message())
}

#[cfg(not(windows))]
pub fn seal(_: &[u8]) -> Result<Vec<u8>, String> {
    Err(super::refused("a secret encrypted at rest"))
}

#[cfg(not(windows))]
pub fn unseal(_: &[u8]) -> Result<Vec<u8>, String> {
    Err(super::refused("a secret encrypted at rest"))
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

#[cfg(windows)]
const ICACLS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// `path` opened to its owner alone: full control for the user, nothing inherited.
#[cfg(windows)]
pub fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    use crate::process::{self, Kill};
    let user = super::paths::user_name().ok_or_else(|| std::io::Error::other("USERNAME is not set"))?;
    let out = process::run(
        // an elevated process's file also carries explicit SYSTEM and Administrators entries
        process::command("icacls")
            .arg(path)
            .args(["/inheritance:r", "/grant:r", &format!("{user}:F"), "/remove:g", "*S-1-5-18", "*S-1-5-32-544"]),
        ICACLS_TIMEOUT,
        Kill::Tree,
    )
    .map_err(|e| std::io::Error::other(format!("icacls {}: {e}", path.display())))?;
    if out.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "icacls {}: {}",
            path.display(),
            out.stdout.trim()
        )))
    }
}

#[cfg(unix)]
pub fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}
