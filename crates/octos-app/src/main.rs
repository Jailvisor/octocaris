#![windows_subsystem = "windows"]

use std::path::{Path as FsPath, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, anyhow};
use octos_core::catalog;
use octos_core::cli::{self, Command};
use octos_core::manifest::ModManifest;
use octos_core::options::OptionsMap;
use octos_core::prefs::{Preferences, WindowPlacement};
use octos_protocol::{PROTOCOL_VERSION, UiRequest, UiResponse};
use octos_storage::Paths;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW,
};
use windows::core::{PCWSTR, w};

static PREFERENCES: Mutex<Option<Preferences>> = Mutex::new(None);

/// `octos run`/`reload`/`dev-tools` need the wallpaper slots; commands that
/// arrive before WebView2 finishes initializing wait here.
static WALLPAPERS_READY: AtomicBool = AtomicBool::new(false);
static PENDING_CLI: Mutex<Vec<Command>> = Mutex::new(Vec::new());

struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn main() {
    let paths = match Paths::discover() {
        Ok(paths) => paths,
        Err(err) => {
            show_error(&format!(
                "Could not initialize Octos data directories:\n{err}"
            ));
            return;
        }
    };

    init_logging(&paths);
    info!("starting {}", octos_core::build_info());
    info!("data root: {}", paths.root.display());

    match run(paths) {
        Ok(()) => info!("octos exited cleanly"),
        Err(err) => {
            error!("fatal: {err:?}");
            show_error(&format!("Octos failed:\n{err:?}"));
        }
    }
}

