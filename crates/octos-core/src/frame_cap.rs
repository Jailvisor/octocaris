//! Optional frame-rate cap injected into wallpaper pages.
//!
//! Wallpapers such as Pixi/Spine run their animation from
//! `requestAnimationFrame`. Capping it at 30 FPS roughly halves the renderer
//! and GPU work while keeping motion readable. The cap is a global on the page
//! (`window.__octosFpsCap`) so the host can change it without reloading.

/// JavaScript injected with `AddScriptToExecuteOnDocumentCreated`.
pub fn script(fps: u32) -> String {
    let fps = fps.min(240);
    format!(
        r#"(function () {{
  if (window.__octosFpsInstalled) return;
  window.__octosFpsInstalled = true;
  if (window.__octosFpsCap === undefined) window.__octosFpsCap = {fps};
  const nativeRaf = window.requestAnimationFrame.bind(window);
  const nativeCancel = window.cancelAnimationFrame.bind(window);
  const callbacks = new Map();
  let nextId = 1;
  let pending = null;
  let last = 0;
  const loop = function (now) {{
    pending = null;
    const cap = Number(window.__octosFpsCap) || 0;
    const interval = cap > 0 ? 1000 / cap : 0;
    if (cap <= 0 || now - last >= interval - 1) {{
      last = now;
      const batch = Array.from(callbacks.values());
      callbacks.clear();
      for (const callback of batch) {{
        try {{ callback(now); }} catch (error) {{ setTimeout(function () {{ throw error; }}, 0); }}
      }}
    }}
    if (callbacks.size > 0) schedule();
  }};
  const schedule = function () {{
    if (pending === null) pending = nativeRaf(loop);
  }};
  window.requestAnimationFrame = function (callback) {{
    const id = nextId++;
    callbacks.set(id, callback);
    schedule();
    return id;
  }};
  window.cancelAnimationFrame = function (id) {{
    callbacks.delete(id);
  }};
}})();"#
    )
}

/// Script that changes the cap on an already loaded page.
pub fn set_script(fps: u32) -> String {
    format!("window.__octosFpsCap = {};", fps.min(240))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_embeds_cap_and_wraps_raf() {
        let script = script(30);
        assert!(script.contains("window.__octosFpsCap = 30"));
        assert!(script.contains("requestAnimationFrame"));
        assert!(script.contains("cancelAnimationFrame"));
        assert!(script.contains("__octosFpsInstalled"));
    }

    #[test]
    fn cap_is_clamped_and_can_be_changed() {
        assert!(script(10_000).contains("window.__octosFpsCap = 240"));
        assert_eq!(set_script(60), "window.__octosFpsCap = 60;");
        assert_eq!(set_script(0), "window.__octosFpsCap = 0;");
    }
}
