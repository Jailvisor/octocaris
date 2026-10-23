//! Atomic file replacement for user state.
//!
//! Writes go to a sibling temp file, are flushed, then replace the destination
//! in one step. On Windows `ReplaceFileW` also keeps the previous file as
//! `<name>.bak`; the non-Windows implementation copies it aside first for the
//! same recovery property in tests.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` through a temporary file in the same directory.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "octos".to_string());
    let temp = parent.join(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    {
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if let Err(err) = replace_file(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(err);
    }
    Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "octos".into());
    name.push(".bak");
    path.with_file_name(name)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        REPLACEFILE_IGNORE_MERGE_ERRORS, ReplaceFileW,
    };
    use windows::core::PCWSTR;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let backup = backup_path(destination);
    let replaced_name = wide(destination);
    let replacement_name = wide(source);
    let backup_name = wide(&backup);

    if destination.exists() {
        let replaced = unsafe {
            ReplaceFileW(
                PCWSTR(replaced_name.as_ptr()),
                PCWSTR(replacement_name.as_ptr()),
                PCWSTR(backup_name.as_ptr()),
                REPLACEFILE_IGNORE_MERGE_ERRORS,
                None,
                None,
            )
        };
        if replaced.is_ok() {
            return Ok(());
        }
        // Some filesystems reject ReplaceFileW; keep a best-effort backup and
        // fall through to the move-based replacement.
        let _ = fs::copy(destination, &backup);
    }

    unsafe {
        MoveFileExW(
            PCWSTR(replacement_name.as_ptr()),
            PCWSTR(replaced_name.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|_| io::Error::last_os_error())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.exists() {
        let _ = fs::copy(destination, backup_path(destination));
    }
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn second_write_keeps_the_previous_file_as_bak() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("preferences.json");

        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"second");
        assert_eq!(fs::read(backup_path(&path)).unwrap(), b"first");
        let leftovers = fs::read_dir(temp.path())
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn first_write_has_no_backup() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("preferences.json");

        write_atomic(&path, b"first").unwrap();

        assert!(!backup_path(&path).exists());
    }
}
