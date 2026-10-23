// M0 TOOLCHAIN SPIKE — NOT THE WALLPAPER IMPLEMENTATION.
// Superseded by crates/octos-win/src/wallpaper.rs.
// WorkerW-first attach, no Chrome_WidgetWin click-through, no fullscreen /
// visibility gating, single primary monitor. Do NOT use this binary to judge
// wallpaper z-order, desktop icons, click-through, or fullscreen behavior.
#![windows_subsystem = "windows"]
#![allow(unsafe_op_in_unsafe_fn)]

use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use webview2_com::{
    take_pwstr, CoreWebView2EnvironmentOptions, CreateCoreWebView2CompositionControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, WebMessageReceivedEventHandler,
    Microsoft::Web::WebView2::Win32::*,
};
use windows::core::{w, BOOL, Error, Interface, PCWSTR, PWSTR, Result, IUnknown};
use windows::Win32::{
    Foundation::{COLORREF, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
        Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL},
        Direct3D11::{
            D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
        },
        DirectComposition::{
            DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
        },
        Dxgi::IDXGIDevice,
        Gdi::ScreenToClient,
    },
    System::{
        Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        LibraryLoader::GetModuleHandleW,
    },
    UI::{
        HiDpi::{GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
        WindowsAndMessaging::*,
    },
};

const APP_CLASS: PCWSTR = w!("OctosSpikeWindow");
thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

struct State {
    hwnd: HWND,
    dcomp: Option<IDCompositionDevice>,
    target: Option<IDCompositionTarget>,
    visual: Option<IDCompositionVisual>,
    controller: Option<ICoreWebView2Controller>,
    comp_controller: Option<ICoreWebView2CompositionController>,
    webview: Option<ICoreWebView2>,
    env: Option<ICoreWebView2Environment3>,
    hook: Option<HHOOK>,
    timer: usize,
    recreating: bool,
    quitting: bool,
}

impl State {
    fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            dcomp: None,
            target: None,
            visual: None,
            controller: None,
            comp_controller: None,
            webview: None,
            env: None,
            hook: None,
            timer: 0,
            recreating: false,
            quitting: false,
        }
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        log(&format!("PANIC: {info}"));
    }));

    log("=== octos spike starting ===");

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            log(&format!("CoInitializeEx failed: {hr:?}"));
            return;
        }

        match run() {
            Ok(()) => log("message loop exited cleanly"),
            Err(err) => {
                log(&format!("fatal: {err:?}"));
                show_error(&format!("Octos spike failed:\n{err:?}"));
            }
        }

        CoUninitialize();
    }

    log("=== octos spike stopped ===");
}

unsafe fn run() -> Result<()> {
    let hwnd = create_window()?;
    STATE.with(|state| *state.borrow_mut() = Some(State::new(hwnd)));
    log(&format!("window created: {:?}", hwnd));

    let dcomp = create_dcomp_device()?;
    with_state(|state| state.dcomp = Some(dcomp));
    log("d3d11 + directcomposition ready");

    create_webview(hwnd)?;
    install_mouse_hook()?;
    log("mouse hook installed");

    let timer = SetTimer(None, 1, 500, Some(watchdog_timer));
    with_state(|state| state.timer = timer);
    log(&format!("watchdog timer started: {timer}"));

    message_loop();
    shutdown();
    Ok(())
}

unsafe fn create_window() -> Result<HWND> {
    let hinstance = GetModuleHandleW(None)?;
    let cursor = LoadCursorW(None, IDC_ARROW)?;

    let wc = WNDCLASSW {
        style: WNDCLASS_STYLES(0),
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: HINSTANCE::from(hinstance),
        hIcon: Default::default(),
        hCursor: cursor,
        hbrBackground: Default::default(),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: APP_CLASS,
    };

    let atom = RegisterClassW(&wc);
    if atom == 0 {
        log("RegisterClassW returned 0 (class may already exist)");
    }

    let width = GetSystemMetrics(SM_CXSCREEN);
    let height = GetSystemMetrics(SM_CYSCREEN);
    log(&format!("primary screen: {width}x{height}"));

    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        APP_CLASS,
        w!("M0 toolchain spike - NOT the wallpaper implementation"),
        WS_POPUP | WS_VISIBLE,
        0,
        0,
        width,
        height,
        None,
        None,
        Some(HINSTANCE::from(hinstance)),
        None,
    )?;

    SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
    if let Err(err) = attach_to_desktop(hwnd, width, height) {
        let _ = DestroyWindow(hwnd);
        return Err(err);
    }
    let _ = ShowWindow(hwnd, SW_SHOW);
    Ok(hwnd)
}

