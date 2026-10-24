//! WebView2 hosting for the main Octos window.
//!
//! The wallpaper path uses a composition controller and lives in the spike /
//! later M2 code; this crate starts with the normal controller used by the app
//! window. The environment is shared so both paths can reuse the same browser
//! process and user data directory.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use octos_protocol::{UiRequest, UiResponse};
use tracing::{debug, info, warn};
use webview2_com::{
    AddScriptToExecuteOnDocumentCreatedCompletedHandler, CoreWebView2EnvironmentOptions,
    CreateCoreWebView2CompositionControllerCompletedHandler,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler, ProcessFailedEventHandler,
    WebMessageReceivedEventHandler, take_pwstr,
};
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{BOOL, IUnknown, Interface, PCWSTR, PWSTR, w};

pub struct MainWebView {
    pub environment: ICoreWebView2Environment3,
    pub controller: ICoreWebView2Controller,
    pub webview: ICoreWebView2,
}

/// Create the shared WebView2 environment asynchronously.
///
/// The callback runs on the UI thread after the message loop has processed the
/// WebView2 completion message; it must not block.
pub fn create_environment(
    user_data_dir: &Path,
    enable_gpu: bool,
    on_ready: impl FnOnce(Result<ICoreWebView2Environment3>) + 'static,
) -> Result<()> {
    let user_data = wide_path(user_data_dir);

    let mut arguments = String::from(
        "--disable-features=TranslateUI,AudioServiceOutOfProcess,LowIntegrityMode \
         --no-first-run --no-default-browser-check --disable-breakpad \
         --disable-component-update --disable-background-networking \
         --autoplay-policy=no-user-gesture-required",
    );
    if !enable_gpu {
        arguments.push_str(" --disable-gpu");
    }
    info!("WebView2 environment: gpu={enable_gpu}");
    let arguments: Vec<u16> = arguments.encode_utf16().chain(std::iter::once(0)).collect();

    let options: ICoreWebView2EnvironmentOptions = CoreWebView2EnvironmentOptions::default().into();
    unsafe {
        options.SetAdditionalBrowserArguments(PCWSTR::from_raw(arguments.as_ptr()))?;
    }

    let mut on_ready = Some(on_ready);
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |result, environment| {
            let outcome = (|| -> Result<ICoreWebView2Environment3> {
                result?;
                let environment =
                    environment.ok_or_else(|| anyhow!("WebView2 returned no environment"))?;
                Ok(environment.cast::<ICoreWebView2Environment3>()?)
            })();

            if let Some(callback) = on_ready.take() {
                callback(outcome);
            }
            Ok(())
        },
    ));

    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR::from_raw(user_data.as_ptr()),
            &options,
            &handler,
        )?;
    }

    Ok(())
}

pub fn create_main_controller(
    environment: &ICoreWebView2Environment3,
    hwnd: HWND,
    app_dir: PathBuf,
    wallpapers_dir: PathBuf,
    startup_url: String,
    on_ready: impl FnOnce(Result<(ICoreWebView2Controller, ICoreWebView2)>) + 'static,
) -> Result<()> {
    let app_dir_wide = wide_path(&app_dir);
    let wallpapers_wide = wide_path(&wallpapers_dir);
    let url_wide = wide(&startup_url);

    let mut on_ready = Some(on_ready);
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
        move |result, controller| {
            let outcome = (|| -> Result<(ICoreWebView2Controller, ICoreWebView2)> {
                result?;
                let controller =
                    controller.ok_or_else(|| anyhow!("WebView2 returned no controller"))?;
                let webview = unsafe { controller.CoreWebView2()? };

                unsafe {
                    let webview3: ICoreWebView2_3 = webview.cast()?;
                    if app_dir.exists() {
                        webview3.SetVirtualHostNameToFolderMapping(
                            w!("app.octos"),
                            PCWSTR::from_raw(app_dir_wide.as_ptr()),
                            COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW,
                        )?;
                    } else {
                        warn!(
                            "app directory {} does not exist; app.octos mapping skipped",
                            app_dir.display()
                        );
                    }

                    webview3.SetVirtualHostNameToFolderMapping(
                        w!("local.octos"),
                        PCWSTR::from_raw(wallpapers_wide.as_ptr()),
                        COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW,
                    )?;

                    let settings = webview.Settings()?;
                    settings.SetAreDefaultContextMenusEnabled(false)?;
                    settings.SetAreDevToolsEnabled(cfg!(debug_assertions))?;
                    settings.SetIsStatusBarEnabled(false)?;
                    settings.SetIsZoomControlEnabled(false)?;
                    settings.SetAreHostObjectsAllowed(false)?;

                    if let Ok(controller2) = controller.cast::<ICoreWebView2Controller2>() {
                        let _ = controller2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
                            A: 255,
                            R: 18,
                            G: 18,
                            B: 22,
                        });
                    }

                    let mut client = RECT::default();
                    GetClientRect(hwnd, &mut client)?;
                    controller.SetBounds(client)?;
                    controller.SetIsVisible(true)?;
                    webview.Navigate(PCWSTR::from_raw(url_wide.as_ptr()))?;

                    let failed_handler =
                        ProcessFailedEventHandler::create(Box::new(move |sender, args| {
                            let kind = args.as_ref().map(process_failed_kind);
                            warn!("main WebView2 process failed ({kind:?}); reloading");
                            if let Some(sender) = sender
                                && let Err(err) = sender.Reload()
                            {
                                warn!("could not reload the main WebView: {err:?}");
                            }
                            Ok(())
                        }));
                    let mut failed_token = 0i64;
                    webview.add_ProcessFailed(&failed_handler, &mut failed_token)?;
                }

                info!("main WebView2 controller ready -> {startup_url}");
                Ok((controller, webview))
            })();

            if let Some(callback) = on_ready.take() {
                callback(outcome);
            }
            Ok(())
        },
    ));

    unsafe {
        environment.CreateCoreWebView2Controller(hwnd, &handler)?;
    }
    Ok(())
}

