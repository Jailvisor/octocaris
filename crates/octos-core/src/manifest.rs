//! Mod manifest parsing (`octos.json`, falling back to `mod.json`).
//!
//! Behavior mirrors the upstream `Storage.cpp` loader:
//! - optional config; missing/invalid config does not make the folder disappear
//! - `name` falls back to the folder name
//! - `image`/`entry` are relative to the mod folder; http(s) entries pass through
//! - `options` or the legacy `prefs` object are exposed as-is
//! - unknown fields are preserved

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("mod folder does not exist or is not a directory: {0}")]
    NotAFolder(PathBuf),
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub authorsite: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub options: Option<Value>,
    #[serde(default)]
    pub prefs: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    External(String),
    Local { relative: String, path: PathBuf },
}

#[derive(Debug, Clone)]
pub struct ModManifest {
    pub folder: PathBuf,
    pub folder_name: String,
    pub config_path: Option<PathBuf>,
    pub config: ModConfig,
    pub name: String,
    pub entry: Option<Entry>,
    pub image_relative: Option<String>,
    pub image: Option<PathBuf>,
    /// `thumbnail.jpg` / `thumbnail.png` when the mod ships one.
    pub thumbnail_relative: Option<String>,
    pub options: Map<String, Value>,
}

impl ModManifest {
    pub fn load(folder: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let folder = folder.as_ref();
        if !folder.is_dir() {
            return Err(ManifestError::NotAFolder(folder.to_path_buf()));
        }

        let folder_name = folder
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("mod")
            .to_string();

        let config_path = ["octos.json", "mod.json"]
            .into_iter()
            .map(|name| folder.join(name))
            .find(|path| path.is_file());

        let config = match &config_path {
            Some(path) => parse_config(path)?,
            None => ModConfig::default(),
        };

        let name = config
            .name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| folder_name.clone());

        let image_relative = config
            .image
            .as_deref()
            .and_then(|relative| confined_relative(folder, relative));
        let image = image_relative
            .as_ref()
            .map(|relative| folder.join(relative))
            .filter(|path| path.is_file());

        let thumbnail_relative = ["thumbnail.jpg", "thumbnail.png"]
            .into_iter()
            .find(|name| folder.join(name).is_file())
            .and_then(|name| confined_relative(folder, name));

        let entry = resolve_entry(folder, config.entry.as_deref());
        let options = config
            .options
            .as_ref()
            .and_then(Value::as_object)
            .or_else(|| config.prefs.as_ref().and_then(Value::as_object))
            .cloned()
            .unwrap_or_default();

        Ok(Self {
            folder: folder.to_path_buf(),
            folder_name,
            config_path,
            config,
            name,
            entry,
            image_relative,
            image,
            thumbnail_relative,
            options,
        })
    }

    pub fn author(&self) -> &str {
        self.config.author.as_deref().unwrap_or("")
    }

    pub fn description(&self) -> &str {
        self.config.description.as_deref().unwrap_or("")
    }

    pub fn authorsite(&self) -> Option<&str> {
        self.config.authorsite.as_deref()
    }

    pub fn source(&self) -> Option<&str> {
        self.config.source.as_deref()
    }

    pub fn version(&self) -> Option<&str> {
        self.config.version.as_deref()
    }

    /// The URL a wallpaper WebView should navigate to, using the frozen virtual host.
    pub fn entry_url(&self) -> Option<String> {
        self.entry.as_ref().map(|entry| match entry {
            Entry::External(url) => url.clone(),
            Entry::Local { relative, .. } => format!(
                "https://local.octos/{}/{}",
                encode_segment(&self.folder_name),
                encode_relative(relative)
            ),
        })
    }

    fn local_url(&self, relative: &str) -> String {
        format!(
            "https://local.octos/{}/{}",
            encode_segment(&self.folder_name),
            encode_relative(relative)
        )
    }

    pub fn image_url(&self) -> Option<String> {
        self.image_relative
            .as_deref()
            .map(|relative| self.local_url(relative))
    }

    /// URL for library cards: a shipped/generated thumbnail when available,
    /// otherwise the manifest preview image.
    pub fn preview_url(&self) -> Option<String> {
        self.thumbnail_relative
            .as_deref()
            .or(self.image_relative.as_deref())
            .map(|relative| self.local_url(relative))
    }
}

