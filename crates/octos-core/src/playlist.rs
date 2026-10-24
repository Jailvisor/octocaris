//! Wallpaper playlist rules: interval and the next-item choice.
//!
//! Kept platform-independent so the scheduling logic can be unit-tested on
//! Linux; the Windows app only evaluates it from the UI-thread timer.

use std::collections::HashSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const DEFAULT_INTERVAL_SECONDS: u32 = 30 * 60;
pub const MIN_INTERVAL_SECONDS: u32 = 5;
/// One week. Anything longer is indistinguishable from "off" in practice.
pub const MAX_INTERVAL_SECONDS: u32 = 7 * 24 * 60 * 60;

/// Global wallpaper rotation. Items are absolute mod folder keys, the same
/// identity `selected` and `modOptions` use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Playlist {
    pub enabled: bool,
    pub items: Vec<String>,
    /// Seconds between switches; 5s ..= 7 days.
    pub interval_seconds: u32,
    pub random: bool,
    /// The item currently applied. Written by the rotation engine so a restart
    /// resumes on the same wallpaper instead of re-deriving it from `selected`.
    pub current: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Playlist {
    fn default() -> Self {
        Self {
            enabled: false,
            items: Vec::new(),
            interval_seconds: DEFAULT_INTERVAL_SECONDS,
            random: false,
            current: None,
            extra: Map::new(),
        }
    }
}

impl Playlist {
    pub fn index_of(&self, path: &str) -> Option<usize> {
        self.items.iter().position(|item| item == path)
    }

    /// Adopt fields written by the first playlist build (minutes plus an
    /// optional time window) and drop the window, which is no longer supported.
    pub fn migrate_legacy(&mut self) {
        if let Some(value) = self.extra.remove("intervalMinutes")
            && let Some(minutes) = value.as_u64()
        {
            self.interval_seconds =
                u32::try_from(minutes.saturating_mul(60)).unwrap_or(MAX_INTERVAL_SECONDS);
        }
        self.extra.remove("window");
        self.interval_seconds = clamp_interval(self.interval_seconds);
    }
}

pub fn clamp_interval(seconds: u32) -> u32 {
    seconds.clamp(MIN_INTERVAL_SECONDS, MAX_INTERVAL_SECONDS)
}

/// Next position inside a runnable list. `rng` is injected so tests stay
/// deterministic; it must return a value below `len`.
pub fn next_index(
    len: usize,
    current: Option<usize>,
    random: bool,
    rng: &mut dyn FnMut(usize) -> usize,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    if len == 1 {
        return Some(0);
    }
    if random {
        let mut index = rng(len) % len;
        if Some(index) == current {
            // Avoid an immediate repeat without a retry loop.
            index = (index + 1) % len;
        }
        Some(index)
    } else {
        Some(current.map(|index| (index + 1) % len).unwrap_or(0))
    }
}

pub fn is_due(elapsed: Duration, interval_seconds: u32) -> bool {
    elapsed >= Duration::from_secs(u64::from(clamp_interval(interval_seconds)))
}

/// Trim, drop empty entries and de-duplicate while preserving order.
pub fn normalize_items(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty() && seen.insert(item.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_selection_wraps_and_starts_at_first() {
        assert_eq!(next_index(3, None, false, &mut |_| 0), Some(0));
        assert_eq!(next_index(3, Some(1), false, &mut |_| 0), Some(2));
        assert_eq!(next_index(3, Some(2), false, &mut |_| 0), Some(0));
        assert_eq!(next_index(1, Some(0), false, &mut |_| 0), Some(0));
        assert_eq!(next_index(0, None, false, &mut |_| 0), None);
    }

    #[test]
    fn random_selection_never_repeats_the_current_item() {
        assert_eq!(next_index(3, Some(1), true, &mut |_| 1), Some(2));
        assert_eq!(next_index(3, Some(2), true, &mut |_| 2), Some(0));
        assert_eq!(next_index(3, Some(0), true, &mut |_| 2), Some(2));
        assert_eq!(next_index(3, None, true, &mut |_| 7), Some(1));
    }

    #[test]
    fn interval_due_uses_clamped_seconds() {
        assert!(!is_due(Duration::from_secs(29), 30));
        assert!(is_due(Duration::from_secs(30), 30));
        assert!(is_due(Duration::from_secs(6), 0));
        assert_eq!(clamp_interval(0), MIN_INTERVAL_SECONDS);
        assert_eq!(clamp_interval(u32::MAX), MAX_INTERVAL_SECONDS);
    }

    #[test]
    fn normalize_items_dedupes_and_keeps_order() {
        let items = vec![
            " a ".to_string(),
            "b".to_string(),
            "".to_string(),
            "a".to_string(),
            "  ".to_string(),
            "c".to_string(),
        ];
        assert_eq!(normalize_items(items), vec!["a", "b", "c"]);
    }

    #[test]
    fn index_of_matches_exact_keys() {
        let playlist = Playlist {
            items: vec![r"C:\mods\a".into(), r"C:\mods\b".into()],
            ..Playlist::default()
        };
        assert_eq!(playlist.index_of(r"C:\mods\b"), Some(1));
        assert_eq!(playlist.index_of(r"C:\MODS\b"), None);
    }

    #[test]
    fn migration_converts_minutes_and_drops_the_window() {
        let mut playlist = Playlist {
            extra: serde_json::json!({
                "intervalMinutes": 2,
                "window": {"enabled": true, "start": "20:00", "end": "06:00"}
            })
            .as_object()
            .cloned()
            .unwrap(),
            ..Playlist::default()
        };
        playlist.migrate_legacy();
        assert_eq!(playlist.interval_seconds, 120);
        assert!(playlist.extra.is_empty());
    }

    #[test]
    fn migration_clamps_absent_or_zero_intervals() {
        let mut playlist = Playlist {
            interval_seconds: 0,
            ..Playlist::default()
        };
        playlist.migrate_legacy();
        assert_eq!(playlist.interval_seconds, MIN_INTERVAL_SECONDS);
    }
}
