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

/// Values seeded into every web wallpaper on first run. Deliberately not
/// user-configurable: BGM off, no eye tracking, no cursor trail, 4k textures.
/// The wallpaper's own panel and per-screen tuning still win afterwards.
pub fn builtin_fields() -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("bgmVolume".to_string(), Value::from(0));
    fields.insert("mouseTracking".to_string(), Value::Bool(false));
    fields.insert("mouseTrial".to_string(), Value::Bool(false));
    fields.insert("textureResolution".to_string(), Value::from("4k"));
    fields
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
    let load_config = read_load_config(folder)?;
    // Newer packages keep one settings object per skeleton in
    // `localStorage["<skel>"]`, wrapped as `{SettingModel, EventInfos, ...}`.
    if let Some(model) = load_config.get("SettingModel").and_then(Value::as_object) {
        let skel = model.get("skel")?.as_str()?.trim();
        return (!skel.is_empty()).then(|| skel.to_string());
    }
    let key = load_config
        .get("fileNames")?
        .as_array()?
        .first()?
        .as_str()?;
    (!key.trim().is_empty()).then(|| key.to_string())
}

fn read_load_config(folder: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(folder.join("loadJson.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Target field name inside a `SettingModel`: the same name when the package
/// has it, or the family's own spelling for the settings we expose.
fn model_field(name: &str, model: &Map<String, Value>) -> Option<String> {
    if model.contains_key(name) {
        return Some(name.to_string());
    }
    let alias = match name {
        "textureResolution" => "resolution",
        _ => return None,
    };
    model.contains_key(alias).then(|| alias.to_string())
}

/// Script for a `SettingModel` package: patch fields inside the stored
/// `{SettingModel, EventInfos, AnimationsTexts}` wrapper, seeding it from the
/// packaged model on first run. Same once-per-field semantics as old packages.
fn model_script(
    key: &str,
    model: &Map<String, Value>,
    extra: &Map<String, Value>,
    fields: &Map<String, Value>,
) -> String {
    let key_json = serde_json::to_string(key).unwrap_or_default();
    let base_json = Value::Object(model.clone()).to_string();
    let extra_json = Value::Object(extra.clone()).to_string();
    let fields_json = Value::Object(fields.clone()).to_string();
    format!(
        r#"(function(){{const key={key_json};const base={base_json};const extra={extra_json};const d={fields_json};const mk="__octos_defaults__"+key;try{{const prev=JSON.parse(localStorage.getItem(mk)||"null");const raw=localStorage.getItem(key);const cur=raw?JSON.parse(raw):Object.assign({{}},extra,{{SettingModel:base}});const model=cur.SettingModel||{{}};const patch={{}};let changed=false;for(const f of Object.keys(d)){{if(!prev||!(f in prev)||JSON.stringify(prev[f])!==JSON.stringify(d[f])){{patch[f]=d[f];changed=true;}}}}if(!changed)return;if(base.version!==undefined)patch.version=base.version;cur.SettingModel=Object.assign({{}},model,patch);localStorage.setItem(key,JSON.stringify(cur));localStorage.setItem(mk,JSON.stringify(d));}}catch(_){{}}}})();"#
    )
}

fn shared_field(name: &str) -> bool {
    !VISUAL_FIELDS.contains(&name)
}

/// Defaults script for one package: app-level defaults, overridden field by
/// field by the package's own `octos-defaults.json`, applied with `once`
/// semantics. `None` when there is nothing to apply.
pub fn script_for_folder(folder: &Path) -> Option<String> {
    let load_config = read_load_config(folder)?;

    // New-style package: the settings live in `SettingModel`, keyed by skel.
    if let Some(model) = load_config.get("SettingModel").and_then(Value::as_object) {
        let skel = model.get("skel")?.as_str()?.trim();
        if skel.is_empty() {
            return None;
        }
        let mut fields = Map::new();
        for (name, value) in &builtin_fields() {
            if let Some(target) = model_field(name, model) {
                fields.insert(target, value.clone());
            }
        }
        if fields.is_empty() {
            return None;
        }
        let mut extra = Map::new();
        for name in ["EventInfos", "AnimationsTexts"] {
            if let Some(value) = load_config.get(name) {
                extra.insert(name.to_string(), value.clone());
            }
        }
        return Some(model_script(skel, model, &extra, &fields));
    }

    // Old-style package: one flat settings object in localStorage.
    let key = load_config
        .get("fileNames")?
        .as_array()?
        .first()?
        .as_str()?
        .to_string();
    let mut merged = builtin_fields();
    // A package's own defaults are more specific than the built-in policy.
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
        assert!(script_for_folder(dir.path()).is_some());
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
    fn package_defaults_refine_the_builtin_policy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("loadJson.json"),
            r#"{"fileNames":["CH0139_home"]}"#,
        )
        .unwrap();
        write_defaults(
            dir.path(),
            r#"{"schemaVersion":1,"mode":"force","storage":{"CH0139_home":{
                "bgmVolume":0.3,"talkVolume":0.9,"position":{"x":1440,"y":1800},"scale":1.5}}}"#,
        );
        let script = script_for_folder(dir.path()).unwrap();
        // Built-in policy: no BGM, no tracking, 4k.
        assert!(script.contains("\"mouseTracking\":false"));
        assert!(script.contains("\"mouseTrial\":false"));
        assert!(script.contains("\"textureResolution\":\"4k\""));
        // A package may override the fields it cares about...
        assert!(script.contains("\"bgmVolume\":0.3"));
        assert!(script.contains("\"talkVolume\":0.9"));
        // ...but geometry never comes from defaults, no matter who wrote it.
        assert!(!script.contains("position"));
        assert!(!script.contains("scale"));
        // The merged script is a once script even when the package said force.
        assert!(script.contains("const o=true"));
        assert!(script.contains("const f=false"));
    }

    #[test]
    fn setting_model_packages_patch_inside_the_stored_wrapper() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("loadJson.json"),
            r#"{"SettingModel":{"version":5,"skel":"CH0284_home.skel","bgmVolume":0.1,
                "resolution":"2k","showTouch":true,"scale":0.57},
                "EventInfos":[1],"AnimationsTexts":{"a":"b"}}"#,
        )
        .unwrap();
        assert_eq!(storage_key(dir.path()).as_deref(), Some("CH0284_home.skel"));
        let script = script_for_folder(dir.path()).unwrap();
        // Fields are written under SettingModel, with the family's own name.
        assert!(script.contains(r#"const key="CH0284_home.skel""#));
        assert!(script.contains(r#""bgmVolume":0"#));
        assert!(script.contains(r#""resolution":"4k""#));
        // Fields this family does not have are not invented.
        assert!(!script.contains("mouseTracking"));
        assert!(!script.contains("mouseTrial"));
        // The stored wrapper keeps the packaged extras and the version.
        assert!(script.contains("EventInfos"));
        assert!(script.contains("AnimationsTexts"));
        assert!(script.contains("patch.version=base.version"));
    }

    #[test]
    fn builtin_policy_covers_packages_without_a_defaults_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("loadJson.json"), r#"{"fileNames":["x"]}"#).unwrap();
        let script = script_for_folder(dir.path()).unwrap();
        assert!(script.contains(r#""bgmVolume":0"#));
        assert!(script.contains(r#""mouseTrial":false"#));
        assert!(script.contains(r#""textureResolution":"4k""#));
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
    fn new_style_packages_ignore_octos_defaults_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("loadJson.json"),
            r#"{"SettingModel":{"version":5,"skel":"CH0355_home.skel","bgmVolume":0.1,
                "resolution":"2k","talkVolume":0.5}}"#,
        )
        .unwrap();
        write_defaults(
            dir.path(),
            r#"{"schemaVersion":1,"storage":{"CH0355_home.skel":{"talkVolume":0.9}}}"#,
        );
        assert!(load(dir.path()).is_some());
        // The package file is for flat (old-style) storage only; new-style
        // packages get the built-in policy through SettingModel instead.
        let script = script_for_folder(dir.path()).unwrap();
        assert!(script.contains(r#""bgmVolume":0"#));
        assert!(script.contains(r#""resolution":"4k""#));
        // talkVolume came only from the ignored octos-defaults.json.
        assert!(!script.contains(r#""talkVolume":0.9"#));
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