pub fn resize(controller: &ICoreWebView2Controller, hwnd: HWND) -> Result<()> {
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client)?;
        controller.SetBounds(client)?;
    }
    Ok(())
}

pub fn add_ui_message_handler<F, G>(
    webview: &ICoreWebView2,
    mut on_request: F,
    mut on_dropped_files: G,
) where
    F: FnMut(UiRequest) -> Option<UiResponse> + 'static,
    G: FnMut(Vec<PathBuf>) + 'static,
{
    let sender = webview.clone();
    let handler = WebMessageReceivedEventHandler::create(Box::new(move |_source, args| {
        if let Some(args) = args {
            // HTML drag/drop sends picked Files as additional objects; recover
            // their local paths through ICoreWebView2File.
            if let Ok(args2) = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>()
                && let Ok(objects) = unsafe { args2.AdditionalObjects() }
            {
                let mut count = 0u32;
                if unsafe { objects.Count(&mut count) }.is_ok() && count > 0 {
                    let mut files = Vec::new();
                    for index in 0..count {
                        if let Ok(value) = unsafe { objects.GetValueAtIndex(index) }
                            && let Ok(file) = value.cast::<ICoreWebView2File>()
                        {
                            let mut path = PWSTR::null();
                            if unsafe { file.Path(&mut path) }.is_ok() {
                                let text = take_pwstr(path);
                                if !text.is_empty() {
                                    files.push(PathBuf::from(text));
                                }
                            }
                        }
                    }
                    if !files.is_empty() {
                        info!("drag-drop -> host: {files:?}");
                        on_dropped_files(files);
                        return Ok(());
                    }
                }
            }

            let mut json = PWSTR::null();
            unsafe { args.WebMessageAsJson(&mut json)? };
            let text = take_pwstr(json);
            info!("ui -> host: {text}");

            match serde_json::from_str::<UiRequest>(&text) {
                Ok(request) => {
                    if let Some(response) = on_request(request) {
                        match serde_json::to_string(&response) {
                            Ok(payload) => {
                                let wide = wide(&payload);
                                unsafe {
                                    sender.PostWebMessageAsJson(PCWSTR::from_raw(wide.as_ptr()))?;
                                }
                            }
                            Err(err) => warn!("could not serialize UI response: {err}"),
                        }
                    }
                }
                Err(err) => warn!("ignoring invalid UI message: {err}"),
            }
        }
        Ok(())
    }));

    let mut token = 0i64;
    unsafe {
        if let Err(err) = webview.add_WebMessageReceived(&handler, &mut token) {
            warn!("could not subscribe to UI messages: {err:?}");
        }
    }
}

pub fn navigate(webview: &ICoreWebView2, url: &str) -> Result<()> {
    let url = wide(url);
    unsafe {
        webview.Navigate(PCWSTR::from_raw(url.as_ptr()))?;
    }
    Ok(())
}

