use std::fs;
use std::io;
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

/// The entry (from `symlink_metadata`) is a reparse point: a symlink, a junction or an App Execution Alias.
#[cfg(windows)]
pub fn is_reparse_point(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
}

/// No reparse points here; a symlink is the caller's own `is_symlink` check.
#[cfg(not(windows))]
pub fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}

/// `file` is a reparse point that is no symlink: an App Execution Alias.
pub fn is_app_alias(file: &Path) -> bool {
    fs::symlink_metadata(file).is_ok_and(|m| is_reparse_point(&m) && !m.file_type().is_symlink())
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn os_error(e: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(e.code().0 & 0xffff)
}

/// `MoveFileExW`, which refuses to replace a running exe; [`rename_over`] is the one that does not.
#[cfg(windows)]
pub(crate) fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW};
    use windows::core::PCWSTR;
    let (from, to) = (wide(from), wide(to));
    unsafe { MoveFileExW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) }.map_err(os_error)
}

#[cfg(not(windows))]
pub(crate) fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// A POSIX-semantics rename (`FileRenameInfoEx`) that replaces even a running exe; FAT and shares refuse it.
#[cfg(windows)]
pub(crate) fn rename_over(from: &Path, to: &Path) -> io::Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, DELETE, FILE_FLAGS_AND_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FileRenameInfoEx, OPEN_EXISTING, SetFileInformationByHandle,
    };
    use windows::core::PCWSTR;
    const REPLACE_IF_EXISTS: u32 = 0x1;
    const POSIX_SEMANTICS: u32 = 0x2;
    let from = wide(from);
    let to = wide(to);
    let name = &to[..to.len() - 1];
    let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let size = offset + std::mem::size_of_val(to.as_slice());
    let mut buf = vec![0u64; size.div_ceil(8)];
    let info = buf.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: `buf` is zeroed, 8-aligned and holds the header and the name with its nul.
    unsafe {
        (*info).Anonymous.Flags = REPLACE_IF_EXISTS | POSIX_SEMANTICS;
        (*info).FileNameLength = std::mem::size_of_val(name) as u32;
        std::ptr::copy_nonoverlapping(name.as_ptr(), info.cast::<u8>().add(offset).cast::<u16>(), name.len());
    }
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    let handle = unsafe { CreateFileW(PCWSTR(from.as_ptr()), DELETE.0, share, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }
        .map_err(os_error)?;
    let renamed = unsafe { SetFileInformationByHandle(handle, FileRenameInfoEx, info.cast(), size as u32) };
    let _ = unsafe { CloseHandle(handle) };
    renamed.map_err(os_error)
}

#[cfg(not(windows))]
pub(crate) fn rename_over(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// `DeleteFileW`, which refuses a running exe, for the same reason as [`move_file`].
#[cfg(windows)]
pub(crate) fn delete_file(path: &Path) -> io::Result<()> {
    use windows::Win32::Storage::FileSystem::DeleteFileW;
    use windows::core::PCWSTR;
    let wide = wide(path);
    unsafe { DeleteFileW(PCWSTR(wide.as_ptr())) }.map_err(os_error)
}

#[cfg(not(windows))]
pub(crate) fn delete_file(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

