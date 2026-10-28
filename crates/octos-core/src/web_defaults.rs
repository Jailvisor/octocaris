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

/// Values the wallpaper's own panel writes per monitor. A shared default must
/// never set them: another screen/user would get someone else's geometry.
pub const VISUAL_FIELDS: [&str; 2] = ["position", "scale"];

const MAX_BYTES: u64 = 64 * 1024;
const MAX_KEYS: usize = 32;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Only fill keys the user has not saved yet.
    #[default]
    Fill,
    /// Apply the defaults once per change, then leave the user's edits alone.
    ///
    /// The applied defaults are remembered per storage key in the (per-monitor)
    /// storage, so only fields whose default changed are written again.
    Once,
    /// Legacy alias of [`Mode::Once`].
    ///
    /// Packages published before 2026-10-28 used `force` for "seed existing
    /// installs"; applied every load it silently reverted every per-screen
    /// tweak. They keep working without a re-upload by reading it as `once`.
    Force,
    /// Overwrite the listed fields on every load ("BGM 永远静音").
    Pin,
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

/// App-level defaults from preferences, applied to every old-style package.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebDefaultsSetting {
    pub enabled: bool,
    /// Field name -> default value, e.g. `{"bgmVolume": 0, "mouseTracking": false}`.
    pub fields: Map<String, Value>,
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
    let force = matches!(defaults.mode, Mode::Pin);
    let once = matches!(defaults.mode, Mode::Once | Mode::Force);
    // `once` keeps the last applied defaults next to the values, so an edit to
    // the file only overwrites the fields that actually changed. Re-applying
    // the whole entry would reset per-screen tuning (position/scale) the user
    // did in the wallpaper's own panel.
    let template = r#"(function(){const d=__DEFAULTS__;const f=__FORCE__;const o=__ONCE__;for(const k of Object.keys(d)){try{if(o){const mk="__octos_defaults__"+k;const prev=JSON.parse(localStorage.getItem(mk)||"null");const c=JSON.parse(localStorage.getItem(k)||"{}");const patch={};let changed=false;for(const field of Object.keys(d[k])){if(!prev||!(field in prev)||JSON.stringify(prev[field])!==JSON.stringify(d[k][field])){patch[field]=d[k][field];changed=true;}}if(!changed)continue;localStorage.setItem(k,JSON.stringify(Object.assign({},c,patch)));localStorage.setItem(mk,JSON.stringify(d[k]));continue;}const c=JSON.parse(localStorage.getItem(k)||"{}");const m=f?Object.assign({},c,d[k]):Object.assign({},d[k],c);localStorage.setItem(k,JSON.stringify(m));}catch(_){}}})();"#;
    template
        .replace("__DEFAULTS__", &storage)
        .replace("__FORCE__", if force { "true" } else { "false" })
        .replace("__ONCE__", if once { "true" } else { "false" })
}

/// The single localStorage key an old-style package stores its settings in.
///
/// Returns `None` for packages that do not follow the `loadJson.fileNames`
/// convention (newer packages keep a `SettingModel` instead).
pub fn storage_key(folder: &Path) -> Option<String> {
    let load_text = std::fs::read_to_string(folder.join("loadJson.json")).ok()?;
    let load_config: Value = serde_json::from_str(&load_text).ok()?;
    if load_config.get("SettingModel").is_some() {
        return None;
    }
    let key = load_config
        .get("fileNames")?
        .as_array()?
        .first()?
        .as_str()?;
    (!key.trim().is_empty()).then(|| key.to_string())
}

fn shared_field(name: &str) -> bool {
    !VISUAL_FIELDS.contains(&name)
}

/// Defaults script for one package: app-level defaults, overridden field by
/// field by the package's own `octos-defaults.json`, applied with `once`
/// semantics. `None` when there is nothing to apply.
pub fn script_for_folder(folder: &Path, global: &WebDefaultsSetting) -> Option<String> {
    let key = storage_key(folder)?;
    let mut merged = Map::new();
    if global.enabled {
        for (field, value) in &global.fields {
            if shared_field(field) {
                merged.insert(field.clone(), value.clone());
            }
        }
    }
    if let Some(defaults) = load(folder)
        && let Some(values) = defaults.storage.get(&key)
    {
        for (field, value) in values {
            if shared_field(field) {
                merged.insert(field.clone(), value.clone());
            }
        }
    }
    if merged.is_empty() {
        return None;
    }
    let defaults = WebDefaults {
        mode: Mode::Once,
        storage: BTreeMap::from([(key, merged)]),
        ..Default::default()
    };
    Some(script(&defaults))
}

