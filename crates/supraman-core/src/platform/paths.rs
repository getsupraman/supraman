use std::ffi::OsString;

/// `%APPDATA%`, the folder that holds the default Supraman home.
#[cfg(windows)]
pub fn data_dir() -> Option<OsString> {
    std::env::var_os("APPDATA")
}

/// No data dir is looked up here: the home comes from `SUPRAMAN_HOME` alone.
#[cfg(not(windows))]
pub fn data_dir() -> Option<OsString> {
    None
}

/// `%USERPROFILE%`, looked at before `HOME`.
#[cfg(windows)]
pub fn user_profile() -> Option<OsString> {
    std::env::var_os("USERPROFILE")
}

/// No profile var here: the user home is `HOME`.
#[cfg(not(windows))]
pub fn user_profile() -> Option<OsString> {
    None
}

/// `%USERNAME%`, the account an owner-only file's ACL grants.
#[cfg(windows)]
pub fn user_name() -> Option<String> {
    std::env::var("USERNAME").ok()
}

/// No account name here: an owner-only file is a mode, not an ACL.
#[cfg(not(windows))]
pub fn user_name() -> Option<String> {
    None
}

/// A normalized path in the case its file system compares: lowercased on Windows.
pub fn compared_case(normal: String) -> String {
    if cfg!(windows) { normal.to_lowercase() } else { normal }
}
