//! Community catalog state, background download/extraction, and the UI-thread
//! commit that swaps an installed wallpaper folder.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use octos_core::community::{
    CommunityIndex, CommunityItem, CommunitySource, InstalledCommunity, MAX_ARCHIVE_BYTES,
    MAX_INDEX_BYTES, app_supports, find_item, is_update, parse_index,
};
use octos_protocol::UiResponse;
use octos_storage::install::{InstallMode, StagedInstall};
use octos_storage::{HashError, Paths};
use serde_json::{Value, json};
use tracing::{error, info, warn};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

static STATE: Mutex<Option<State>> = Mutex::new(None);
static PATHS: OnceLock<Paths> = OnceLock::new();
static CANCEL_DOWNLOAD: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Idle,
    Loading,
    Ready,
    Error,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Idle => "idle",
            Status::Loading => "loading",
            Status::Ready => "ready",
            Status::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
struct ErrorInfo {
    code: String,
    detail: String,
}

#[derive(Debug)]
enum Job {
    Idle,
    WaitingForIndex {
        folder: String,
    },
    Downloading {
        folder: String,
        phase: Phase,
        received: u64,
        total: Option<u64>,
    },
    PendingCommit {
        folder: String,
        staged: StagedInstall,
        marker: InstalledCommunity,
    },
    Done {
        folder: String,
        version: String,
    },
    Failed {
        folder: String,
        code: String,
        detail: String,
    },
    Cancelled {
        folder: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Downloading,
    Verifying,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Downloading => "downloading",
            Phase::Verifying => "verifying",
        }
    }
}

#[derive(Debug, Clone)]
struct InstalledState {
    version: Option<String>,
    managed: bool,
    update_available: bool,
    folder_path: String,
}

struct State {
    source: CommunitySource,
    status: Status,
    index: Option<CommunityIndex>,
    warnings: Vec<String>,
    fetched_at: Option<u64>,
    from_cache: bool,
    error: Option<ErrorInfo>,
    etag: Option<String>,
    installed: BTreeMap<String, InstalledState>,
    installed_dirty: bool,
    index_fetching: bool,
    job: Job,
}

impl State {
    fn new(source: CommunitySource) -> Self {
        Self {
            source,
            status: Status::Idle,
            index: None,
            warnings: Vec::new(),
            fetched_at: None,
            from_cache: false,
            error: None,
            etag: None,
            installed: BTreeMap::new(),
            installed_dirty: true,
            index_fetching: false,
            job: Job::Idle,
        }
    }
}

pub fn init(paths: Paths) {
    let _ = PATHS.set(paths.clone());
    let source = resolve_source();
    let mut state = State::new(source);
    if let Some((body, meta)) = octos_storage::load_index_cache(&paths, &state.source.index_url) {
        match parse_index(&body, &state.source) {
            Ok(parsed) => {
                state.index = Some(parsed.index);
                state.warnings = parsed.warnings;
                state.fetched_at = Some(meta.fetched_at);
                state.from_cache = true;
                state.etag = meta.etag;
                state.status = Status::Ready;
                state.installed_dirty = true;
                info!(
                    "loaded cached community index ({} items)",
                    state
                        .index
                        .as_ref()
                        .map(|index| index.items.len())
                        .unwrap_or(0)
                );
            }
            Err(err) => warn!("cached community index is unusable: {err}"),
        }
    }
    let source_url = state.source.index_url.clone();
    let cached_index = state.index.clone();
    *STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(state);
    info!("community source: {source_url}");
    if let Some(index) = cached_index {
        queue_preview_cache(&paths, &index);
    }
    request_index(&paths, false);
}

fn resolve_source() -> CommunitySource {
    let mut source = CommunitySource::from_env();
    let env_override = std::env::var("OCTOS_COMMUNITY_URL")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if env_override {
        return source;
    }
    if let Some(preferences) = crate::PREFERENCES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        && let Some(url) = preferences
            .app_options
            .community_index_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
    {
        source.index_url = url.to_string();
    }
    source
}

