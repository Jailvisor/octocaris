//! Per-monitor wallpaper windows, desktop attach, input forwarding and watchdog.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::mem::size_of;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use octos_core::hooks::{HookEvent, scripts_for_event};
use octos_core::options::OptionsMap;
use octos_core::session::Session;
use octos_protocol::{WallpaperHost, dispatch, options_event, redirect_event};
use octos_webview::{CompositionDevice, WallpaperWebView, execute_script, navigate, post_json};
use tracing::{debug, info, warn};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_DOWN, COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_UP,
    COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_DOWN,
    COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_UP, COREWEBVIEW2_MOUSE_EVENT_KIND_MOVE,
    COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_DOWN, COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_UP,
    COREWEBVIEW2_MOUSE_EVENT_KIND_WHEEL, COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_LEFT_BUTTON,
    COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_MIDDLE_BUTTON,
    COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_NONE, COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_RIGHT_BUTTON,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2_8, ICoreWebView2Environment3,
};
use webview2_com::take_pwstr;
use windows::Win32::Foundation::{
    COLORREF, ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFOEXW,
    MonitorFromRect, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Interface, PCWSTR, PWSTR, w};

const WALLPAPER_CLASS: PCWSTR = w!("OctosWallpaperWindow");
const WATCHDOG_TIMER_ID: usize = 2;
const WATCHDOG_INTERVAL_MS: u32 = 500;
const DESKTOP_CLASSES: [&str; 4] = ["SysListView32", "SHELLDLL_DefView", "Progman", "WorkerW"];

#[derive(Debug, Clone)]
struct MonitorInfo {
    device_name: String,
    rect: RECT,
    primary: bool,
}

struct MonitorSlot {
    device_name: String,
    rect: RECT,
    primary: bool,
    hwnd: HWND,
    parent: HWND,
    insert_after: Option<HWND>,
    webview: Option<WallpaperWebView>,
    entry_url: Option<String>,
    mod_key: Option<String>,
    user_visible: bool,
    fullscreen: bool,
    /// A controller create call is in flight; another reconcile must not start
    /// a second composition target on the same HWND.
    creating: bool,
    /// Set by a ProcessFailed handler; the watchdog recreates the controller
    /// after this deadline so WebView2 has time to tear the old one down.
    recreate_after: Option<Instant>,
    /// Last visibility applied to the controller/window. The watchdog runs
    /// every 500ms; re-applying it makes the composition surface flash.
    applied_visible: Option<bool>,
    /// Last client size passed to the WebView controller.
    applied_client: Option<(i32, i32)>,
    /// Monitor DPI used when the current controller was created.
    applied_dpi: u32,
}

impl MonitorSlot {
    fn new(device_name: String, rect: RECT, primary: bool) -> Self {
        Self {
            device_name,
            rect,
            primary,
            hwnd: HWND::default(),
            parent: HWND::default(),
            insert_after: None,
            webview: None,
            entry_url: None,
            mod_key: None,
            user_visible: true,
            fullscreen: false,
            creating: false,
            recreate_after: None,
            applied_visible: None,
            applied_client: None,
            applied_dpi: 0,
        }
    }

    fn close_webview(&mut self) {
        if let Some(webview) = self.webview.take() {
            webview.close();
        }
        self.creating = false;
        self.recreate_after = None;
        self.applied_visible = None;
        self.applied_client = None;
        self.applied_dpi = 0;
    }

    fn close(&mut self) {
        self.close_webview();
        if !self.hwnd.is_invalid() && unsafe { IsWindow(Some(self.hwnd)).as_bool() } {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
        self.hwnd = HWND::default();
        self.parent = HWND::default();
        self.insert_after = None;
    }
}

struct WallpaperState {
    environment: Option<ICoreWebView2Environment3>,
    dcomp: Option<CompositionDevice>,
    wallpapers_dir: PathBuf,
    slots: Vec<MonitorSlot>,
    hook: Option<HHOOK>,
    timer: usize,
    global_visible: bool,
    pause_fullscreen: bool,
    fixed_chrome_windows: HashSet<isize>,
    session: Session,
    fps_cap: u32,
    muted: bool,
    last_foreground: HWND,
    last_fullscreen_check: Instant,
    last_overlay_check: Instant,
}

impl Default for WallpaperState {
    fn default() -> Self {
        Self {
            environment: None,
            dcomp: None,
            wallpapers_dir: PathBuf::new(),
            slots: Vec::new(),
            hook: None,
            timer: 0,
            global_visible: true,
            pause_fullscreen: true,
            fixed_chrome_windows: HashSet::new(),
            session: Session::default(),
            fps_cap: 30,
            muted: false,
            last_foreground: HWND::default(),
            last_fullscreen_check: Instant::now(),
            last_overlay_check: Instant::now(),
        }
    }
}

thread_local! {
    static STATE: RefCell<WallpaperState> = RefCell::new(WallpaperState::default());
}

fn with_state<R>(f: impl FnOnce(&mut WallpaperState) -> R) -> Option<R> {
    STATE.with(|state| state.try_borrow_mut().ok().map(|mut state| f(&mut state)))
}

pub fn initialize(environment: ICoreWebView2Environment3, wallpapers_dir: PathBuf) -> Result<()> {
    let dcomp = CompositionDevice::new()?;
    with_state(|state| {
        state.environment = Some(environment);
        state.dcomp = Some(dcomp);
        state.wallpapers_dir = wallpapers_dir;
        state.reconcile()
    })
    .ok_or_else(|| anyhow!("wallpaper state is already borrowed"))??;

    let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0)? };
    with_state(|state| state.hook = Some(hook));

    let timer = unsafe {
        SetTimer(
            None,
            WATCHDOG_TIMER_ID,
            WATCHDOG_INTERVAL_MS,
            Some(watchdog_timer),
        )
    };
    with_state(|state| state.timer = timer);

    info!(
        "wallpaper manager initialized ({} monitor(s))",
        slot_count()
    );
    Ok(())
}

pub fn show_all(entry_url: &str) -> Result<()> {
    with_state(|state| {
        for slot in &mut state.slots {
            slot.entry_url = Some(entry_url.to_string());
            slot.user_visible = true;
        }
        state.global_visible = true;
        state.reconcile()
    })
    .ok_or_else(|| anyhow!("wallpaper state is already borrowed"))?
}

