use std::fs;
use std::path::Path;

/// What an executable's file name ends with: `.exe` on Windows.
pub const EXE_SUFFIX: &str = std::env::consts::EXE_SUFFIX;

/// The file name of the program `stem`: `sm.exe` for `sm` on Windows.
pub fn exe_name(stem: &str) -> String {
    format!("{stem}{EXE_SUFFIX}")
}

/// The entry carries the hidden attribute.
#[cfg(windows)]
pub fn hidden(entry: &fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN;
    entry.metadata().is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN.0 != 0)
}

/// No hidden attribute to read here; a dot name is the caller's own check.
#[cfg(not(windows))]
pub fn hidden(_: &fs::DirEntry) -> bool {
    false
}

/// `file` is a reparse point that is no symlink: an App Execution Alias.
#[cfg(windows)]
pub fn is_app_alias(file: &Path) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    fs::symlink_metadata(file).is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 && !m.file_type().is_symlink())
}

/// No App Execution Alias exists here.
#[cfg(not(windows))]
pub fn is_app_alias(_: &Path) -> bool {
    false
}
