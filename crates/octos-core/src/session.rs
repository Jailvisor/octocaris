//! Per-monitor mod selection and option state.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde_json::Value;

use crate::options::{OptionsMap, merge_options, set_option_value};

static EMPTY_OPTIONS: LazyLock<OptionsMap> = LazyLock::new(OptionsMap::default);

#[derive(Debug, Clone, Default)]
pub struct Session {
    /// monitor id (`\\.\DISPLAYn`) -> absolute mod folder key
    selected: BTreeMap<String, String>,
    /// absolute mod folder key -> merged option specs
    options: BTreeMap<String, OptionsMap>,
    /// absolute mod folder key -> default option specs
    defaults: BTreeMap<String, OptionsMap>,
}

impl Session {
    pub fn select(&mut self, monitor_id: impl Into<String>, mod_key: impl Into<String>) {
        self.selected.insert(monitor_id.into(), mod_key.into());
    }

    pub fn clear(&mut self, monitor_id: &str) {
        self.selected.remove(monitor_id);
    }

    pub fn mod_for_monitor(&self, monitor_id: &str) -> Option<&str> {
        self.selected.get(monitor_id).map(String::as_str)
    }

    pub fn selected(&self) -> &BTreeMap<String, String> {
        &self.selected
    }

    /// Other monitors running the same mod as the given monitor.
    pub fn siblings(&self, monitor_id: &str) -> Vec<String> {
        let Some(current) = self.mod_for_monitor(monitor_id) else {
            return Vec::new();
        };
        self.selected
            .iter()
            .filter(|(monitor, key)| monitor.as_str() != monitor_id && key.as_str() == current)
            .map(|(monitor, _)| monitor.clone())
            .collect()
    }

    pub fn load_options(
        &mut self,
        mod_key: impl Into<String>,
        defaults: &OptionsMap,
        stored: Option<&OptionsMap>,
    ) {
        let mod_key = mod_key.into();
        self.defaults.insert(mod_key.clone(), defaults.clone());
        self.options
            .insert(mod_key, merge_options(defaults, stored));
    }

    pub fn remove_mod(&mut self, mod_key: &str) {
        self.options.remove(mod_key);
        self.defaults.remove(mod_key);
        self.selected.retain(|_, key| key != mod_key);
    }

    /// Every loaded mod's option specs, used when persisting preferences.
    pub fn all_options(&self) -> &BTreeMap<String, OptionsMap> {
        &self.options
    }

    /// Load a mod's defaults if it has no option state yet. Existing values win.
    pub fn ensure_options(
        &mut self,
        mod_key: impl Into<String>,
        defaults: &OptionsMap,
        stored: Option<&OptionsMap>,
    ) {
        let mod_key = mod_key.into();
        if !self.options.contains_key(&mod_key) {
            self.defaults.insert(mod_key.clone(), defaults.clone());
            self.options
                .insert(mod_key, merge_options(defaults, stored));
        }
    }

    pub fn options_for(&self, mod_key: &str) -> &OptionsMap {
        self.options.get(mod_key).unwrap_or(&EMPTY_OPTIONS)
    }

    /// Set a stored value without emitting an event. Returns true if it changed.
    pub fn set_option(&mut self, mod_key: &str, id: &str, value: Value) -> bool {
        self.options
            .get_mut(mod_key)
            .map(|options| set_option_value(options, id, value))
            .unwrap_or(false)
    }

    pub fn defaults_for(&self, mod_key: &str) -> &OptionsMap {
        self.defaults.get(mod_key).unwrap_or(&EMPTY_OPTIONS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn options() -> OptionsMap {
        json!({
            "speed": { "type": "range", "label": "Speed", "value": 3 }
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn selection_and_siblings() {
        let mut session = Session::default();
        session.select("\\\\.\\DISPLAY1", "/mods/a");
        session.select("\\\\.\\DISPLAY2", "/mods/a");
        session.select("\\\\.\\DISPLAY3", "/mods/b");

        assert_eq!(session.mod_for_monitor("\\\\.\\DISPLAY1"), Some("/mods/a"));
        assert_eq!(
            session.siblings("\\\\.\\DISPLAY1"),
            vec!["\\\\.\\DISPLAY2".to_string()]
        );
        assert!(session.siblings("\\\\.\\DISPLAY3").is_empty());
    }

    #[test]
    fn missing_mod_options_are_empty_not_pending() {
        let session = Session::default();
        assert!(session.options_for("/mods/missing").is_empty());
    }

    #[test]
    fn ensure_options_loads_defaults_and_keeps_existing_values() {
        let mut session = Session::default();
        session.ensure_options("/mods/a", &options(), None);
        assert_eq!(session.options_for("/mods/a")["speed"]["value"], 3);

        assert!(session.set_option("/mods/a", "speed", json!(9)));
        session.ensure_options("/mods/a", &options(), None);
        assert_eq!(session.options_for("/mods/a")["speed"]["value"], 9);

        let stored = json!({ "speed": { "value": 12 } })
            .as_object()
            .unwrap()
            .clone();
        session.ensure_options("/mods/b", &options(), Some(&stored));
        assert_eq!(session.options_for("/mods/b")["speed"]["value"], 12);
    }

    #[test]
    fn merges_stored_values_and_sets_without_events() {
        let mut session = Session::default();
        let stored = json!({ "speed": { "value": 5 } })
            .as_object()
            .unwrap()
            .clone();
        session.load_options("/mods/a", &options(), Some(&stored));
        assert_eq!(session.options_for("/mods/a")["speed"]["value"], 5);

        assert!(session.set_option("/mods/a", "speed", json!(4)));
        assert_eq!(session.options_for("/mods/a")["speed"]["value"], 4);
        assert!(!session.set_option("/mods/missing", "speed", json!(1)));
        assert_eq!(session.defaults_for("/mods/a")["speed"]["value"], 3);
    }
}
