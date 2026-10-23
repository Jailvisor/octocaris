#![allow(unsafe_op_in_unsafe_fn)]
//! Win32 shell for Octos: main window, tray icon, single-instance guard.
//!
//! All functions here are called from the UI thread. The WebView2 controller is
//! owned by `octos-app`, but stored here so the window procedure can resize it
//! without reaching back through a raw pointer.

use std::cell::RefCell;
use std::mem::size_of;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow};
use octos_core::prefs::WindowPlacement;
use tracing::{info, warn};
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2, ICoreWebView2Controller};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HLOCAL, HWND, LPARAM, LRESULT,
    LocalFree, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromRect,
    MonitorFromWindow,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Environment::GetCommandLineW;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{
    CommandLineToArgvW, DragAcceptFiles, DragFinish, DragQueryFileW, HDROP, NIF_ICON, NIF_MESSAGE,
    NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

pub mod desktop_icons;
pub mod startup;
pub mod wallpaper;

pub const MAIN_CLASS: PCWSTR = w!("OctosMainWindow");
pub const WM_APP_RESTORE: u32 = WM_APP + 1;
pub const WM_APP_TRAY: u32 = WM_APP + 2;

const TRAY_ID: u32 = 1;
const MENU_OPEN: usize = 1;
const MENU_QUIT: usize = 2;
const TRAY_TIP: &str = "Octos";

/// One-shot timer that opens DevTools after the wallpaper controllers exist.
const DEVTOOLS_TIMER_ID: usize = 3;
static DEVTOOLS_ATTEMPTS: AtomicU8 = AtomicU8::new(0);

#[derive(Default)]
struct WinState {
    hwnd: HWND,
    controller: Option<ICoreWebView2Controller>,
    webview: Option<ICoreWebView2>,
    tray_added: bool,
    taskbar_created: u32,
    quitting: bool,
    drop_handler: Option<Rc<dyn Fn(Vec<PathBuf>)>>,
    command_handler: Option<Rc<dyn Fn(&str)>>,
    placement_handler: Option<Rc<dyn Fn(WindowPlacement)>>,
}

thread_local! {
    static STATE: RefCell<WinState> = RefCell::new(WinState::default());
}

fn with_state<R>(f: impl FnOnce(&mut WinState) -> R) -> R {
    STATE.with(|state| f(&mut state.borrow_mut()))
}

pub struct InstanceGuard {
    handle: HANDLE,
    pub already_running: bool,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

pub fn acquire_single_instance() -> Result<InstanceGuard> {
    let handle = unsafe { CreateMutexW(None, true, w!("Global\\OctosMutex"))? };
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    Ok(InstanceGuard {
        handle,
        already_running,
    })
}

/// Register the handler that receives a command line forwarded by a second
/// instance. It runs on the UI thread, inside the window procedure.
pub fn set_command_handler(handler: impl Fn(&str) + 'static) {
    let _ = with_state(|state| state.command_handler = Some(Rc::new(handler)));
}

/// The process command line, including the executable path.
pub fn current_command_line() -> String {
    let line = unsafe { GetCommandLineW() };
    if line.is_null() {
        return String::new();
    }
    unsafe {
        String::from_utf16_lossy(std::slice::from_raw_parts(line.as_ptr(), line.len() + 1))
            .trim_end_matches('\0')
            .to_string()
    }
}

/// Split a Windows command line with the same rules as `CommandLineToArgvW`.
pub fn split_command_line(line: &str) -> Vec<String> {
    let wide: Vec<u16> = line.encode_utf16().chain(std::iter::once(0)).collect();
    let mut argc = 0i32;
    let argv = unsafe { CommandLineToArgvW(PCWSTR(wide.as_ptr()), &mut argc) };
    if argv.is_null() || argc <= 0 {
        return Vec::new();
    }
    let mut args = Vec::with_capacity(argc as usize);
    for index in 0..argc as usize {
        let value = unsafe { (*argv.add(index)).to_string().unwrap_or_default() };
        args.push(value);
    }
    unsafe {
        let _ = LocalFree(Some(HLOCAL(argv.cast())));
    }
    args
}

/// Send a command line to the running instance through `WM_COPYDATA`.
///
/// The old app sent the full command line and parsed it on the receiver. We do
/// the same so quoting and Unicode survive unchanged.
pub fn forward_command_line(line: &str) -> bool {
    let wide: Vec<u16> = line.encode_utf16().chain(std::iter::once(0)).collect();
    let data = COPYDATASTRUCT {
        dwData: 1,
        cbData: (wide.len() * size_of::<u16>()) as u32,
        lpData: wide.as_ptr() as *mut _,
    };

    // The first instance may still be creating its window right after launch.
    for _ in 0..20 {
        if let Ok(hwnd) = unsafe { FindWindowW(MAIN_CLASS, PCWSTR::null()) }
            && !hwnd.is_invalid()
        {
            unsafe {
                SendMessageW(
                    hwnd,
                    WM_COPYDATA,
                    Some(WPARAM(0)),
                    Some(LPARAM((&data as *const COPYDATASTRUCT) as isize)),
                );
            }
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Open DevTools on all wallpaper WebViews after their controllers are ready.
///
/// `octos run --dev-tools` and `octos dev-tools` both use this; the first
/// attempt runs one second after the command, and one retry covers slow
/// WebView2 startup.
pub fn schedule_open_dev_tools() {
    info!("opening DevTools after wallpaper controllers are ready");
    DEVTOOLS_ATTEMPTS.store(0, Ordering::Relaxed);
    let _ = unsafe { SetTimer(None, DEVTOOLS_TIMER_ID, 1000, Some(devtools_timer)) };
}

unsafe extern "system" fn devtools_timer(_hwnd: HWND, _message: u32, id: usize, _time: u32) {
    unsafe {
        let _ = KillTimer(None, id);
    }
    let opened = wallpaper::open_dev_tools_all();
    if opened > 0 {
        info!("opened DevTools on {opened} wallpaper(s)");
    } else if DEVTOOLS_ATTEMPTS.fetch_add(1, Ordering::Relaxed) == 0 {
        let _ = unsafe { SetTimer(None, id, 1000, Some(devtools_timer)) };
    }
}

/// Ask the already-running instance to show its main window.
pub fn restore_existing_instance() -> bool {
    unsafe {
        if let Ok(hwnd) = FindWindowW(MAIN_CLASS, PCWSTR::null())
            && !hwnd.is_invalid()
        {
            let _ = PostMessageW(Some(hwnd), WM_APP_RESTORE, WPARAM(0), LPARAM(0));
            return true;
        }
    }
    false
}

pub fn create_main_window(saved: Option<WindowPlacement>) -> Result<HWND> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let cursor = LoadCursorW(None, IDC_ARROW)?;
        let icon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();

        let window_class = WNDCLASSW {
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: windows::Win32::Foundation::HINSTANCE::from(hinstance),
            hIcon: icon,
            hCursor: cursor,
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: MAIN_CLASS,
        };

        let atom = RegisterClassW(&window_class);
        if atom == 0 {
            warn!("RegisterClassW returned 0; class may already be registered");
        }

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            MAIN_CLASS,
            w!("Octos"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1100,
            720,
            None,
            None,
            Some(windows::Win32::Foundation::HINSTANCE::from(hinstance)),
            None,
        )?;

        DragAcceptFiles(hwnd, true);

        let saved_rect = saved.and_then(|placement| {
            if placement.width < 320 || placement.height < 240 {
                return None;
            }
            let rect = RECT {
                left: placement.x,
                top: placement.y,
                right: placement.x + placement.width,
                bottom: placement.y + placement.height,
            };
            (!MonitorFromRect(&rect, MONITOR_DEFAULTTONULL).is_invalid()).then_some(rect)
        });

        if let Some(rect) = saved_rect {
            let _ = SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        } else {
            // `GetDpiForSystem` reports 96 for a per-monitor-aware (v2) process, so
            // derive the window size from the DPI of the monitor it was placed on
            // and clamp it to that monitor's work area.
            let dpi = GetDpiForWindow(hwnd).max(96);
            let desired_width = (1100 * dpi / 96) as i32;
            let desired_height = (720 * dpi / 96) as i32;
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut monitor_info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let (width, height, x, y) = if GetMonitorInfoW(monitor, &mut monitor_info).as_bool() {
                let work = monitor_info.rcWork;
                let max_width = (work.right - work.left).max(320);
                let max_height = (work.bottom - work.top).max(240);
                let width = desired_width.min(max_width - 48).max(320);
                let height = desired_height.min(max_height - 48).max(240);
                (
                    width,
                    height,
                    work.left + (max_width - width) / 2,
                    work.top + (max_height - height) / 2,
                )
            } else {
                (desired_width, desired_height, CW_USEDEFAULT, CW_USEDEFAULT)
            };
            let _ = SetWindowPos(
                hwnd,
                None,
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }

        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        with_state(|state| {
            state.hwnd = hwnd;
            state.taskbar_created = taskbar_created;
        });

        let _ = ShowWindow(hwnd, SW_SHOW);
        info!("main window created: {hwnd:?}");
        Ok(hwnd)
    }
}

/// Receive the main window's normal (non-maximized) rectangle as it moves or
/// resizes. The app stores it in preferences and writes it on exit.
pub fn set_window_placement_handler(handler: impl Fn(WindowPlacement) + 'static) {
    let _ = with_state(|state| state.placement_handler = Some(Rc::new(handler)));
}

fn report_window_placement(hwnd: HWND) {
    let mut placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_err() {
        return;
    }
    let rect = placement.rcNormalPosition;
    let placement = WindowPlacement {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    };
    if placement.width < 320 || placement.height < 240 {
        return;
    }
    let handler = with_state(|state| state.placement_handler.clone());
    if let Some(handler) = handler {
        handler(placement);
    }
}

pub fn set_drop_handler(handler: impl Fn(Vec<PathBuf>) + 'static) {
    with_state(|state| state.drop_handler = Some(Rc::new(handler)));
}

pub fn post_to_main_webview(payload: &str) {
    let webview = with_state(|state| state.webview.clone());
    if let Some(webview) = webview
        && let Err(err) = octos_webview::post_json(&webview, payload)
    {
        warn!("could not post to main webview: {err:?}");
    }
}

pub fn set_webview(controller: ICoreWebView2Controller, webview: ICoreWebView2) {
    with_state(|state| {
        state.controller = Some(controller);
        state.webview = Some(webview);
    });
}

pub fn resize_webview(hwnd: HWND) {
    let controller = with_state(|state| state.controller.clone());
    if let Some(controller) = controller
        && let Err(err) = octos_webview::resize(&controller, hwnd)
    {
        warn!("could not resize WebView2: {err:?}");
    }
}

pub fn open_devtools() {
    let webview = with_state(|state| state.webview.clone());
    if let Some(webview) = webview
        && let Err(err) = octos_webview::open_devtools(&webview)
    {
        warn!("could not open DevTools: {err:?}");
    }
}

pub fn restore_main_window() {
    let hwnd = with_state(|state| state.hwnd);
    let hwnd = if !hwnd.is_invalid() && unsafe { IsWindow(Some(hwnd)).as_bool() } {
        Some(hwnd)
    } else {
        unsafe { FindWindowW(MAIN_CLASS, PCWSTR::null()).ok() }
    };

    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

pub fn add_tray_icon(hwnd: HWND) -> Result<()> {
    let data = tray_icon_data(hwnd)?;
    unsafe {
        if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
            return Err(anyhow!("Shell_NotifyIconW(NIM_ADD) failed"));
        }
    }
    with_state(|state| state.tray_added = true);
    info!("tray icon added");
    Ok(())
}

pub fn remove_tray_icon(hwnd: HWND) {
    let added = with_state(|state| state.tray_added);
    if !added {
        return;
    }
    if let Ok(data) = tray_icon_data(hwnd) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
    with_state(|state| state.tray_added = false);
    info!("tray icon removed");
}

pub fn run_message_loop() {
    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn tray_icon_data(hwnd: HWND) -> Result<NOTIFYICONDATAW> {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default(),
        ..Default::default()
    };

    let tip: Vec<u16> = TRAY_TIP.encode_utf16().collect();
    let count = tip.len().min(data.szTip.len() - 1);
    data.szTip[..count].copy_from_slice(&tip[..count]);
    Ok(data)
}

fn show_tray_menu(hwnd: HWND) {
    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(err) => {
                warn!("CreatePopupMenu failed: {err:?}");
                return;
            }
        };

        let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN, w!("Open Octos"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("Quit"));

        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(hwnd);

        let command = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            point.x,
            point.y,
            Some(0),
            hwnd,
            None,
        )
        .0 as usize;

        let _ = DestroyMenu(menu);

        match command {
            MENU_OPEN => restore_main_window(),
            MENU_QUIT => quit(hwnd),
            _ => {}
        }
    }
}

fn quit(hwnd: HWND) {
    with_state(|state| state.quitting = true);
    remove_tray_icon(hwnd);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

unsafe fn read_dropped_paths(drop: HDROP) -> Vec<PathBuf> {
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    let mut paths = Vec::new();
    for index in 0..count {
        let length = unsafe { DragQueryFileW(drop, index, None) };
        if length == 0 {
            continue;
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let written = unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) };
        if written == 0 {
            continue;
        }
        buffer.truncate(written as usize);
        paths.push(PathBuf::from(String::from_utf16_lossy(&buffer)));
    }
    unsafe { DragFinish(drop) };
    paths
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_SIZE => {
            resize_webview(hwnd);
            report_window_placement(hwnd);
            LRESULT(0)
        }
        WM_MOVE => {
            report_window_placement(hwnd);
            LRESULT(0)
        }
        WM_CLOSE => {
            // Close to tray; Quit is a separate tray-menu action.
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            info!("main window destroyed");
            remove_tray_icon(hwnd);
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_APP_RESTORE => {
            restore_main_window();
            LRESULT(0)
        }
        WM_COPYDATA => {
            if lparam.0 != 0 {
                let data = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
                if data.dwData == 1 && !data.lpData.is_null() && data.cbData >= 2 {
                    let words = data.cbData as usize / 2;
                    let wide =
                        unsafe { std::slice::from_raw_parts(data.lpData as *const u16, words) };
                    let length = wide
                        .iter()
                        .position(|&unit| unit == 0)
                        .unwrap_or(wide.len());
                    let line = String::from_utf16_lossy(&wide[..length]);
                    info!("received command line from another instance: {line}");
                    let handler = with_state(|state| state.command_handler.clone());
                    if let Some(handler) = handler {
                        handler(&line);
                    }
                }
            }
            // WM_COPYDATA must answer TRUE once the data has been consumed.
            LRESULT(1)
        }
        WM_APP_TRAY => {
            let event = (lparam.0 & 0xFFFF) as u32;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                show_tray_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            wallpaper::reconcile_now();
            LRESULT(0)
        }
        WM_DROPFILES => {
            let paths = read_dropped_paths(HDROP(wparam.0 as *mut core::ffi::c_void));
            if !paths.is_empty() {
                info!("dropped paths: {paths:?}");
                let handler = with_state(|state| state.drop_handler.clone());
                if let Some(handler) = handler {
                    handler(paths);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xFFFF;
            match id {
                MENU_OPEN => restore_main_window(),
                MENU_QUIT => quit(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        _ => {
            let taskbar_created = with_state(|state| state.taskbar_created);
            if taskbar_created != 0 && message == taskbar_created {
                info!("TaskbarCreated received; re-adding tray icon");
                with_state(|state| state.tray_added = false);
                if let Err(err) = add_tray_icon(hwnd) {
                    warn!("could not re-add tray icon: {err:?}");
                }
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}
