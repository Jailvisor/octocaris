//! Community catalog cache, marker files and download paths.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use octos_core::community::{CommunityItem, InstalledCommunity, MARKER_FILE};
use serde::{Deserialize, Serialize};

use crate::Paths;
use crate::atomic::write_atomic;
use crate::hash::sha256_bytes;

const INDEX_FILE: &str = "index.json";
const INDEX_META_FILE: &str = "index-meta.json";
const DOWNLOADS_DIR: &str = "downloads";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexCacheMeta {
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    pub fetched_at: u64,
    pub sha256: String,
    pub item_count: usize,
    pub source_url: String,
}

pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

pub fn index_path(paths: &Paths) -> PathBuf {
    paths.community.join(INDEX_FILE)
}

pub fn index_meta_path(paths: &Paths) -> PathBuf {
    paths.community.join(INDEX_META_FILE)
}

pub fn downloads_dir(paths: &Paths) -> PathBuf {
    paths.community.join(DOWNLOADS_DIR)
}

pub fn previews_dir(paths: &Paths) -> PathBuf {
    paths.community.join("previews")
}

/// Stable local name for a remote preview URL (thumbnails only).
pub fn preview_cache_name(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let extension = if path.ends_with(".png") {
        "png"
    } else if path.ends_with(".gif") {
        "gif"
    } else {
        "jpg"
    };
    format!("{}.{extension}", sha256_bytes(url.as_bytes()))
}

pub fn preview_cache_path(paths: &Paths, url: &str) -> PathBuf {
    previews_dir(paths).join(preview_cache_name(url))
}

/// Return the cached index only when it matches both the recorded hash and source.
pub fn load_index_cache(paths: &Paths, source_url: &str) -> Option<(String, IndexCacheMeta)> {
    let body = fs::read_to_string(index_path(paths)).ok()?;
    let meta_text = fs::read_to_string(index_meta_path(paths)).ok()?;
    let meta: IndexCacheMeta = serde_json::from_str(&meta_text).ok()?;
    if meta.source_url != source_url {
        return None;
    }
    if sha256_bytes(body.as_bytes()) != meta.sha256 {
        return None;
    }
    Some((body, meta))
}

pub fn store_index_cache(
    paths: &Paths,
    body: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
    source_url: &str,
    item_count: usize,
) -> io::Result<IndexCacheMeta> {
    fs::create_dir_all(&paths.community)?;
    let meta = IndexCacheMeta {
        etag: etag.map(str::to_string),
        last_modified: last_modified.map(str::to_string),
        fetched_at: now_seconds(),
        sha256: sha256_bytes(body.as_bytes()),
        item_count,
        source_url: source_url.to_string(),
    };
    write_atomic(&index_path(paths), body.as_bytes())?;
    let encoded = serde_json::to_vec_pretty(&meta).map_err(io::Error::other)?;
    write_atomic(&index_meta_path(paths), &encoded)?;
    Ok(meta)
}

pub fn read_marker(folder: &Path) -> Option<InstalledCommunity> {
    let text = fs::read_to_string(folder.join(MARKER_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_marker(folder: &Path, marker: &InstalledCommunity) -> io::Result<()> {
    let encoded = serde_json::to_vec_pretty(marker).map_err(io::Error::other)?;
    write_atomic(&folder.join(MARKER_FILE), &encoded)
}

pub fn download_file_name(item: &CommunityItem) -> String {
    format!("octocaris-{}-v{}.zip", item.folder, item.version)
}

pub fn download_path(paths: &Paths, item: &CommunityItem) -> PathBuf {
    downloads_dir(paths).join(download_file_name(item))
}

pub fn part_path(paths: &Paths, item: &CommunityItem) -> PathBuf {
    downloads_dir(paths).join(format!("{}.part", download_file_name(item)))
}

/// Remove partial downloads left by cancelled or crashed installs.
pub fn cleanup_community(paths: &Paths) -> usize {
    let Ok(entries) = fs::read_dir(downloads_dir(paths)) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".part") && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(root: &Path) -> Paths {
        let root = root.to_path_buf();
        Paths {
            wallpapers: root.join("wallpapers"),
            logs: root.join("logs"),
            webview_data: root.join("webview2"),
            preferences: root.join("preferences.json"),
            community: root.join("community"),
            exe_dir: root.clone(),
            root,
        }
    }

    #[test]
    fn cache_round_trips_and_rejects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        paths.ensure().unwrap();
        let body = r#"{"schemaVersion":1,"items":[]}"#;
        let meta =
            store_index_cache(&paths, body, Some("etag"), None, "https://x/index.json", 0).unwrap();
        assert_eq!(meta.item_count, 0);
        let (cached, cached_meta) =
            load_index_cache(&paths, "https://x/index.json").expect("cache");
        assert_eq!(cached, body);
        assert_eq!(cached_meta.etag.as_deref(), Some("etag"));
        assert!(load_index_cache(&paths, "https://y/index.json").is_none());

        fs::write(index_path(&paths), "{}\n").unwrap();
        assert!(load_index_cache(&paths, "https://x/index.json").is_none());
    }

    #[test]
    fn marker_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let marker = InstalledCommunity {
            folder: "1".into(),
            version: "1.0.0".into(),
            sha256: "a".repeat(64),
            installed_at: 7,
            source_url: None,
        };
        write_marker(dir.path(), &marker).unwrap();
        assert_eq!(read_marker(dir.path()), Some(marker));
    }

    #[test]
    fn preview_cache_names_follow_the_url_version() {
        let first = preview_cache_name("https://x/items/a/thumb.jpg?v=11111111");
        let second = preview_cache_name("https://x/items/a/thumb.jpg?v=22222222");
        assert_ne!(first, second);
        assert_eq!(
            first,
            preview_cache_name("https://x/items/a/thumb.jpg?v=11111111")
        );
        assert!(first.ends_with(".jpg"));
    }

    #[test]
    fn cleans_partial_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        paths.ensure().unwrap();
        fs::create_dir_all(downloads_dir(&paths)).unwrap();
        fs::write(downloads_dir(&paths).join("a.zip.part"), b"x").unwrap();
        fs::write(downloads_dir(&paths).join("a.zip"), b"x").unwrap();
        assert_eq!(cleanup_community(&paths), 1);
        assert!(!downloads_dir(&paths).join("a.zip.part").exists());
        assert!(downloads_dir(&paths).join("a.zip").exists());
    }
}