unsafe fn attach_to_desktop(hwnd: HWND, width: i32, height: i32) -> Result<()> {
    let progman = FindWindowW(w!("Progman"), PCWSTR::null())?;

    // Undocumented message that makes Explorer create the WorkerW used by wallpaper engines.
    let _ = SendMessageTimeoutW(progman, 0x052C, WPARAM(0), LPARAM(0), SMTO_NORMAL, 1000, None);

    let mut workerw = HWND::default();
    let _ = EnumWindows(Some(enum_workerw), LPARAM(&mut workerw as *mut HWND as isize));

    if !workerw.is_invalid() {
        SetParent(hwnd, Some(workerw))?;
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            width,
            height,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
        );
        log(&format!("attached to WorkerW {:?}", workerw));
    } else {
        SetParent(hwnd, Some(progman))?;
        let shell_view = FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), PCWSTR::null())?;
        let insert_after = if shell_view.is_invalid() { None } else { Some(shell_view) };
        let _ = SetWindowPos(
            hwnd,
            insert_after,
            0,
            0,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        log("attached to Progman below SHELLDLL_DefView (Win11 path)");
    }

    Ok(())
}

unsafe extern "system" fn enum_workerw(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let shell_view = FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), PCWSTR::null());
    if shell_view.is_err() {
        return true.into();
    }

    if let Ok(worker) = FindWindowExW(None, Some(hwnd), w!("WorkerW"), PCWSTR::null()) {
        if !worker.is_invalid() {
            *(lparam.0 as *mut HWND) = worker;
            return false.into();
        }
    }

    true.into()
}

unsafe fn create_dcomp_device() -> Result<IDCompositionDevice> {
    let mut d3d_device: Option<ID3D11Device> = None;
    D3D11CreateDevice(
        None,
        D3D_DRIVER_TYPE_HARDWARE,
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        None::<&[D3D_FEATURE_LEVEL]>,
        D3D11_SDK_VERSION,
        Some(&mut d3d_device),
        None,
        None,
    )?;

    let d3d_device = d3d_device.ok_or_else(|| Error::from(windows::Win32::Foundation::E_POINTER))?;
    let dxgi_device: IDXGIDevice = d3d_device.cast()?;
    let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;
    Ok(dcomp)
}

unsafe fn create_webview(hwnd: HWND) -> Result<()> {
    if let Some(environment) = with_state(|state| state.env.clone()).flatten() {
        log("reusing WebView2 environment");
        return create_composition_controller(environment, hwnd);
    }

    let user_data = wide("E:\\file\\octos\\webview2-data");

    let options: ICoreWebView2EnvironmentOptions = CoreWebView2EnvironmentOptions::default().into();
    options.SetAdditionalBrowserArguments(w!(
        "--disable-features=TranslateUI,AudioServiceOutOfProcess,LowIntegrityMode \
         --no-first-run --no-default-browser-check --disable-breakpad \
         --disable-component-update --disable-background-networking"
    ))?;

    let environment_handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |result, environment| {
            result?;
            let environment = environment
                .ok_or_else(|| Error::from(windows::Win32::Foundation::E_POINTER))?;
            let environment3: ICoreWebView2Environment3 = environment.cast()?;
            with_state(|state| state.env = Some(environment3.clone()));
            create_composition_controller(environment3, hwnd)?;
            Ok(())
        },
    ));

    CreateCoreWebView2EnvironmentWithOptions(
        PCWSTR::null(),
        PCWSTR::from_raw(user_data.as_ptr()),
        &options,
        &environment_handler,
    )?;

    log("requested WebView2 environment (async)");
    Ok(())
}

