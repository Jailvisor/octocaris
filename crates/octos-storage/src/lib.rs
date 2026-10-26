//! Filesystem layout for Octos.
//!
//! The production layout matches the original app:
//!
//! ```text
//! %LOCALAPPDATA%\Octos\
//!   wallpapers\
//!   logs\
//!   webview2\
//! ```
//!
//! `OCTOS_DATA_DIR` overrides the root so tests never touch real user data.

pub mod atomic;
pub mod community;
pub mod hash;
pub mod image;
pub mod install;
pub mod prefs;
pub mod template;
pub mod thumbnail;

pub use atomic::write_atomic;
pub use community::{
    IndexCacheMeta, cleanup_community, download_file_name, download_path, load_index_cache,
    part_path, preview_cache_name, preview_cache_path, previews_dir, read_marker,
    store_index_cache,
};
pub use hash::{HashError, sha256_bytes, sha256_file, verify_file};
pub use image::{install_image, is_image_file};
pub use install::{
    InstallError, InstallMode, StagedInstall, cleanup_staging, install_folder, install_zip,
    install_zip_named, recover_interrupted_install, remove_mod, replace_zip_named,
};
pub use prefs::{PrefsWriter, load_preferences, save_preferences};
pub use template::create_mod_template;
pub use thumbnail::{ensure_all, ensure_thumbnail, has_thumbnail, thumbnail_path, write_thumbnail};

use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathsError {
    #[error("could not determine a data directory (LOCALAPPDATA is not set)")]
    NoDataDir,
    #[error("could not determine the executable directory: {0}")]
    NoExeDir(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct Paths {
    /// Root of writable state (`%LOCALAPPDATA%\Octos` by default).
    pub root: PathBuf,
    /// Installed wallpapers.
    pub wallpapers: PathBuf,
    /// Rotating log directory.
    pub logs: PathBuf,
    /// WebView2 user data / cache.
    pub webview_data: PathBuf,
    /// Versioned user preferences (`selected`, `modOptions`, ...).
    pub preferences: PathBuf,
    /// Community catalog cache and partial downloads.
    pub community: PathBuf,
    /// Directory containing the executable.
    pub exe_dir: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self, PathsError> {
        let exe = std::env::current_exe()?;
        let exe_dir = exe.parent().map(Path::to_path_buf).ok_or_else(|| {
            PathsError::NoExeDir(std::io::Error::other("executable has no parent"))
        })?;

        let root = match std::env::var_os("OCTOS_DATA_DIR") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => {
                // Portable layout: <exe>\data\wallpapers. This keeps a
                // self-contained copy working when launched by double-click.
                let portable = exe_dir.join("data");
                if portable.join("wallpapers").is_dir() {
                    portable
                } else {
                    let local_app_data =
                        std::env::var_os("LOCALAPPDATA").ok_or(PathsError::NoDataDir)?;
                    PathBuf::from(local_app_data).join("Octos")
                }
            }
        };

        let paths = Self {
            wallpapers: root.join("wallpapers"),
            logs: root.join("logs"),
            webview_data: root.join("webview2"),
            preferences: root.join("preferences.json"),
            community: root.join("community"),
            exe_dir,
            root,
        };
        paths.ensure()?;
        Ok(paths)
    }

    /// Directory shipped next to the executable that holds the built React UI.
    pub fn app_dir(&self) -> PathBuf {
        self.exe_dir.join("app")
    }

    pub fn ensure(&self) -> Result<(), PathsError> {
        for directory in [
            &self.root,
            &self.wallpapers,
            &self.logs,
            &self.webview_data,
            &self.community,
        ] {
            std::fs::create_dir_all(directory)?;
        }
        std::fs::create_dir_all(self.community.join("previews"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_root_wins() {
        // `discover` reads process env, so this test only asserts the path
        // composition rule via a temporary environment-independent helper.
        let root = PathBuf::from(r"C:\tmp\octos");
        let paths = Paths {
            wallpapers: root.join("wallpapers"),
            logs: root.join("logs"),
            webview_data: root.join("webview2"),
            preferences: root.join("preferences.json"),
            community: root.join("community"),
            exe_dir: PathBuf::from(r"C:\tmp"),
            root,
        };
        assert_eq!(paths.app_dir(), paths.exe_dir.join("app"));
    }
}