/// Show one mod on every monitor, reusing stored option values when present.
///
/// Used by `octos run`: unlike `set_all_mod`, this also replaces each slot's
/// entry URL and reloads the WebViews in one reconcile pass.
pub fn set_all_mod_with(
    mod_key: impl Into<String>,
    entry_url: String,
    defaults: OptionsMap,
    stored: Option<OptionsMap>,
) -> Result<()> {
    let mod_key = mod_key.into();
    with_state(|state| {
        let monitors: Vec<String> = state
            .slots
            .iter()
            .map(|slot| slot.device_name.clone())
            .collect();
        for monitor in monitors {
            state.session.select(monitor, mod_key.clone());
        }
        state
            .session
            .load_options(mod_key.clone(), &defaults, stored.as_ref());

        for slot in &mut state.slots {
            slot.close_webview();
            slot.mod_key = Some(mod_key.clone());
            slot.entry_url = Some(entry_url.clone());
            slot.user_visible = true;
        }
        state.global_visible = true;
        state.reconcile()
    })
    .ok_or_else(|| anyhow!("wallpaper state is already borrowed"))?
}

/// Reload every wallpaper WebView (`octos reload`). Returns how many were asked.
pub fn reload_all() -> usize {
    with_state(|state| {
        let mut count = 0;
        for slot in &state.slots {
            if let Some(wallpaper) = &slot.webview {
                match unsafe { wallpaper.webview.Reload() } {
                    Ok(()) => count += 1,
                    Err(err) => warn!("could not reload {}: {err:?}", slot.device_name),
                }
            }
        }
        count
    })
    .unwrap_or(0)
}

/// Open DevTools on every wallpaper WebView (`octos dev-tools`).
pub fn open_dev_tools_all() -> usize {
    with_state(|state| {
        let mut count = 0;
        for slot in &state.slots {
            if let Some(wallpaper) = &slot.webview {
                match unsafe { wallpaper.webview.OpenDevToolsWindow() } {
                    Ok(()) => count += 1,
                    Err(err) => {
                        warn!("could not open DevTools for {}: {err:?}", slot.device_name);
                    }
                }
            }
        }
        count
    })
    .unwrap_or(0)
}

/// Toggle the "pause on fullscreen" setting at runtime.
pub fn set_fps_cap(fps: u32) {
    let script = octos_core::frame_cap::set_script(fps);
    let _ = with_state(|state| {
        state.fps_cap = fps;
        for slot in &state.slots {
            if let Some(webview) = &slot.webview
                && let Err(err) = execute_script(&webview.webview, &script)
            {
                warn!("could not update frame cap: {err:?}");
            }
        }
    });
}