unsafe fn create_composition_controller(
    environment: ICoreWebView2Environment3,
    hwnd: HWND,
) -> Result<()> {
    let controller_handler = CreateCoreWebView2CompositionControllerCompletedHandler::create(Box::new(
        move |result, controller| {
            result?;
            let controller = controller
                .ok_or_else(|| Error::from(windows::Win32::Foundation::E_POINTER))?;
            on_composition_controller_created(hwnd, controller)
        },
    ));

    environment.CreateCoreWebView2CompositionController(hwnd, &controller_handler)?;
    log("requested composition controller");
    Ok(())
}

unsafe fn recreate_wallpaper() -> Result<()> {
    let old_hwnd = with_state(|state| state.hwnd).unwrap_or_default();
    with_state(|state| {
        if let Some(controller) = state.controller.take() {
            let _ = controller.Close();
        }
        state.comp_controller = None;
        state.webview = None;
        state.target = None;
        state.visual = None;
    });

    if !old_hwnd.is_invalid() && IsWindow(Some(old_hwnd)).as_bool() {
        let _ = DestroyWindow(old_hwnd);
    }

    let hwnd = create_window()?;
    with_state(|state| state.hwnd = hwnd);
    log(&format!("wallpaper recreated: {:?}", hwnd));
    create_webview(hwnd)
}

unsafe extern "system" fn watchdog_timer(_hwnd: HWND, _message: u32, _id: usize, _time: u32) {
    let Some((hwnd, recreating, quitting)) =
        with_state(|state| (state.hwnd, state.recreating, state.quitting))
    else {
        return;
    };

    if quitting || recreating {
        return;
    }

    if !IsWindow(Some(hwnd)).as_bool() {
        log("watchdog: wallpaper window lost, recreating");
        with_state(|state| state.recreating = true);
        if let Err(err) = recreate_wallpaper() {
            log(&format!("watchdog: recreate failed: {err:?}"));
            with_state(|state| state.recreating = false);
        }
    }
}