fn run(paths: Paths) -> Result<()> {
    let instance = octos_win::acquire_single_instance()?;
    if instance.already_running {
        let command_line = octos_win::current_command_line();
        let args = octos_win::split_command_line(&command_line);
        if args.len() <= 1 {
            info!("another Octos instance is running; asking it to show its window");
            if !octos_win::restore_existing_instance() {
                warn!("existing instance was not found by window class");
            }
        } else {
            info!("forwarding command line to the running instance");
            if !octos_win::forward_command_line(&command_line) {
                warn!("could not reach the running instance");
            }
        }
        return Ok(());
    }

    let args = octos_win::split_command_line(&octos_win::current_command_line());
    let startup_command = cli::parse(args.get(1..).unwrap_or(&[]));

    let preferences = octos_storage::load_preferences(&paths.preferences);
    let saved_window: Option<WindowPlacement> = preferences.window;
    info!(
        "preferences loaded: {} monitor selection(s)",
        preferences.selected.len()
    );
    *PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(preferences.clone());

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr.is_err() {
        return Err(anyhow!("CoInitializeEx failed: {hr:?}"));
    }
    let _com = ComGuard;

    let hwnd = octos_win::create_main_window(saved_window)?;
    octos_win::add_tray_icon(hwnd).context("could not add tray icon")?;

    octos_win::set_window_placement_handler(|placement| {
        if let Some(preferences) = PREFERENCES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_mut()
        {
            preferences.window = Some(placement);
        }
    });

    let command_paths = paths.clone();
    octos_win::set_command_handler(move |line| {
        let args = octos_win::split_command_line(line);
        if let Some(command) = cli::parse(args.get(1..).unwrap_or(&[])) {
            dispatch_cli(command, &command_paths);
        }
    });
    if let Some(command) = startup_command {
        dispatch_cli(command, &paths);
    }

    let app_dir = paths.app_dir();
    let wallpapers_dir = paths.wallpapers.clone();
    let wallpaper_folder =
        std::env::var("OCTOS_TEST_MOD_FOLDER").unwrap_or_else(|_| "gradient".to_string());
    let wallpaper_key = paths
        .wallpapers
        .join(&wallpaper_folder)
        .to_string_lossy()
        .to_string();
    let wallpaper_entry = format!("https://local.octos/{wallpaper_folder}/index.html");
    let wallpaper_options = ModManifest::load(paths.wallpapers.join(&wallpaper_folder))
        .map(|manifest| manifest.options)
        .unwrap_or_default();
    let library_paths = paths.clone();
    let enable_gpu = preferences.app_options.enable_gpu;
    let startup_preferences = preferences;
    {
        let drop_paths = library_paths.clone();
        octos_win::set_drop_handler(move |dropped| install_dropped(dropped, &drop_paths));
    }
    let user_data_dir = paths.webview_data.clone();
    let startup_url = std::env::var("OCTOS_UI_DEV_URL")
        .unwrap_or_else(|_| "https://app.octos/index.html".to_string());

    if !app_dir.exists() {
        warn!(
            "UI directory {} does not exist yet; run `cargo xtask deploy` or set OCTOS_UI_DEV_URL",
            app_dir.display()
        );
    }

    let flush_paths = paths.clone();
    octos_webview::create_environment(&user_data_dir, enable_gpu, move |result| match result {
        Ok(environment) => {
            let app_dir = app_dir.clone();
            let wallpapers_dir = wallpapers_dir.clone();
            let startup_url = startup_url.clone();
            let library_paths = library_paths.clone();
            let push_library_paths = library_paths.clone();

            if let Err(err) = octos_webview::create_main_controller(
                &environment,
                hwnd,
                app_dir,
                wallpapers_dir,
                startup_url,
                move |result| match result {
                    Ok((controller, webview)) => {
                        octos_win::set_webview(controller, webview.clone());
                        let drop_library_paths = library_paths.clone();
                        octos_webview::add_ui_message_handler(
                            &webview,
                            move |request| match request {
                                UiRequest::Ping { id } => Some(UiResponse::Pong {
                                    id,
                                    version: PROTOCOL_VERSION.to_string(),
                                }),
                                UiRequest::ShowWallpaper => {
                                    let url = "https://local.octos/gradient/index.html";
                                    match octos_win::wallpaper::show_all(url) {
                                        Ok(()) => Some(UiResponse::WallpaperState { active: true }),
                                        Err(err) => {
                                            error!("could not show wallpaper: {err:?}");
                                            Some(UiResponse::Error {
                                                message: format!("{err:?}"),
                                            })
                                        }
                                    }
                                }
                                UiRequest::ClearWallpaper => {
                                    octos_win::wallpaper::clear_all();
                                    Some(UiResponse::WallpaperState { active: false })
                                }
                                UiRequest::GetDesktopIcons => Some(UiResponse::DesktopIcons {
                                    visible: octos_win::desktop_icons::is_visible(),
                                }),
                                UiRequest::SetDesktopIcons { visible } => {
                                    match octos_win::desktop_icons::set_visible(visible) {
                                        Ok(()) => Some(UiResponse::DesktopIcons { visible }),
                                        Err(err) => {
                                            error!(
                                                "could not set desktop icon visibility: {err:?}"
                                            );
                                            Some(UiResponse::Error {
                                                message: format!("{err:?}"),
                                            })
                                        }
                                    }
                                }
                                UiRequest::GetSettings => Some(UiResponse::Settings {
                                    data: settings_snapshot(),
                                }),
                                UiRequest::SetSetting { key, value } => {
                                    match apply_setting(&library_paths, &key, &value) {
                                        Ok(()) => Some(UiResponse::Settings {
                                            data: settings_snapshot(),
                                        }),
                                        Err(err) => {
                                            error!("could not apply setting {key}: {err:?}");
                                            Some(UiResponse::Error {
                                                message: format!("{err:?}"),
                                            })
                                        }
                                    }
                                }
                                UiRequest::GetLibrary => match library_snapshot(&library_paths) {
                                    Ok(data) => Some(UiResponse::Library { data }),
                                    Err(err) => {
                                        error!("could not scan library: {err:?}");
                                        Some(UiResponse::Error {
                                            message: format!("{err:?}"),
                                        })
                                    }
                                },
                                UiRequest::SetMonitorMod {
                                    monitor_id,
                                    folder_path,
                                } => {
                                    match set_monitor_mod(&library_paths, &monitor_id, &folder_path)
                                    {
                                        Ok(()) => {
                                            save_preferences(&library_paths);
                                            match library_snapshot(&library_paths) {
                                                Ok(data) => Some(UiResponse::Library { data }),
                                                Err(err) => Some(UiResponse::Error {
                                                    message: format!("{err:?}"),
                                                }),
                                            }
                                        }
                                        Err(err) => {
                                            error!("could not set monitor mod: {err:?}");
                                            Some(UiResponse::Error {
                                                message: format!("{err:?}"),
                                            })
                                        }
                                    }
                                }
                                UiRequest::SetOption {
                                    folder_path,
                                    id,
                                    value,
                                } => {
                                    if let Ok(manifest) =
                                        ModManifest::load(FsPath::new(&folder_path))
                                    {
                                        let stored = PREFERENCES
                                            .lock()
                                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                                            .as_ref()
                                            .and_then(|prefs| {
                                                prefs.options_for(&folder_path).cloned()
                                            });
                                        octos_win::wallpaper::ensure_mod_options(
                                            folder_path.clone(),
                                            manifest.options,
                                            stored,
                                        );
                                    }
                                    if octos_win::wallpaper::apply_option_from_ui(
                                        &folder_path,
                                        &id,
                                        value,
                                    ) {
                                        save_preferences(&library_paths);
                                    }
                                    match library_snapshot(&library_paths) {
                                        Ok(data) => Some(UiResponse::Library { data }),
                                        Err(err) => Some(UiResponse::Error {
                                            message: format!("{err:?}"),
                                        }),
                                    }
                                }
                                UiRequest::InstallMod => {
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("Octos mod", &["zip"])
                                        .pick_file()
                                    {
                                        match octos_storage::install_zip(
                                            &path,
                                            &library_paths.wallpapers,
                                        ) {
                                            Ok(_) => match library_snapshot(&library_paths) {
                                                Ok(data) => Some(UiResponse::Library { data }),
                                                Err(err) => Some(UiResponse::Error {
                                                    message: format!("{err:?}"),
                                                }),
                                            },
                                            Err(err) => {
                                                error!("could not install mod: {err:?}");
                                                Some(UiResponse::Error {
                                                    message: format!("{err:?}"),
                                                })
                                            }
                                        }
                                    } else {
                                        match library_snapshot(&library_paths) {
                                            Ok(data) => Some(UiResponse::Library { data }),
                                            Err(err) => Some(UiResponse::Error {
                                                message: format!("{err:?}"),
                                            }),
                                        }
                                    }
                                }
                                UiRequest::InstallFolder => {
                                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                        match octos_storage::install_folder(
                                            &path,
                                            &library_paths.wallpapers,
                                        ) {
                                            Ok(installed) => {
                                                info!("imported folder {}", installed.display());
                                                match library_snapshot(&library_paths) {
                                                    Ok(data) => Some(UiResponse::Library { data }),
                                                    Err(err) => Some(UiResponse::Error {
                                                        message: format!("{err:?}"),
                                                    }),
                                                }
                                            }
                                            Err(err) => {
                                                error!("could not import folder: {err:?}");
                                                Some(UiResponse::Error {
                                                    message: format!("{err:?}"),
                                                })
                                            }
                                        }
                                    } else {
                                        match library_snapshot(&library_paths) {
                                            Ok(data) => Some(UiResponse::Library { data }),
                                            Err(err) => Some(UiResponse::Error {
                                                message: format!("{err:?}"),
                                            }),
                                        }
                                    }
                                }
                                UiRequest::RemoveMod { folder_path } => {
                                    if FsPath::new(&folder_path)
                                        == library_paths.wallpapers.join("gradient")
                                    {
                                        return Some(UiResponse::Error {
                                            message:
                                                "the built-in Gradient wallpaper cannot be removed"
                                                    .to_string(),
                                        });
                                    }
                                    let affected: Vec<String> =
                                        octos_win::wallpaper::selected_monitors()
                                            .into_iter()
                                            .filter(|(_, selected)| selected == &folder_path)
                                            .map(|(monitor, _)| monitor)
                                            .collect();
                                    match octos_storage::remove_mod(
                                        &library_paths.wallpapers,
                                        FsPath::new(&folder_path),
                                    ) {
                                        Ok(()) => {
                                            octos_win::wallpaper::forget_mod(&folder_path);
                                            for monitor in affected {
                                                fallback_to_gradient(&library_paths, &monitor);
                                            }
                                            save_preferences(&library_paths);
                                            match library_snapshot(&library_paths) {
                                                Ok(data) => Some(UiResponse::Library { data }),
                                                Err(err) => Some(UiResponse::Error {
                                                    message: format!("{err:?}"),
                                                }),
                                            }
                                        }
                                        Err(err) => {
                                            error!("could not remove mod: {err:?}");
                                            Some(UiResponse::Error {
                                                message: format!("{err:?}"),
                                            })
                                        }
                                    }
                                }
                            },
                            move |dropped| install_dropped(dropped, &drop_library_paths),
                        );
                        info!("React UI is ready");
                    }
                    Err(err) => error!("could not create main WebView2 controller: {err:?}"),
                },
            ) {
                error!("could not request main WebView2 controller: {err:?}");
            }

            let wallpapers_dir = paths.wallpapers.clone();
            match octos_win::wallpaper::initialize(environment, wallpapers_dir) {
                Ok(()) => {
                    octos_win::wallpaper::set_pause_fullscreen(
                        startup_preferences.app_options.pause_fullscreen,
                    );
                    apply_startup_preferences(
                        &startup_preferences,
                        &wallpaper_key,
                        &wallpaper_entry,
                        &wallpaper_options,
                        &push_library_paths,
                    );
                    if let Ok(snapshot) = library_snapshot(&push_library_paths) {
                        let payload = serde_json::json!({
                            "type": "library",
                            "data": snapshot
                        })
                        .to_string();
                        octos_win::post_to_main_webview(&payload);
                    }
                    WALLPAPERS_READY.store(true, Ordering::SeqCst);
                    for command in PENDING_CLI
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .drain(..)
                        .collect::<Vec<_>>()
                    {
                        execute_cli(command, &push_library_paths);
                    }
                }
                Err(err) => error!("could not initialize wallpapers: {err:?}"),
            }
        }
        Err(err) => error!("could not create WebView2 environment: {err:?}"),
    })?;

    octos_win::run_message_loop();
    // Flush the latest window placement and session state before tearing down.
    save_preferences(&flush_paths);
    octos_win::wallpaper::shutdown();
    octos_win::remove_tray_icon(hwnd);
    info!("message loop stopped");
    drop(instance);
    Ok(())
}

