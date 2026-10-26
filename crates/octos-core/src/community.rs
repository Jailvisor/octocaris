//! Remote wallpaper catalog used by the Community tab.
//!
//! The index is a static JSON file hosted on a public repository. Everything in
//! this module is platform independent and covered by Linux unit tests; network
//! I/O and installation live in `octos-win` / `octos-storage` / `octos-app`.

use std::cmp::Ordering;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

pub const INDEX_SCHEMA_VERSION: u32 = 1;
pub const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ITEMS: usize = 10_000;
pub const MAX_FILES_PER_ITEM: usize = 1_000;

/// Marker written into a community-installed folder.
pub const MARKER_FILE: &str = "octos-community.json";

pub const DEFAULT_INDEX_URL: &str =
    "https://raw.githubusercontent.com/Jailvisor/octocaris-community/main/index.json";

const RESERVED_FOLDERS: [&str; 4] = ["default", "gradient", "conformance", "community"];
const DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Debug, Clone)]
pub struct CommunitySource {
    pub index_url: String,
    pub allow_insecure_loopback: bool,
}

impl CommunitySource {
    pub fn default_index() -> Self {
        Self {
            index_url: DEFAULT_INDEX_URL.to_string(),
            allow_insecure_loopback: false,
        }
    }

    /// `OCTOS_COMMUNITY_URL` overrides the catalog; `OCTOS_COMMUNITY_ALLOW_INSECURE=1`
    /// permits loopback `http://` catalogs (local end-to-end tests).
    pub fn from_env() -> Self {
        let index_url = std::env::var("OCTOS_COMMUNITY_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_INDEX_URL.to_string());
        let allow_insecure_loopback = std::env::var("OCTOS_COMMUNITY_ALLOW_INSECURE")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Self {
            index_url,
            allow_insecure_loopback,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommunityIndex {
    pub schema_version: u32,
    pub generated_at: Option<String>,
    pub repository: Option<String>,
    pub release_tag: Option<String>,
    pub asset_base_url: Option<String>,
    pub items: Vec<CommunityItem>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for CommunityIndex {
    fn default() -> Self {
        Self {
            schema_version: INDEX_SCHEMA_VERSION,
            generated_at: None,
            repository: None,
            release_tag: None,
            asset_base_url: None,
            items: Vec::new(),
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommunityItem {
    pub folder: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub entry: Option<String>,
    pub license: Option<String>,
    pub min_app_version: Option<String>,
    pub preview: Option<CommunityPreview>,
    pub download: CommunityDownload,
    pub file_count: Option<usize>,
    pub files_truncated: bool,
    pub files: Vec<CommunityFile>,
    pub source: Option<CommunitySourceInfo>,
    pub page_url: Option<String>,
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for CommunityItem {
    fn default() -> Self {
        Self {
            folder: String::new(),
            name: String::new(),
            author: String::new(),
            description: String::new(),
            version: String::new(),
            tags: Vec::new(),
            entry: None,
            license: None,
            min_app_version: None,
            preview: None,
            download: CommunityDownload::default(),
            file_count: None,
            files_truncated: false,
            files: Vec::new(),
            source: None,
            page_url: None,
            updated_at: None,
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommunityPreview {
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bytes: Option<u64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommunityDownload {
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommunityFile {
    pub path: String,
    pub bytes: u64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommunitySourceInfo {
    pub kind: String,
    pub workshop_id: Option<String>,
    pub workshop_url: Option<String>,
    pub title: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `octos-community.json` inside an installed folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledCommunity {
    pub folder: String,
    pub version: String,
    pub sha256: String,
    #[serde(default)]
    pub installed_at: u64,
    #[serde(default)]
    pub source_url: Option<String>,
}

#[derive(Debug, Default)]
pub struct ParsedIndex {
    pub index: CommunityIndex,
    pub warnings: Vec<String>,
}

#[derive(Debug, Error)]
pub enum CommunityError {
    #[error("community index is too large: {bytes} bytes (limit {limit})")]
    TooLarge { bytes: u64, limit: u64 },
    #[error("unsupported community index schema {found} (supported {supported})")]
    UnsupportedSchema { found: u32, supported: u32 },
    #[error("community index has too many items: {count}")]
    TooManyItems { count: usize },
    #[error("invalid community index JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("insecure URL is not allowed: {0}")]
    InsecureUrl(String),
    #[error("community item not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlScheme {
    Http,
    Https,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpUrl {
    pub scheme: UrlScheme,
    pub host: String,
    pub port: u16,
    pub path_and_query: String,
}

impl HttpUrl {
    pub fn is_loopback(&self) -> bool {
        self.host.eq_ignore_ascii_case("localhost")
            || self.host == "127.0.0.1"
            || self.host == "::1"
            || self.host.eq_ignore_ascii_case("[::1]")
    }
}

pub fn parse_index(text: &str, source: &CommunitySource) -> Result<ParsedIndex, CommunityError> {
    if text.len() as u64 > MAX_INDEX_BYTES {
        return Err(CommunityError::TooLarge {
            bytes: text.len() as u64,
            limit: MAX_INDEX_BYTES,
        });
    }
    let index: CommunityIndex = serde_json::from_str(text)?;
    if index.schema_version != INDEX_SCHEMA_VERSION {
        return Err(CommunityError::UnsupportedSchema {
            found: index.schema_version,
            supported: INDEX_SCHEMA_VERSION,
        });
    }
    if index.items.len() > MAX_ITEMS {
        return Err(CommunityError::TooManyItems {
            count: index.items.len(),
        });
    }

    let mut parsed = ParsedIndex {
        index: CommunityIndex {
            items: Vec::new(),
            ..index
        },
        warnings: Vec::new(),
    };
    let mut folders: HashSet<String> = HashSet::new();
    let mut raw_items = index.items;
    raw_items.sort_by(|left, right| left.folder.cmp(&right.folder));

    for mut item in raw_items {
        match normalize_item(&mut item, source) {
            Ok(()) => {}
            Err(reason) => {
                parsed
                    .warnings
                    .push(format!("skipped community item {}: {reason}", item.folder));
                continue;
            }
        }
        let folder_key = item.folder.to_lowercase();
        if !folders.insert(folder_key) {
            parsed.warnings.push(format!(
                "skipped duplicate community folder {}",
                item.folder
            ));
            continue;
        }
        parsed.index.items.push(item);
    }
    Ok(parsed)
}

fn normalize_item(item: &mut CommunityItem, source: &CommunitySource) -> Result<(), String> {
    if !validate_folder_name(&item.folder) {
        return Err(format!("invalid folder {:?}", item.folder));
    }
    if parse_version(&item.version).is_none() {
        return Err(format!("invalid version {:?}", item.version));
    }
    if item.name.trim().is_empty() {
        item.name = item.folder.clone();
    }
    if let Some(min_app) = &item.min_app_version
        && parse_version(min_app).is_none()
    {
        item.min_app_version = None;
    }

    if !url_allowed(&item.download.url, source) {
        return Err(format!("invalid download URL {:?}", item.download.url));
    }
    if !is_valid_sha256(&item.download.sha256) {
        return Err("invalid download sha256".to_string());
    }
    if item.download.bytes == 0 || item.download.bytes > MAX_ARCHIVE_BYTES {
        return Err(format!("invalid archive size {}", item.download.bytes));
    }

    for preview in [&mut item.preview].into_iter().flatten() {
        let resolved = resolve_url(&source.index_url, &preview.url);
        if !url_allowed(&resolved, source) {
            return Err(format!("invalid preview URL {:?}", preview.url));
        }
        preview.url = resolved;
    }

    if item.files.len() > MAX_FILES_PER_ITEM {
        item.files.truncate(MAX_FILES_PER_ITEM);
        item.files_truncated = true;
    }
    item.files.retain(|file| validate_relative_path(&file.path));
    let sorted = item
        .files
        .windows(2)
        .all(|pair| pair[0].path <= pair[1].path);
    if !sorted {
        item.files.sort_by(|left, right| left.path.cmp(&right.path));
    }
    Ok(())
}

pub fn validate_folder_name(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    if name.starts_with('.') || name.ends_with([' ', '.']) {
        return false;
    }
    if name.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    }) {
        return false;
    }
    if name.split('.').next().is_some_and(|stem| {
        DEVICE_NAMES
            .iter()
            .any(|device| stem.eq_ignore_ascii_case(device))
    }) {
        return false;
    }
    !RESERVED_FOLDERS
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

pub fn validate_relative_path(path: &str) -> bool {
    if path.is_empty() || path.starts_with(['/', '\\']) {
        return false;
    }
    if path.contains(['\\', '\0']) {
        return false;
    }
    if path.chars().any(char::is_control) {
        return false;
    }
    path.split('/').all(|segment| {
        !segment.is_empty() && segment != "." && segment != ".." && !segment.contains(':')
    })
}

pub fn is_valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    let left = parse_version(left)?;
    let right = parse_version(right)?;
    Some(left.cmp(&right))
}

/// True when `available` is strictly newer than `installed` (downgrades are not updates).
pub fn is_update(installed: &str, available: &str) -> bool {
    matches!(
        compare_versions(available, installed),
        Some(Ordering::Greater)
    )
}

/// True when `current` satisfies a catalog `minAppVersion` (missing/invalid passes).
pub fn app_supports(required: Option<&str>, current: &str) -> bool {
    required
        .and_then(|required| compare_versions(current, required))
        .map(|ordering| ordering != Ordering::Less)
        .unwrap_or(true)
}

pub fn resolve_url(base: &str, reference: &str) -> String {
    let reference = reference.trim();
    let lower = reference.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return reference.to_string();
    }
    let base = base.trim();
    let prefix = match base.rfind('/') {
        Some(index) => &base[..=index],
        None => base,
    };
    format!("{prefix}{}", reference.trim_start_matches('/'))
}

pub fn find_item<'a>(index: &'a CommunityIndex, folder: &str) -> Option<&'a CommunityItem> {
    index.items.iter().find(|item| item.folder == folder)
}

pub fn require_item<'a>(
    index: &'a CommunityIndex,
    folder: &str,
) -> Result<&'a CommunityItem, CommunityError> {
    find_item(index, folder).ok_or_else(|| CommunityError::NotFound(folder.to_string()))
}

fn url_allowed(url: &str, source: &CommunitySource) -> bool {
    match parse_http_url(url) {
        Ok(parsed) => match parsed.scheme {
            UrlScheme::Https => true,
            UrlScheme::Http => source.allow_insecure_loopback && parsed.is_loopback(),
        },
        Err(_) => false,
    }
}

pub fn parse_http_url(url: &str) -> Result<HttpUrl, CommunityError> {
    let trimmed = url.trim();
    if trimmed.len() != url.len() || trimmed.is_empty() {
        return Err(CommunityError::InvalidUrl(url.to_string()));
    }
    if trimmed
        .chars()
        .any(|character| character.is_control() || character == ' ')
    {
        return Err(CommunityError::InvalidUrl(url.to_string()));
    }
    let lower = trimmed.to_ascii_lowercase();
    let (scheme, rest) = if let Some(rest) = lower
        .strip_prefix("https://")
        .map(|_| &trimmed["https://".len()..])
    {
        (UrlScheme::Https, rest)
    } else if let Some(rest) = lower
        .strip_prefix("http://")
        .map(|_| &trimmed["http://".len()..])
    {
        (UrlScheme::Http, rest)
    } else {
        return Err(CommunityError::InvalidUrl(url.to_string()));
    };

    let rest = rest.split('#').next().unwrap_or(rest);
    if rest.is_empty() {
        return Err(CommunityError::InvalidUrl(url.to_string()));
    }
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let remainder = &rest[authority_end..];
    if authority.is_empty() || authority.contains('@') {
        return Err(CommunityError::InvalidUrl(url.to_string()));
    }

    let (host, port_text) = if let Some(stripped) = authority.strip_prefix('[') {
        let end = stripped
            .find(']')
            .ok_or_else(|| CommunityError::InvalidUrl(url.to_string()))?;
        let host = &stripped[..end];
        let after = &stripped[end + 1..];
        let port = after.strip_prefix(':');
        if port.is_none() && !after.is_empty() {
            return Err(CommunityError::InvalidUrl(url.to_string()));
        }
        (host.to_string(), port.map(str::to_string))
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => {
                if port.is_empty() || !port.chars().all(|character| character.is_ascii_digit()) {
                    return Err(CommunityError::InvalidUrl(url.to_string()));
                }
                (host.to_string(), Some(port.to_string()))
            }
            None => (authority.to_string(), None),
        }
    };
    if host.is_empty()
        || !host.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '.' | '_' | ':')
        })
    {
        return Err(CommunityError::InvalidUrl(url.to_string()));
    }

    let port = match port_text {
        Some(text) => {
            let value: u16 = text
                .parse()
                .map_err(|_| CommunityError::InvalidUrl(url.to_string()))?;
            if value == 0 {
                return Err(CommunityError::InvalidUrl(url.to_string()));
            }
            value
        }
        None => match scheme {
            UrlScheme::Https => 443,
            UrlScheme::Http => 80,
        },
    };

    let path_and_query = if remainder.is_empty() {
        "/".to_string()
    } else if remainder.starts_with('?') {
        format!("/{remainder}")
    } else {
        remainder.to_string()
    };

    Ok(HttpUrl {
        scheme,
        host,
        port,
        path_and_query,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/community-index.json");

    fn source() -> CommunitySource {
        CommunitySource {
            index_url:
                "https://raw.githubusercontent.com/Jailvisor/octocaris-community/main/index.json"
                    .to_string(),
            allow_insecure_loopback: false,
        }
    }

    #[test]
    fn parses_fixture_and_resolves_previews() {
        let parsed = parse_index(FIXTURE, &source()).unwrap();
        assert_eq!(parsed.index.items.len(), 2);
        assert!(parsed.warnings.is_empty());
        let item = find_item(&parsed.index, "2961764492").unwrap();
        assert_eq!(item.folder, "2961764492");
        assert_eq!(
            item.preview.as_ref().unwrap().url,
            "https://raw.githubusercontent.com/Jailvisor/octocaris-community/main/previews/2961764492.gif?v=b1949134"
        );
        assert_eq!(item.files.len(), 2);
        assert!(find_item(&parsed.index, "image-demo").is_some());
        assert!(find_item(&parsed.index, "we-2961764492").is_none());
    }

    #[test]
    fn rejects_schema_and_bad_json() {
        let wrong = FIXTURE.replace("\"schemaVersion\": 1", "\"schemaVersion\": 2");
        assert!(matches!(
            parse_index(&wrong, &source()),
            Err(CommunityError::UnsupportedSchema { found: 2, .. })
        ));
        assert!(matches!(
            parse_index("{", &source()),
            Err(CommunityError::Json(_))
        ));
    }

    #[test]
    fn rejects_oversized_index() {
        let mut text = String::with_capacity(MAX_INDEX_BYTES as usize + 1);
        text.push_str(&" ".repeat(MAX_INDEX_BYTES as usize + 1));
        assert!(matches!(
            parse_index(&text, &source()),
            Err(CommunityError::TooLarge { .. })
        ));
    }

    #[test]
    fn skips_invalid_items_but_keeps_the_rest() {
        let text = r#"{
          "schemaVersion": 1,
          "items": [
            {"folder":"good","name":"Good","version":"1.0.0",
             "download":{"url":"https://example.com/a.zip","bytes":10,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
             "files":[]},
            {"folder":"CON","name":"Bad","version":"1.0.0",
             "download":{"url":"https://example.com/c.zip","bytes":10,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
             "files":[]},
            {"folder":"bad3","name":"Bad","version":"1.0",
             "download":{"url":"https://example.com/d.zip","bytes":10,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
             "files":[]},
            {"folder":"bad4","name":"Bad","version":"1.0.0",
             "download":{"url":"https://example.com/e.zip","bytes":10,"sha256":"nope"},
             "files":[]},
            {"folder":"bad5","name":"Bad","version":"1.0.0",
             "download":{"url":"http://example.com/f.zip","bytes":10,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
             "files":[]}
          ]
        }"#;
        let parsed = parse_index(text, &source()).unwrap();
        assert_eq!(parsed.index.items.len(), 1);
        assert_eq!(parsed.index.items[0].folder, "good");
        assert_eq!(parsed.warnings.len(), 4);
    }

    #[test]
    fn loopback_http_requires_the_test_override() {
        let text = r#"{
          "schemaVersion": 1,
          "items": [{"id":"demo","folder":"demo","name":"Demo","version":"1.0.0",
            "download":{"url":"http://127.0.0.1:8765/demo.zip","bytes":10,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "files":[]}]
        }"#;
        assert!(parse_index(text, &source()).unwrap().index.items.is_empty());
        let mut local = source();
        local.allow_insecure_loopback = true;
        assert_eq!(parse_index(text, &local).unwrap().index.items.len(), 1);
    }

    #[test]
    fn skips_duplicate_folders_case_insensitively() {
        let text = r#"{
          "schemaVersion": 1,
          "items": [
            {"folder":"Same","name":"One","version":"1.0.0",
             "download":{"url":"https://example.com/a.zip","bytes":10,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"files":[]},
            {"folder":"same","name":"Two","version":"1.0.0",
             "download":{"url":"https://example.com/b.zip","bytes":10,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"files":[]}
          ]
        }"#;
        let parsed = parse_index(text, &source()).unwrap();
        assert_eq!(parsed.index.items.len(), 1);
        assert_eq!(parsed.warnings.len(), 1);
    }

    #[test]
    fn version_compare_and_updates() {
        assert_eq!(compare_versions("1.0.0", "1.0.1"), Some(Ordering::Less));
        assert_eq!(compare_versions("1.2.0", "1.10.0"), Some(Ordering::Less));
        assert_eq!(compare_versions("2.0.0", "2.0.0"), Some(Ordering::Equal));
        assert_eq!(compare_versions("1.0", "1.0.0"), None);
        assert!(is_update("1.0.0", "1.0.1"));
        assert!(!is_update("1.0.1", "1.0.0"));
        assert!(!is_update("1.0.0", "1.0.0"));
        assert!(app_supports(Some("0.1.0"), "0.1.0"));
        assert!(!app_supports(Some("9.0.0"), "0.1.0"));
        assert!(app_supports(None, "0.0.1"));
    }

    #[test]
    fn url_parser_matrix() {
        let url = parse_http_url("https://example.com/a/b?x=1#frag").unwrap();
        assert_eq!(url.scheme, UrlScheme::Https);
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, 443);
        assert_eq!(url.path_and_query, "/a/b?x=1");

        let url = parse_http_url("http://127.0.0.1:8765").unwrap();
        assert_eq!(url.port, 8765);
        assert_eq!(url.path_and_query, "/");

        assert_eq!(parse_http_url("https://[::1]:443/x").unwrap().host, "::1");
        assert_eq!(
            parse_http_url("HTTPS://Example.com/X").unwrap().host,
            "Example.com"
        );
        assert!(parse_http_url("ftp://example.com").is_err());
        assert!(parse_http_url("https://user:pass@example.com").is_err());
        assert!(parse_http_url("https://example.com:0").is_err());
        assert!(parse_http_url("https://exa mple.com").is_err());
        assert!(parse_http_url("https://example.com:notaport").is_err());
    }

    #[test]
    fn resolve_url_handles_relative_and_absolute() {
        let base = "https://example.com/repo/main/index.json";
        assert_eq!(
            resolve_url(base, "items/a/preview.gif"),
            "https://example.com/repo/main/items/a/preview.gif"
        );
        assert_eq!(
            resolve_url(base, "/items/a/preview.gif"),
            "https://example.com/repo/main/items/a/preview.gif"
        );
        assert_eq!(
            resolve_url(base, "https://cdn.example.com/x.gif"),
            "https://cdn.example.com/x.gif"
        );
        assert_eq!(
            resolve_url(base, "items/a/thumb.jpg?v=b1949134"),
            "https://example.com/repo/main/items/a/thumb.jpg?v=b1949134"
        );
    }

    #[test]
    fn path_and_folder_rules() {
        assert!(validate_folder_name("2961764492"));
        assert!(validate_folder_name("my-wallpaper"));
        assert!(!validate_folder_name("default"));
        assert!(!validate_folder_name("Default"));
        assert!(!validate_folder_name("CON"));
        assert!(!validate_folder_name("con.txt"));
        assert!(!validate_folder_name(".hidden"));
        assert!(!validate_folder_name("trail."));
        assert!(validate_relative_path("assets/index.js"));
        assert!(!validate_relative_path("../escape"));
        assert!(!validate_relative_path("/abs"));
        assert!(!validate_relative_path("C:/x"));
        assert!(!validate_relative_path("a//b"));
        assert!(is_valid_sha256(&"a".repeat(64)));
        assert!(!is_valid_sha256(&"A".repeat(64)));
        assert!(!is_valid_sha256("abc"));
    }

    #[test]
    fn parses_marker() {
        let marker: InstalledCommunity = serde_json::from_str(
            r#"{"folder":"1","version":"1.0.0","sha256":"abc","installedAt":7}"#,
        )
        .unwrap();
        assert_eq!(marker.installed_at, 7);
        assert_eq!(marker.folder, "1");
    }
}
