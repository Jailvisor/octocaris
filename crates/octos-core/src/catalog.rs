//! Scan the wallpapers directory into UI-facing mod cards.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::manifest::{ManifestError, ModManifest};
use crate::options::OptionsMap;

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("could not read wallpaper root {path}: {source}")]
    Root {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not load {path}: {source}")]
    Manifest {
        path: PathBuf,
        #[source]
        source: ManifestError,
    },
}

/// Built-in mods that act as fallbacks or diagnostics; never shown as user
/// wallpapers and never allowed into rotation.
pub const HIDDEN_MODS: [&str; 3] = ["default", "gradient", "conformance"];

pub fn is_hidden_mod(folder_name: &str) -> bool {
    HIDDEN_MODS.contains(&folder_name)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModCard {
    pub folder_name: String,
    pub folder_path: PathBuf,
    pub name: String,
    pub author: String,
    pub description: String,
    pub image_url: Option<String>,
    pub entry_url: Option<String>,
    pub authorsite: Option<String>,
    pub source: Option<String>,
    pub version: Option<String>,
    pub keywords: Vec<String>,
    pub options: OptionsMap,
}

impl ModCard {
    pub fn from_manifest(manifest: ModManifest) -> Self {
        Self {
            folder_name: manifest.folder_name.clone(),
            folder_path: manifest.folder.clone(),
            name: manifest.name.clone(),
            author: manifest.author().to_string(),
            description: manifest.description().to_string(),
            image_url: manifest.preview_url(),
            entry_url: manifest.entry_url(),
            authorsite: manifest.authorsite().map(str::to_string),
            source: manifest.source().map(str::to_string),
            version: manifest.version().map(str::to_string),
            keywords: manifest.config.keywords.clone(),
            options: manifest.options.clone(),
        }
    }

    pub fn is_runnable(&self) -> bool {
        self.entry_url.is_some()
    }
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub mods: Vec<ModCard>,
    pub errors: Vec<CatalogError>,
}

pub fn scan(root: &Path) -> Result<ScanReport, CatalogError> {
    let entries = std::fs::read_dir(root).map_err(|source| CatalogError::Root {
        path: root.to_path_buf(),
        source,
    })?;

    let mut report = ScanReport::default();
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        match ModManifest::load(&path) {
            Ok(manifest) => report.mods.push(ModCard::from_manifest(manifest)),
            Err(source) => report.errors.push(CatalogError::Manifest { path, source }),
        }
    }

    report.mods.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.folder_name.cmp(&right.folder_name))
    });
    Ok(report)
}

/// Reorder cards so entries listed in `order` (absolute folder keys) come
/// first in that sequence. Mods absent from `order` keep their existing
/// (name-sorted) order at the end; stale keys are ignored; duplicate keys use
/// the first position.
pub fn sort_by_order(mods: &mut [ModCard], order: &[String]) {
    if order.is_empty() {
        return;
    }
    let mut rank: HashMap<&str, usize> = HashMap::with_capacity(order.len());
    for (index, key) in order.iter().enumerate() {
        rank.entry(key.as_str()).or_insert(index);
    }
    mods.sort_by_key(|card| {
        rank.get(card.folder_path.to_string_lossy().as_ref())
            .copied()
            .unwrap_or(usize::MAX)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
    }

    #[test]
    fn scans_and_sorts_mods_and_reports_bad_json() {
        let report = scan(&fixtures()).unwrap();
        let names: Vec<_> = report.mods.iter().map(|card| card.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "External",
                "Fallback",
                "Legacy Mod",
                "Minimal Mod",
                "Traversal",
                "Unknown"
            ]
        );
        assert_eq!(report.errors.len(), 1);
        assert!(matches!(report.errors[0], CatalogError::Manifest { .. }));
    }

    #[test]
    fn cards_expose_urls_and_options() {
        let report = scan(&fixtures()).unwrap();
        let minimal = report
            .mods
            .iter()
            .find(|card| card.name == "Minimal Mod")
            .unwrap();
        assert_eq!(
            minimal.entry_url.as_deref(),
            Some("https://local.octos/minimal/index.html")
        );
        assert_eq!(
            minimal.image_url.as_deref(),
            Some("https://local.octos/minimal/image.png")
        );
        assert!(minimal.is_runnable());
        assert_eq!(minimal.options["speed"]["value"], 3);
    }

    #[test]
    fn sort_by_order_puts_known_mods_first_and_appends_new_ones() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["beta", "alpha", "gamma"] {
            let folder = temp.path().join(name);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join("octos.json"),
                format!(
                    r#"{{"name":"{}","image":"image.png","entry":"index.html"}}"#,
                    name.to_uppercase()
                ),
            )
            .unwrap();
            std::fs::write(folder.join("image.png"), b"x").unwrap();
            std::fs::write(folder.join("index.html"), b"x").unwrap();
        }

        let mut cards = scan(temp.path()).unwrap().mods;
        let names = |cards: &[ModCard]| {
            cards
                .iter()
                .map(|card| card.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&cards), vec!["ALPHA", "BETA", "GAMMA"]);

        let gamma = cards[2].folder_path.to_string_lossy().to_string();
        let alpha = cards[0].folder_path.to_string_lossy().to_string();
        let stale = temp.path().join("gone").to_string_lossy().to_string();
        sort_by_order(&mut cards, &[gamma.clone(), stale, alpha.clone(), gamma]);

        assert_eq!(names(&cards), vec!["GAMMA", "ALPHA", "BETA"]);
    }

    #[test]
    fn empty_order_keeps_scan_order() {
        let report = scan(&fixtures()).unwrap();
        let mut cards = report.mods;
        let before: Vec<_> = cards.iter().map(|card| card.name.clone()).collect();
        sort_by_order(&mut cards, &[]);
        let after: Vec<_> = cards.iter().map(|card| card.name.clone()).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn cards_prefer_the_thumbnail() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("thumbed");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("octos.json"),
            r#"{"name":"Thumbed","image":"image.png","entry":"index.html"}"#,
        )
        .unwrap();
        std::fs::write(folder.join("image.png"), b"x").unwrap();
        std::fs::write(folder.join("thumbnail.jpg"), b"x").unwrap();
        std::fs::write(folder.join("index.html"), b"x").unwrap();

        let report = scan(temp.path()).unwrap();

        assert_eq!(
            report.mods[0].image_url.as_deref(),
            Some("https://local.octos/thumbed/thumbnail.jpg")
        );
    }

    #[test]
    fn external_mod_is_runnable() {
        let report = scan(&fixtures()).unwrap();
        let external = report
            .mods
            .iter()
            .find(|card| card.name == "External")
            .unwrap();
        assert_eq!(
            external.entry_url.as_deref(),
            Some("https://example.com/app")
        );
    }
}
