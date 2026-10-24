#![windows_subsystem = "windows"]

use std::path::{Path as FsPath, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

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
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW, SetTimer,
};
use windows::core::{PCWSTR, w};

mod rotation;

pub(crate) static PREFERENCES: Mutex<Option<Preferences>> = Mutex::new(None);

/// `octos run`/`reload`/`dev-tools` need the wallpaper slots; commands that
/// arrive before WebView2 finishes initializing wait here.
pub(crate) static WALLPAPERS_READY: AtomicBool = AtomicBool::new(false);
static PENDING_CLI: Mutex<Vec<Command>> = Mutex::new(Vec::new());
static PREFS_WRITER: Mutex<Option<octos_storage::PrefsWriter>> = Mutex::new(None);
static THUMBNAIL_BACKFILL_RUNNING: AtomicBool = AtomicBool::new(false);

/// Built-in wallpaper used when nothing is selected or a mod disappears.
const DEFAULT_WALLPAPER: &str = "default";
const DEFAULT_WALLPAPER_ENTRY: &str = "https://local.octos/default/index.html";

/// One-second timer that drives wallpaper rotation. Timer ids 2 (watchdog) and
/// 3 (devtools) are already taken on the UI thread.
const ROTATION_TIMER_ID: usize = 4;
const ROTATION_TICK_MS: u32 = 1000;

static ROTATION_PATHS: OnceLock<Paths> = OnceLock::new();