/// Mute or unmute every wallpaper page. New controllers inherit the flag.
pub fn set_muted(muted: bool) {
    with_state(|state| state.muted = muted);
    let webviews = with_state(|state| {
        state
            .slots
            .iter()
            .filter_map(|slot| slot.webview.as_ref().map(|webview| webview.webview.clone()))
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    let mut applied = 0;
    for webview in webviews {
        if let Ok(muted_webview) = webview.cast::<ICoreWebView2_8>() {
            let mut current = BOOL::from(false);
            unsafe {
                let _ = muted_webview.SetIsMuted(muted);
                if muted_webview.IsMuted(&mut current).is_ok() && current.as_bool() == muted {
                    applied += 1;
                }
            }
        }
    }
    info!("wallpaper audio muted={muted} ({applied} page(s))");
}

pub fn is_muted() -> bool {
    with_state(|state| state.muted).unwrap_or(false)
}

pub fn set_pause_fullscreen(enabled: bool) {
    let _ = with_state(|state| {
        if state.pause_fullscreen == enabled {
            return;
        }
        state.pause_fullscreen = enabled;
        for slot in &mut state.slots {
            slot.fullscreen = false;
        }
        state.apply_visibility();
    });
}

/// Associate every current monitor with a mod folder key and load its default
/// options. The M4 library will replace this with per-monitor selection.
pub fn set_all_mod(mod_key: impl Into<String>, defaults: OptionsMap) {
    let mod_key = mod_key.into();
    let _ = with_state(|state| {
        let monitors: Vec<String> = state
            .slots
            .iter()
            .map(|slot| slot.device_name.clone())
            .collect();
        for monitor in monitors {
            state.session.select(monitor, mod_key.clone());
        }
        state.session.load_options(mod_key, &defaults, None);
    });
}

pub fn set_all_mod_key(mod_key: impl Into<String>) {
    set_all_mod(mod_key, OptionsMap::default());
}

pub fn monitor_ids() -> Vec<String> {
    with_state(|state| {
        state
            .slots
            .iter()
            .map(|slot| slot.device_name.clone())
            .collect()
    })
    .unwrap_or_default()
}

pub fn primary_monitor_id() -> Option<String> {
    with_state(|state| {
        state
            .slots
            .iter()
            .find(|slot| slot.primary)
            .map(|slot| slot.device_name.clone())
    })
    .flatten()
}

/// True once a slot has an entry URL. A slot created by a display change has
/// none until the app assigns the saved or default wallpaper.
pub fn monitor_has_wallpaper(monitor_id: &str) -> bool {
    with_state(|state| {
        state
            .slots
            .iter()
            .find(|slot| slot.device_name == monitor_id)
            .is_some_and(|slot| slot.entry_url.is_some())
    })
    .unwrap_or(false)
}

pub fn selected_monitors() -> BTreeMap<String, String> {
    with_state(|state| state.session.selected().clone()).unwrap_or_default()
}

pub fn ensure_mod_options(mod_key: String, defaults: OptionsMap, stored: Option<OptionsMap>) {
    let _ = with_state(|state| {
        state
            .session
            .ensure_options(mod_key, &defaults, stored.as_ref())
    });
}

pub fn all_options() -> BTreeMap<String, OptionsMap> {
    with_state(|state| state.session.all_options().clone()).unwrap_or_default()
}

pub fn forget_mod(mod_key: &str) {
    let _ = with_state(|state| state.session.remove_mod(mod_key));
}

pub fn options_snapshot() -> BTreeMap<String, OptionsMap> {
    let mut snapshot = BTreeMap::new();
    let _ = with_state(|state| {
        for mod_key in state.session.selected().values() {
            if !snapshot.contains_key(mod_key) {
                snapshot.insert(mod_key.clone(), state.session.options_for(mod_key).clone());
            }
        }
    });
    snapshot
}

pub fn set_monitor_mod(
    monitor_id: &str,
    mod_key: String,
    entry_url: String,
    defaults: OptionsMap,
) -> Result<()> {
    // Re-applying the same mod must not throw away values the user already
    // changed in this session.
    let stored =
        with_state(|state| state.session.options_for(&mod_key).clone()).unwrap_or_default();
    set_monitor_mod_with(monitor_id, mod_key, entry_url, defaults, Some(stored))
}

pub fn set_monitor_mod_with(
    monitor_id: &str,
    mod_key: String,
    entry_url: String,
    defaults: OptionsMap,
    stored: Option<OptionsMap>,
) -> Result<()> {
    with_state(|state| {
        let Some(slot) = state
            .slots
            .iter_mut()
            .find(|slot| slot.device_name == monitor_id)
        else {
            return Err(anyhow!("monitor not found: {monitor_id}"));
        };
        if let Some(webview) = slot.webview.take() {
            webview.close();
        }
        slot.entry_url = Some(entry_url);
        slot.mod_key = Some(mod_key.clone());
        slot.user_visible = true;

        state.session.select(monitor_id, mod_key.clone());
        state
            .session
            .load_options(mod_key, &defaults, stored.as_ref());
        state.reconcile()
    })
    .ok_or_else(|| anyhow!("wallpaper state is already borrowed"))?
}

/// Switch every monitor to a new mod without tearing down the WebView2
/// controllers. Returns how many live webviews were navigated in place.
pub fn switch_mod_all(
    mod_key: String,
    entry_url: String,
    defaults: OptionsMap,
    stored: Option<OptionsMap>,
) -> Result<usize> {
    with_state(|state| {
        let mut navigated = 0usize;
        {
            let WallpaperState { slots, session, .. } = state;
            // Load the incoming mod's options before navigating: the
            // NavigationCompleted handler reads the live session when it emits
            // options-load, and the page must not see the previous mod's values.
            session.load_options(mod_key.clone(), &defaults, stored.as_ref());
            for slot in slots.iter_mut() {
                session.select(slot.device_name.clone(), mod_key.clone());
                slot.mod_key = Some(mod_key.clone());
                slot.entry_url = Some(entry_url.clone());
                slot.user_visible = true;
                let mut failed = false;
                if let Some(webview) = slot.webview.as_ref() {
                    match navigate(&webview.webview, &entry_url) {
                        Ok(()) => navigated += 1,
                        Err(err) => {
                            warn!(
                                "could not navigate wallpaper for {}: {err:?}",
                                slot.device_name
                            );
                            failed = true;
                        }
                    }
                }
                if failed {
                    // The watchdog recreates the controller with the new URL.
                    slot.close_webview();
                }
            }
        }
        state.reconcile()?;
        Ok(navigated)
    })
    .ok_or_else(|| anyhow!("wallpaper state is already borrowed"))?
}

/// Rotation pauses while every slot is fullscreen, and while no wallpaper is
/// configured (cleared desktop) so a timer cannot resurrect it.
pub fn rotation_should_pause() -> bool {
    with_state(|state| {
        if !state.slots.iter().any(|slot| slot.entry_url.is_some()) {
            return true;
        }
        state.slots.iter().all(|slot| slot.fullscreen)
    })
    .unwrap_or(true)
}

pub fn clear_monitor_mod(monitor_id: &str) -> Result<()> {
    with_state(|state| {
        if let Some(slot) = state
            .slots
            .iter_mut()
            .find(|slot| slot.device_name == monitor_id)
        {
            slot.entry_url = None;
            slot.mod_key = None;
            if let Some(webview) = slot.webview.take() {
                webview.close();
            }
            if !slot.hwnd.is_invalid() {
                unsafe {
                    let _ = ShowWindow(slot.hwnd, SW_HIDE);
                }
            }
        }
        state.session.clear(monitor_id);
        Ok(())
    })
    .ok_or_else(|| anyhow!("wallpaper state is already borrowed"))?
}

pub fn clear_all() {
    let _ = with_state(|state| {
        for slot in &mut state.slots {
            slot.entry_url = None;
            slot.mod_key = None;
            slot.close_webview();
            if !slot.hwnd.is_invalid() {
                unsafe {
                    let _ = ShowWindow(slot.hwnd, SW_HIDE);
                }
            }
        }
    });
}

/// Current global wallpaper visibility (session only, not persisted).
pub fn is_visible() -> bool {
    with_state(|state| state.global_visible).unwrap_or(true)
}

pub fn set_visible(visible: bool) {
    let _ = with_state(|state| {
        state.global_visible = visible;
        state.apply_visibility();
    });
}

pub fn shutdown() {
    let _ = with_state(|state| {
        if state.timer != 0 {
            unsafe {
                let _ = KillTimer(None, state.timer);
            }
            state.timer = 0;
        }
        if let Some(hook) = state.hook.take() {
            unsafe {
                let _ = UnhookWindowsHookEx(hook);
            }
        }
        for slot in &mut state.slots {
            slot.close();
        }
        state.slots.clear();
    });
}

pub fn reconcile_now() {
    let _ = with_state(|state| state.reconcile());
}

fn slot_count() -> usize {
    with_state(|state| state.slots.len()).unwrap_or(0)
}

impl WallpaperState {
    fn reconcile(&mut self) -> Result<()> {
        let monitors = enumerate_monitors();
        let mut old: Vec<Option<MonitorSlot>> = std::mem::take(&mut self.slots)
            .into_iter()
            .map(Some)
            .collect();
        let mut chosen: Vec<Option<MonitorSlot>> = (0..monitors.len()).map(|_| None).collect();

        // Device names are the stable identity, so resolve every exact match
        // before the rectangle fallback. The old single-pass loop let a
        // renamed/primary display steal another display's slot and name.
        for (monitor_index, monitor) in monitors.iter().enumerate() {
            if let Some(slot_index) = old.iter().position(|slot| {
                slot.as_ref()
                    .is_some_and(|slot| slot.device_name == monitor.device_name)
            }) {
                chosen[monitor_index] = old[slot_index].take();
            }
        }

        for (monitor_index, monitor) in monitors.iter().enumerate() {
            if chosen[monitor_index].is_some() {
                continue;
            }
            if let Some(slot_index) = old.iter().position(|slot| {
                slot.as_ref()
                    .is_some_and(|slot| rects_overlap(slot.rect, monitor.rect))
            }) {
                chosen[monitor_index] = old[slot_index].take();
            }
        }

        for mut removed in old.into_iter().flatten() {
            removed.close();
        }

        let mut next = Vec::with_capacity(monitors.len());
        for (monitor_index, monitor) in monitors.into_iter().enumerate() {
            let mut slot = chosen[monitor_index].take().unwrap_or_else(|| {
                MonitorSlot::new(monitor.device_name.clone(), monitor.rect, monitor.primary)
            });
            slot.device_name = monitor.device_name;
            slot.rect = monitor.rect;
            slot.primary = monitor.primary;
            next.push(slot);
        }
        self.slots = next;

        for index in 0..self.slots.len() {
            // Do not create a controller for a window that failed to attach.
            // On Explorer restart Progman can be missing for a few ticks; the
            // previous code still created a controller for a hidden 0x0 window,
            // which then never rendered after the real window appeared.
            if let Err(err) = self.ensure_window(index) {
                debug!(
                    "could not prepare wallpaper window for {}: {err:?}",
                    self.slots[index].device_name
                );
                continue;
            }

            if self.slots[index].entry_url.is_some()
                && let Err(err) = self.ensure_webview(index)
            {
                warn!(
                    "could not create wallpaper WebView for {}: {err:?}",
                    self.slots[index].device_name
                );
            }
        }

        self.apply_visibility();
        Ok(())
    }

    fn make_chrome_overlays_click_through(&mut self) {
        let rects: Vec<RECT> = self.slots.iter().map(|slot| slot.rect).collect();
        if rects.is_empty() {
            return;
        }

        let mut search = ChromeSearch {
            rects: &rects,
            found: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(
                Some(chrome_overlay_callback),
                LPARAM(&mut search as *mut _ as isize),
            );
        }

        for hwnd in search.found {
            let key = hwnd.0 as isize;
            if self.fixed_chrome_windows.contains(&key) {
                continue;
            }
            unsafe {
                let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(
                    hwnd,
                    GWL_EXSTYLE,
                    ex_style | WS_EX_LAYERED.0 as isize | WS_EX_TRANSPARENT.0 as isize,
                );
                // Do NOT call SetLayeredWindowAttributes(alpha=255) here. The
                // upstream Octos workaround only adds the ex-style; WebView2's
                // composition surface is per-pixel transparent. Forcing uniform
                // alpha makes the top-level Chrome host opaque and it covers
                // the desktop icons.
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
            info!("made WebView2 overlay {hwnd:?} click-through");
            self.fixed_chrome_windows.insert(key);
        }
    }

    fn ensure_window(&mut self, index: usize) -> Result<()> {
        let needs_window = {
            let hwnd = self.slots[index].hwnd;
            hwnd.is_invalid() || !unsafe { IsWindow(Some(hwnd)).as_bool() }
        };

        if needs_window {
            self.slots[index].close_webview();
            let hwnd = create_wallpaper_window()?;
            self.slots[index].hwnd = hwnd;
            self.slots[index].parent = HWND::default();
            self.slots[index].insert_after = None;
        }

        let hwnd = self.slots[index].hwnd;
        let parent_valid = {
            let parent = self.slots[index].parent;
            !parent.is_invalid() && unsafe { IsWindow(Some(parent)).as_bool() }
        };
        let insert_valid = self.slots[index]
            .insert_after
            .map(|window| !window.is_invalid() && unsafe { IsWindow(Some(window)).as_bool() })
            .unwrap_or(true);

        // Explorer can recreate SHELLDLL_DefView while Progman survives. A stale
        // insert_after would silently drop us above the icon layer, so reattach
        // whenever either handle is no longer valid.
        if !parent_valid || !insert_valid {
            let attached = attach_to_desktop(hwnd)?;
            self.slots[index].parent = attached.parent;
            self.slots[index].insert_after = attached.insert_after;
        }

        let rect = self.slots[index].rect;
        let parent = self.slots[index].parent;
        let insert_after = self.slots[index].insert_after;

        // Scaling can be changed in Windows while the app runs. The controller
        // captured the old DPI, so recreate it; WebView2 then picks up the new
        // device scale factor and the zoom is recomputed at creation.
        let dpi = monitor_dpi(rect);
        let stale_dpi = self.slots[index].webview.is_some()
            && self.slots[index].applied_dpi != 0
            && self.slots[index].applied_dpi != dpi;
        if stale_dpi {
            info!(
                "wallpaper DPI changed for {}: {} -> {}; recreating WebView",
                self.slots[index].device_name, self.slots[index].applied_dpi, dpi
            );
            self.slots[index].close_webview();
        }

        // Re-running SetWindowPos every watchdog tick re-composites a layered
        // window and shows up as a flash. Only touch it when geometry or the
        // z-order sibling actually changed.
        let mut current = RECT::default();
        let geometry_ok = unsafe { GetWindowRect(hwnd, &mut current) }.is_ok() && current == rect;
        let z_ok = insert_after.is_none_or(|expected| z_order_below(hwnd, expected));
        if !geometry_ok || !z_ok {
            debug!(
                "repositioning wallpaper for {} (geometry_ok={geometry_ok}, z_ok={z_ok})",
                self.slots[index].device_name
            );
            if !z_ok {
                debug_z_order(&self.slots[index].device_name, hwnd, insert_after);
            }
            position_wallpaper(hwnd, parent, insert_after, rect)?;
        }

        // Re-sync controller bounds only when the client size changed.
        let mut client = RECT::default();
        if unsafe { GetClientRect(hwnd, &mut client) }.is_ok() {
            let size = (client.right, client.bottom);
            if self.slots[index].applied_client != Some(size) {
                let resized = self.slots[index]
                    .webview
                    .as_ref()
                    .map(|webview| webview.resize(hwnd));
                if let Some(result) = resized {
                    match result {
                        Ok(()) => {
                            debug!(
                                "resized wallpaper WebView for {} to {}x{}",
                                self.slots[index].device_name, size.0, size.1
                            );
                            self.slots[index].applied_client = Some(size);
                        }
                        Err(err) => warn!(
                            "could not resize wallpaper WebView for {}: {err:?}",
                            self.slots[index].device_name
                        ),
                    }
                }
            }
        }
        Ok(())
    }

    fn ensure_webview(&mut self, index: usize) -> Result<()> {
        if let Some(retry_after) = self.slots[index].recreate_after {
            if Instant::now() < retry_after {
                return Ok(());
            }
            self.slots[index].recreate_after = None;
            self.slots[index].close_webview();
            info!(
                "recreating wallpaper WebView for {} after a process failure",
                self.slots[index].device_name
            );
        }
        if self.slots[index].webview.is_some() || self.slots[index].creating {
            return Ok(());
        }

        let environment = self
            .environment
            .clone()
            .ok_or_else(|| anyhow!("WebView2 environment is not ready"))?;
        let dcomp = self
            .dcomp
            .clone()
            .ok_or_else(|| anyhow!("DirectComposition device is not ready"))?;
        let hwnd = self.slots[index].hwnd;
        let entry_url = self.slots[index]
            .entry_url
            .clone()
            .ok_or_else(|| anyhow!("no wallpaper entry URL"))?;
        let device_name = self.slots[index].device_name.clone();
        let wallpapers_dir = self.wallpapers_dir.clone();
        let dpi = monitor_dpi(self.slots[index].rect);
        let created_url = entry_url.clone();
        let message_device = device_name.clone();
        let loaded_device = device_name.clone();
        let failed_device = device_name.clone();

        let defaults_script = self.slots[index]
            .mod_key
            .as_deref()
            .and_then(|key| octos_core::web_defaults::script_for_folder(std::path::Path::new(key)));
        self.slots[index].creating = true;
        let created = dcomp.create_wallpaper_controller(
            &environment,
            hwnd,
            wallpapers_dir,
            entry_url,
            device_name.clone(),
            self.fps_cap,
            self.muted,
            dpi,
            defaults_script,
            move |webview, text| {
                handle_wallpaper_message(&message_device, hwnd, webview, text);
            },
            move |webview| {
                handle_wallpaper_loaded(&loaded_device, webview);
            },
            move |_webview, kind| {
                warn!(
                    "wallpaper process failed for {failed_device} (kind={})",
                    kind.0
                );
                let _ = with_state(|state| {
                    if let Some(slot) = state
                        .slots
                        .iter_mut()
                        .find(|slot| slot.device_name == failed_device)
                    {
                        slot.recreate_after = Some(Instant::now() + Duration::from_secs(1));
                    }
                });
            },
            move |result| match result {
                Ok(webview) => {
                    let installed = with_state(|state| {
                        let installed = if let Some(slot) = state
                            .slots
                            .iter_mut()
                            .find(|slot| slot.device_name == device_name && slot.hwnd == hwnd)
                        {
                            slot.webview = Some(webview);
                            slot.creating = false;
                            slot.recreate_after = None;
                            slot.applied_dpi = dpi;
                            // A rotation may have pointed the slot at another
                            // mod while this controller was being created; the
                            // in-flight navigation still targets the old URL.
                            if let Some(current) = slot.entry_url.clone()
                                && current != created_url
                                && let Some(webview) = &slot.webview
                                && let Err(err) = navigate(&webview.webview, &current)
                            {
                                warn!(
                                    "could not follow a rotation that landed during controller creation: {err:?}"
                                );
                            }
                            true
                        } else {
                            false
                        };
                        state.make_chrome_overlays_click_through();
                        installed
                    })
                    .unwrap_or(false);
                    if !installed {
                        warn!("wallpaper controller finished for a vanished monitor");
                    }
                }
                Err(err) => {
                    warn!("wallpaper controller failed for {device_name}: {err:?}");
                    let _ = with_state(|state| {
                        if let Some(slot) = state
                            .slots
                            .iter_mut()
                            .find(|slot| slot.device_name == device_name && slot.hwnd == hwnd)
                        {
                            slot.creating = false;
                            slot.recreate_after = Some(Instant::now() + Duration::from_secs(1));
                        }
                    });
                }
            },
        );
        if let Err(err) = created {
            self.slots[index].creating = false;
            self.slots[index].recreate_after = Some(Instant::now() + Duration::from_secs(1));
            return Err(err);
        }
        Ok(())
    }

    fn apply_visibility(&mut self) {
        for slot in &mut self.slots {
            let visible = self.global_visible
                && slot.user_visible
                && !slot.fullscreen
                && slot.webview.is_some()
                && !slot.hwnd.is_invalid();

            if slot.applied_visible == Some(visible) {
                continue;
            }

            debug!(
                "applying wallpaper visibility {visible} for {}",
                slot.device_name
            );
            let mut applied = true;
            if let Some(webview) = &slot.webview
                && let Err(err) = webview.set_visible(visible)
            {
                warn!("could not set wallpaper visibility: {err:?}");
                applied = false;
            }

            if !slot.hwnd.is_invalid() {
                unsafe {
                    let _ = ShowWindow(slot.hwnd, if visible { SW_SHOW } else { SW_HIDE });
                }
            }
            if applied {
                slot.applied_visible = Some(visible);
            }
        }
    }

    fn update_fullscreen(&mut self) {
        if !self.pause_fullscreen {
            if self.slots.iter().any(|slot| slot.fullscreen) {
                for slot in &mut self.slots {
                    slot.fullscreen = false;
                }
                self.apply_visibility();
            }
            return;
        }
        let mut changed = false;
        for slot in &mut self.slots {
            let fullscreen = is_fullscreen(slot.rect);
            if fullscreen != slot.fullscreen {
                slot.fullscreen = fullscreen;
                changed = true;
                info!("monitor {} fullscreen={}", slot.device_name, fullscreen);
            }
        }
        if changed {
            self.apply_visibility();
        }
    }
}

#[derive(Clone, Copy)]
struct DesktopParent {
    parent: HWND,
    insert_after: Option<HWND>,
}

unsafe extern "system" fn watchdog_timer(_hwnd: HWND, _message: u32, _id: usize, _time: u32) {
    let _ = with_state(|state| {
        if let Err(err) = state.reconcile() {
            warn!("wallpaper watchdog reconcile failed: {err:?}");
        }

        // Fullscreen changes normally arrive with a foreground change. Keep a
        // 2s fallback for apps that go fullscreen without activating.
        let foreground = GetForegroundWindow();
        let now = Instant::now();
        if foreground != state.last_foreground
            || now.duration_since(state.last_fullscreen_check) >= Duration::from_secs(2)
        {
            state.last_foreground = foreground;
            state.last_fullscreen_check = now;
            state.update_fullscreen();
        }

        // Chrome overlays only change when a controller is created; on_ready
        // handles that immediately. The periodic pass is a safety net.
        if now.duration_since(state.last_overlay_check) >= Duration::from_secs(2) {
            state.last_overlay_check = now;
            state.make_chrome_overlays_click_through();
        }
    });
}

unsafe extern "system" fn monitor_callback(
    monitor: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
        let device_name = String::from_utf16_lossy(
            &info.szDevice[..info
                .szDevice
                .iter()
                .position(|ch| *ch == 0)
                .unwrap_or(info.szDevice.len())],
        );
        let monitors = &mut *(data.0 as *mut Vec<MonitorInfo>);
        monitors.push(MonitorInfo {
            device_name,
            rect: info.monitorInfo.rcMonitor,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    true.into()
}

fn enumerate_monitors() -> Vec<MonitorInfo> {
    let mut monitors = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(monitor_callback),
            LPARAM(&mut monitors as *mut _ as isize),
        );
    }
    monitors
}

fn create_wallpaper_window() -> Result<HWND> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let cursor = LoadCursorW(None, IDC_ARROW)?;
        let window_class = WNDCLASSW {
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(wallpaper_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: windows::Win32::Foundation::HINSTANCE::from(hinstance),
            hIcon: Default::default(),
            hCursor: cursor,
            hbrBackground: Default::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: WALLPAPER_CLASS,
        };
        let _ = RegisterClassW(&window_class);

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            WALLPAPER_CLASS,
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(windows::Win32::Foundation::HINSTANCE::from(hinstance)),
            None,
        )?;

        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        let _ = ShowWindow(hwnd, SW_HIDE);
        Ok(hwnd)
    }
}

unsafe extern "system" fn wallpaper_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => LRESULT(1),
        WM_WINDOWPOSCHANGING => {
            // WebView2 raises its host HWND when SendMouseInput receives a
            // click. Our wallpaper must stay below SHELLDLL_DefView, so force
            // any z-order change back to the sibling we attached under.
            if lparam.0 != 0 {
                let pos = &mut *(lparam.0 as *mut WINDOWPOS);
                let expected = with_state(|state| {
                    state
                        .slots
                        .iter()
                        .find(|slot| slot.hwnd == hwnd)
                        .and_then(|slot| slot.insert_after)
                })
                .flatten()
                .filter(|window| {
                    !window.is_invalid() && unsafe { IsWindow(Some(*window)).as_bool() }
                });
                if let Some(expected) = expected
                    && pos.hwndInsertAfter != expected
                {
                    debug!(
                        "keeping wallpaper {hwnd:?} below {expected:?} (requested {:?})",
                        pos.hwndInsertAfter
                    );
                    pos.hwndInsertAfter = expected;
                    pos.flags = SET_WINDOW_POS_FLAGS(pos.flags.0 & !SWP_NOZORDER.0);
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn attach_to_desktop(hwnd: HWND) -> Result<DesktopParent> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null())?;
        let _ = SendMessageTimeoutW(
            progman,
            0x052C,
            WPARAM(0),
            LPARAM(0),
            SMTO_NORMAL,
            1000,
            None,
        );

        // Windows 11 keeps SHELLDLL_DefView directly under Progman. Parent the
        // wallpaper there and insert it *below* the icon view. Preferring a
        // random top-level WorkerW here is what puts the wallpaper above icons.
        if let Ok(shell_view) =
            FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), PCWSTR::null())
        {
            SetParent(hwnd, Some(progman))?;
            info!(
                "desktop attach: Progman path (progman={progman:?}, insert_after={shell_view:?})"
            );
            return Ok(DesktopParent {
                parent: progman,
                insert_after: Some(shell_view),
            });
        }

        let mut workerw = HWND::default();
        let _ = EnumWindows(
            Some(enum_workerw),
            LPARAM(&mut workerw as *mut HWND as isize),
        );

        if !workerw.is_invalid() {
            SetParent(hwnd, Some(workerw))?;
            info!("desktop attach: WorkerW path (workerw={workerw:?})");
            Ok(DesktopParent {
                parent: workerw,
                insert_after: None,
            })
        } else {
            SetParent(hwnd, Some(progman))?;
            info!("desktop attach: Progman fallback (no SHELLDLL_DefView, no WorkerW)");
            Ok(DesktopParent {
                parent: progman,
                insert_after: None,
            })
        }
    }
}