unsafe fn on_composition_controller_created(
    hwnd: HWND,
    comp_controller: ICoreWebView2CompositionController,
) -> Result<()> {
    let controller: ICoreWebView2Controller = comp_controller.cast()?;
    let webview = controller.CoreWebView2()?;

    let dcomp = with_state(|state| state.dcomp.clone())
        .flatten()
        .ok_or_else(|| Error::from(windows::Win32::Foundation::E_POINTER))?;

    let target = dcomp.CreateTargetForHwnd(hwnd, false)?;
    let visual = dcomp.CreateVisual()?;
    target.SetRoot(&visual)?;

    let visual_unknown: IUnknown = visual.cast()?;
    comp_controller.SetRootVisualTarget(&visual_unknown)?;

    let mut client = RECT::default();
    GetClientRect(hwnd, &mut client)?;
    controller.SetBounds(client)?;
    controller.SetIsVisible(true)?;

    let mut window_rect = RECT::default();
    let _ = GetWindowRect(hwnd, &mut window_rect);
    let parent_rect = GetParent(hwnd)
        .ok()
        .filter(|parent| !parent.is_invalid())
        .map(|parent| {
            let mut rect = RECT::default();
            let _ = GetWindowRect(parent, &mut rect);
            rect
        });
    let mut controller_bounds = RECT::default();
    let _ = controller.Bounds(&mut controller_bounds);
    let mut rasterization_scale = 0.0;
    if let Ok(controller3) = controller.cast::<ICoreWebView2Controller3>() {
        let _ = controller3.RasterizationScale(&mut rasterization_scale);
    }
    log(&format!(
        "geometry: dpi={} window={:?} client={:?} parent={:?} bounds={:?} raster={}",
        GetDpiForWindow(hwnd),
        window_rect,
        client,
        parent_rect,
        controller_bounds,
        rasterization_scale
    ));

    if let Ok(controller2) = controller.cast::<ICoreWebView2Controller2>() {
        let _ = controller2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
            A: 0,
            R: 0,
            G: 0,
            B: 0,
        });
    }

    let settings = webview.Settings()?;
    settings.SetAreDefaultContextMenusEnabled(false)?;
    settings.SetAreDevToolsEnabled(true)?;
    settings.SetIsStatusBarEnabled(false)?;
    settings.SetIsZoomControlEnabled(false)?;
    settings.SetAreHostObjectsAllowed(false)?;

    let message_handler = WebMessageReceivedEventHandler::create(Box::new(|_sender, args| {
        if let Some(args) = args {
            let mut json = PWSTR::null();
            args.WebMessageAsJson(&mut json)?;
            let message = take_pwstr(json);
            log(&format!("page message: {message}"));

            if message.contains("\"quit\"") {
                let hwnd = with_state(|state| state.hwnd);
                if let Some(hwnd) = hwnd {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        }
        Ok(())
    }));
    let mut token = 0i64;
    webview.add_WebMessageReceived(&message_handler, &mut token)?;

    let html = wide(SPIKE_HTML);
    webview.NavigateToString(PCWSTR::from_raw(html.as_ptr()))?;

    with_state(|state| {
        state.target = Some(target);
        state.visual = Some(visual);
        state.controller = Some(controller);
        state.comp_controller = Some(comp_controller);
        state.webview = Some(webview);
    });

    dcomp.Commit()?;
    with_state(|state| state.recreating = false);
    log("composition controller ready, HTML navigated");
    Ok(())
}

unsafe fn install_mouse_hook() -> Result<()> {
    let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None, 0)?;
    with_state(|state| state.hook = Some(hook));
    Ok(())
}

unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let message = wparam.0 as u32;
        let mouse = &*(lparam.0 as *const MSLLHOOKSTRUCT);

        if let Some((hwnd, comp)) = with_state(|state| (state.hwnd, state.comp_controller.clone())) {
            if let Some(comp) = comp {
                forward_mouse(&comp, hwnd, message, mouse);
            }
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}

unsafe fn forward_mouse(
    comp: &ICoreWebView2CompositionController,
    hwnd: HWND,
    message: u32,
    mouse: &MSLLHOOKSTRUCT,
) {
    let mut window_rect = RECT::default();
    if GetWindowRect(hwnd, &mut window_rect).is_err() {
        return;
    }

    if mouse.pt.x < window_rect.left
        || mouse.pt.x >= window_rect.right
        || mouse.pt.y < window_rect.top
        || mouse.pt.y >= window_rect.bottom
    {
        return;
    }

    // Only forward when the pointer is actually over the desktop. Without this guard,
    // clicks in an app that happens to cover the wallpaper rect would leak into the mod.
    let top_window = WindowFromPoint(mouse.pt);
    let mut class_buffer = [0u16; 256];
    let class_len = GetClassNameW(top_window, &mut class_buffer);
    let class_name = String::from_utf16_lossy(&class_buffer[..class_len.max(0) as usize]);
    let over_desktop = matches!(
        class_name.as_str(),
        "SysListView32" | "SHELLDLL_DefView" | "Progman"
    );
    if message == WM_LBUTTONDOWN || message == WM_LBUTTONUP {
        log(&format!(
            "mouse button {:x}: screen={:?} top={:?} class={} desktop={}",
            message, mouse.pt, top_window, class_name, over_desktop
        ));
    }
    if !over_desktop && top_window != hwnd {
        return;
    }

    let mut point = POINT {
        x: mouse.pt.x,
        y: mouse.pt.y,
    };
    let _ = ScreenToClient(hwnd, &mut point);

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
        _ => return,
    };

    let _ = comp.SendMouseInput(kind, virtual_keys, mouse_data, point);
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CLOSE => {
            log("WM_CLOSE");
            with_state(|state| state.quitting = true);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let quitting = with_state(|state| state.quitting).unwrap_or(true);
            if quitting {
                log("WM_DESTROY (quitting)");
                PostQuitMessage(0);
            } else {
                log("WM_DESTROY (unexpected; watchdog will recreate)");
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn message_loop() {
    let mut message = MSG::default();
    while GetMessageW(&mut message, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
}

unsafe fn shutdown() {
    with_state(|state| {
        if state.timer != 0 {
            let _ = KillTimer(None, state.timer);
            state.timer = 0;
        }
        if let Some(hook) = state.hook.take() {
            let _ = UnhookWindowsHookEx(hook);
        }
        if let Some(controller) = state.controller.take() {
            let _ = controller.Close();
        }
        state.webview.take();
        state.comp_controller.take();
        state.target.take();
        state.visual.take();
        state.env.take();
        state.dcomp.take();
    });
    log("shutdown complete");
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|state| state.borrow_mut().as_mut().map(f))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn log_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("logs")))
        .unwrap_or_else(std::env::temp_dir)
        .join("spike.log")
}

