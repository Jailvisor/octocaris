//! Compatibility for `onchange` / `onload` / `onchangeload` option hooks.
//!
//! Upstream builds and executes `{ const f = <source>; f("<value>"); }`. The
//! value was always stringified, so `true` becomes `"true"` and `3` becomes
//! `"3"`. We keep that behavior but JSON-escape the argument.

use serde_json::Value;

use crate::options::stringify_value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    Load,
    Change,
}

/// Sources to execute for an event, in upstream order:
/// - change: `onchange`, then `onchangeload`
/// - load:   `onload`, then `onchangeload`
pub fn sources_for_event(spec: &Value, event: HookEvent) -> Vec<String> {
    let key = match event {
        HookEvent::Load => "onload",
        HookEvent::Change => "onchange",
    };

    let mut sources = Vec::new();
    if let Some(source) = spec.get(key).and_then(Value::as_str)
        && !source.is_empty()
    {
        sources.push(source.to_string());
    }
    if let Some(source) = spec.get("onchangeload").and_then(Value::as_str)
        && !source.is_empty()
    {
        sources.push(source.to_string());
    }
    sources
}

/// Build the `ExecuteScript` payload for one hook.
pub fn build_script(source: &str, value: &Value) -> String {
    let argument =
        serde_json::to_string(&stringify_value(value)).unwrap_or_else(|_| "\"\"".to_string());
    format!("{{ const f = {source}; f({argument}); }}")
}

pub fn scripts_for_event(spec: &Value, value: &Value, event: HookEvent) -> Vec<String> {
    sources_for_event(spec, event)
        .into_iter()
        .map(|source| build_script(&source, value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn change_order_and_stringification() {
        let spec = json!({
            "onchange": "(value) => console.log(value)",
            "onchangeload": "(value) => console.log('both', value)",
            "onload": "(value) => console.log('load', value)"
        });
        let scripts = scripts_for_event(&spec, &json!(3), HookEvent::Change);
        assert_eq!(scripts.len(), 2);
        assert!(scripts[0].contains("console.log(value)"));
        assert!(scripts[0].contains("f(\"3\")"));
        assert!(scripts[1].contains("console.log('both'"));
    }

    #[test]
    fn load_uses_onload_and_changeload() {
        let spec = json!({
            "onload": "(value) => console.log('load', value)",
            "onchangeload": "(value) => console.log('both', value)"
        });
        let scripts = scripts_for_event(&spec, &json!(true), HookEvent::Load);
        assert_eq!(scripts.len(), 2);
        assert!(scripts[0].contains("console.log('load'"));
        assert!(scripts[0].contains("f(\"true\")"));
    }

    #[test]
    fn quotes_are_escaped() {
        let script = build_script("(value) => console.log(value)", &json!("a\"b\\c"));
        assert!(script.contains("f(\"a\\\"b\\\\c\")"));
    }

    #[test]
    fn missing_hooks_produce_nothing() {
        let scripts = scripts_for_event(
            &json!({"type": "checkbox"}),
            &json!(true),
            HookEvent::Change,
        );
        assert!(scripts.is_empty());
    }
}