unsafe extern "system" fn enum_workerw(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), PCWSTR::null()).is_err() {
        return true.into();
    }
    if let Ok(worker) = FindWindowExW(None, Some(hwnd), w!("WorkerW"), PCWSTR::null())
        && !worker.is_invalid()
    {
        *(lparam.0 as *mut HWND) = worker;
        return false.into();
    }
    true.into()
}

/// Effective DPI of the monitor that contains `rect`.
///
/// `GetDpiForWindow` is unreliable for the wallpaper window: it is a child of
/// Progman/WorkerW (an Explorer window with its own DPI context), so it can
/// report the primary monitor's DPI even after moving to a mixed-DPI monitor.
fn monitor_dpi(rect: RECT) -> u32 {
    unsafe {
        let monitor = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);
        let mut x = 96u32;
        let mut y = 96u32;
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_ok() && x >= 96 {
            x
        } else {
            96
        }
    }
}

fn position_wallpaper(
    hwnd: HWND,
    parent: HWND,
    insert_after: Option<HWND>,
    rect: RECT,
) -> Result<()> {
    unsafe {
        let mut parent_rect = RECT::default();
        let _ = GetWindowRect(parent, &mut parent_rect);
        let x = rect.left - parent_rect.left;
        let y = rect.top - parent_rect.top;
        let width = (rect.right - rect.left).max(1);
        let height = (rect.bottom - rect.top).max(1);

        let flags = if insert_after.is_some() {
            SWP_NOACTIVATE
        } else {
            SWP_NOACTIVATE | SWP_NOZORDER
        };
        SetWindowPos(hwnd, insert_after, x, y, width, height, flags)?;

        if let Some(expected) = insert_after
            && !z_order_below(hwnd, expected)
        {
            debug!(
                "wallpaper z-order mismatch for {hwnd:?}, expected below {expected:?}; reinserting"
            );
            SetWindowPos(hwnd, Some(expected), x, y, width, height, SWP_NOACTIVATE)?;
        }
    }
    Ok(())
}