unsafe extern "system" fn rotation_timer(_hwnd: HWND, _message: u32, _id: usize, _time: u32) {
    if let Some(paths) = ROTATION_PATHS.get() {
        rotation::tick(paths);
    }
}

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

    let mut preferences = octos_storage::load_preferences(&paths.preferences);
    // Lite mode is a per-session tray action now, not a persisted setting.
    let had_lite_mode = preferences.app_options.extra.remove("liteMode").is_some();
    let saved_window: Option<WindowPlacement> = preferences.window;
    let removed_staging = octos_storage::cleanup_staging(&paths.wallpapers);
    if removed_staging > 0 {
        info!("removed {removed_staging} stale install staging director(ies)");
    }
    info!(
        "preferences loaded: {} monitor selection(s)",
        preferences.selected.len()
    );
    *PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(preferences.clone());
    if had_lite_mode
        && let Err(err) = octos_storage::save_preferences(&paths.preferences, &preferences)
    {
        warn!("could not remove the stale liteMode preference: {err}");
    }
    *PREFS_WRITER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(octos_storage::PrefsWriter::start());

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

    let display_paths = paths.clone();
    octos_win::set_display_change_handler(move || handle_display_change(&display_paths));

    let library_paths = paths.clone();
    octos_win::set_library_change_handler(move || {
        if WALLPAPERS_READY.load(Ordering::SeqCst) {
            push_library(&library_paths);
        }
    });

    let command_paths = paths.clone();
    octos_win::set_command_handler(move |line| {
        let args = octos_win::split_command_line(line);
        if let Some(command) = cli::parse(args.get(1..).unwrap_or(&[])) {
            dispatch_cli(command, &command_paths);
        }
    });
    let lite_paths = paths.clone();
    octos_win::set_lite_mode_handler(move |enabled| {
        if enabled {
            if let Some(preferences) = PREFERENCES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_mut()
            {
                preferences.app_options.pause_fullscreen = true;
                preferences.app_options.memory_saver = true;
            }
            octos_win::wallpaper::set_pause_fullscreen(true);
            octos_win::set_memory_saver(true);
        }
        save_preferences(&lite_paths);
    });
    if let Some(command) = startup_command {
        dispatch_cli(command, &paths);
    }

    let app_dir = paths.app_dir();
    let wallpapers_dir = paths.wallpapers.clone();
    let wallpaper_folder =
        std::env::var("OCTOS_TEST_MOD_FOLDER").unwrap_or_else(|_| DEFAULT_WALLPAPER.to_string());
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

            let frontend_environment = environment.clone();
            let attach_frontend: Rc<dyn Fn()> = {
                let app_dir = app_dir.clone();
                let wallpapers_dir = wallpapers_dir.clone();
                let startup_url = startup_url.clone();
                let library_paths = library_paths.clone();
                Rc::new(move || {
                    let app_dir = app_dir.clone();
                    let wallpapers_dir = wallpapers_dir.clone();
                    let startup_url = startup_url.clone();
                    let library_paths = library_paths.clone();
                    if let Err(err) = octos_webview::create_main_controller(
                        &frontend_environment,
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
                                            let url = DEFAULT_WALLPAPER_ENTRY;
                                            match octos_win::wallpaper::show_all(url) {
                                                Ok(()) => Some(UiResponse::WallpaperState {
                                                    active: true,
                                                }),
                                                Err(err) => {
                                                    error!("could not show wallpaper: {err:?}");
                                                    Some(UiResponse::Error {
                                                        message: format!("{err}"),
                                                    })
                                                }
                                            }
                                        }
                                        UiRequest::ClearWallpaper => {
                                            octos_win::wallpaper::clear_all();
                                            Some(UiResponse::WallpaperState { active: false })
                                        }
                                        UiRequest::GetDesktopIcons => {
                                            Some(UiResponse::DesktopIcons {
                                                visible: octos_win::desktop_icons::is_visible(),
                                            })
                                        }
                                        UiRequest::SetDesktopIcons { visible } => {
                                            match octos_win::desktop_icons::set_visible(visible) {
                                                Ok(()) => {
                                                    Some(UiResponse::DesktopIcons { visible })
                                                }
                                                Err(err) => {
                                                    error!(
                                                        "could not set desktop icon visibility: {err:?}"
                                                    );
                                                    Some(UiResponse::Error {
                                                        message: format!("{err}"),
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
                                                    error!(
                                                        "could not apply setting {key}: {err:?}"
                                                    );
                                                    Some(UiResponse::Error {
                                                        message: format!("{err}"),
                                                    })
                                                }
                                            }
                                        }
                                        UiRequest::GetLibrary => {
                                            match library_snapshot(&library_paths) {
                                                Ok(data) => Some(UiResponse::Library { data }),
                                                Err(err) => {
                                                    error!("could not scan library: {err:?}");
                                                    Some(UiResponse::Error {
                                                        message: format!("{err}"),
                                                    })
                                                }
                                            }
                                        }
                                        UiRequest::SetMonitorMod {
                                            monitor_id,
                                            folder_path,
                                        } => {
                                            match set_monitor_mod(
                                                &library_paths,
                                                &monitor_id,
                                                &folder_path,
                                            ) {
                                                Ok(()) => {
                                                    rotation::on_manual_selection(&folder_path);
                                                    save_preferences(&library_paths);
                                                    match library_snapshot(&library_paths) {
                                                        Ok(data) => {
                                                            Some(UiResponse::Library { data })
                                                        }
                                                        Err(err) => Some(UiResponse::Error {
                                                            message: format!("{err}"),
                                                        }),
                                                    }
                                                }
                                                Err(err) => {
                                                    error!("could not set monitor mod: {err:?}");
                                                    Some(UiResponse::Error {
                                                        message: format!("{err}"),
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
                                                    .unwrap_or_else(|poisoned| {
                                                        poisoned.into_inner()
                                                    })
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
                                                    message: format!("{err}"),
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
                                                    Ok(_) => match library_snapshot(&library_paths)
                                                    {
                                                        Ok(data) => {
                                                            Some(UiResponse::Library { data })
                                                        }
                                                        Err(err) => Some(UiResponse::Error {
                                                            message: format!("{err}"),
                                                        }),
                                                    },
                                                    Err(err) => {
                                                        error!("could not install mod: {err:?}");
                                                        Some(UiResponse::Error {
                                                            message: format!("{err}"),
                                                        })
                                                    }
                                                }
                                            } else {
                                                match library_snapshot(&library_paths) {
                                                    Ok(data) => Some(UiResponse::Library { data }),
                                                    Err(err) => Some(UiResponse::Error {
                                                        message: format!("{err}"),
                                                    }),
                                                }
                                            }
                                        }
                                        UiRequest::InstallFolder => {
                                            if let Some(path) = rfd::FileDialog::new().pick_folder()
                                            {
                                                match octos_storage::install_folder(
                                                    &path,
                                                    &library_paths.wallpapers,
                                                ) {
                                                    Ok(installed) => {
                                                        info!(
                                                            "imported folder {}",
                                                            installed.display()
                                                        );
                                                        match library_snapshot(&library_paths) {
                                                            Ok(data) => {
                                                                Some(UiResponse::Library { data })
                                                            }
                                                            Err(err) => Some(UiResponse::Error {
                                                                message: format!("{err}"),
                                                            }),
                                                        }
                                                    }
                                                    Err(err) => {
                                                        error!("could not import folder: {err:?}");
                                                        Some(UiResponse::Error {
                                                            message: format!("{err}"),
                                                        })
                                                    }
                                                }
                                            } else {
                                                match library_snapshot(&library_paths) {
                                                    Ok(data) => Some(UiResponse::Library { data }),
                                                    Err(err) => Some(UiResponse::Error {
                                                        message: format!("{err}"),
                                                    }),
                                                }
                                            }
                                        }
                                        UiRequest::InstallImage => {
                                            if let Some(path) = rfd::FileDialog::new()
                                                .add_filter(
                                                    "Images",
                                                    &["png", "jpg", "jpeg", "webp", "gif", "bmp"],
                                                )
                                                .pick_file()
                                            {
                                                match octos_storage::install_image(
                                                    &path,
                                                    &library_paths.wallpapers,
                                                ) {
                                                    Ok(installed) => {
                                                        info!(
                                                            "imported image {}",
                                                            installed.display()
                                                        );
                                                        match library_snapshot(&library_paths) {
                                                            Ok(data) => {
                                                                Some(UiResponse::Library { data })
                                                            }
                                                            Err(err) => Some(UiResponse::Error {
                                                                message: format!("{err}"),
                                                            }),
                                                        }
                                                    }
                                                    Err(err) => {
                                                        error!("could not import image: {err:?}");
                                                        Some(UiResponse::Error {
                                                            message: format!("{err}"),
                                                        })
                                                    }
                                                }
                                            } else {
                                                match library_snapshot(&library_paths) {
                                                    Ok(data) => Some(UiResponse::Library { data }),
                                                    Err(err) => Some(UiResponse::Error {
                                                        message: format!("{err}"),
                                                    }),
                                                }
                                            }
                                        }
                                        UiRequest::RemoveMod { folder_path } => {
                                            if FsPath::new(&folder_path)
                                                == library_paths.wallpapers.join(DEFAULT_WALLPAPER)
                                                || FsPath::new(&folder_path)
                                                    == library_paths.wallpapers.join("gradient")
                                            {
                                                return Some(UiResponse::Error {
                                            message:
                                                "the built-in default wallpaper cannot be removed"
                                                    .to_string(),
                                        });
                                            }
                                            let affected: Vec<String> =
                                                octos_win::wallpaper::selected_monitors()
                                                    .into_iter()
                                                    .filter(|(_, selected)| {
                                                        selected == &folder_path
                                                    })
                                                    .map(|(monitor, _)| monitor)
                                                    .collect();
                                            match octos_storage::remove_mod(
                                                &library_paths.wallpapers,
                                                FsPath::new(&folder_path),
                                            ) {
                                                Ok(()) => {
                                                    octos_win::wallpaper::forget_mod(&folder_path);
                                                    for monitor in affected {
                                                        fallback_to_default(
                                                            &library_paths,
                                                            &monitor,
                                                        );
                                                    }
                                                    if let Some(preferences) = PREFERENCES
                                                        .lock()
                                                        .unwrap_or_else(|poisoned| {
                                                            poisoned.into_inner()
                                                        })
                                                        .as_mut()
                                                    {
                                                        preferences
                                                            .library_order
                                                            .retain(|item| item != &folder_path);
                                                    }
                                                    rotation::on_mod_removed(
                                                        &library_paths,
                                                        &folder_path,
                                                    );
                                                    save_preferences(&library_paths);
                                                    match library_snapshot(&library_paths) {
                                                        Ok(data) => {
                                                            Some(UiResponse::Library { data })
                                                        }
                                                        Err(err) => Some(UiResponse::Error {
                                                            message: format!("{err}"),
                                                        }),
                                                    }
                                                }
                                                Err(err) => {
                                                    error!("could not remove mod: {err:?}");
                                                    Some(UiResponse::Error {
                                                        message: format!("{err}"),
                                                    })
                                                }
                                            }
                                        }
                                        UiRequest::SetPlaylistItems { items } => {
                                            let items =
                                                octos_core::playlist::normalize_items(items);
                                            let enabled = {
                                                let mut guard =
                                                    PREFERENCES.lock().unwrap_or_else(|poisoned| {
                                                        poisoned.into_inner()
                                                    });
                                                match guard.as_mut() {
                                                    Some(preferences) => {
                                                        preferences.playlist.items = items;
                                                        if preferences.playlist.items.is_empty() {
                                                            preferences.playlist.enabled = false;
                                                        }
                                                        let stale = preferences
                                                            .playlist
                                                            .current
                                                            .as_ref()
                                                            .is_some_and(|current| {
                                                                preferences
                                                                    .playlist
                                                                    .index_of(current)
                                                                    .is_none()
                                                            });
                                                        if stale {
                                                            preferences.playlist.current = None;
                                                        }
                                                        preferences.playlist.enabled
                                                    }
                                                    None => false,
                                                }
                                            };
                                            rotation::reset_schedule();
                                            if enabled
                                                && let Err(err) =
                                                    rotation::apply_current(&library_paths)
                                            {
                                                error!(
                                                    "could not align the playlist after an edit: {err:?}"
                                                );
                                            }
                                            save_preferences(&library_paths);
                                            library_response(&library_paths)
                                        }
                                        UiRequest::SetPlaylistEnabled { enabled } => {
                                            let result = (|| -> anyhow::Result<()> {
                                                {
                                                    let mut guard =
                                                        PREFERENCES.lock().unwrap_or_else(
                                                            |poisoned| poisoned.into_inner(),
                                                        );
                                                    let preferences =
                                                        guard.as_mut().ok_or_else(|| {
                                                            anyhow!("preferences are not loaded")
                                                        })?;
                                                    if enabled
                                                        && preferences.playlist.items.is_empty()
                                                    {
                                                        return Err(anyhow!(
                                                            "add at least one wallpaper to the playlist first"
                                                        ));
                                                    }
                                                    preferences.playlist.enabled = enabled;
                                                }
                                                rotation::reset_schedule();
                                                if enabled {
                                                    rotation::apply_current(&library_paths)?;
                                                }
                                                save_preferences(&library_paths);
                                                Ok(())
                                            })(
                                            );
                                            match result {
                                                Ok(()) => library_response(&library_paths),
                                                Err(err) => Some(UiResponse::Error {
                                                    message: format!("{err}"),
                                                }),
                                            }
                                        }
                                        UiRequest::SetLibraryOrder { items } => {
                                            let items =
                                                octos_core::playlist::normalize_items(items);
                                            if items.len() > 10_000 {
                                                return Some(UiResponse::Error {
                                                    message: "the library order is too large"
                                                        .to_string(),
                                                });
                                            }
                                            if let Some(preferences) = PREFERENCES
                                                .lock()
                                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                                .as_mut()
                                            {
                                                // Keep entries the UI cannot see right now
                                                // (offline drive, temporarily broken mod).
                                                let mut next = items;
                                                for old in preferences.library_order.clone() {
                                                    if !next.iter().any(|item| item == &old) {
                                                        next.push(old);
                                                    }
                                                }
                                                preferences.library_order = next;
                                            }
                                            rotation::reset_schedule();
                                            save_preferences(&library_paths);
                                            library_response(&library_paths)
                                        }
                                        UiRequest::SetPlaylistInterval { seconds } => {
                                            let seconds =
                                                octos_core::playlist::clamp_interval(seconds);
                                            if let Some(preferences) = PREFERENCES
                                                .lock()
                                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                                .as_mut()
                                            {
                                                preferences.playlist.interval_seconds = seconds;
                                            }
                                            rotation::reset_schedule();
                                            save_preferences(&library_paths);
                                            library_response(&library_paths)
                                        }
                                        UiRequest::SetPlaylistRandom { random } => {
                                            if let Some(preferences) = PREFERENCES
                                                .lock()
                                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                                .as_mut()
                                            {
                                                preferences.playlist.random = random;
                                            }
                                            rotation::reset_schedule();
                                            save_preferences(&library_paths);
                                            library_response(&library_paths)
                                        }
                                        UiRequest::PlaylistNext => {
                                            match rotation::next_now(&library_paths) {
                                                Ok(true) => library_response(&library_paths),
                                                Ok(false) => Some(UiResponse::Error {
                                                    message:
                                                        "the playlist needs at least two wallpapers"
                                                            .to_string(),
                                                }),
                                                Err(err) => Some(UiResponse::Error {
                                                    message: format!("{err}"),
                                                }),
                                            }
                                        }
                                        UiRequest::OpenWallpapersFolder => {
                                            if let Err(err) =
                                                std::fs::create_dir_all(&library_paths.wallpapers)
                                            {
                                                return Some(UiResponse::Error {
                                                    message: format!("{err}"),
                                                });
                                            }
                                            match std::process::Command::new("explorer")
                                                .arg(&library_paths.wallpapers)
                                                .spawn()
                                            {
                                                Ok(_) => None,
                                                Err(err) => Some(UiResponse::Error {
                                                    message: format!(
                                                        "could not open the wallpapers folder: {err}"
                                                    ),
                                                }),
                                            }
                                        }
                                        UiRequest::RefreshLibrary => {
                                            // Match upstream "refresh": rescan the
                                            // folder and recreate the wallpaper
                                            // controllers so external changes are
                                            // picked up.
                                            let selected =
                                                octos_win::wallpaper::selected_monitors();
                                            for (monitor, folder) in selected {
                                                if let Err(err) = set_monitor_mod(
                                                    &library_paths,
                                                    &monitor,
                                                    &folder,
                                                ) {
                                                    warn!(
                                                        "could not recreate {folder} on {monitor}: {err:?}"
                                                    );
                                                }
                                            }
                                            backfill_thumbnails(&library_paths);
                                            library_response(&library_paths)
                                        }
                                        UiRequest::GetWallpaperVisibility => {
                                            Some(UiResponse::WallpaperVisibility {
                                                visible: octos_win::wallpaper::is_visible(),
                                            })
                                        }
                                        UiRequest::SetWallpaperVisibility { visible } => {
                                            octos_win::wallpaper::set_visible(visible);
                                            info!("wallpaper visibility set to {visible}");
                                            Some(UiResponse::WallpaperVisibility { visible })
                                        }
                                    },
                                    move |dropped| install_dropped(dropped, &drop_library_paths),
                                );
                                info!("React UI is ready");
                            }
                            Err(err) => {
                                error!("could not create main WebView2 controller: {err:?}")
                            }
                        },
                    ) {
                        error!("could not request main WebView2 controller: {err:?}");
                    }
                })
            };
            attach_frontend();
            octos_win::set_frontend_factory(move || attach_frontend());

            let wallpapers_dir = paths.wallpapers.clone();
            match octos_win::wallpaper::initialize(environment, wallpapers_dir) {
                Ok(()) => {
                    octos_win::wallpaper::set_pause_fullscreen(
                        startup_preferences.app_options.pause_fullscreen,
                    );
                    octos_win::wallpaper::set_fps_cap(startup_preferences.app_options.max_fps);
                    // Lite mode implies MemorySaver; honour a persisted opt-out otherwise.
                    octos_win::set_memory_saver(startup_preferences.app_options.memory_saver);
                    apply_startup_preferences(
                        &startup_preferences,
                        &wallpaper_key,
                        &wallpaper_entry,
                        &wallpaper_options,
                        &push_library_paths,
                    );
                    let _ = ROTATION_PATHS.set(push_library_paths.clone());
                    unsafe {
                        let _ = SetTimer(
                            None,
                            ROTATION_TIMER_ID,
                            ROTATION_TICK_MS,
                            Some(rotation_timer),
                        );
                    }
                    rotation::reset_schedule();
                    rotation::sanitize_items(&push_library_paths);
                    backfill_thumbnails(&push_library_paths);
                    if startup_preferences.playlist.enabled
                        && let Err(err) = rotation::apply_current(&push_library_paths)
                    {
                        warn!("could not align the playlist on startup: {err:?}");
                    }
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
    unsafe {
        let _ = KillTimer(None, ROTATION_TIMER_ID);
    }
    // Flush the latest window placement and session state before tearing down.
    flush_preferences(&flush_paths);
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
        } else if octos_storage::is_image_file(&path) {
            octos_storage::install_image(&path, &paths.wallpapers)
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
        "maxFps": options.map(|options| options.max_fps).unwrap_or(30),
        "memorySaver": options.map(|options| options.memory_saver).unwrap_or(true),
    })
}