fn log(message: &str) {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        let _ = writeln!(file, "[{now}] {message}");
    }
}

fn show_error(message: &str) {
    let text = wide(message);
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR::from_raw(text.as_ptr()),
            w!("M0 toolchain spike - NOT the wallpaper implementation"),
            MB_OK | MB_ICONERROR,
        );
    }
}

const SPIKE_HTML: &str = r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>
  html, body { margin: 0; height: 100%; background: transparent; overflow: hidden; }
  body { font: 15px/1.4 "Segoe UI", sans-serif; color: #fff; }
  .card {
    position: absolute; left: 50%; top: 42%; transform: translate(-50%, -50%);
    width: 420px; padding: 22px 24px; border-radius: 18px;
    background: rgba(18, 18, 22, 0.82);
    border: 1px solid rgba(255, 255, 255, 0.14);
    box-shadow: 0 18px 60px rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(18px);
  }
  h1 { margin: 0 0 6px; font-size: 20px; font-weight: 650; }
  p { margin: 6px 0; color: #b8b8c2; }
  #clock { font-variant-numeric: tabular-nums; color: #fff; font-size: 28px; margin: 10px 0; }
  .row { display: flex; gap: 10px; margin-top: 14px; }
  button {
    appearance: none; border: 0; border-radius: 10px; padding: 9px 14px;
    background: #e52e2e; color: #fff; font: inherit; font-weight: 600; cursor: pointer;
  }
  button.secondary { background: rgba(255, 255, 255, 0.12); }
  button:active { transform: translateY(1px); }
  #count { color: #ff8a8a; font-weight: 700; }
</style>
</head>
<body>
  <div class="card">
    <h1>M0 toolchain spike</h1>
    <p style="color:#ff9a9a">NOT the wallpaper implementation · retired</p>
    <p>CompositionController + DirectComposition + WorkerW proof</p>
    <div id="clock">--:--:--</div>
    <p>Clicked <span id="count">0</span> time(s). Native log: <code>spike.log</code></p>
    <div class="row">
      <button id="ping">Click me</button>
      <button id="quit" class="secondary">Quit spike</button>
    </div>
  </div>
<script>
  let count = 0;
  const clock = document.getElementById('clock');
  const counter = document.getElementById('count');
  document.getElementById('ping').addEventListener('click', () => {
    count += 1;
    counter.textContent = String(count);
    window.chrome.webview.postMessage({ type: 'ping', count, at: Date.now() });
  });
  document.getElementById('quit').addEventListener('click', () => {
    window.chrome.webview.postMessage({ type: 'quit' });
  });
  const tick = () => {
    clock.textContent = new Date().toLocaleTimeString();
    requestAnimationFrame(tick);
  };
  tick();
  window.chrome.webview.postMessage({ type: 'ready', url: location.href });
</script>
</body>
</html>
"#;