fn install_dropped(dropped: Vec<PathBuf>, paths: &Paths) {
    let mut changed = false;
    let mut problems: Vec<String> = Vec::new();
    for path in dropped {
        let result = if path.is_dir() {
            octos_storage::install_folder(&path, &paths.wallpapers)
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        {
            octos_storage::install_zip(&path, &paths.wallpapers)
        } else {
            warn!("ignoring dropped path {}", path.display());
            continue;
        };
        match result {
            Ok(installed) => {
                info!("installed dropped mod at {}", installed.display());
                changed = true;
            }
            Err(err) => {
                error!("could not install dropped path {}: {err:?}", path.display());
                problems.push(format!("{}: {err}", path.display()));
            }
        }
    }
    if changed && let Ok(snapshot) = library_snapshot(paths) {
        let payload = serde_json::json!({
            "type": "library",
            "data": snapshot
        })
        .to_string();
        octos_win::post_to_main_webview(&payload);
    }
    if !problems.is_empty()
        && let Ok(payload) = serde_json::to_string(&UiResponse::Notice {
            message: problems.join("\n"),
        })
    {
        octos_win::post_to_main_webview(&payload);
    }
}

fn settings_snapshot() -> serde_json::Value {
    let guard = PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let options = guard.as_ref().map(|preferences| &preferences.app_options);
    serde_json::json!({
        "runOnStartup": options.map(|options| options.run_on_startup).unwrap_or(false),
        "enableGpu": options.map(|options| options.enable_gpu).unwrap_or(true),
        "pauseFullscreen": options.map(|options| options.pause_fullscreen).unwrap_or(true),
    })
}

fn apply_setting(paths: &Paths, key: &str, value: &serde_json::Value) -> Result<()> {
    let enabled = value
        .as_bool()
        .ok_or_else(|| anyhow!("setting {key} expects a boolean value"))?;

    {
        let mut guard = PREFERENCES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let preferences = guard
            .as_mut()
            .ok_or_else(|| anyhow!("preferences are not loaded"))?;
        match key {
            "runOnStartup" => {
                octos_win::startup::set_run_on_startup(enabled)?;
                preferences.app_options.run_on_startup = enabled;
            }
            "enableGpu" => {
                preferences.app_options.enable_gpu = enabled;
                info!("GPU setting saved; it takes effect after restart");
            }
            "pauseFullscreen" => {
                preferences.app_options.pause_fullscreen = enabled;
                octos_win::wallpaper::set_pause_fullscreen(enabled);
                info!("pause on fullscreen set to {enabled}");
            }
            other => return Err(anyhow!("unknown setting: {other}")),
        }
    }

    save_preferences(paths);
    Ok(())
}

fn save_preferences(paths: &Paths) {
    let selected = octos_win::wallpaper::selected_monitors();
    let options = octos_win::wallpaper::all_options();
    let mut guard = PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(preferences) = guard.as_mut() else {
        return;
    };
    preferences.set_selected(selected);
    preferences.set_mod_options(options);
    if let Err(err) = octos_storage::save_preferences(&paths.preferences, preferences) {
        error!("could not save preferences: {err}");
    }
}

fn resolved_mod(paths: &Paths, folder_path: &str) -> Option<(String, OptionsMap)> {
    let report = catalog::scan(&paths.wallpapers).ok()?;
    let card = report
        .mods
        .into_iter()
        .find(|card| card.folder_path.to_string_lossy() == folder_path)?;
    let entry_url = card.entry_url?;
    let defaults = ModManifest::load(&card.folder_path).ok()?.options;
    Some((entry_url, defaults))
}

fn fallback_to_gradient(paths: &Paths, monitor: &str) {
    let key = paths
        .wallpapers
        .join("gradient")
        .to_string_lossy()
        .to_string();
    if let Err(err) = set_monitor_mod(paths, monitor, &key) {
        warn!("could not restore the gradient on {monitor}: {err:?}");
    }
}

fn apply_startup_preferences(
    preferences: &Preferences,
    gradient_key: &str,
    gradient_entry: &str,
    gradient_options: &OptionsMap,
    paths: &Paths,
) {
    let monitors = octos_win::wallpaper::monitor_ids();
    if preferences.selected.is_empty() {
        octos_win::wallpaper::set_all_mod(gradient_key.to_string(), gradient_options.clone());
        if let Err(err) = octos_win::wallpaper::show_all(gradient_entry) {
            error!("could not show the built-in gradient: {err:?}");
        }
        return;
    }

    for monitor in monitors {
        let applied = preferences.selected.get(&monitor).is_some_and(|folder| {
            match resolved_mod(paths, folder) {
                Some((entry_url, defaults)) => {
                    let stored = preferences.options_for(folder).cloned();
                    match octos_win::wallpaper::set_monitor_mod_with(
                        &monitor,
                        folder.clone(),
                        entry_url,
                        defaults,
                        stored,
                    ) {
                        Ok(()) => true,
                        Err(err) => {
                            warn!("could not restore {folder} on {monitor}: {err:?}");
                            false
                        }
                    }
                }
                None => {
                    warn!("saved wallpaper no longer exists: {folder}");
                    false
                }
            }
        });
        if !applied {
            let stored = preferences.options_for(gradient_key).cloned();
            if let Err(err) = octos_win::wallpaper::set_monitor_mod_with(
                &monitor,
                gradient_key.to_string(),
                gradient_entry.to_string(),
                gradient_options.clone(),
                stored,
            ) {
                error!("could not restore the gradient on {monitor}: {err:?}");
            }
        }
    }
}

fn library_snapshot(paths: &Paths) -> anyhow::Result<serde_json::Value> {
    let octos_core::catalog::ScanReport { mods, errors } = catalog::scan(&paths.wallpapers)?;
    let mods: Vec<_> = mods
        .into_iter()
        .filter(|card| card.folder_name != "conformance")
        .collect();
    {
        let guard = PREFERENCES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for card in &mods {
            let folder = card.folder_path.to_string_lossy().to_string();
            let Ok(manifest) = ModManifest::load(&card.folder_path) else {
                continue;
            };
            let stored = guard
                .as_ref()
                .and_then(|prefs| prefs.options_for(&folder).cloned());
            octos_win::wallpaper::ensure_mod_options(folder, manifest.options, stored);
        }
    }
    let monitors = octos_win::wallpaper::monitor_ids();
    let selected = octos_win::wallpaper::selected_monitors();
    info!(
        "library snapshot: {} mod(s), monitors={monitors:?}, selected={selected:?}",
        mods.len()
    );
    Ok(serde_json::json!({
        "mods": mods,
        "monitors": monitors,
        "selected": selected,
        "options": octos_win::wallpaper::all_options(),
        "errors": errors.iter().map(ToString::to_string).collect::<Vec<_>>(),
    }))
}

fn set_monitor_mod(paths: &Paths, monitor_id: &str, folder_path: &str) -> anyhow::Result<()> {
    let report = catalog::scan(&paths.wallpapers)?;
    let card = report
        .mods
        .into_iter()
        .find(|card| card.folder_path.to_string_lossy() == folder_path)
        .ok_or_else(|| anyhow!("mod not found: {folder_path}"))?;
    let entry_url = card
        .entry_url
        .clone()
        .ok_or_else(|| anyhow!("mod has no HTML entry: {folder_path}"))?;
    let manifest = ModManifest::load(&card.folder_path)?;
    octos_win::wallpaper::set_monitor_mod(
        monitor_id,
        folder_path.to_string(),
        entry_url,
        manifest.options,
    )
}

fn show_info(message: &str) {
    let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR::from_raw(text.as_ptr()),
            w!("Octos"),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

fn init_logging(paths: &Paths) {
    let file_appender = tracing_appender::rolling::daily(&paths.logs, "octos.log");
    let filter = std::env::var("OCTOS_LOG")
        .ok()
        .and_then(|value| EnvFilter::try_new(value).ok())
        .or_else(|| EnvFilter::try_from_default_env().ok())
        .unwrap_or_else(|| EnvFilter::new("info"));

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(file_appender)
        .with_ansi(false)
        .finish();

    if tracing::subscriber::set_global_default(subscriber).is_ok() {
        info!("log directory: {}", paths.logs.display());
    }

    std::panic::set_hook(Box::new(|info| {
        error!("panic: {info}");
    }));
}

fn dispatch_cli(command: Command, paths: &Paths) {
    if matches!(command, Command::New { .. }) || WALLPAPERS_READY.load(Ordering::SeqCst) {
        execute_cli(command, paths);
    } else {
        PENDING_CLI
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(command);
    }
}

fn execute_cli(command: Command, paths: &Paths) {
    match command {
        Command::Run { folder, dev_tools } => {
            if let Err(err) = cli_run(paths, &folder) {
                error!("cli run failed: {err:?}");
                show_error(&format!("[Octos CLI] Run failed\n\n{err}"));
            } else if dev_tools {
                octos_win::schedule_open_dev_tools();
            }
        }
        Command::Reload => {
            let reloaded = octos_win::wallpaper::reload_all();
            info!("cli reload: {reloaded} wallpaper(s)");
        }
        Command::New { folder } => match cli_new(&folder) {
            Ok(path) => show_info(&format!(
                "[Octos CLI] New succeeded\n\nSuccessfully created new mod at {}",
                path.display()
            )),
            Err(err) => {
                error!("cli new failed: {err:?}");
                show_error(&format!("[Octos CLI] New failed\n\n{err}"));
            }
        },
        Command::DevTools => octos_win::schedule_open_dev_tools(),
    }
}

fn cli_run(paths: &Paths, folder: &FsPath) -> Result<()> {
    let source = std::path::absolute(folder)?;
    if !source.exists() {
        return Err(anyhow!(
            "Provided folderPath {} does not exist! Try specifying the path to a valid mod folder. Ex. octos run path/to/mod",
            source.display()
        ));
    }
    let source = if source.is_dir() {
        source
    } else {
        source
            .parent()
            .map(FsPath::to_path_buf)
            .ok_or_else(|| anyhow!("{} has no parent folder", source.display()))?
    };

    let wallpapers = paths.wallpapers.canonicalize()?;
    let canonical = source.canonicalize()?;
    let installed = if canonical.starts_with(&wallpapers) {
        if canonical.parent() != Some(wallpapers.as_path()) {
            return Err(anyhow!(
                "{} is inside the wallpapers directory but is not a top-level mod folder",
                canonical.display()
            ));
        }
        // Keep the non-verbatim absolute path: catalog cards and prefs keys use
        // `E:\...`, while `canonicalize` yields `\\?\E:\...`.
        source
    } else {
        octos_storage::install_folder(&source, &paths.wallpapers)?
    };

    let manifest = ModManifest::load(&installed)?;
    let entry_url = manifest.entry_url().ok_or_else(|| {
        anyhow!(
            "Could not find a valid HTML file in folder {}",
            installed.display()
        )
    })?;
    let mod_key = installed.to_string_lossy().to_string();
    let stored = PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .and_then(|prefs| prefs.options_for(&mod_key).cloned());

    octos_win::wallpaper::set_all_mod_with(mod_key, entry_url, manifest.options, stored)?;
    save_preferences(paths);
    if let Ok(snapshot) = library_snapshot(paths)
        && let Ok(payload) =
            serde_json::to_string(&serde_json::json!({"type": "library", "data": snapshot}))
    {
        octos_win::post_to_main_webview(&payload);
    }
    info!("cli run applied {} on all monitors", installed.display());
    Ok(())
}

fn cli_new(folder: &FsPath) -> Result<PathBuf> {
    let folder = std::path::absolute(folder)?;
    if folder.exists() {
        if !folder.is_dir() {
            return Err(anyhow!("{} is not a folder", folder.display()));
        }
    } else {
        std::fs::create_dir(&folder)
            .with_context(|| format!("Failed to create new mod at {}", folder.display()))?;
    }
    octos_storage::create_mod_template(&folder)?;
    Ok(folder)
}

fn show_error(message: &str) {
    let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR::from_raw(text.as_ptr()),
            w!("Octos"),
            MB_OK | MB_ICONERROR,
        );
    }
}
