//! Mod option defaults, user-value merging and JS stringification.

use serde_json::{Map, Value};

pub type OptionsMap = Map<String, Value>;

/// Merge user values into a mod's default options without mutating the defaults.
pub fn merge_options(defaults: &OptionsMap, stored: Option<&OptionsMap>) -> OptionsMap {
    let mut merged = defaults.clone();
    let Some(stored) = stored else {
        return merged;
    };

    for (id, spec) in &mut merged {
        let Some(stored_spec) = stored.get(id).and_then(Value::as_object) else {
            continue;
        };
        let Some(value) = stored_spec.get("value") else {
            continue;
        };
        if let Some(spec) = spec.as_object_mut() {
            spec.insert("value".to_string(), value.clone());
        }
    }

    merged
}

/// Set one option value. Returns true when the value actually changed.
///
/// This intentionally does not emit `options-change`; the caller decides whether
/// a UI-originated change should notify the wallpaper.
pub fn set_option_value(options: &mut OptionsMap, id: &str, value: Value) -> bool {
    let Some(spec) = options.get_mut(id).and_then(Value::as_object_mut) else {
        return false;
    };
    if spec.get("value") == Some(&value) {
        return false;
    }
    spec.insert("value".to_string(), value);
    true
}

/// Mirror the old `f("${value}")` template: values arrive as strings in JS.
pub fn stringify_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(stringify_value)
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn defaults() -> OptionsMap {
        json!({
            "speed": { "type": "range", "label": "Speed", "value": 3, "min": 1, "max": 5 },
            "dark": { "type": "checkbox", "label": "Dark", "value": false }
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn merge_keeps_user_values_and_other_fields() {
        let stored = json!({
            "speed": { "value": 5 },
            "unknown": { "value": true }
        })
        .as_object()
        .unwrap()
        .clone();

        let merged = merge_options(&defaults(), Some(&stored));
        assert_eq!(merged["speed"]["value"], 5);
        assert_eq!(merged["speed"]["label"], "Speed");
        assert_eq!(merged["dark"]["value"], false);
        assert!(!merged.contains_key("unknown"));
    }

    #[test]
    fn set_option_reports_changes_only() {
        let mut options = defaults();
        assert!(set_option_value(&mut options, "speed", json!(4)));
        assert_eq!(options["speed"]["value"], 4);
        assert!(!set_option_value(&mut options, "speed", json!(4)));
        assert!(!set_option_value(&mut options, "missing", json!(1)));
    }

    #[test]
    fn stringifies_like_the_old_template() {
        assert_eq!(stringify_value(&json!(true)), "true");
        assert_eq!(stringify_value(&json!(3)), "3");
        assert_eq!(stringify_value(&json!(3.5)), "3.5");
        assert_eq!(stringify_value(&json!("dark")), "dark");
        assert_eq!(stringify_value(&json!([1, 2])), "1,2");
        assert_eq!(stringify_value(&json!({"a": 1})), "[object Object]");
    }
}