pub fn open_devtools(webview: &ICoreWebView2) -> Result<()> {
    unsafe {
        webview.OpenDevToolsWindow()?;
    }
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn process_failed_kind(
    args: &ICoreWebView2ProcessFailedEventArgs,
) -> COREWEBVIEW2_PROCESS_FAILED_KIND {
    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND_UNKNOWN_PROCESS_EXITED;
    unsafe {
        let _ = args.ProcessFailedKind(&mut kind);
    }
    kind
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

// ---------------------------------------------------------------------------
// Composition (wallpaper) hosting
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CompositionDevice {
    device: IDCompositionDevice,
}

impl CompositionDevice {
    pub fn new() -> Result<Self> {
        unsafe {
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

            let d3d_device =
                d3d_device.ok_or_else(|| anyhow!("D3D11CreateDevice returned no device"))?;
            let dxgi_device: IDXGIDevice = d3d_device.cast()?;
            let device: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;
            Ok(Self { device })
        }
    }
}

pub struct WallpaperWebView {
    pub controller: ICoreWebView2Controller,
    pub composition_controller: ICoreWebView2CompositionController,
    pub webview: ICoreWebView2,
    pub target: IDCompositionTarget,
    pub visual: IDCompositionVisual,
    dcomp: IDCompositionDevice,
}

impl WallpaperWebView {
    pub fn resize(&self, hwnd: HWND) -> Result<()> {
        let mut client = RECT::default();
        unsafe {
            GetClientRect(hwnd, &mut client)?;
            self.controller.SetBounds(client)?;
            self.dcomp.Commit()?;
        }
        Ok(())
    }

    pub fn set_visible(&self, visible: bool) -> Result<()> {
        unsafe {
            self.controller.SetIsVisible(visible)?;
            self.dcomp.Commit()?;
        }
        Ok(())
    }

    pub fn close(&self) {
        unsafe {
            let _ = self.controller.Close();
        }
    }

    pub fn is_visible(&self) -> bool {
        let mut visible = BOOL::from(false);
        unsafe {
            let _ = self.controller.IsVisible(&mut visible);
        }
        visible.as_bool()
    }
}

pub fn post_json(webview: &ICoreWebView2, payload: &str) -> Result<()> {
    let payload = wide(payload);
    unsafe {
        webview.PostWebMessageAsJson(PCWSTR::from_raw(payload.as_ptr()))?;
    }
    Ok(())
}

pub fn execute_script(webview: &ICoreWebView2, script: &str) -> Result<()> {
    let script = wide(script);
    unsafe {
        webview.ExecuteScript(PCWSTR::from_raw(script.as_ptr()), None)?;
    }
    Ok(())
}

impl CompositionDevice {
    #[allow(clippy::too_many_arguments)]
    pub fn create_wallpaper_controller(
        &self,
        environment: &ICoreWebView2Environment3,
        hwnd: HWND,
        wallpapers_dir: PathBuf,
        entry_url: String,
        monitor_id: String,
        fps_cap: u32,
        muted: bool,
        mut on_message: impl FnMut(&ICoreWebView2, &str) + 'static,
        mut on_loaded: impl FnMut(&ICoreWebView2) + 'static,
        mut on_process_failed: impl FnMut(&ICoreWebView2, COREWEBVIEW2_PROCESS_FAILED_KIND) + 'static,
        on_ready: impl FnOnce(Result<WallpaperWebView>) + 'static,
    ) -> Result<()> {
        let dcomp = self.device.clone();
        let wallpapers_wide = wide_path(&wallpapers_dir);
        let url_wide = wide(&entry_url);
        let mut on_ready = Some(on_ready);

        let handler = CreateCoreWebView2CompositionControllerCompletedHandler::create(Box::new(
            move |result, controller| {
                let outcome = (|| -> Result<WallpaperWebView> {
                    result?;
                    let composition_controller = controller
                        .ok_or_else(|| anyhow!("WebView2 returned no composition controller"))?;
                    let controller: ICoreWebView2Controller = composition_controller.cast()?;
                    let webview = unsafe { controller.CoreWebView2()? };

                    let target = unsafe { dcomp.CreateTargetForHwnd(hwnd, false)? };
                    let visual = unsafe { dcomp.CreateVisual()? };
                    unsafe { target.SetRoot(&visual)? };
                    let visual_unknown: IUnknown = visual.cast()?;
                    unsafe { composition_controller.SetRootVisualTarget(&visual_unknown)? };

                    unsafe {
                        let webview3: ICoreWebView2_3 = webview.cast()?;
                        webview3.SetVirtualHostNameToFolderMapping(
                            w!("local.octos"),
                            PCWSTR::from_raw(wallpapers_wide.as_ptr()),
                            COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW,
                        )?;

                        let settings = webview.Settings()?;
                        settings.SetAreDefaultContextMenusEnabled(false)?;
                        settings.SetAreDevToolsEnabled(cfg!(debug_assertions))?;
                        settings.SetIsStatusBarEnabled(false)?;
                        settings.SetIsZoomControlEnabled(false)?;
                        settings.SetAreHostObjectsAllowed(false)?;

                        if let Ok(controller2) = controller.cast::<ICoreWebView2Controller2>() {
                            let _ = controller2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
                                A: 0,
                                R: 0,
                                G: 0,
                                B: 0,
                            });
                        }

                        let mut client = RECT::default();
                        GetClientRect(hwnd, &mut client)?;
                        controller.SetBounds(client)?;
                        controller.SetIsVisible(true)?;

                        // Give wallpapers a CSS viewport equal to the physical
                        // pixel count (Wallpaper Engine does the same). Most web
                        // wallpapers size their canvas from innerWidth/innerHeight
                        // and ignore devicePixelRatio, so at 200% scaling they
                        // would otherwise render at half resolution and look soft.
                        let dpi = GetDpiForWindow(hwnd);
                        let zoom = if dpi > 96 { 96.0 / f64::from(dpi) } else { 1.0 };
                        let _ = controller.SetZoomFactor(zoom);
                        info!("wallpaper DPI {dpi} -> page zoom {zoom}");

                        // Isolate local/session storage per monitor before the
                        // page runs, so wallpapers with their own settings panel
                        // can keep different scale/position values per display.
                        let scope = octos_core::storage_scope::script(&monitor_id);
                        let scope_wide = wide(&scope);
                        let scope_handler =
                            AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                                |result, script_id| {
                                    match result {
                                        Ok(()) => {
                                            debug!("storage scope script installed: {script_id}")
                                        }
                                        Err(err) => {
                                            warn!("could not install storage scope script: {err:?}")
                                        }
                                    }
                                    Ok(())
                                },
                            ));
                        webview.AddScriptToExecuteOnDocumentCreated(
                            PCWSTR::from_raw(scope_wide.as_ptr()),
                            &scope_handler,
                        )?;

                        let frame_script = octos_core::frame_cap::script(fps_cap);
                        let frame_wide = wide(&frame_script);
                        let frame_handler =
                            AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                                |result, script_id| {
                                    match result {
                                        Ok(()) => {
                                            debug!("frame cap script installed: {script_id}")
                                        }
                                        Err(err) => {
                                            warn!("could not install frame cap script: {err:?}")
                                        }
                                    }
                                    Ok(())
                                },
                            ));
                        webview.AddScriptToExecuteOnDocumentCreated(
                            PCWSTR::from_raw(frame_wide.as_ptr()),
                            &frame_handler,
                        )?;
                        // Mute before navigating so a wallpaper that autoplays
                        // audio never gets a chance to make noise.
                        if muted && let Ok(muted_webview) = webview.cast::<ICoreWebView2_8>() {
                            let _ = muted_webview.SetIsMuted(true);
                            let mut value = BOOL::from(false);
                            let applied =
                                muted_webview.IsMuted(&mut value).is_ok() && value.as_bool();
                            debug!("wallpaper audio muted at creation: {applied}");
                        }
                        webview.Navigate(PCWSTR::from_raw(url_wide.as_ptr()))?;

                        let message_handler = WebMessageReceivedEventHandler::create(Box::new(
                            move |sender, args| {
                                if let Some(args) = args {
                                    let mut json = PWSTR::null();
                                    args.WebMessageAsJson(&mut json)?;
                                    let text = take_pwstr(json);
                                    debug!("wallpaper page: {text}");
                                    if let Some(sender) = sender {
                                        on_message(&sender, &text);
                                    }
                                }
                                Ok(())
                            },
                        ));
                        let mut token = 0i64;
                        webview.add_WebMessageReceived(&message_handler, &mut token)?;

                        let loaded_handler = NavigationCompletedEventHandler::create(Box::new(
                            move |sender, _args| {
                                if let Some(sender) = sender {
                                    on_loaded(&sender);
                                }
                                Ok(())
                            },
                        ));
                        let mut loaded_token = 0i64;
                        webview.add_NavigationCompleted(&loaded_handler, &mut loaded_token)?;

                        let failed_handler =
                            ProcessFailedEventHandler::create(Box::new(move |sender, args| {
                                let kind = args.as_ref().map(process_failed_kind).unwrap_or(
                                    COREWEBVIEW2_PROCESS_FAILED_KIND_UNKNOWN_PROCESS_EXITED,
                                );
                                if let Some(sender) = sender {
                                    on_process_failed(&sender, kind);
                                }
                                Ok(())
                            }));
                        let mut failed_token = 0i64;
                        webview.add_ProcessFailed(&failed_handler, &mut failed_token)?;
                        dcomp.Commit()?;
                    }

                    info!("wallpaper controller ready -> {entry_url}");
                    Ok(WallpaperWebView {
                        controller,
                        composition_controller,
                        webview,
                        target,
                        visual,
                        dcomp: dcomp.clone(),
                    })
                })();

                if let Some(callback) = on_ready.take() {
                    callback(outcome);
                }
                Ok(())
            },
        ));

        unsafe {
            environment.CreateCoreWebView2CompositionController(hwnd, &handler)?;
        }
        Ok(())
    }
}