pub fn set_source(paths: &Paths, url: &str) -> Result<()> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(anyhow!(
            "community index URL must start with http:// or https://"
        ));
    }
    {
        let mut guard = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = guard.as_mut() else {
            return Err(anyhow!("community state is not initialized"));
        };
        state.source.index_url = url.to_string();
        state.index = None;
        state.status = Status::Idle;
        state.fetched_at = None;
        state.etag = None;
        state.from_cache = false;
        state.error = None;
        state.warnings.clear();
        state.installed.clear();
        state.installed_dirty = true;
    }
    request_index(paths, true);
    Ok(())
}

pub fn request_index(paths: &Paths, refresh: bool) {
    {
        let mut guard = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = guard.as_mut() else {
            return;
        };
        if state.index_fetching {
            return;
        }
        if !refresh && state.index.is_some() {
            return;
        }
        state.index_fetching = true;
        if state.index.is_none() {
            state.status = Status::Loading;
        }
    }
    let paths = paths.clone();
    let spawned = thread::Builder::new()
        .name("octos-community-index".to_string())
        .spawn(move || fetch_index(&paths));
    if spawned.is_err()
        && let Some(state) = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_mut()
    {
        state.index_fetching = false;
    }
}

fn fetch_index(paths: &Paths) {
    let (url, etag, source) = {
        let guard = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = guard.as_ref() else {
            return;
        };
        (
            state.source.index_url.clone(),
            state.etag.clone(),
            state.source.clone(),
        )
    };
    let mut headers = Vec::new();
    if let Some(etag) = etag {
        headers.push(("If-None-Match".to_string(), etag));
    }
    let cancel = AtomicBool::new(false);
    let agent = format!("Octocaris/{}", octos_core::VERSION);
    let result = octos_win::net::http_execute(
        &url,
        &agent,
        &headers,
        None,
        MAX_INDEX_BYTES,
        &cancel,
        &mut |_, _| {},
    );
    let now = octos_storage::community::now_seconds();

    let mut preview_index = None;
    let waiting_id = {
        let mut guard = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = guard.as_mut() else {
            return;
        };
        state.index_fetching = false;
        let waiting = match &state.job {
            Job::WaitingForIndex { folder } => Some(folder.clone()),
            _ => None,
        };
        match result {
            Ok(response) if response.status == 304 => {
                state.fetched_at = Some(now);
                state.from_cache = false;
                state.error = None;
                state.status = Status::Ready;
                state.etag = response.etag.or(state.etag.take());
                info!("community index not modified (304)");
            }
            Ok(response) => {
                let body = String::from_utf8_lossy(&response.body).to_string();
                match parse_index(&body, &source) {
                    Ok(parsed) => {
                        let item_count = parsed.index.items.len();
                        if let Err(err) = octos_storage::store_index_cache(
                            paths,
                            &body,
                            response.etag.as_deref(),
                            response.last_modified.as_deref(),
                            &source.index_url,
                            item_count,
                        ) {
                            warn!("could not cache community index: {err}");
                        }
                        preview_index = Some(parsed.index.clone());
                        state.index = Some(parsed.index);
                        state.warnings = parsed.warnings;
                        state.fetched_at = Some(now);
                        state.from_cache = false;
                        state.error = None;
                        state.status = Status::Ready;
                        state.etag = response.etag;
                        state.installed_dirty = true;
                        info!("community index loaded: {item_count} items");
                    }
                    Err(err) => {
                        let info = ErrorInfo {
                            code: "invalid-index".to_string(),
                            detail: err.to_string(),
                        };
                        if state.index.is_some() {
                            state.error = Some(info);
                            state.from_cache = true;
                            state.status = Status::Ready;
                        } else {
                            state.status = Status::Error;
                            state.error = Some(info);
                        }
                        warn!("community index rejected: {err}");
                    }
                }
            }
            Err(err) => {
                let info = ErrorInfo {
                    code: net_error_code(&err).to_string(),
                    detail: err.to_string(),
                };
                if state.index.is_some() {
                    state.error = Some(info);
                    state.from_cache = true;
                    state.status = Status::Ready;
                } else {
                    state.status = Status::Error;
                    state.error = Some(info);
                }
                warn!("community index fetch failed: {err}");
            }
        }
        waiting
    };
    octos_win::notify_community_changed();
    if let Some(index) = preview_index {
        queue_preview_cache(paths, &index);
    }
    if let Some(folder) = waiting_id
        && let Err(err) = start_install(paths, &folder)
    {
        warn!("queued community install failed to start: {err:?}");
    }
}

