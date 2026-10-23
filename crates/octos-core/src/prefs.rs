//! Versioned user preferences.
//!
//! The on-disk shape stays close to the original Octos file so old
//! `selected`/`modOptions` keys can be merged directly. Unknown top-level keys
//! are preserved through `extra` and written back untouched.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::options::OptionsMap;

pub const CURRENT_VERSION: u32 = 2;

fn current_version() -> u32 {
    CURRENT_VERSION
}

fn default_max_fps() -> u32 {
    30
}

fn default_true() -> bool {
    true
}

/// Application settings stored under `appOptions`.
///
/// Only settings the Rust host actually honours are typed. Everything else the
/// original app wrote (`allowNotifs`, `memorySaver`, `enableSandboxing`, ...)
/// stays in `extra` and is written back unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppOptions {
    pub run_on_startup: bool,
    /// Defaults to `true`: the Rust host has always run with the GPU enabled.
    #[serde(rename = "enableGPU")]
    pub enable_gpu: bool,
    /// Pause wallpaper rendering while another window is fullscreen.
    pub pause_fullscreen: bool,
    /// requestAnimationFrame cap for wallpaper pages (0 = unlimited).
    #[serde(default = "default_max_fps")]
    pub max_fps: u32,
    /// Suspend wallpaper WebViews after they have been hidden for a few seconds.
    #[serde(default = "default_true")]
    pub memory_saver: bool,
    /// Preset that lowers the frame rate and enables MemorySaver.
    #[serde(default)]
    pub lite_mode: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            run_on_startup: false,
            enable_gpu: true,
            pause_fullscreen: true,
            max_fps: default_max_fps(),
            memory_saver: true,
            lite_mode: false,
            extra: Map::new(),
        }
    }
}

/// Restored main-window rectangle in physical pixels (`WINDOWPLACEMENT`'s
/// normal position, so a maximized window restores to its previous size).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default = "current_version")]
    pub version: u32,
    /// monitor id (`\\.\DISPLAYn`) -> absolute mod folder key
    #[serde(default)]
    pub selected: BTreeMap<String, String>,
    /// absolute mod folder key -> option specs (including their current values)
    #[serde(default, rename = "modOptions")]
    pub mod_options: BTreeMap<String, OptionsMap>,
    #[serde(default, rename = "appOptions")]
    pub app_options: AppOptions,
    #[serde(default)]
    pub window: Option<WindowPlacement>,
    /// Other fields written by Octos versions (`modData`, ...).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            selected: BTreeMap::new(),
            mod_options: BTreeMap::new(),
            app_options: AppOptions::default(),
            window: None,
            extra: Map::new(),
        }
    }
}

impl Preferences {
    /// Parse a preferences file, upgrading the version marker in memory.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        let mut preferences: Self = serde_json::from_str(text)?;
        preferences.version = CURRENT_VERSION;
        Ok(preferences)
    }

    pub fn options_for(&self, mod_key: &str) -> Option<&OptionsMap> {
        self.mod_options.get(mod_key)
    }

    pub fn set_selected(&mut self, selected: BTreeMap<String, String>) {
        self.selected = selected;
    }

    pub fn set_mod_options(&mut self, options: BTreeMap<String, OptionsMap>) {
        self.mod_options = options;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_v2_and_empty() {
        let preferences = Preferences::default();
        assert_eq!(preferences.version, 2);
        assert!(preferences.selected.is_empty());
        assert!(preferences.mod_options.is_empty());
    }

    #[test]
    fn parses_legacy_and_unknown_fields() {
        let text = r#"{
            "selected": {"\\\\.\\DISPLAY1": "C:/wallpapers/one"},
            "modOptions": {"C:/wallpapers/one": {"speed": {"value": 30}}},
            "modData": {"C:/wallpapers/one": {"note": "keep"}},
            "appOptions": {"allowNotifs": true}
        }"#;
        let preferences = Preferences::from_json(text).unwrap();
        assert_eq!(preferences.version, 2);
        assert_eq!(
            preferences
                .selected
                .get("\\\\.\\DISPLAY1")
                .map(String::as_str),
            Some("C:/wallpapers/one")
        );
        assert_eq!(
            preferences.mod_options["C:/wallpapers/one"]["speed"],
            json!({"value": 30})
        );
        assert_eq!(preferences.app_options.extra["allowNotifs"], json!(true));
        assert!(preferences.app_options.enable_gpu);
        assert!(preferences.app_options.pause_fullscreen);
        assert_eq!(preferences.app_options.max_fps, 30);
        assert!(!preferences.app_options.run_on_startup);
        assert_eq!(
            preferences.extra["modData"]["C:/wallpapers/one"]["note"],
            json!("keep")
        );
    }

    #[test]
    fn window_placement_round_trips() {
        let text = r#"{"window":{"x":120,"y":80,"width":1100,"height":720}}"#;
        let preferences = Preferences::from_json(text).unwrap();
        assert_eq!(
            preferences.window,
            Some(WindowPlacement {
                x: 120,
                y: 80,
                width: 1100,
                height: 720,
            })
        );
        let encoded = serde_json::to_string(&preferences).unwrap();
        let decoded: Preferences = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.window, preferences.window);
    }

    #[test]
    fn app_options_round_trip_and_keep_unknown_keys() {
        let text = r#"{
            "appOptions": {
                "runOnStartup": true,
                "enableGPU": false,
                "pauseFullscreen": false,
                "maxFps": 60,
                "memorySaver": true,
                "allowNotifs": false
            }
        }"#;
        let preferences = Preferences::from_json(text).unwrap();
        assert!(preferences.app_options.run_on_startup);
        assert!(!preferences.app_options.enable_gpu);
        assert!(!preferences.app_options.pause_fullscreen);
        assert_eq!(preferences.app_options.max_fps, 60);
        assert!(preferences.app_options.memory_saver);
        assert!(!preferences.app_options.lite_mode);
        assert_eq!(preferences.app_options.extra["allowNotifs"], json!(false));

        let encoded = serde_json::to_string(&preferences).unwrap();
        let decoded: Preferences = serde_json::from_str(&encoded).unwrap();
        assert!(decoded.app_options.run_on_startup);
        assert!(decoded.app_options.memory_saver);
        assert_eq!(decoded.app_options.max_fps, 60);
    }

    #[test]
    fn round_trip_keeps_unknown_fields() {
        let text = r#"{
            "version": 1,
            "selected": {},
            "appOptions": {"runOnStartup": false}
        }"#;
        let preferences = Preferences::from_json(text).unwrap();
        let encoded = serde_json::to_string(&preferences).unwrap();
        let decoded: Preferences = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.version, 2);
        assert!(!decoded.app_options.run_on_startup);
        assert!(decoded.app_options.enable_gpu);
    }
}