fn apply_setting(paths: &Paths, key: &str, value: &serde_json::Value) -> Result<()> {
    let boolean = |key: &str| {
        value
            .as_bool()
            .ok_or_else(|| anyhow!("setting {key} expects a boolean value"))
    };

    {
        let mut guard = PREFERENCES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let preferences = guard
            .as_mut()
            .ok_or_else(|| anyhow!("preferences are not loaded"))?;
        match key {
            "runOnStartup" => {
                let enabled = boolean(key)?;
                octos_win::startup::set_run_on_startup(enabled)?;
                preferences.app_options.run_on_startup = enabled;
            }
            "enableGpu" => {
                let enabled = boolean(key)?;
                preferences.app_options.enable_gpu = enabled;
                info!("GPU setting saved; it takes effect after restart");
            }
            "pauseFullscreen" => {
                let enabled = boolean(key)?;
                preferences.app_options.pause_fullscreen = enabled;
                octos_win::wallpaper::set_pause_fullscreen(enabled);
                info!("pause on fullscreen set to {enabled}");
            }
            "maxFps" => {
                let fps = value
                    .as_u64()
                    .ok_or_else(|| anyhow!("setting maxFps expects a number"))?
                    .min(240) as u32;
                preferences.app_options.max_fps = fps;
                octos_win::wallpaper::set_fps_cap(fps);
                info!("wallpaper frame cap set to {fps} (0 = unlimited)");
            }
            "memorySaver" => {
                let enabled = boolean(key)?;
                preferences.app_options.memory_saver = enabled;
                octos_win::set_memory_saver(enabled);
            }
            other => return Err(anyhow!("unknown setting: {other}")),
        }
    }

    save_preferences(paths);
    Ok(())
}

