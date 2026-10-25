//! Wallpaper rotation engine.
//!
//! Runs on the UI thread from a one-second timer. All scheduling rules live in
//! `octos_core::playlist`; this module owns the runtime clock, the random seed
//! and the catalog lookup that turns folder keys into entry URLs.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use octos_core::catalog;
use octos_core::options::OptionsMap;
use octos_core::playlist::{self, Playlist};
use octos_storage::Paths;
use tracing::{info, warn};

struct Runtime {
    last_shown: Instant,
    seed: u64,
}

impl Runtime {
    fn new() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15)
            | 1;
        Self {
            last_shown: Instant::now(),
            seed,
        }
    }
}

fn runtime() -> &'static Mutex<Runtime> {
    static RUNTIME: OnceLock<Mutex<Runtime>> = OnceLock::new();
    RUNTIME.get_or_init(|| Mutex::new(Runtime::new()))
}

fn lock_runtime() -> std::sync::MutexGuard<'static, Runtime> {
    runtime()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn random_below(runtime: &mut Runtime, len: usize) -> usize {
    // xorshift64; the seed is never zero (see `Runtime::new`).
    let mut value = runtime.seed;
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    runtime.seed = value;
    (value % len as u64) as usize
}

struct Item {
    key: String,
    entry_url: String,
    defaults: OptionsMap,
}

fn rotation_state() -> Option<Playlist> {
    let guard = crate::PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let preferences = guard.as_ref()?;
    Some(preferences.playlist.clone())
}

/// Queue members that resolve to a runnable mod, in queue order.
fn runnable_items(paths: &Paths, playlist: &Playlist) -> Vec<Item> {
    let report = match catalog::scan(&paths.wallpapers) {
        Ok(report) => report,
        Err(err) => {
            warn!("could not scan wallpapers for the playlist: {err:?}");
            return Vec::new();
        }
    };
    let cards: HashMap<String, &catalog::ModCard> = report
        .mods
        .iter()
        .filter(|card| !catalog::is_hidden_mod(&card.folder_name))
        .map(|card| (card.folder_path.to_string_lossy().into_owned(), card))
        .collect();
    playlist
        .items
        .iter()
        .filter_map(|key| {
            let card = cards.get(key)?;
            let entry_url = card.entry_url.clone()?;
            Some(Item {
                key: key.clone(),
                entry_url,
                defaults: card.options.clone(),
            })
        })
        .collect()
}

fn all_monitors_show(key: &str) -> bool {
    let monitors = octos_win::wallpaper::monitor_ids();
    if monitors.is_empty() {
        return false;
    }
    let selected = octos_win::wallpaper::selected_monitors();
    monitors
        .iter()
        .all(|monitor| selected.get(monitor).map(String::as_str) == Some(key))
}

fn set_current(key: &str) {
    if let Some(preferences) = crate::PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_mut()
    {
        preferences.playlist.current = Some(key.to_string());
    }
}

/// Restart the interval from now. Called whenever the schedule changed so a
/// settings edit never triggers an immediate switch.
pub fn reset_schedule() {
    lock_runtime().last_shown = Instant::now();
}

/// Built-in mods are fallbacks, not user wallpapers: keep them out of playlists
/// written by earlier builds.
pub fn sanitize_items(paths: &Paths) {
    let hidden: Vec<String> = catalog::HIDDEN_MODS
        .iter()
        .map(|name| paths.wallpapers.join(name).to_string_lossy().to_string())
        .collect();
    let changed = {
        let mut guard = crate::PREFERENCES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(preferences) = guard.as_mut() else {
            return;
        };
        let before = preferences.playlist.items.len();
        preferences
            .playlist
            .items
            .retain(|item| !hidden.contains(item));
        if preferences
            .playlist
            .current
            .as_ref()
            .is_some_and(|current| hidden.contains(current))
        {
            preferences.playlist.current = None;
        }
        let emptied = preferences.playlist.items.is_empty();
        if emptied {
            // A rotation with nothing to show must not stay "on".
            preferences.playlist.enabled = false;
        }
        let before_order = preferences.library_order.len();
        preferences
            .library_order
            .retain(|item| !hidden.contains(item));
        before != preferences.playlist.items.len()
            || emptied
            || before_order != preferences.library_order.len()
    };
    if changed {
        reset_schedule();
        crate::save_preferences(paths);
    }
}

pub fn tick(paths: &Paths) {
    if !crate::WALLPAPERS_READY.load(Ordering::SeqCst) {
        return;
    }
    let Some(playlist) = rotation_state() else {
        return;
    };
    if !playlist.enabled {
        return;
    }
    // Fullscreen hides the wallpaper and a cleared desktop has no target; wait
    // without advancing the clock so the switch happens as soon as it can.
    if octos_win::wallpaper::rotation_should_pause() {
        return;
    }
    if !playlist::is_due(
        lock_runtime().last_shown.elapsed(),
        playlist.interval_seconds,
    ) {
        return;
    }
    perform_switch(paths, &playlist, "interval");
}