/// Script that forgets which defaults were applied, so the next load seeds the
/// wallpaper again. Used by the "restore defaults" action.
pub fn reset_script(folder: &Path) -> Option<String> {
    let key = storage_key(folder)?;
    let marker = serde_json::to_string(&format!("__octos_defaults__{key}")).ok()?;
    Some(format!(
        "try{{localStorage.removeItem({marker});}}catch(_){{}}\
         if(window.location&&window.location.reload)window.location.reload();"
    ))
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
        assert!(script_for_folder(dir.path(), &WebDefaultsSetting::default()).is_some());
    }

    #[test]
    fn pin_mode_overwrites_on_every_load_and_force_is_legacy_once() {
        let pin = script(&WebDefaults {
            mode: Mode::Pin,
            ..Default::default()
        });
        assert!(pin.contains("const f=true"));
        assert!(pin.contains("const o=false"));
        // Published packages used `force` for seeding; it must not clobber.
        let legacy = script(&WebDefaults {
            mode: Mode::Force,
            ..Default::default()
        });
        assert!(legacy.contains("const f=false"));
        assert!(legacy.contains("const o=true"));
    }

    #[test]
    fn once_mode_only_patches_new_or_changed_fields() {
        let mut defaults = WebDefaults {
            mode: Mode::Once,
            ..Default::default()
        };
        defaults.storage.insert(
            "CH0139_home".to_string(),
            serde_json::from_str(r#"{"mouseTrial":false,"scale":1.5}"#).unwrap(),
        );
        let script = script(&defaults);
        assert!(script.contains("const o=true"));
        assert!(script.contains("const f=false"));
        assert!(script.contains("__octos_defaults__"));
        // The marker stores the applied defaults; only differing fields merge.
        assert!(script.contains("JSON.stringify(prev[field])!==JSON.stringify(d[k][field])"));
        assert!(script.contains("if(!changed)continue"));
        assert!(script.contains(r#""CH0139_home":{"mouseTrial":false,"scale":1.5}"#));
        // Deterministic: the same file produces the same script.
        assert_eq!(script, super::script(&defaults));
    }

    #[test]
    fn app_defaults_merge_with_package_overrides_and_skip_visual_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("loadJson.json"),
            r#"{"fileNames":["CH0139_home"]}"#,
        )
        .unwrap();
        write_defaults(
            dir.path(),
            r#"{"schemaVersion":1,"mode":"force","storage":{"CH0139_home":{
                "bgmVolume":0.3,"position":{"x":1440,"y":1800},"scale":1.5}}}"#,
        );
        let global: WebDefaultsSetting = serde_json::from_str(
            r#"{"enabled":true,"fields":{"bgmVolume":0,"mouseTracking":false,
                "textureResolution":"4k","position":{"x":0,"y":0}}}"#,
        )
        .unwrap();
        let script = script_for_folder(dir.path(), &global).unwrap();
        // Global fields apply...
        assert!(script.contains("\"mouseTracking\":false"));
        assert!(script.contains("\"textureResolution\":\"4k\""));
        // ...the package overrides the shared value...
        assert!(script.contains("\"bgmVolume\":0.3"));
        // ...and geometry never comes from defaults, no matter who wrote it.
        assert!(!script.contains("position"));
        assert!(!script.contains("scale"));
        // The merged script is a once script even when the package said force.
        assert!(script.contains("const o=true"));
        assert!(script.contains("const f=false"));
    }

    #[test]
    fn disabled_app_defaults_leave_the_package_alone() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("loadJson.json"), r#"{"fileNames":["x"]}"#).unwrap();
        let global = WebDefaultsSetting {
            enabled: false,
            fields: serde_json::from_str(r#"{"bgmVolume":0}"#).unwrap(),
        };
        assert!(script_for_folder(dir.path(), &global).is_none());
        write_defaults(
            dir.path(),
            r#"{"schemaVersion":1,"storage":{"x":{"bgmVolume":0}}}"#,
        );
        assert!(script_for_folder(dir.path(), &global).is_some());
    }

    #[test]
    fn reset_script_targets_the_package_storage_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("loadJson.json"),
            r#"{"fileNames":["CH0139_home"]}"#,
        )
        .unwrap();
        let script = reset_script(dir.path()).unwrap();
        assert!(script.contains("\"__octos_defaults__CH0139_home\""));
        assert!(script.contains("removeItem"));
        assert!(script.contains("reload"));
        std::fs::write(
            dir.path().join("loadJson.json"),
            r#"{"SettingModel":{"version":5}}"#,
        )
        .unwrap();
        assert!(reset_script(dir.path()).is_none());
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
        assert!(script_for_folder(dir.path(), &WebDefaultsSetting::default()).is_none());
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
