//! The OS boundary: each module holds the Windows code of one concern, and elsewhere a stub that refuses with
//! [`NOT_HERE`].

pub mod fs;
pub mod locale;
pub mod open;
pub mod process;

/// What a stub says off Windows, the one platform implemented.
pub const NOT_HERE: &str = "not on this platform";

#[cfg(not(windows))]
pub(crate) fn refused(what: &str) -> String {
    format!("{what}: {NOT_HERE}")
}
