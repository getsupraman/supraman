//! The OS boundary: the Windows code of each concern, elsewhere a stub that refuses with [`NOT_HERE`].

pub mod desktop;
pub mod fs;
pub mod locale;
pub mod open;
pub mod paths;
pub mod process;
pub mod secret;
pub mod shell;

/// What a stub says off Windows, the one platform implemented.
pub const NOT_HERE: &str = "not on this platform";

#[cfg(not(windows))]
pub(crate) fn refused(what: &str) -> String {
    format!("{what}: {NOT_HERE}")
}