/// Download card thumbnails into `data/community/previews` once per URL. The UI
/// then loads them from `https://cache.octos/previews/...`, so browsing the
/// catalog does not keep hitting raw.githubusercontent.com.
fn queue_preview_cache(paths: &Paths, index: &CommunityIndex) {
    let urls: Vec<String> = index
        .items
        .iter()
        .filter_map(|item| item.preview.as_ref().map(|preview| preview.url.clone()))
        .filter(|url| !octos_storage::preview_cache_path(paths, url).is_file())
        .collect();
    if urls.is_empty() {
        return;
    }
    let paths = paths.clone();
    let _ = thread::Builder::new()
        .name("octos-community-previews".to_string())
        .spawn(move || {
            let agent = format!("Octocaris/{}", octos_core::VERSION);
            let cancel = AtomicBool::new(false);
            let mut downloaded = 0usize;
            for url in urls {
                let path = octos_storage::preview_cache_path(&paths, &url);
                let temporary = path.with_extension("gif.part");
                let result = octos_win::net::http_execute(
                    &url,
                    &agent,
                    &[],
                    Some(&temporary),
                    8 * 1024 * 1024,
                    &cancel,
                    &mut |_, _| {},
                );
                match result {
                    Ok(response) if response.bytes > 0 => {
                        if std::fs::rename(&temporary, &path).is_ok() {
                            downloaded += 1;
                        }
                    }
                    _ => {
                        let _ = std::fs::remove_file(&temporary);
                    }
                }
            }
            if downloaded > 0 {
                info!("cached {downloaded} community preview(s)");
                octos_win::notify_community_changed();
            }
        });
}

fn cached_preview_url(paths: &Paths, url: &str) -> String {
    let path = octos_storage::preview_cache_path(paths, url);
    if path.is_file()
        && let Some(name) = path.file_name().and_then(|name| name.to_str())
    {
        return format!("https://cache.octos/previews/{name}");
    }
    url.to_string()
}

pub fn start_install(paths: &Paths, folder: &str) -> Result<()> {
    let mut needs_index = false;
    let item = {
        let mut guard = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = guard.as_mut() else {
            return Err(anyhow!("community state is not initialized"));
        };
        if matches!(
            state.job,
            Job::Downloading { .. } | Job::PendingCommit { .. }
        ) {
            return Err(anyhow!("busy"));
        }
        match state
            .index
            .as_ref()
            .and_then(|index| find_item(index, folder))
        {
            Some(item) => {
                if !app_supports(item.min_app_version.as_deref(), octos_core::VERSION) {
                    return Err(anyhow!("app-too-old"));
                }
                let item = item.clone();
                state.job = Job::Downloading {
                    folder: item.folder.clone(),
                    phase: Phase::Downloading,
                    received: 0,
                    total: Some(item.download.bytes),
                };
                Some(item)
            }
            None if state.index.is_none() => {
                state.job = Job::WaitingForIndex {
                    folder: folder.to_string(),
                };
                needs_index = true;
                None
            }
            None => return Err(anyhow!("unknown-item")),
        }
    };
    if needs_index {
        request_index(paths, true);
        return Ok(());
    }
    let Some(item) = item else {
        return Err(anyhow!("unknown-item"));
    };
    CANCEL_DOWNLOAD.store(false, Ordering::SeqCst);
    info!(
        "community install started: {} v{}",
        item.folder, item.version
    );
    let paths = paths.clone();
    if thread::Builder::new()
        .name("octos-community-download".to_string())
        .spawn(move || install_worker(paths, item))
        .is_err()
    {
        set_job_failed("network", "could not start the download thread");
        octos_win::notify_community_changed();
    }
    Ok(())
}

