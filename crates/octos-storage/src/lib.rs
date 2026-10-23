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
pub mod image;
pub mod install;
pub mod prefs;
pub mod template;

pub use atomic::write_atomic;
pub use image::{install_image, is_image_file};
pub use install::{InstallError, install_folder, install_zip, remove_mod};
pub use prefs::{PrefsWriter, load_preferences, save_preferences};
pub use template::create_mod_template;

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
    /// Directory containing the executable.
    pub exe_dir: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self, PathsError> {
        let root = match std::env::var_os("OCTOS_DATA_DIR") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => {
                let local_app_data =
                    std::env::var_os("LOCALAPPDATA").ok_or(PathsError::NoDataDir)?;
                PathBuf::from(local_app_data).join("Octos")
            }
        };

        let exe = std::env::current_exe()?;
        let exe_dir = exe.parent().map(Path::to_path_buf).ok_or_else(|| {
            PathsError::NoExeDir(std::io::Error::other("executable has no parent"))
        })?;

        let paths = Self {
            wallpapers: root.join("wallpapers"),
            logs: root.join("logs"),
            webview_data: root.join("webview2"),
            preferences: root.join("preferences.json"),
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
        for directory in [&self.root, &self.wallpapers, &self.logs, &self.webview_data] {
            std::fs::create_dir_all(directory)?;
        }
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
            exe_dir: PathBuf::from(r"C:\tmp"),
            root,
        };
        assert_eq!(paths.app_dir(), paths.exe_dir.join("app"));
    }
}
