//! Load and persist `preferences.json` without losing unknown fields.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use octos_core::prefs::Preferences;
use tracing::{debug, warn};

use crate::atomic::write_atomic;

/// Coalesce writes triggered by sliders and window moves.
const DEBOUNCE: Duration = Duration::from_millis(250);

enum SaveMessage {
    Schedule {
        path: PathBuf,
        preferences: Box<Preferences>,
    },
    Flush(Sender<()>),
}

/// Serializes preferences writes on a background thread and debounces bursts.
///
/// The UI thread calls [`schedule`](Self::schedule) with a snapshot; the writer
/// keeps only the newest snapshot and writes after `DEBOUNCE` of quiet. On
/// exit [`flush`](Self::flush) writes the pending snapshot and joins.
pub struct PrefsWriter {
    tx: Sender<SaveMessage>,
    handle: Option<JoinHandle<()>>,
}

impl PrefsWriter {
    pub fn start() -> Self {
        Self::start_with_debounce(DEBOUNCE)
    }

    fn start_with_debounce(debounce: Duration) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("octos-prefs".to_string())
            .spawn(move || writer_loop(rx, debounce))
            .expect("failed to spawn the preferences writer");
        Self {
            tx,
            handle: Some(handle),
        }
    }

    /// Queue a snapshot. Returns false when the writer is no longer running.
    pub fn schedule(&self, path: PathBuf, preferences: Preferences) -> bool {
        self.tx
            .send(SaveMessage::Schedule {
                path,
                preferences: Box::new(preferences),
            })
            .is_ok()
    }

    /// Write anything pending and stop the writer.
    pub fn flush(&mut self) {
        let (ack_tx, ack_rx) = mpsc::channel();
        if self.tx.send(SaveMessage::Flush(ack_tx)).is_ok() {
            let _ = ack_rx.recv();
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for PrefsWriter {
    fn drop(&mut self) {
        self.flush();
    }
}

fn writer_loop(rx: Receiver<SaveMessage>, debounce: Duration) {
    let mut pending: Option<(PathBuf, Preferences)> = None;
    loop {
        let message = if pending.is_some() {
            match rx.recv_timeout(debounce) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => {
                    if let Some((path, preferences)) = pending.take() {
                        write_now(&path, &preferences);
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    if let Some((path, preferences)) = pending.take() {
                        write_now(&path, &preferences);
                    }
                    break;
                }
            }
        } else {
            match rx.recv() {
                Ok(message) => message,
                Err(_) => break,
            }
        };

        match message {
            SaveMessage::Schedule { path, preferences } => {
                pending = Some((path, *preferences));
            }
            SaveMessage::Flush(ack) => {
                if let Some((path, preferences)) = pending.take() {
                    write_now(&path, &preferences);
                }
                let _ = ack.send(());
                break;
            }
        }
    }
}

fn write_now(path: &Path, preferences: &Preferences) {
    match save_preferences(path, preferences) {
        Ok(()) => debug!("preferences written to {}", path.display()),
        Err(err) => warn!("could not save preferences: {err}"),
    }
}

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
        assert_eq!(preferences.version, 3);
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
    fn writer_flushes_the_newest_snapshot() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("preferences.json");
        let mut writer = PrefsWriter::start_with_debounce(Duration::from_millis(20));

        let mut first = Preferences::default();
        first.selected.insert("monitor".into(), "first".into());
        assert!(writer.schedule(path.clone(), first));
        let mut second = Preferences::default();
        second.selected.insert("monitor".into(), "second".into());
        assert!(writer.schedule(path.clone(), second));

        writer.flush();
        let loaded = load_preferences(&path);
        assert_eq!(
            loaded.selected.get("monitor").map(String::as_str),
            Some("second")
        );
    }

    #[test]
    fn writer_writes_after_the_debounce_window() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("preferences.json");
        let writer = PrefsWriter::start_with_debounce(Duration::from_millis(20));
        assert!(writer.schedule(path.clone(), Preferences::default()));

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(path.exists());
    }

    #[test]
    fn corrupt_file_is_quarantined() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("preferences.json");
        std::fs::write(&path, "{ not json").unwrap();

        let preferences = load_preferences(&path);
        assert_eq!(preferences.version, 3);
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