fn install_worker(paths: Paths, item: CommunityItem) {
    info!("community download starting: {}", item.download.url);
    let agent = format!("Octocaris/{}", octos_core::VERSION);
    let part = octos_storage::part_path(&paths, &item);
    let target = octos_storage::download_path(&paths, &item);
    let mut last_notify = Instant::now() - Duration::from_secs(1);

    let response = octos_win::net::http_execute(
        &item.download.url,
        &agent,
        &[],
        Some(&part),
        MAX_ARCHIVE_BYTES,
        &CANCEL_DOWNLOAD,
        &mut |received, total| {
            update_progress(Phase::Downloading, received, total);
            if last_notify.elapsed() >= PROGRESS_INTERVAL {
                last_notify = Instant::now();
                octos_win::notify_community_changed();
            }
        },
    );

    let response = match response {
        Ok(response) => {
            info!("community download finished: {} bytes", response.bytes);
            response
        }
        Err(error) => {
            let _ = std::fs::remove_file(&part);
            if matches!(error, octos_win::net::NetError::Cancelled) {
                info!("community download cancelled: {}", item.folder);
                set_job(Job::Cancelled {
                    folder: item.folder.clone(),
                });
            } else {
                set_job_failed(net_error_code(&error), &error.to_string());
            }
            octos_win::notify_community_changed();
            return;
        }
    };

    if response.bytes != item.download.bytes {
        let _ = std::fs::remove_file(&part);
        set_job_failed(
            "size-mismatch",
            &format!(
                "expected {} bytes, received {}",
                item.download.bytes, response.bytes
            ),
        );
        octos_win::notify_community_changed();
        return;
    }

    update_progress(Phase::Verifying, response.bytes, Some(response.bytes));
    octos_win::notify_community_changed();
    if let Err(error) = octos_storage::verify_file(&part, &item.download.sha256) {
        let _ = std::fs::remove_file(&part);
        let detail = match error {
            HashError::Mismatch { expected, actual } => {
                format!("expected {expected}, got {actual}")
            }
            other => other.to_string(),
        };
        set_job_failed("hash-mismatch", &detail);
        octos_win::notify_community_changed();
        return;
    }

    let _ = std::fs::remove_file(&target);
    if let Err(error) = std::fs::rename(&part, &target) {
        let _ = std::fs::remove_file(&part);
        set_job_failed("install", &format!("could not move the archive: {error}"));
        octos_win::notify_community_changed();
        return;
    }

    let staged =
        match StagedInstall::stage(&target, &paths.wallpapers, &item.folder, &CANCEL_DOWNLOAD) {
            Ok(staged) => staged,
            Err(error) => {
                let _ = std::fs::remove_file(&target);
                let code = if matches!(error, octos_storage::InstallError::Cancelled) {
                    "cancelled"
                } else {
                    "install"
                };
                set_job_failed(code, &error.to_string());
                octos_win::notify_community_changed();
                return;
            }
        };
    let marker = InstalledCommunity {
        folder: item.folder.clone(),
        version: item.version.clone(),
        sha256: item.download.sha256.clone(),
        installed_at: octos_storage::community::now_seconds(),
        source_url: item
            .source
            .as_ref()
            .and_then(|source| source.workshop_url.clone()),
    };
    if let Err(error) = staged.write_marker(&marker) {
        staged.discard();
        let _ = std::fs::remove_file(&target);
        set_job_failed("install", &format!("could not write the marker: {error}"));
        octos_win::notify_community_changed();
        return;
    }
    let _ = std::fs::remove_file(&target);

    set_job(Job::PendingCommit {
        folder: item.folder.clone(),
        staged,
        marker,
    });
    octos_win::notify_community_changed();
}

fn update_progress(phase: Phase, received: u64, total: Option<u64>) {
    let mut guard = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(state) = guard.as_mut()
        && let Job::Downloading {
            phase: current,
            received: current_received,
            total: current_total,
            ..
        } = &mut state.job
    {
        *current = phase;
        *current_received = received;
        *current_total = total;
    }
}

fn set_job(job: Job) {
    if let Some(state) = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_mut()
    {
        state.job = job;
    }
}

fn set_job_failed(code: &str, detail: &str) {
    warn!("community install failed: {code}: {detail}");
    let folder = {
        let guard = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .as_ref()
            .and_then(|state| match &state.job {
                Job::Downloading { folder, .. } => Some(folder.clone()),
                Job::PendingCommit { folder, .. } => Some(folder.clone()),
                _ => None,
            })
            .unwrap_or_default()
    };
    set_job(Job::Failed {
        folder,
        code: code.to_string(),
        detail: detail.to_string(),
    });
}

pub fn cancel_download() {
    CANCEL_DOWNLOAD.store(true, Ordering::SeqCst);
}

pub fn shutdown() {
    CANCEL_DOWNLOAD.store(true, Ordering::SeqCst);
}

