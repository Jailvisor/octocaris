//! Safe zip install/uninstall for wallpaper mods.
//!
//! Invariants:
//! - extract into a staging directory inside the wallpapers root
//! - reject absolute paths, `..`, symlinks and oversized archives
//! - flatten a single top-level directory
//! - never remove anything outside the wallpapers root

use std::fs::{self, File};
use std::io::{self};
use std::path::{Component, Path, PathBuf};

use thiserror::Error;
use zip::ZipArchive;

const MAX_ENTRIES: usize = 10_000;
const MAX_TOTAL_UNCOMPRESSED: u64 = 512 * 1024 * 1024;
const MAX_ENTRY_UNCOMPRESSED: u64 = 128 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("invalid mod folder name: {0:?}")]
    InvalidFolderName(String),
    #[error("unsafe zip entry: {0}")]
    UnsafeEntry(String),
    #[error("zip has too many entries")]
    TooManyEntries,
    #[error("zip exceeds the uncompressed size limit")]
    TooLarge,
    #[error("zip is empty")]
    Empty,
    #[error("refusing to remove a path outside the wallpapers directory: {0}")]
    UnsafeRemove(PathBuf),
    #[error("not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("mod is already installed at {0}")]
    AlreadyInstalled(PathBuf),
    #[error("not an Octos mod (missing octos.json or mod.json): {0}")]
    NotAMod(PathBuf),
    #[error("unsupported image type: {0}")]
    UnsupportedImage(String),
    #[error("image is too large: {size} bytes (limit {limit})")]
    ImageTooLarge { size: u64, limit: u64 },
    #[error("not a valid image file: {0}")]
    InvalidImage(PathBuf),
}

