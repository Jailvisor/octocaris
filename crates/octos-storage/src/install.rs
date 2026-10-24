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
    #[error(
        "Wallpaper Engine {kind} packages are not supported; only \"web\" wallpapers can be imported"
    )]
    UnsupportedWorkshop { kind: String },
    #[error("unsupported image type: {0}")]
    UnsupportedImage(String),
    #[error("image is too large: {size} bytes (limit {limit})")]
    ImageTooLarge { size: u64, limit: u64 },
    #[error("not a valid image file: {0}")]
    InvalidImage(PathBuf),
}

pub fn install_zip(zip_path: &Path, wallpapers_root: &Path) -> Result<PathBuf, InstallError> {
    fs::create_dir_all(wallpapers_root)?;
    cleanup_staging(wallpapers_root);

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
    result.map_err(|err| match err {
        InstallError::NotAMod(_) => InstallError::NotAMod(zip_path.to_path_buf()),
        other => other,
    })
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
        match workshop_project(&source) {
            WorkshopProject::Web => {}
            WorkshopProject::Unsupported(kind) => {
                return Err(InstallError::UnsupportedWorkshop { kind });
            }
            WorkshopProject::None => return Err(InstallError::NotAMod(source)),
        }
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
    if !has_manifest(&source) && !convert_wallpaper_engine(&source)? {
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

enum WorkshopProject {
    None,
    Web,
    Unsupported(String),
}

fn workshop_project(folder: &Path) -> WorkshopProject {
    let path = folder.join("project.json");
    if !path.is_file() {
        return WorkshopProject::None;
    }
    let Ok(text) = fs::read_to_string(&path) else {
        return WorkshopProject::None;
    };
    let Ok(project) = serde_json::from_str::<serde_json::Value>(&text) else {
        return WorkshopProject::None;
    };
    match project
        .get("type")
        .and_then(|value| value.as_str())
        .map(str::trim)
    {
        Some(kind) if kind.eq_ignore_ascii_case("web") => WorkshopProject::Web,
        Some(kind) if !kind.is_empty() => WorkshopProject::Unsupported(kind.to_string()),
        _ => WorkshopProject::None,
    }
}

/// Wallpaper Engine "web" packages are plain HTML apps. Synthesize the
/// `octos.json` the rest of the pipeline expects so they can be installed.
fn convert_wallpaper_engine(folder: &Path) -> Result<bool, InstallError> {
    match workshop_project(folder) {
        WorkshopProject::None => Ok(false),
        WorkshopProject::Unsupported(kind) => Err(InstallError::UnsupportedWorkshop { kind }),
        WorkshopProject::Web => {
            let text = fs::read_to_string(folder.join("project.json"))?;
            let project: serde_json::Value = serde_json::from_str(&text)
                .map_err(|_| InstallError::NotAMod(folder.to_path_buf()))?;
            let object = project
                .as_object()
                .ok_or_else(|| InstallError::NotAMod(folder.to_path_buf()))?;
            let entry = object
                .get("file")
                .and_then(|value| value.as_str())
                .unwrap_or("index.html");
            if !safe_relative(Path::new(entry)) || !folder.join(entry).is_file() {
                return Err(InstallError::NotAMod(folder.to_path_buf()));
            }
            let title = object
                .get("title")
                .and_then(|value| value.as_str())
                .filter(|title| !title.trim().is_empty())
                .unwrap_or("Wallpaper Engine");
            let description = object
                .get("description")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let preview = object
                .get("preview")
                .and_then(|value| value.as_str())
                .filter(|preview| {
                    safe_relative(Path::new(preview)) && folder.join(preview).is_file()
                });

            let mut config = serde_json::Map::new();
            config.insert("name".into(), serde_json::json!(title));
            config.insert("author".into(), serde_json::json!("Wallpaper Engine"));
            config.insert("description".into(), serde_json::json!(description));
            if let Some(preview) = preview {
                config.insert("image".into(), serde_json::json!(preview));
            }
            config.insert("entry".into(), serde_json::json!(entry));
            config.insert("version".into(), serde_json::json!("1.0.0"));
            fs::write(
                folder.join("octos.json"),
                serde_json::to_vec_pretty(&serde_json::Value::Object(config))
                    .map_err(io::Error::other)?,
            )?;
            Ok(true)
        }
    }
}

/// Remove staging directories left behind by an interrupted install.
pub fn cleanup_staging(wallpapers_root: &Path) -> usize {
    let Ok(entries) = fs::read_dir(wallpapers_root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if (name.starts_with(".octos-install-") || name.starts_with(".octos-image-"))
            && entry.path().is_dir()
            && fs::remove_dir_all(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
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
    fn converts_wallpaper_engine_web_folders() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("2961764492");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("project.json"),
            r#"{"type":"web","file":"index.html","title":"Saori","description":"desc","preview":"preview.gif"}"#,
        )
        .unwrap();
        fs::write(source.join("index.html"), "<!doctype html>").unwrap();
        fs::write(source.join("preview.gif"), "GIF89a").unwrap();
        let wallpapers = temp.path().join("wallpapers");

        let installed = install_folder(&source, &wallpapers).unwrap();

        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(installed.join("octos.json")).unwrap())
                .unwrap();
        assert_eq!(config["name"], "Saori");
        assert_eq!(config["entry"], "index.html");
        assert_eq!(config["image"], "preview.gif");
        assert_eq!(config["author"], "Wallpaper Engine");
    }

    #[test]
    fn accepts_capitalized_web_package_types() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("3253308696");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("project.json"),
            r#"{"type":"Web","file":"index.html","title":"Momoi"}"#,
        )
        .unwrap();
        fs::write(source.join("index.html"), "<!doctype html>").unwrap();

        let installed = install_folder(&source, &temp.path().join("wallpapers")).unwrap();

        assert!(installed.join("octos.json").is_file());
    }

    #[test]
    fn converts_wallpaper_engine_web_zips() {
        let temp = tempdir().unwrap();
        let zip_path = temp.path().join("2961764492.zip");
        write_zip(
            &zip_path,
            &[
                (
                    "2961764492/project.json",
                    r#"{"type":"web","file":"index.html","title":"Saori"}"#,
                ),
                ("2961764492/index.html", "<!doctype html>"),
            ],
        )
        .unwrap();
        let wallpapers = temp.path().join("wallpapers");

        let installed = install_zip(&zip_path, &wallpapers).unwrap();

        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(installed.join("octos.json")).unwrap())
                .unwrap();
        assert_eq!(config["name"], "Saori");
        assert_eq!(config["entry"], "index.html");
    }

    #[test]
    fn rejects_wallpaper_engine_scene_packages() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("scene-package");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("project.json"),
            r#"{"type":"scene","file":"scene.pkg","title":"Scene"}"#,
        )
        .unwrap();
        fs::write(source.join("scene.pkg"), "pkg").unwrap();

        let error = install_folder(&source, &temp.path().join("wallpapers")).unwrap_err();
        assert!(matches!(
            error,
            InstallError::UnsupportedWorkshop { kind } if kind == "scene"
        ));
    }

    #[test]
    fn cleanup_removes_stale_staging_directories() {
        let temp = tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".octos-install-1-2")).unwrap();
        fs::create_dir_all(temp.path().join(".octos-image-3-4")).unwrap();
        fs::create_dir_all(temp.path().join("real-mod")).unwrap();

        assert_eq!(cleanup_staging(temp.path()), 2);
        assert!(!temp.path().join(".octos-install-1-2").exists());
        assert!(!temp.path().join(".octos-image-3-4").exists());
        assert!(temp.path().join("real-mod").is_dir());
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