/// UI-thread callback for `WM_APP_COMMUNITY`.
pub fn on_ui_wake(paths: &Paths) {
    let pending = {
        let mut guard = STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = guard.as_mut() else {
            return;
        };
        if matches!(state.job, Job::PendingCommit { .. }) {
            let job = std::mem::replace(&mut state.job, Job::Idle);
            match job {
                Job::PendingCommit {
                    folder,
                    staged,
                    marker,
                } => Some((folder, staged, marker)),
                _ => None,
            }
        } else {
            None
        }
    };
    match pending {
        Some((folder, staged, marker)) => commit_install(paths, folder, staged, marker),
        None => push(paths),
    }
}

fn commit_install(
    paths: &Paths,
    folder: String,
    staged: StagedInstall,
    marker: InstalledCommunity,
) {
    let folder_key = paths.wallpapers.join(&folder).to_string_lossy().to_string();
    let affected: Vec<String> = octos_win::wallpaper::selected_monitors()
        .into_iter()
        .filter(|(_, key)| key == &folder_key)
        .map(|(monitor, _)| monitor)
        .collect();

    // Flush live option values before touching the folder, then release the
    // WebView file handles so the directory can be renamed.
    crate::save_preferences(paths);
    for monitor in &affected {
        let _ = octos_win::wallpaper::clear_monitor_mod(monitor);
    }

    match staged.commit(&paths.wallpapers, &folder, InstallMode::Replace) {
        Ok(_) => {
            for monitor in &affected {
                if let Err(err) = crate::set_monitor_mod(paths, monitor, &folder_key) {
                    warn!("could not restore wallpaper on {monitor}: {err:?}");
                }
            }
            crate::rotation::reset_schedule();
            crate::rotation::sanitize_items(paths);
            crate::save_preferences(paths);
            crate::push_library(paths);
            crate::backfill_thumbnails(paths);
            invalidate_installed();
            set_job(Job::Done {
                folder,
                version: marker.version,
            });
            info!("community install committed: {}", marker.folder);
        }
        Err(err) => {
            for monitor in &affected {
                let _ = crate::set_monitor_mod(paths, monitor, &folder_key);
            }
            let code = if matches!(err, octos_storage::InstallError::Locked(_)) {
                "locked"
            } else {
                "install"
            };
            set_job(Job::Failed {
                folder,
                code: code.to_string(),
                detail: err.to_string(),
            });
            error!("community install failed: {err:?}");
        }
    }
    octos_win::notify_community_changed();
}

pub fn on_library_changed(_paths: &Paths) {
    invalidate_installed();
}

fn invalidate_installed() {
    if let Some(state) = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_mut()
    {
        state.installed_dirty = true;
    }
}

pub fn push(paths: &Paths) {
    let snapshot = snapshot(paths);
    match serde_json::to_string(&UiResponse::Community { data: snapshot }) {
        Ok(payload) => octos_win::post_to_main_webview(&payload),
        Err(err) => warn!("could not encode community snapshot: {err}"),
    }
}

pub fn page_url(folder: &str) -> Option<String> {
    let guard = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state = guard.as_ref()?;
    let index = state.index.as_ref()?;
    let item = find_item(index, folder)?;
    if let Some(page) = &item.page_url {
        return Some(page.clone());
    }
    let repo = index.repository.as_deref()?;
    Some(format!("{repo}/blob/main/README.md#{folder}"))
}

pub fn source_url(folder: &str) -> Option<String> {
    let guard = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state = guard.as_ref()?;
    let item = find_item(state.index.as_ref()?, folder)?;
    item.source
        .as_ref()
        .and_then(|source| source.workshop_url.clone())
}

pub fn snapshot(paths: &Paths) -> Value {
    let mut guard = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(state) = guard.as_mut() else {
        return json!({
            "status": "idle",
            "source": null,
            "items": [],
            "job": Value::Null,
        });
    };
    if state.installed_dirty {
        refresh_installed(state, &paths.wallpapers);
        state.installed_dirty = false;
    }

    let items: Vec<Value> = state
        .index
        .as_ref()
        .map(|index| {
            index
                .items
                .iter()
                .map(|item| item_json(item, state.installed.get(&item.folder), paths))
                .collect()
        })
        .unwrap_or_default();

    json!({
        "status": state.status.as_str(),
        "source": state.source.index_url.clone(),
        "fetched_at": state.fetched_at,
        "from_cache": state.from_cache,
        "error": state.error.as_ref().map(|error| json!({
            "code": error.code,
            "detail": error.detail,
        })),
        "warnings": state.warnings.clone(),
        "job": job_json(&state.job),
        "items": items,
    })
}

