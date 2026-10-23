//! Per-monitor storage namespacing for wallpaper pages.
//!
//! All wallpapers share the `https://local.octos` origin, so `localStorage`
//! keys are global. A wallpaper with its own settings panel (scale/position,
//! volume, language, ...) would otherwise apply one setting to every monitor,
//! even though each monitor has a different CSS viewport and DPI. The script
//! below runs before page scripts and prefixes every storage key with the
//! monitor ID. Existing unprefixed values are copied once into each monitor's
//! namespace, so upgrades do not reset settings.

/// JavaScript injected with `AddScriptToExecuteOnDocumentCreated`.
pub fn script(monitor_id: &str) -> String {
    let literal = serde_json::to_string(monitor_id).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"(function () {{
  if (window.__octosStorageScoped) return;
  window.__octosStorageScoped = true;
  const marker = "__octos_monitor__";
  const prefix = marker + {literal} + ":";
  const proto = Storage.prototype;
  const originalGet = proto.getItem;
  const originalSet = proto.setItem;
  const originalRemove = proto.removeItem;
  const originalKey = proto.key;
  const allKeys = function (storage) {{
    const keys = [];
    for (let i = 0; ; i++) {{
      const key = originalKey.call(storage, i);
      if (key === null) break;
      keys.push(key);
    }}
    return keys;
  }};
  const scopedKeys = function (storage) {{
    return allKeys(storage).filter(function (key) {{ return key.startsWith(prefix); }});
  }};
  proto.getItem = function (key) {{ return originalGet.call(this, prefix + key); }};
  proto.setItem = function (key, value) {{ return originalSet.call(this, prefix + key, value); }};
  proto.removeItem = function (key) {{ return originalRemove.call(this, prefix + key); }};
  proto.clear = function () {{
    scopedKeys(this).forEach(function (key) {{ originalRemove.call(this, key); }}, this);
  }};
  try {{
    Object.defineProperty(proto, "length", {{
      configurable: true,
      get: function () {{ return scopedKeys(this).length; }}
    }});
  }} catch (_) {{}}
  proto.key = function (index) {{
    const key = scopedKeys(this)[index];
    return key === undefined ? null : key.slice(prefix.length);
  }};
  try {{
    const storage = window.localStorage;
    // Repair keys created by the first version of this script, which copied
    // another monitor's already-prefixed key into this namespace.
    allKeys(storage).forEach(function (key) {{
      if (key.startsWith(marker) && key.indexOf(marker, marker.length) !== -1) {{
        originalRemove.call(storage, key);
      }}
    }});
    // Migrate only legacy keys (no monitor prefix at all).
    allKeys(storage).forEach(function (key) {{
      if (key.startsWith(marker)) return;
      if (originalGet.call(storage, prefix + key) !== null) return;
      const value = originalGet.call(storage, key);
      if (value !== null) originalSet.call(storage, prefix + key, value);
    }});
  }} catch (_) {{}}
}})();"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_embeds_the_monitor_id_and_patches_storage() {
        let script = script(r"\\.\DISPLAY1");
        assert!(script.contains("__octosStorageScoped"));
        assert!(script.contains("__octos_monitor__"));
        assert!(script.contains(r#"\\\\.\\DISPLAY1"#));
        assert!(script.contains("Storage.prototype"));
        assert!(script.contains("originalGet"));
        assert!(script.contains("originalSet"));
    }

    #[test]
    fn monitor_id_is_json_escaped() {
        let script = script("a\"b\\c");
        assert!(script.contains(r#""a\"b\\c""#));
        assert!(!script.contains(r#""a"b\c""#));
    }
}