fn snapshot_preferences() -> Option<Preferences> {
    let selected = octos_win::wallpaper::selected_monitors();
    let options = octos_win::wallpaper::all_options();
    let mut guard = PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let preferences = guard.as_mut()?;
    preferences.set_selected(selected);
    preferences.set_mod_options(options);
    Some(preferences.clone())
}

/// Queue a snapshot; the writer coalesces bursts (slider drags, window moves)
/// and writes after 250ms of quiet.
pub(crate) fn save_preferences(paths: &Paths) {
    let Some(preferences) = snapshot_preferences() else {
        return;
    };
    let writer = PREFS_WRITER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(writer) = writer.as_ref()
        && writer.schedule(paths.preferences.clone(), preferences.clone())
    {
        return;
    }
    drop(writer);
    if let Err(err) = octos_storage::save_preferences(&paths.preferences, &preferences) {
        error!("could not save preferences: {err}");
    }
}

/// Write the newest snapshot and join the writer before the process exits.
fn flush_preferences(paths: &Paths) {
    let preferences = snapshot_preferences();
    let mut guard = PREFS_WRITER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match (guard.as_ref(), preferences) {
        (Some(writer), Some(preferences)) => {
            let _ = writer.schedule(paths.preferences.clone(), preferences);
        }
        (None, Some(preferences)) => {
            if let Err(err) = octos_storage::save_preferences(&paths.preferences, &preferences) {
                error!("could not save preferences: {err}");
            }
        }
        _ => {}
    }
    if let Some(mut writer) = guard.take() {
        writer.flush();
    }
}

