//! The app's WorkState sign-in (m5/43): one [`Auth`] per Foreman home, shared by the read API, the
//! supervisors' mints, the hand mints and Settings, and the browser its authorize step opens.
//! The selftests swap the system browser for [`foreman_workstate::auth::http_browser`]; their refresh
//! tokens stay in their scratch home.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use foreman_workstate::Auth;

static AUTHS: OnceLock<Mutex<HashMap<PathBuf, Arc<Auth>>>> = OnceLock::new();
static HTTP_BROWSER: AtomicBool = AtomicBool::new(false);
/// Issuers signed in to by this process, for the selftest's cleanup.
static ISSUERS: Mutex<Vec<(PathBuf, String)>> = Mutex::new(Vec::new());

pub fn auth(home: &Path) -> Arc<Auth> {
    let mut auths = AUTHS.get_or_init(Mutex::default).lock().unwrap_or_else(|e| e.into_inner());
    auths.entry(home.to_path_buf()).or_insert_with(|| Auth::open(home)).clone()
}

/// The selftests: no real browser, ever.
pub fn use_http_browser() {
    HTTP_BROWSER.store(true, Ordering::SeqCst);
}

/// The system browser, or the selftests' HTTP stand-in.
pub fn browser() -> Box<dyn Fn(&str) -> Result<(), String> + Send + Sync> {
    if HTTP_BROWSER.load(Ordering::SeqCst) {
        Box::new(foreman_workstate::auth::http_browser())
    } else {
        Box::new(open_system_browser)
    }
}

/// [`Auth::sign_in`] with [`browser`], blocking; remembers the issuer for [`forget_own_credentials`].
pub fn sign_in(home: &Path, origin: &str) -> Result<foreman_workstate::AuthStatus, foreman_workstate::AuthError> {
    let auth = auth(home);
    let status = auth.sign_in(origin, &browser())?;
    if let Some(issuer) = auth.issuer_of(origin) {
        ISSUERS.lock().unwrap_or_else(|e| e.into_inner()).push((home.to_path_buf(), issuer));
    }
    Ok(status)
}

/// Selftest cleanup: deletes every refresh token this process signed in with.
pub fn forget_own_credentials() -> Vec<String> {
    let issuers = std::mem::take(&mut *ISSUERS.lock().unwrap_or_else(|e| e.into_inner()));
    issuers.iter().filter_map(|(home, issuer)| foreman_workstate::secret::default_store(home).delete(issuer).err()).collect()
}

#[cfg(windows)]
fn open_system_browser(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(format!("{url} is not a web address"));
    }
    foreman_core::process::shell_execute("open", url.as_ref(), None, None).map_err(|code| format!("ShellExecuteW returned {code}"))
}

#[cfg(not(windows))]
fn open_system_browser(_url: &str) -> Result<(), String> {
    Err("only Windows opens the browser".into())
}