fn parse_config(path: &Path) -> Result<ModConfig, ManifestError> {
    let text = std::fs::read_to_string(path).map_err(|source| ManifestError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| ManifestError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn resolve_entry(folder: &Path, configured: Option<&str>) -> Option<Entry> {
    if let Some(configured) = configured {
        let lower = configured.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return Some(Entry::External(configured.to_string()));
        }
        if let Some(relative) = confined_relative(folder, configured) {
            let path = folder.join(&relative);
            if path.is_file() {
                return Some(Entry::Local { relative, path });
            }
        }
    }

    let mut fallback: Option<String> = None;
    let reader = std::fs::read_dir(folder).ok()?;
    for entry in reader.flatten() {
        if !entry.path().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.to_ascii_lowercase().ends_with(".html") {
            continue;
        }
        if name.eq_ignore_ascii_case("index.html") {
            fallback = Some(name);
            break;
        }
        if fallback.is_none() {
            fallback = Some(name);
        }
    }

    fallback.map(|relative| Entry::Local {
        path: folder.join(&relative),
        relative,
    })
}

fn confined_relative(folder: &Path, relative: &str) -> Option<String> {
    if relative.is_empty() {
        return None;
    }

    let path = Path::new(relative);
    if path.is_absolute() {
        return None;
    }

    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    if out.as_os_str().is_empty() {
        return None;
    }

    // Ensure the resolved path is still inside the mod folder even on weird
    // Windows path syntax that `components()` may not normalize.
    let normalized = out.to_string_lossy().replace('\\', "/");
    if normalized.split('/').any(|part| part == "..") {
        return None;
    }
    let full = folder.join(&out);
    let folder = folder.canonicalize().ok()?;
    let full = full.canonicalize().ok()?;
    full.starts_with(&folder).then_some(normalized)
}

fn encode_segment(value: &str) -> String {
    percent_encode(value, false)
}

fn encode_relative(value: &str) -> String {
    percent_encode(value, true)
}

fn percent_encode(value: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        let unreserved = byte.is_ascii_alphanumeric()
            || matches!(*byte, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && *byte == b'/');
        if unreserved {
            out.push(*byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    #[test]
    fn parses_minimal_manifest() {
        let manifest = ModManifest::load(fixture("minimal")).unwrap();
        assert_eq!(manifest.name, "Minimal Mod");
        assert_eq!(manifest.author(), "Tester");
        assert_eq!(manifest.description(), "A minimal test mod.");
        assert!(manifest.image.is_some());
        assert_eq!(
            manifest.entry_url().as_deref(),
            Some("https://local.octos/minimal/index.html")
        );
        assert!(manifest.options.contains_key("speed"));
        assert_eq!(manifest.options["speed"]["value"], 3);
    }

    #[test]
    fn falls_back_to_mod_json_and_legacy_prefs() {
        let manifest = ModManifest::load(fixture("legacy")).unwrap();
        assert_eq!(manifest.name, "Legacy Mod");
        assert!(manifest.options.contains_key("enabled"));
        assert_eq!(manifest.options["enabled"]["value"], true);
        assert!(manifest.config_path.as_ref().unwrap().ends_with("mod.json"));
    }

    #[test]
    fn external_entries_pass_through() {
        let manifest = ModManifest::load(fixture("external")).unwrap();
        assert_eq!(
            manifest.entry,
            Some(Entry::External("https://example.com/app".to_string()))
        );
        assert_eq!(
            manifest.entry_url().as_deref(),
            Some("https://example.com/app")
        );
    }

    #[test]
    fn missing_entry_falls_back_to_index_html() {
        let manifest = ModManifest::load(fixture("fallback")).unwrap();
        assert_eq!(
            manifest.entry_url().as_deref(),
            Some("https://local.octos/fallback/index.html")
        );
    }

    #[test]
    fn preview_prefers_a_shipped_thumbnail() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("withthumb");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("octos.json"),
            r#"{"name":"Thumbed","image":"image.png","entry":"index.html"}"#,
        )
        .unwrap();
        std::fs::write(folder.join("image.png"), b"x").unwrap();
        std::fs::write(folder.join("thumbnail.jpg"), b"x").unwrap();
        std::fs::write(folder.join("index.html"), b"x").unwrap();

        let manifest = ModManifest::load(&folder).unwrap();

        assert_eq!(
            manifest.image_url().as_deref(),
            Some("https://local.octos/withthumb/image.png")
        );
        assert_eq!(
            manifest.preview_url().as_deref(),
            Some("https://local.octos/withthumb/thumbnail.jpg")
        );
    }

    #[test]
    fn preview_uses_a_png_thumbnail_when_no_jpeg_exists() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("png-thumb");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("octos.json"),
            r#"{"name":"Png","image":"image.png","entry":"index.html"}"#,
        )
        .unwrap();
        std::fs::write(folder.join("image.png"), b"x").unwrap();
        std::fs::write(folder.join("thumbnail.png"), b"x").unwrap();

        let manifest = ModManifest::load(&folder).unwrap();

        assert_eq!(
            manifest.preview_url().as_deref(),
            Some("https://local.octos/png-thumb/thumbnail.png")
        );
    }

    #[test]
    fn preview_falls_back_to_the_manifest_image() {
        let manifest = ModManifest::load(fixture("minimal")).unwrap();
        assert_eq!(manifest.preview_url(), manifest.image_url());
    }

    #[test]
    fn unknown_fields_are_preserved() {
        let manifest = ModManifest::load(fixture("unknown")).unwrap();
        assert_eq!(manifest.config.extra["customField"], "kept");
        assert_eq!(manifest.config.extra["nested"]["x"], 1);
    }

    #[test]
    fn traversal_is_rejected_and_falls_back() {
        let manifest = ModManifest::load(fixture("traversal")).unwrap();
        assert!(manifest.image.is_none());
        assert_eq!(
            manifest.entry_url().as_deref(),
            Some("https://local.octos/traversal/index.html")
        );
    }

    #[test]
    fn missing_folder_is_an_error() {
        let error = ModManifest::load(fixture("does-not-exist")).unwrap_err();
        assert!(matches!(error, ManifestError::NotAFolder(_)));
    }
}
