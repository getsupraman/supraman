//! One [`Auth`] per Supraman home ([`Auth::shared`]), shared by the read API, the mints and Settings.

use std::path::{Path, PathBuf};
#[cfg(feature = "selftest")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use supraman_workman::Auth;

#[cfg(feature = "selftest")]
static HTTP_BROWSER: AtomicBool = AtomicBool::new(false);
static ISSUERS: Mutex<Vec<(PathBuf, String)>> = Mutex::new(Vec::new());

pub fn auth(home: &Path) -> Arc<Auth> {
    Auth::shared(home)
}

#[cfg(feature = "selftest")]
pub fn use_http_browser() {
    HTTP_BROWSER.store(true, Ordering::SeqCst);
}

pub fn browser() -> Box<dyn Fn(&str) -> Result<(), String> + Send + Sync> {
    #[cfg(feature = "selftest")]
    if HTTP_BROWSER.load(Ordering::SeqCst) {
        return Box::new(supraman_workman::auth::http_browser());
    }
    Box::new(open_system_browser)
}

/// Blocks; the [+] tabs started signed out get their tokens after it ([`crate::hand::mint_helped`]).
pub fn sign_in(home: &Path, origin: &str, hands: Option<&Mutex<crate::hand::Hands>>) -> Result<crate::hand::HelpedMinted, supraman_workman::AuthError> {
    let auth = auth(home);
    supraman_supervisor::signing_in();
    auth.sign_in(origin, &browser())?;
    crate::keeping::signed_in();
    if let Some(issuer) = auth.issuer_of(origin) {
        ISSUERS.lock().unwrap_or_else(|e| e.into_inner()).push((home.to_path_buf(), issuer));
    }
    Ok(hands.map(crate::hand::mint_helped).unwrap_or_default())
}

#[cfg(feature = "selftest")]
pub fn forget_own_credentials() -> Vec<String> {
    let issuers = std::mem::take(&mut *ISSUERS.lock().unwrap_or_else(|e| e.into_inner()));
    issuers.iter().filter_map(|(home, issuer)| supraman_core::secret::default_store(home).delete(issuer).err()).collect()
}

fn open_system_browser(url: &str) -> Result<(), String> {
    use supraman_ui::open;
    open::allowed(url, &open::WEB_SCHEMES).map_err(|_| format!("{url} is not a web address"))?;
    open::open(std::ffi::OsStr::new(url))
}