pub(crate) fn stored_options(folder: &str) -> Option<OptionsMap> {
    PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .and_then(|preferences| preferences.options_for(folder).cloned())
}

fn default_wallpaper(paths: &Paths) -> (String, String, OptionsMap) {
    let folder =
        std::env::var("OCTOS_TEST_MOD_FOLDER").unwrap_or_else(|_| DEFAULT_WALLPAPER.to_string());
    let folder_path = paths.wallpapers.join(&folder);
    let manifest = ModManifest::load(&folder_path).ok();
    let key = folder_path.to_string_lossy().to_string();
    let entry = manifest
        .as_ref()
        .and_then(ModManifest::entry_url)
        .unwrap_or_else(|| format!("https://local.octos/{folder}/index.html"));
    let options = manifest
        .map(|manifest| manifest.options)
        .unwrap_or_default();
    (key, entry, options)
}

fn apply_monitor_mod(paths: &Paths, monitor: &str, folder: Option<&str>) -> anyhow::Result<()> {
    if let Some(folder) = folder
        && let Some((entry, defaults)) = resolved_mod(paths, folder)
    {
        octos_win::wallpaper::set_monitor_mod_with(
            monitor,
            folder.to_string(),
            entry,
            defaults,
            stored_options(folder),
        )?;
        return Ok(());
    }

    let (key, entry, options) = default_wallpaper(paths);
    let stored = stored_options(&key);
    octos_win::wallpaper::set_monitor_mod_with(monitor, key, entry, options, stored)?;
    Ok(())
}

