//! Per-wallpaper default settings injected before page scripts.
//!
//! Old "Vue + loadJson" wallpaper packages keep their settings in
//! `localStorage[loadJson.fileNames[0]]` and merge it over hardcoded JS
//! defaults. This module reads `octos-defaults.json` from the mod folder and
//! generates a script that seeds that storage entry. New packages that carry a
//! `SettingModel` in `loadJson.json` are intentionally not handled here: their
//! defaults live in the package file and need a different (versioned) write.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const FILE: &str = "octos-defaults.json";

const MAX_BYTES: u64 = 64 * 1024;
const MAX_KEYS: usize = 32;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Only fill keys the user has not saved yet.
    #[default]
    Fill,
    /// Overwrite the listed keys on every load.
    Force,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebDefaults {
    pub schema_version: u32,
    pub mode: Mode,
    /// localStorage key -> values to merge.
    pub storage: BTreeMap<String, Map<String, Value>>,
}

impl Default for WebDefaults {
    fn default() -> Self {
        Self {
            schema_version: 1,
            mode: Mode::Fill,
            storage: BTreeMap::new(),
        }
    }
}

/// Load and validate `<folder>/octos-defaults.json`.
pub fn load(folder: &Path) -> Option<WebDefaults> {
    let path = folder.join(FILE);
    let metadata = std::fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    let defaults: WebDefaults = serde_json::from_str(&text).ok()?;
    if defaults.schema_version != 1
        || defaults.storage.is_empty()
        || defaults.storage.len() > MAX_KEYS
        || defaults
            .storage
            .iter()
            .any(|(key, values)| key.trim().is_empty() || key.len() > 128 || values.is_empty())
    {
        return None;
    }
    Some(defaults)
}

/// Generate the script installed after the per-monitor storage scope.
pub fn script(defaults: &WebDefaults) -> String {
    let storage = serde_json::to_string(&defaults.storage).unwrap_or_else(|_| "{}".to_string());
    let force = matches!(defaults.mode, Mode::Force);
    let template = r#"(function(){const d=__DEFAULTS__;const f=__FORCE__;for(const k of Object.keys(d)){try{const c=JSON.parse(localStorage.getItem(k)||"{}");const m=f?Object.assign({},c,d[k]):Object.assign({},d[k],c);localStorage.setItem(k,JSON.stringify(m));}catch(_){}}})();"#;
    template
        .replace("__DEFAULTS__", &storage)
        .replace("__FORCE__", if force { "true" } else { "false" })
}

/// Only old-style packages (`loadJson.json` with `fileNames`, no `SettingModel`).
pub fn script_for_folder(folder: &Path) -> Option<String> {
    let load_text = std::fs::read_to_string(folder.join("loadJson.json")).ok()?;
    let load_config: Value = serde_json::from_str(&load_text).ok()?;
    if load_config.get("SettingModel").is_some() || load_config.get("fileNames").is_none() {
        return None;
    }
    let defaults = load(folder)?;
    Some(script(&defaults))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_defaults(folder: &Path, text: &str) {
        std::fs::write(folder.join(FILE), text).unwrap();
    }

    #[test]
    fn loads_old_style_defaults_and_generates_fill_script() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("loadJson.json"),
            r#"{"fileNames":["CH0178_home"]}"#,
        )
        .unwrap();
        write_defaults(
            dir.path(),
            r#"{"schemaVersion":1,"mode":"fill","storage":{"CH0178_home":{"bgmVolume":0,"textureResolution":"4k"}}}"#,
        );
        let defaults = load(dir.path()).unwrap();
        assert_eq!(defaults.mode, Mode::Fill);
        let script = script(&defaults);
        assert!(script.contains("CH0178_home"));
        assert!(script.contains("\"bgmVolume\":0"));
        assert!(script.contains("const f=false"));
        assert!(script_for_folder(dir.path()).is_some());
    }

    #[test]
    fn force_mode_sets_the_flag() {
        let defaults = WebDefaults {
            mode: Mode::Force,
            ..Default::default()
        };
        assert!(script(&defaults).contains("const f=true"));
    }

    #[test]
    fn ignores_new_style_packages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("loadJson.json"),
            r#"{"SettingModel":{"version":5,"skel":"CH0355_home.skel"}}"#,
        )
        .unwrap();
        write_defaults(
            dir.path(),
            r#"{"schemaVersion":1,"storage":{"CH0355_home.skel":{"bgmVolume":0}}}"#,
        );
        assert!(load(dir.path()).is_some());
        assert!(script_for_folder(dir.path()).is_none());
    }

    #[test]
    fn rejects_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("loadJson.json"), r#"{"fileNames":["x"]}"#).unwrap();
        write_defaults(dir.path(), "not json");
        assert!(load(dir.path()).is_none());
        write_defaults(dir.path(), r#"{"schemaVersion":2,"storage":{"x":{"a":1}}}"#);
        assert!(load(dir.path()).is_none());
        write_defaults(dir.path(), r#"{"schemaVersion":1,"storage":{}}"#);
        assert!(load(dir.path()).is_none());
        write_defaults(dir.path(), r#"{"schemaVersion":1,"storage":{"x":{}}}"#);
        assert!(load(dir.path()).is_none());
    }
}