fn rects_overlap(a: RECT, b: RECT) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

fn debug_z_order(device: &str, hwnd: HWND, expected: Option<HWND>) {
    let parent = unsafe { GetAncestor(hwnd, GA_PARENT) };
    let previous = unsafe { GetWindow(hwnd, GW_HWNDPREV).ok() };
    let next = unsafe { GetWindow(hwnd, GW_HWNDNEXT).ok() };
    debug!(
        "z-debug {device}: hwnd={hwnd:?} parent={parent:?}({}) prev={previous:?}({}) next={next:?}({})",
        window_class_name(parent),
        previous.map(window_class_name).unwrap_or_default(),
        next.map(window_class_name).unwrap_or_default()
    );
    if let Some(expected) = expected {
        let mut current = expected;
        let mut chain = Vec::new();
        for _ in 0..16 {
            let Some(next) = (unsafe { GetWindow(current, GW_HWNDNEXT).ok() }) else {
                break;
            };
            if next.is_invalid() {
                break;
            }
            chain.push(format!("{next:?}({})", window_class_name(next)));
            if next == hwnd {
                break;
            }
            current = next;
        }
        debug!("z-debug {device}: chain from {expected:?}: {chain:?}");
    }
}

/// True when `hwnd` is somewhere below `expected` in the z-order chain.
///
/// This mirrors upstream `FixWallpaperOrder`: walk `GW_HWNDNEXT` from
/// `SHELLDLL_DefView` and look for our window. Explorer inserts helper windows
/// between DefView and the wallpaper when the desktop is clicked; those do not
/// mean the wallpaper escaped above the icons, so strict adjacency would cause
/// a needless (visible) SetWindowPos on every click.
fn z_order_below(hwnd: HWND, expected: HWND) -> bool {
    let mut current = expected;
    for _ in 0..64 {
        let Some(next) = (unsafe { GetWindow(current, GW_HWNDNEXT).ok() }) else {
            return false;
        };
        if next == hwnd {
            return true;
        }
        if next.is_invalid() {
            return false;
        }
        current = next;
    }
    false
}