fn perform_switch(paths: &Paths, playlist: &Playlist, reason: &str) -> bool {
    let items = runnable_items(paths, playlist);
    if items.len() < 2 {
        reset_schedule();
        return false;
    }
    let current = playlist
        .current
        .as_ref()
        .and_then(|key| items.iter().position(|item| &item.key == key));
    let chosen = {
        let mut runtime = lock_runtime();
        playlist::next_index(items.len(), current, playlist.random, &mut |len| {
            random_below(&mut runtime, len)
        })
    };
    let Some(mut index) = chosen else {
        reset_schedule();
        return false;
    };

    for _ in 0..items.len() {
        let item = &items[index];
        if all_monitors_show(&item.key) {
            set_current(&item.key);
            reset_schedule();
            return false;
        }
        let stored = crate::stored_options(&item.key);
        match octos_win::wallpaper::switch_mod_all(
            item.key.clone(),
            item.entry_url.clone(),
            item.defaults.clone(),
            stored,
        ) {
            Ok(_) => {
                set_current(&item.key);
                reset_schedule();
                crate::save_preferences(paths);
                crate::push_library(paths);
                info!(
                    "playlist rotation: {} (reason={reason}, index={}/{}, random={})",
                    item.key,
                    index + 1,
                    items.len(),
                    playlist.random
                );
                return true;
            }
            Err(err) => {
                warn!("could not switch to playlist item {}: {err:?}", item.key);
                index = (index + 1) % items.len();
            }
        }
    }
    reset_schedule();
    warn!("playlist rotation failed: no item could be applied");
    false
}

/// Align the desktop with the playlist's `current` item (or the first runnable
/// item when `current` is stale). Used when rotation is enabled or the list is
/// edited. Does not save preferences; the caller owns the command snapshot.
pub fn apply_current(paths: &Paths) -> Result<bool> {
    let playlist = rotation_state().ok_or_else(|| anyhow!("preferences are not loaded"))?;
    let items = runnable_items(paths, &playlist);
    let Some(item) = playlist
        .current
        .as_ref()
        .and_then(|key| items.iter().find(|item| &item.key == key))
        .or_else(|| items.first())
    else {
        return Ok(false);
    };
    if all_monitors_show(&item.key) {
        set_current(&item.key);
        reset_schedule();
        return Ok(false);
    }
    let stored = crate::stored_options(&item.key);
    octos_win::wallpaper::switch_mod_all(
        item.key.clone(),
        item.entry_url.clone(),
        item.defaults.clone(),
        stored,
    )?;
    set_current(&item.key);
    reset_schedule();
    info!("playlist aligned to {}", item.key);
    Ok(true)
}

/// One explicit "next wallpaper" step (the UI button); ignores the interval.
/// Returns false when nothing could be switched.
pub fn next_now(paths: &Paths) -> Result<bool> {
    let playlist = rotation_state().ok_or_else(|| anyhow!("preferences are not loaded"))?;
    Ok(perform_switch(paths, &playlist, "next"))
}

/// A wallpaper was applied manually: restart the interval and adopt it as the
/// current item when it belongs to the playlist.
pub fn on_manual_selection(folder_path: &str) {
    reset_schedule();
    if let Some(preferences) = crate::PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_mut()
        && preferences.playlist.index_of(folder_path).is_some()
    {
        preferences.playlist.current = Some(folder_path.to_string());
    }
}

/// Drop a removed mod from the playlist and fall forward to the next item.
pub fn on_mod_removed(paths: &Paths, folder_path: &str) {
    let (changed, enabled) = {
        let mut guard = crate::PREFERENCES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(preferences) = guard.as_mut() else {
            return;
        };
        let before = preferences.playlist.items.len();
        preferences
            .playlist
            .items
            .retain(|item| item != folder_path);
        let was_current = preferences.playlist.current.as_deref() == Some(folder_path);
        if was_current {
            preferences.playlist.current = None;
        }
        if preferences.playlist.items.is_empty() {
            // Nothing left to rotate; otherwise the switch stays on but cannot
            // be toggled because the list is empty.
            preferences.playlist.enabled = false;
        }
        (
            before != preferences.playlist.items.len() || was_current,
            preferences.playlist.enabled,
        )
    };
    if !changed {
        return;
    }
    reset_schedule();
    if enabled && let Err(err) = apply_current(paths) {
        warn!("could not move to the next playlist item after a removal: {err:?}");
    }
}