fn item_json(item: &CommunityItem, installed: Option<&InstalledState>, paths: &Paths) -> Value {
    json!({
        "folder": item.folder.clone(),
        "folder": item.folder.clone(),
        "name": item.name.clone(),
        "author": item.author.clone(),
        "description": item.description.clone(),
        "version": item.version.clone(),
        "tags": item.tags.clone(),
        "entry": item.entry.clone(),
        "license": item.license.clone(),
        "min_app_version": item.min_app_version.clone(),
        "preview_url": item
            .preview
            .as_ref()
            .map(|preview| cached_preview_url(paths, &preview.url)),
        "preview_bytes": item.preview.as_ref().and_then(|preview| preview.bytes),
        "download_bytes": item.download.bytes,
        "sha256": item.download.sha256.clone(),
        "file_count": item.file_count.unwrap_or(item.files.len()),
        "files_truncated": item.files_truncated,
        "files": item.files.iter().map(|file| json!({
            "path": file.path,
            "bytes": file.bytes,
        })).collect::<Vec<_>>(),
        "page_url": item.page_url.clone(),
        "workshop_url": item.source.as_ref().and_then(|source| source.workshop_url.clone()),
        "updated_at": item.updated_at.clone(),
        "installed": installed.map(|installed| json!({
            "version": installed.version,
            "managed": installed.managed,
            "update_available": installed.update_available,
            "folder_path": installed.folder_path,
        })),
    })
}

fn job_json(job: &Job) -> Value {
    match job {
        Job::Idle => Value::Null,
        Job::WaitingForIndex { folder } => json!({"folder": folder, "phase": "waiting"}),
        Job::Downloading {
            folder,
            phase,
            received,
            total,
        } => json!({
            "folder": folder,
            "phase": phase.as_str(),
            "received": received,
            "total": total,
        }),
        Job::PendingCommit { folder, .. } => json!({"folder": folder, "phase": "installing"}),
        Job::Done { folder, version } => {
            json!({"folder": folder, "phase": "done", "version": version})
        }
        Job::Failed {
            folder,
            code,
            detail,
        } => json!({
            "folder": folder,
            "phase": "failed",
            "error": {"code": code, "detail": detail},
        }),
        Job::Cancelled { folder } => json!({"folder": folder, "phase": "cancelled"}),
    }
}

fn refresh_installed(state: &mut State, wallpapers: &Path) {
    state.installed.clear();
    let index = state.index.take();
    if let Some(index) = &index {
        for item in &index.items {
            let destination = wallpapers.join(&item.folder);
            if !destination.is_dir() {
                continue;
            }
            let marker = octos_storage::read_marker(&destination);
            let version = marker
                .as_ref()
                .map(|marker| marker.version.clone())
                .or_else(|| manifest_version(&destination));
            let update_available = version
                .as_deref()
                .map(|installed| is_update(installed, &item.version))
                .unwrap_or(false);
            state.installed.insert(
                item.folder.clone(),
                InstalledState {
                    version,
                    managed: marker.is_some(),
                    update_available,
                    folder_path: destination.to_string_lossy().to_string(),
                },
            );
        }
    }
    state.index = index;
}

fn manifest_version(folder: &Path) -> Option<String> {
    for name in ["octos.json", "mod.json"] {
        let text = std::fs::read_to_string(folder.join(name)).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        if let Some(version) = value.get("version").and_then(Value::as_str) {
            return Some(version.to_string());
        }
    }
    None
}

fn net_error_code(error: &octos_win::net::NetError) -> &'static str {
    match error {
        octos_win::net::NetError::Cancelled => "cancelled",
        octos_win::net::NetError::Status { .. } => "http-status",
        octos_win::net::NetError::RateLimited { .. } => "rate-limited",
        octos_win::net::NetError::TooLarge { .. } => "too-large",
        _ => "network",
    }
}