/// Generate missing card thumbnails off the UI thread. The window procedure
/// pushes a fresh snapshot once the files exist.
fn backfill_thumbnails(paths: &Paths) {
    if THUMBNAIL_BACKFILL_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let wallpapers = paths.wallpapers.clone();
    let spawned = std::thread::Builder::new()
        .name("octos-thumbnails".into())
        .spawn(move || {
            let generated = octos_storage::ensure_all(&wallpapers);
            THUMBNAIL_BACKFILL_RUNNING.store(false, Ordering::SeqCst);
            if generated > 0 {
                info!("generated {generated} wallpaper thumbnail(s)");
                octos_win::notify_library_changed();
            }
        });
    if let Err(err) = spawned {
        THUMBNAIL_BACKFILL_RUNNING.store(false, Ordering::SeqCst);
        warn!("could not start the thumbnail backfill: {err}");
    }
}

pub(crate) fn push_library(paths: &Paths) {
    if let Ok(snapshot) = library_snapshot(paths)
        && let Ok(payload) = serde_json::to_string(&serde_json::json!({
            "type": "library",
            "data": snapshot,
        }))
    {
        octos_win::post_to_main_webview(&payload);
    }
}

/// A monitor was added, removed or moved. Reconcile already rebuilt the slots;
/// here we restore saved selections and give brand-new monitors a wallpaper
/// (the primary display's mod, or the built-in default wallpaper).
fn handle_display_change(paths: &Paths) {
    if !WALLPAPERS_READY.load(Ordering::SeqCst) {
        return;
    }
    let monitors = octos_win::wallpaper::monitor_ids();
    let selected = octos_win::wallpaper::selected_monitors();
    let fallback = octos_win::wallpaper::primary_monitor_id()
        .and_then(|monitor| selected.get(&monitor).cloned())
        .or_else(|| selected.values().next().cloned());

    let mut changed = false;
    for monitor in &monitors {
        if octos_win::wallpaper::monitor_has_wallpaper(monitor) {
            continue;
        }
        let folder = selected.get(monitor).cloned().or_else(|| fallback.clone());
        match apply_monitor_mod(paths, monitor, folder.as_deref()) {
            Ok(()) => {
                changed = true;
                info!("assigned wallpaper on {monitor} after a display change");
            }
            Err(err) => warn!("could not assign wallpaper on {monitor}: {err:?}"),
        }
    }

    if changed {
        save_preferences(paths);
    }
    push_library(paths);
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

fn fallback_to_default(paths: &Paths, monitor: &str) {
    let key = paths
        .wallpapers
        .join(DEFAULT_WALLPAPER)
        .to_string_lossy()
        .to_string();
    if let Err(err) = set_monitor_mod(paths, monitor, &key) {
        warn!("could not restore the default wallpaper on {monitor}: {err:?}");
    }
}

fn apply_startup_preferences(
    preferences: &Preferences,
    fallback_key: &str,
    fallback_entry: &str,
    fallback_options: &OptionsMap,
    paths: &Paths,
) {
    let monitors = octos_win::wallpaper::monitor_ids();
    if preferences.selected.is_empty() {
        octos_win::wallpaper::set_all_mod(fallback_key.to_string(), fallback_options.clone());
        if let Err(err) = octos_win::wallpaper::show_all(fallback_entry) {
            error!("could not show the built-in default wallpaper: {err:?}");
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
            let stored = preferences.options_for(fallback_key).cloned();
            if let Err(err) = octos_win::wallpaper::set_monitor_mod_with(
                &monitor,
                fallback_key.to_string(),
                fallback_entry.to_string(),
                fallback_options.clone(),
                stored,
            ) {
                error!("could not restore the default wallpaper on {monitor}: {err:?}");
            }
        }
    }
}

fn library_snapshot(paths: &Paths) -> anyhow::Result<serde_json::Value> {
    let octos_core::catalog::ScanReport { mods, errors } = catalog::scan(&paths.wallpapers)?;
    let mut mods: Vec<_> = mods
        .into_iter()
        .filter(|card| !catalog::is_hidden_mod(&card.folder_name))
        .collect();
    {
        let guard = PREFERENCES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(preferences) = guard.as_ref() {
            catalog::sort_by_order(&mut mods, &preferences.library_order);
        }
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
    let mut selected = octos_win::wallpaper::selected_monitors();
    selected.retain(|monitor, _| monitors.contains(monitor));
    info!(
        "library snapshot: {} mod(s), monitors={monitors:?}, selected={selected:?}",
        mods.len()
    );
    let playlist = playlist_snapshot(&mods);
    Ok(serde_json::json!({
        "mods": mods,
        "monitors": monitors,
        "monitorNames": octos_win::monitor_names(),
        "selected": selected,
        "options": octos_win::wallpaper::all_options(),
        "errors": errors.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "playlist": playlist,
    }))
}

fn library_response(paths: &Paths) -> Option<UiResponse> {
    match library_snapshot(paths) {
        Ok(data) => Some(UiResponse::Library { data }),
        Err(err) => Some(UiResponse::Error {
            message: format!("{err}"),
        }),
    }
}

/// Rotation state for the UI. `missing` keeps entries that no longer resolve so
/// the list does not silently lose wallpapers while a drive is offline.
fn playlist_snapshot(mods: &[catalog::ModCard]) -> serde_json::Value {
    let guard = PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(preferences) = guard.as_ref() else {
        return serde_json::json!({
            "enabled": false,
            "items": [],
            "intervalSeconds": octos_core::playlist::DEFAULT_INTERVAL_SECONDS,
            "random": false,
            "current": null,
            "missing": [],
        });
    };
    let playlist = &preferences.playlist;
    let grid: Vec<String> = mods
        .iter()
        .filter(|card| card.entry_url.is_some())
        .map(|card| card.folder_path.to_string_lossy().to_string())
        .collect();
    let items = octos_core::playlist::members_in_order(&playlist.items, &grid);
    let present: std::collections::HashSet<&str> = grid.iter().map(String::as_str).collect();
    let missing: Vec<&String> = items
        .iter()
        .filter(|key| !present.contains(key.as_str()))
        .collect();
    serde_json::json!({
        "enabled": playlist.enabled,
        "items": items,
        "intervalSeconds": playlist.interval_seconds,
        "random": playlist.random,
        "current": playlist.current,
        "missing": missing,
    })
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
    let installed = if source.is_file() && octos_storage::is_image_file(&source) {
        octos_storage::install_image(&source, &paths.wallpapers)?
    } else {
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
        if canonical.starts_with(&wallpapers) {
            if canonical.parent() != Some(wallpapers.as_path()) {
                return Err(anyhow!(
                    "{} is inside the wallpapers directory but is not a top-level mod folder",
                    canonical.display()
                ));
            }
            // Keep the non-verbatim absolute path: catalog cards and prefs keys
            // use `E:\...`, while `canonicalize` yields `\\?\E:\...`.
            source
        } else {
            octos_storage::install_folder(&source, &paths.wallpapers)?
        }
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
