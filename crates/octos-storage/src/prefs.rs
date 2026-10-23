//! Load and persist `preferences.json` without losing unknown fields.

use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use octos_core::prefs::Preferences;
use tracing::warn;

use crate::atomic::write_atomic;

pub fn load_preferences(path: &Path) -> Preferences {
    match std::fs::read_to_string(path) {
        Ok(text) => match Preferences::from_json(&text) {
            Ok(preferences) => preferences,
            Err(err) => {
                warn!(
                    "could not parse {}: {err}; starting with defaults",
                    path.display()
                );
                quarantine(path);
                Preferences::default()
            }
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => Preferences::default(),
        Err(err) => {
            warn!(
                "could not read {}: {err}; starting with defaults",
                path.display()
            );
            Preferences::default()
        }
    }
}

pub fn save_preferences(path: &Path, preferences: &Preferences) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(preferences).map_err(io::Error::other)?;
    write_atomic(path, &bytes)
}

fn quarantine(path: &Path) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let backup = path.with_file_name(format!("preferences.corrupt-{stamp}.json"));
    if let Err(err) = std::fs::rename(path, &backup) {
        warn!("could not move corrupt preferences aside: {err}");
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn missing_file_is_default() {
        let temp = tempdir().unwrap();
        let preferences = load_preferences(&temp.path().join("preferences.json"));
        assert_eq!(preferences.version, 2);
        assert!(preferences.selected.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("preferences.json");
        let mut preferences = Preferences::default();
        preferences
            .selected
            .insert("\\\\.\\DISPLAY1".into(), r"C:\mods\one".into());
        save_preferences(&path, &preferences).unwrap();

        let loaded = load_preferences(&path);
        assert_eq!(
            loaded.selected.get("\\\\.\\DISPLAY1").map(String::as_str),
            Some(r"C:\mods\one")
        );
    }

    #[test]
    fn corrupt_file_is_quarantined() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("preferences.json");
        std::fs::write(&path, "{ not json").unwrap();

        let preferences = load_preferences(&path);
        assert_eq!(preferences.version, 2);
        assert!(!path.exists());
        let backups = std::fs::read_dir(temp.path())
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("preferences.corrupt-")
            })
            .count();
        assert_eq!(backups, 1);
    }
}