pub fn install_zip(zip_path: &Path, wallpapers_root: &Path) -> Result<PathBuf, InstallError> {
    fs::create_dir_all(wallpapers_root)?;

    let stem = zip_path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| InstallError::InvalidFolderName(String::new()))?
        .to_string();
    if !valid_folder_name(&stem) {
        return Err(InstallError::InvalidFolderName(stem));
    }

    let staging = wallpapers_root.join(format!(
        ".octos-install-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;

    let result = extract_into(&staging, zip_path)
        .and_then(|()| finalize_install(&staging, wallpapers_root, &stem));

    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

/// Copy an existing mod folder into the wallpapers directory.
///
/// The folder name becomes the installed mod folder. A folder that already
/// lives inside the wallpapers directory is treated as already installed.
pub fn install_folder(source: &Path, wallpapers_root: &Path) -> Result<PathBuf, InstallError> {
    fs::create_dir_all(wallpapers_root)?;
    let source = source.canonicalize()?;
    if !source.is_dir() {
        return Err(InstallError::NotADirectory(source));
    }
    let root = wallpapers_root.canonicalize()?;
    if source.starts_with(&root) {
        return Err(InstallError::AlreadyInstalled(source));
    }
    if !has_manifest(&source) {
        return Err(InstallError::NotAMod(source));
    }

    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| InstallError::InvalidFolderName(String::new()))?
        .to_string();
    if !valid_folder_name(&name) {
        return Err(InstallError::InvalidFolderName(name));
    }

    let staging = wallpapers_root.join(format!(
        ".octos-import-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;

    let result = copy_tree(&source, &staging)
        .and_then(|()| finalize_install(&staging, wallpapers_root, &name));
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), InstallError> {
    let mut files = 0usize;
    let mut total = 0u64;
    copy_dir(source, destination, &mut files, &mut total)
}

fn copy_dir(
    source: &Path,
    destination: &Path,
    files: &mut usize,
    total: &mut u64,
) -> Result<(), InstallError> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        if is_link(&metadata) {
            return Err(InstallError::UnsafeEntry(format!(
                "symlink: {}",
                path.display()
            )));
        }
        if file_type.is_dir() {
            copy_dir(&path, &destination.join(entry.file_name()), files, total)?;
        } else if file_type.is_file() {
            *files += 1;
            if *files > MAX_ENTRIES {
                return Err(InstallError::TooManyEntries);
            }
            *total = total.saturating_add(metadata.len());
            if *total > MAX_TOTAL_UNCOMPRESSED {
                return Err(InstallError::TooLarge);
            }
            fs::copy(&path, destination.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn extract_into(staging: &Path, zip_path: &Path) -> Result<(), InstallError> {
    let file = File::open(zip_path)?;
    let mut archive = ZipArchive::new(file)?;
    if archive.is_empty() {
        return Err(InstallError::Empty);
    }
    if archive.len() > MAX_ENTRIES {
        return Err(InstallError::TooManyEntries);
    }

    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let raw_name = entry.name()?.into_owned();
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| InstallError::UnsafeEntry(raw_name.clone()))?;
        if !safe_relative(&relative) {
            return Err(InstallError::UnsafeEntry(raw_name));
        }
        if entry.is_symlink() {
            return Err(InstallError::UnsafeEntry(format!(
                "symlink: {}",
                relative.display()
            )));
        }

        let size = entry.size();
        if size > MAX_ENTRY_UNCOMPRESSED {
            return Err(InstallError::TooLarge);
        }
        total = total.saturating_add(size);
        if total > MAX_TOTAL_UNCOMPRESSED {
            return Err(InstallError::TooLarge);
        }

        let destination = staging.join(&relative);
        if !destination.starts_with(staging) {
            return Err(InstallError::UnsafeEntry(raw_name));
        }
        if entry.is_dir() {
            fs::create_dir_all(&destination)?;
        } else {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = File::create(&destination)?;
            io::copy(&mut entry, &mut output)?;
        }
    }
    Ok(())
}

fn finalize_install(
    staging: &Path,
    wallpapers_root: &Path,
    stem: &str,
) -> Result<PathBuf, InstallError> {
    let entries: Vec<PathBuf> = fs::read_dir(staging)?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    if entries.is_empty() {
        return Err(InstallError::Empty);
    }

    let source = if entries.len() == 1 && entries[0].is_dir() {
        entries[0].clone()
    } else {
        staging.to_path_buf()
    };
    if !has_manifest(&source) {
        return Err(InstallError::NotAMod(source));
    }

    let destination = wallpapers_root.join(stem);
    if destination.exists() {
        fs::remove_dir_all(&destination)?;
    }
    fs::rename(&source, &destination)?;
    if source != staging {
        let _ = fs::remove_dir_all(staging);
    }
    Ok(destination)
}

fn has_manifest(folder: &Path) -> bool {
    folder.join("octos.json").is_file() || folder.join("mod.json").is_file()
}

fn safe_relative(path: &Path) -> bool {
    if path.is_absolute() || path.as_os_str().is_empty() {
        return false;
    }
    path.components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn valid_folder_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.ends_with([' ', '.']) {
        return false;
    }
    if name.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    }) {
        return false;
    }
    let upper = name.to_ascii_uppercase();
    !matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Remove an installed mod, but only when it is inside `wallpapers_root`.
pub fn remove_mod(wallpapers_root: &Path, folder: &Path) -> Result<(), InstallError> {
    let root = wallpapers_root.canonicalize()?;
    let target = folder.canonicalize()?;
    if target == root || !target.starts_with(&root) {
        return Err(InstallError::UnsafeRemove(folder.to_path_buf()));
    }
    fs::remove_dir_all(target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::*;

    fn write_zip(path: &Path, entries: &[(&str, &str)]) -> io::Result<()> {
        let file = File::create(path)?;
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, contents) in entries {
            writer.start_file(*name, options)?;
            writer.write_all(contents.as_bytes())?;
        }
        writer.finish()?;
        Ok(())
    }

    #[test]
    fn flattens_single_top_level_folder() {
        let temp = tempdir().unwrap();
        let zip_path = temp.path().join("MyMod.zip");
        write_zip(
            &zip_path,
            &[
                ("MyMod/octos.json", r#"{"name":"My Mod"}"#),
                ("MyMod/index.html", "<!doctype html>"),
            ],
        )
        .unwrap();

        let wallpapers = temp.path().join("wallpapers");
        let installed = install_zip(&zip_path, &wallpapers).unwrap();
        assert_eq!(installed, wallpapers.join("MyMod"));
        assert!(installed.join("octos.json").is_file());
        assert!(installed.join("index.html").is_file());
    }

    #[test]
    fn rejects_path_traversal() {
        let temp = tempdir().unwrap();
        let zip_path = temp.path().join("Evil.zip");
        write_zip(&zip_path, &[("../evil.txt", "nope")]).unwrap();

        let error = install_zip(&zip_path, &temp.path().join("wallpapers")).unwrap_err();
        assert!(matches!(error, InstallError::UnsafeEntry(_)));
    }

    #[test]
    fn rejects_symlinks() {
        let temp = tempdir().unwrap();
        let zip_path = temp.path().join("Link.zip");
        let file = File::create(&zip_path).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer
            .add_symlink("Link/link", "/etc/passwd", options)
            .unwrap();
        writer.finish().unwrap();

        let error = install_zip(&zip_path, &temp.path().join("wallpapers")).unwrap_err();
        assert!(matches!(error, InstallError::UnsafeEntry(_)));
    }

    #[test]
    fn rejects_invalid_folder_names() {
        let temp = tempdir().unwrap();
        let zip_path = temp.path().join("bad:name.zip");
        write_zip(&zip_path, &[("index.html", "<!doctype html>")]).unwrap();

        let error = install_zip(&zip_path, &temp.path().join("wallpapers")).unwrap_err();
        assert!(matches!(error, InstallError::InvalidFolderName(_)));
    }

    #[test]
    fn imports_a_folder_and_replaces_the_destination() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("Imported");
        fs::create_dir_all(source.join("assets")).unwrap();
        fs::write(source.join("octos.json"), r#"{"name":"Imported"}"#).unwrap();
        fs::write(source.join("assets/one.txt"), "one").unwrap();

        let wallpapers = temp.path().join("wallpapers");
        let installed = install_folder(&source, &wallpapers).unwrap();
        assert_eq!(installed, wallpapers.join("Imported"));
        assert!(installed.join("assets/one.txt").is_file());

        fs::write(source.join("index.html"), "<!doctype html>").unwrap();
        let installed = install_folder(&source, &wallpapers).unwrap();
        assert!(installed.join("index.html").is_file());
    }

    #[test]
    fn refuses_to_import_an_installed_folder_again() {
        let temp = tempdir().unwrap();
        let wallpapers = temp.path().join("wallpapers");
        let source = wallpapers.join("AlreadyThere");
        fs::create_dir_all(&source).unwrap();

        let error = install_folder(&source, &wallpapers).unwrap_err();
        assert!(matches!(error, InstallError::AlreadyInstalled(_)));
    }

    #[cfg(unix)]
    #[test]
    fn import_rejects_folders_without_manifest() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("NotAMod");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("index.html"), "<!doctype html>").unwrap();

        let error = install_folder(&source, &temp.path().join("wallpapers")).unwrap_err();
        assert!(matches!(error, InstallError::NotAMod(_)));
    }

    #[test]
    fn zip_rejects_archives_without_manifest() {
        let temp = tempdir().unwrap();
        let zip_path = temp.path().join("Assets.zip");
        write_zip(&zip_path, &[("Assets/index.html", "<!doctype html>")]).unwrap();

        let error = install_zip(&zip_path, &temp.path().join("wallpapers")).unwrap_err();
        assert!(matches!(error, InstallError::NotAMod(_)));
    }

    #[cfg(unix)]
    #[test]
    fn import_rejects_symlinks() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("Linked");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("octos.json"), r#"{"name":"Linked"}"#).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", source.join("passwd")).unwrap();
        let wallpapers = temp.path().join("wallpapers");
        let error = install_folder(&source, &wallpapers).unwrap_err();
        assert!(matches!(error, InstallError::UnsafeEntry(_)));
    }

    #[test]
    fn remove_only_inside_wallpapers_root() {
        let temp = tempdir().unwrap();
        let wallpapers = temp.path().join("wallpapers");
        let installed = wallpapers.join("MyMod");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("index.html"), "hi").unwrap();

        remove_mod(&wallpapers, &installed).unwrap();
        assert!(!installed.exists());

        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let error = remove_mod(&wallpapers, &outside).unwrap_err();
        assert!(matches!(error, InstallError::UnsafeRemove(_)));
        assert!(outside.exists());
    }
}