fn point_in_rect(point: POINT, rect: RECT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

fn window_class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code != HC_ACTION as i32 {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    // Diagnostic escape hatch: OCTOS_DISABLE_MOUSE_FORWARD=1 keeps the hook
    // installed but stops SendMouseInput, useful when isolating z-order issues.
    if std::env::var_os("OCTOS_DISABLE_MOUSE_FORWARD").is_some() {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let message = wparam.0 as u32;
    let mouse = &*(lparam.0 as *const MSLLHOOKSTRUCT);
    let top_window = WindowFromPoint(mouse.pt);
    let class_name = window_class_name(top_window);

    if !DESKTOP_CLASSES.contains(&class_name.as_str()) {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    if message == WM_LBUTTONDOWN || message == WM_LBUTTONUP {
        debug!(
            "mouse {:x} at {:?} over class={class_name}",
            message, mouse.pt
        );
    }

    let target = with_state(|state| {
        state
            .slots
            .iter()
            .find(|slot| {
                state.global_visible
                    && slot.user_visible
                    && !slot.fullscreen
                    && point_in_rect(mouse.pt, slot.rect)
            })
            .and_then(|slot| {
                slot.webview
                    .as_ref()
                    .map(|webview| (slot.hwnd, webview.composition_controller.clone()))
            })
    })
    .flatten();

    let Some((hwnd, controller)) = target else {
        if message == WM_LBUTTONDOWN || message == WM_LBUTTONUP {
            debug!("no wallpaper target for {:?}", mouse.pt);
        }
        return CallNextHookEx(None, code, wparam, lparam);
    };

    let mut client_point = mouse.pt;
    let _ = ScreenToClient(hwnd, &mut client_point);

    let (kind, virtual_keys, mouse_data) = match message {
        WM_MOUSEMOVE => (
            COREWEBVIEW2_MOUSE_EVENT_KIND_MOVE,
            COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_NONE,
            0,
        ),
        WM_LBUTTONDOWN => (
            COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_DOWN,
            COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_LEFT_BUTTON,
            0,
        ),
        WM_LBUTTONUP => (
            COREWEBVIEW2_MOUSE_EVENT_KIND_LEFT_BUTTON_UP,
            COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_LEFT_BUTTON,
            0,
        ),
        WM_RBUTTONDOWN => (
            COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_DOWN,
            COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_RIGHT_BUTTON,
            0,
        ),
        WM_RBUTTONUP => (
            COREWEBVIEW2_MOUSE_EVENT_KIND_RIGHT_BUTTON_UP,
            COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_RIGHT_BUTTON,
            0,
        ),
        WM_MBUTTONDOWN => (
            COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_DOWN,
            COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_MIDDLE_BUTTON,
            0,
        ),
        WM_MBUTTONUP => (
            COREWEBVIEW2_MOUSE_EVENT_KIND_MIDDLE_BUTTON_UP,
            COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_MIDDLE_BUTTON,
            0,
        ),
        WM_MOUSEWHEEL => (
            COREWEBVIEW2_MOUSE_EVENT_KIND_WHEEL,
            COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS_NONE,
            mouse.mouseData,
        ),
        _ => return CallNextHookEx(None, code, wparam, lparam),
    };

    if message == WM_LBUTTONDOWN || message == WM_LBUTTONUP {
        debug!(
            "forwarding mouse {:x} to {hwnd:?} client={client_point:?}",
            message
        );
    }
    let _ = controller.SendMouseInput(kind, virtual_keys, mouse_data, client_point);
    CallNextHookEx(None, code, wparam, lparam)
}

struct ProtocolHost {
    device_name: String,
    mod_key: Option<String>,
    source: String,
    theme_light: bool,
    visible: bool,
    desktop_icons: bool,
    options: OptionsMap,
    siblings: Vec<String>,
    pending_option: Option<(String, String, serde_json::Value)>,
    pending_visible: Option<bool>,
    pending_desktop_icons: Option<bool>,
    pending_redirect: Option<(String, serde_json::Value)>,
    pending_devtools: bool,
}

impl WallpaperHost for ProtocolHost {
    fn source(&mut self) -> String {
        self.source.clone()
    }

    fn is_theme_light(&mut self) -> bool {
        self.theme_light
    }

    fn visibility(&mut self) -> bool {
        self.visible
    }

    fn monitor_id(&mut self) -> String {
        self.device_name.clone()
    }

    fn siblings(&mut self) -> Vec<String> {
        self.siblings.clone()
    }

    fn desktop_icons_visible(&mut self) -> bool {
        self.desktop_icons
    }

    fn options(&mut self) -> OptionsMap {
        self.options.clone()
    }

    fn set_option(&mut self, id: &str, value: serde_json::Value) -> bool {
        let Some(mod_key) = &self.mod_key else {
            return false;
        };
        self.pending_option = Some((mod_key.clone(), id.to_string(), value));
        true
    }

    fn set_visible(&mut self, visible: bool) -> bool {
        self.pending_visible = Some(visible);
        true
    }

    fn set_desktop_icons_visible(&mut self, visible: bool) -> bool {
        self.pending_desktop_icons = Some(visible);
        true
    }

    fn open_dev_tools(&mut self) {
        self.pending_devtools = true;
    }

    fn redirect(&mut self, receiver_id: &str, message: serde_json::Value) {
        self.pending_redirect = Some((receiver_id.to_string(), message));
    }
}

fn handle_wallpaper_message(device_name: &str, hwnd: HWND, webview: &ICoreWebView2, text: &str) {
    let Ok(message) = serde_json::from_str::<serde_json::Value>(text) else {
        debug!("ignoring non-JSON wallpaper message");
        return;
    };

    let source = unsafe {
        let mut source = PWSTR::null();
        let _ = webview.Source(&mut source);
        take_pwstr(source)
    };

    let Some((visible, mod_key, options, siblings)) = with_state(|state| {
        let slot = state
            .slots
            .iter()
            .find(|slot| slot.device_name == device_name && slot.hwnd == hwnd)?;
        let visible = slot
            .webview
            .as_ref()
            .map(WallpaperWebView::is_visible)
            .unwrap_or(false);
        let mod_key = state
            .session
            .mod_for_monitor(device_name)
            .map(str::to_string);
        let options = mod_key
            .as_deref()
            .map(|key| state.session.options_for(key).clone())
            .unwrap_or_default();
        let siblings = state.session.siblings(device_name);
        Some((visible, mod_key, options, siblings))
    })
    .flatten() else {
        return;
    };

    let mut host = ProtocolHost {
        device_name: device_name.to_string(),
        mod_key,
        source,
        theme_light: is_theme_light(),
        visible,
        desktop_icons: crate::desktop_icons::is_visible(),
        options,
        siblings,
        pending_option: None,
        pending_visible: None,
        pending_desktop_icons: None,
        pending_redirect: None,
        pending_devtools: false,
    };

    // Test-only internal trigger for the UI-originated option path. This is not
    // part of the frozen mod protocol.
    if std::env::var_os("OCTOS_TEST_HOOKS").is_some()
        && message.get("type").and_then(serde_json::Value::as_str) == Some("command")
        && message
            .get("commandType")
            .and_then(serde_json::Value::as_str)
            == Some("octos-internal-apply-option")
        && let Some(mod_key) = host.mod_key.as_deref()
        && let Some(id) = message
            .get("data")
            .and_then(|data| data.get("id"))
            .and_then(serde_json::Value::as_str)
        && let Some(value) = message
            .get("data")
            .and_then(|data| data.get("value"))
            .cloned()
    {
        apply_option_from_ui(mod_key, id, value);
        return;
    }

    if let Some(response) = dispatch(&message, &mut host) {
        debug!("wallpaper protocol response: {response}");
        if let Err(err) = post_json(webview, &response.to_string()) {
            warn!("could not post protocol response: {err:?}");
        }
    }

    if let Some((mod_key, id, value)) = host.pending_option.take() {
        let _ = with_state(|state| {
            state.session.set_option(&mod_key, &id, value);
        });
    }

    if let Some(visible) = host.pending_visible {
        let _ = with_state(|state| {
            if let Some(slot) = state
                .slots
                .iter_mut()
                .find(|slot| slot.device_name == device_name && slot.hwnd == hwnd)
            {
                slot.user_visible = visible;
            }
            state.apply_visibility();
        });
    }

    if let Some(visible) = host.pending_desktop_icons
        && let Err(err) = crate::desktop_icons::set_visible(visible)
    {
        warn!("could not set desktop icon visibility: {err:?}");
    }

    if host.pending_devtools {
        unsafe {
            let _ = webview.OpenDevToolsWindow();
        }
    }

    if let Some((receiver_id, message)) = host.pending_redirect.take() {
        let event = redirect_event(device_name, &receiver_id, message);
        let target = with_state(|state| {
            state
                .slots
                .iter()
                .find(|slot| slot.device_name == receiver_id)
                .and_then(|slot| slot.webview.as_ref().map(|webview| webview.webview.clone()))
        })
        .flatten();
        if let Some(target) = target {
            if let Err(err) = post_json(&target, &event.to_string()) {
                warn!("could not redirect to {receiver_id}: {err:?}");
            }
        } else {
            debug!("redirect target not found: {receiver_id}");
        }
    }
}

/// UI-originated option change: update state, notify wallpapers, run hooks.
/// The mod-facing `setOption` command intentionally does not use this path.
pub fn apply_option_from_ui(mod_key: &str, id: &str, value: serde_json::Value) -> bool {
    let (spec, changed) = with_state(|state| {
        let spec = state.session.options_for(mod_key).get(id).cloned();
        let changed = state.session.set_option(mod_key, id, value.clone());
        (spec, changed)
    })
    .unwrap_or((None, false));

    if !changed {
        return false;
    }

    let targets = with_state(|state| {
        state
            .slots
            .iter()
            .filter(|slot| state.session.mod_for_monitor(&slot.device_name) == Some(mod_key))
            .filter_map(|slot| slot.webview.as_ref().map(|webview| webview.webview.clone()))
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    for webview in targets {
        let event = options_event("options-change", id, &value);
        if let Err(err) = post_json(&webview, &event.to_string()) {
            warn!("could not post options-change: {err:?}");
        }
        if let Some(spec) = &spec {
            for script in scripts_for_event(spec, &value, HookEvent::Change) {
                if let Err(err) = execute_script(&webview, &script) {
                    warn!("could not execute option hook: {err:?}");
                }
            }
        }
    }
    true
}

fn handle_wallpaper_loaded(device_name: &str, webview: &ICoreWebView2) {
    let options = with_state(|state| {
        let mod_key = state.session.mod_for_monitor(device_name)?;
        Some(state.session.options_for(mod_key).clone())
    })
    .flatten()
    .unwrap_or_default();

    for (id, spec) in &options {
        let Some(value) = spec.get("value") else {
            continue;
        };
        let event = options_event("options-load", id, value);
        if let Err(err) = post_json(webview, &event.to_string()) {
            warn!("could not post options-load: {err:?}");
        }
        for script in scripts_for_event(spec, value, HookEvent::Load) {
            if let Err(err) = execute_script(webview, &script) {
                warn!("could not execute option load hook: {err:?}");
            }
        }
    }
}

pub fn system_theme_is_light() -> bool {
    is_theme_light()
}

fn is_theme_light() -> bool {
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    result == ERROR_SUCCESS && value != 0
}

struct ChromeSearch<'a> {
    rects: &'a [RECT],
    found: Vec<HWND>,
}

unsafe extern "system" fn chrome_overlay_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let search = &mut *(lparam.0 as *mut ChromeSearch);
    let class_name = window_class_name(hwnd);
    if !class_name.starts_with("Chrome_WidgetWin_") || !IsWindowVisible(hwnd).as_bool() {
        return true.into();
    }
    // Chrome/Edge browser windows share this class; only our own WebView2
    // hosts may be made click-through.
    if !crate::is_our_webview_window(hwnd) {
        return true.into();
    }

    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() {
        return true.into();
    }

    if search
        .rects
        .iter()
        .any(|monitor| rect_matches(rect, *monitor))
    {
        search.found.push(hwnd);
    }
    true.into()
}

fn rect_matches(a: RECT, b: RECT) -> bool {
    const TOLERANCE: i32 = 2;
    (a.left - b.left).abs() <= TOLERANCE
        && (a.top - b.top).abs() <= TOLERANCE
        && (a.right - b.right).abs() <= TOLERANCE
        && (a.bottom - b.bottom).abs() <= TOLERANCE
}

fn is_fullscreen(monitor_rect: RECT) -> bool {
    struct Search {
        rect: RECT,
        found: bool,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return true.into();
        }

        // "Show desktop" cloaks windows via DWM instead of minimizing them, so
        // IsIconic stays false and the window still has a full-monitor rect.
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return true.into();
        }

        let class_name = window_class_name(hwnd);
        // Our own overlay hosts are part of the wallpaper and never count as a
        // fullscreen app. A browser with the same Chrome class must fall
        // through to the rectangle check below.
        if class_name.starts_with("Chrome_WidgetWin_") && crate::is_our_webview_window(hwnd) {
            return true.into();
        }
        if matches!(
            class_name.as_str(),
            "OctosMainWindow" | "OctosWallpaperWindow" | "Shell_TrayWnd" | "Progman" | "WorkerW"
        ) || class_name.starts_with("Chrome_RenderWidgetHostHWND")
            || class_name.starts_with("WebView2")
            || class_name.starts_with("ShellHandwritingCanvas")
            || class_name.starts_with("XamlExplorerHost")
            || class_name.ends_with("OverlayWindow")
        {
            return true.into();
        }

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return true.into();
        }

        const TOLERANCE: i32 = 2;
        let covers = (rect.left - search.rect.left).abs() <= TOLERANCE
            && (rect.top - search.rect.top).abs() <= TOLERANCE
            && (rect.right - search.rect.right).abs() <= TOLERANCE
            && (rect.bottom - search.rect.bottom).abs() <= TOLERANCE;

        if covers {
            debug!(
                "fullscreen candidate class={class_name} rect={},{},{},{}",
                rect.left, rect.top, rect.right, rect.bottom
            );
            search.found = true;
            return false.into();
        }
        true.into()
    }

    let mut search = Search {
        rect: monitor_rect,
        found: false,
    };
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut search as *mut _ as isize));
    }
    search.found
}
