//! Import a single image file as a generated wallpaper mod.
//!
//! The generated folder is a normal Octos mod: `octos.json` + `index.html` +
//! the copied image. It can be selected, removed and re-imported like any
//! other wallpaper. Only a small allowlist of raster formats is accepted.

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::json;
use tracing::warn;

use crate::install::InstallError;

pub const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];
const MAX_IMAGE_BYTES: u64 = 128 * 1024 * 1024;

pub fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn install_image(source: &Path, wallpapers_root: &Path) -> Result<PathBuf, InstallError> {
    fs::create_dir_all(wallpapers_root)?;
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(InstallError::InvalidImage(source.to_path_buf()));
    }
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .ok_or_else(|| InstallError::UnsupportedImage(String::new()))?;
    if !IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        return Err(InstallError::UnsupportedImage(extension));
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(InstallError::ImageTooLarge {
            size: metadata.len(),
            limit: MAX_IMAGE_BYTES,
        });
    }

    // Wallpaper sites often keep JPEG/WebP bytes under a `.png` name, so trust
    // the content signature instead of the extension and generate the mod file
    // with the detected format.
    let mut header = [0u8; 16];
    let read = File::open(source)?.read(&mut header)?;
    let format = detect_format(&header[..read])
        .ok_or_else(|| InstallError::InvalidImage(source.to_path_buf()))?;

    let canonical = source.canonicalize()?;
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    if let Ok(modified) = metadata.modified()
        && let Ok(duration) = modified.duration_since(UNIX_EPOCH)
    {
        duration.as_nanos().hash(&mut hasher);
    }
    let hash = format!("{:08x}", hasher.finish());

    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("image")
        .to_string();
    let slug = sanitize_slug(&stem);
    let base = format!("image-{slug}-{hash}");
    let mut folder_name = base.clone();
    for suffix in 2..100 {
        if !wallpapers_root.join(&folder_name).exists() {
            break;
        }
        folder_name = format!("{base}-{suffix}");
    }
    if wallpapers_root.join(&folder_name).exists() {
        return Err(InstallError::InvalidFolderName(base));
    }

    let staging = wallpapers_root.join(format!(
        ".octos-image-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;

    let image_name = format!("image.{format}");
    let result = (|| -> Result<PathBuf, InstallError> {
        fs::copy(source, staging.join(&image_name))?;
        // Cards load this instead of the full-size original.
        if let Err(err) = crate::thumbnail::write_thumbnail(
            &staging.join(&image_name),
            &staging.join("thumbnail.jpg"),
        ) {
            warn!(
                "could not create a thumbnail for {}: {err}",
                source.display()
            );
        }
        let config = json!({
            "name": stem,
            "author": "",
            "description": "Image wallpaper",
            "image": image_name,
            "entry": "index.html",
            "version": "1.0.0",
            "options": {
                "fit": {
                    "type": "select",
                    "label": "Image fit",
                    "options": ["cover", "contain", "fill"],
                    "value": "cover"
                }
            }
        });
        fs::write(
            staging.join("octos.json"),
            serde_json::to_vec_pretty(&config).map_err(io::Error::other)?,
        )?;
        fs::write(staging.join("index.html"), index_html(&image_name))?;
        let destination = wallpapers_root.join(&folder_name);
        fs::rename(&staging, &destination)?;
        Ok(destination)
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn sanitize_slug(stem: &str) -> String {
    let mut slug = String::with_capacity(stem.len());
    let mut last_dash = false;
    for ch in stem.chars() {
        let keep = ch.is_ascii_alphanumeric() || ch == '_';
        if keep {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    let slug = if slug.is_empty() {
        "image".to_string()
    } else {
        slug
    };
    slug.chars().take(40).collect()
}

fn detect_format(header: &[u8]) -> Option<&'static str> {
    if header.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("png")
    } else if header.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        Some("gif")
    } else if header.len() >= 12 && &header[0..4] == b"RIFF" && &header[8..12] == b"WEBP" {
        Some("webp")
    } else if header.starts_with(b"BM") {
        Some("bmp")
    } else {
        None
    }
}

pub(crate) fn index_html(image_name: &str) -> String {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="octos-image-view" content="2">
  <title>Image wallpaper</title>
  <style>
    html, body { margin: 0; width: 100%; height: 100%; overflow: hidden; background: #000; }
    #placeholder {
      position: fixed;
      inset: 0;
      background: #000 url('./thumbnail.jpg') center / cover no-repeat;
    }
    #image {
      position: fixed;
      inset: 0;
      display: block;
      width: 100%;
      height: 100%;
      object-fit: cover;
      opacity: 0;
      transition: opacity 220ms ease;
    }
    #image.loaded { opacity: 1; }
  </style>
</head>
<body>
  <div id="placeholder"></div>
  <img id="image" src="./IMAGE_FILENAME" alt="">
  <script>
    (() => {
      const image = document.getElementById('image');
      const placeholder = document.getElementById('placeholder');

      const setFit = (fit) => {
        if (fit !== 'cover' && fit !== 'contain' && fit !== 'fill') return;
        image.style.objectFit = fit;
        placeholder.style.backgroundSize = fit === 'fill' ? '100% 100%' : fit;
      };

      const reveal = () => image.classList.add('loaded');
      if (image.complete && image.naturalWidth > 0) reveal();
      image.addEventListener('load', () => {
        reveal();
        setTimeout(() => placeholder.remove(), 260);
      }, { once: true });

      const apply = (options) => {
        setFit(options && options.fit && options.fit.value);
      };
      const bridge = window.chrome && window.chrome.webview;
      if (!bridge) return;
      const requestId = 'image-options-' + Math.random().toString(36).slice(2);
      const onMessage = (event) => {
        const message = event.data;
        if (!message) return;
        if (message.type === 'response' && message.requestId === requestId) {
          apply(message.data);
        } else if (message.type === 'event' && message.eventType === 'options-change'
          && message.data && message.data.id === 'fit') {
          apply({ fit: { value: message.data.value } });
        }
      };
      bridge.addEventListener('message', onMessage);
      bridge.postMessage({ type: 'request', requestType: 'options', requestId, data: {} });
    })();
  </script>
</body>
</html>
"#
    .replace("IMAGE_FILENAME", image_name)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use tempfile::tempdir;

    use super::*;

    fn png_bytes() -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&[0u8; 32]);
        bytes
    }

    fn write_image(path: &Path, bytes: &[u8]) {
        let mut file = File::create(path).unwrap();
        file.write_all(bytes).unwrap();
    }

    #[test]
    fn installs_a_generated_mod_with_options() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("My Photo.PNG");
        write_image(&source, &png_bytes());
        let wallpapers = temp.path().join("wallpapers");

        let installed = install_image(&source, &wallpapers).unwrap();

        assert!(installed.starts_with(&wallpapers));
        assert!(installed.join("image.png").is_file());
        assert!(installed.join("index.html").is_file());
        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(installed.join("octos.json")).unwrap())
                .unwrap();
        assert_eq!(config["name"], "My Photo");
        assert_eq!(config["image"], "image.png");
        assert_eq!(config["options"]["fit"]["value"], "cover");
        let html = fs::read_to_string(installed.join("index.html")).unwrap();
        assert!(html.contains("./image.png"));
        assert!(html.contains("options-change"));
        assert!(html.contains("thumbnail.jpg"));
        assert!(html.contains("octos-image-view"));
    }

    #[test]
    fn installs_a_thumbnail_for_valid_images() {
        use image::{Rgba, RgbaImage};

        let temp = tempdir().unwrap();
        let source = temp.path().join("photo.png");
        RgbaImage::from_pixel(640, 360, Rgba([10, 200, 30, 255]))
            .save(&source)
            .unwrap();
        let wallpapers = temp.path().join("wallpapers");

        let installed = install_image(&source, &wallpapers).unwrap();

        assert!(installed.join("thumbnail.jpg").is_file());
        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(installed.join("octos.json")).unwrap())
                .unwrap();
        assert_eq!(config["image"], "image.png");
    }

    #[test]
    fn duplicate_imports_get_distinct_folders() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("photo.png");
        write_image(&source, &png_bytes());
        let wallpapers = temp.path().join("wallpapers");

        let first = install_image(&source, &wallpapers).unwrap();
        let second = install_image(&source, &wallpapers).unwrap();

        assert_ne!(first, second);
        assert!(first.is_dir());
        assert!(second.is_dir());
    }

    #[test]
    fn rejects_non_images_and_bad_magic() {
        let temp = tempdir().unwrap();
        let wallpapers = temp.path().join("wallpapers");
        let text = temp.path().join("notes.txt");
        fs::write(&text, b"hello").unwrap();
        assert!(matches!(
            install_image(&text, &wallpapers),
            Err(InstallError::UnsupportedImage(_))
        ));

        let fake = temp.path().join("fake.png");
        fs::write(&fake, b"not a png").unwrap();
        assert!(matches!(
            install_image(&fake, &wallpapers),
            Err(InstallError::InvalidImage(_))
        ));
    }

    #[test]
    fn is_image_file_is_case_insensitive() {
        assert!(is_image_file(Path::new("a.PNG")));
        assert!(is_image_file(Path::new("a.webp")));
        assert!(!is_image_file(Path::new("a.txt")));
        assert!(!is_image_file(Path::new("a")));
    }

    #[test]
    fn accepts_content_that_does_not_match_the_extension() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("mislabelled.png");
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
        bytes.extend_from_slice(&[0u8; 28]);
        write_image(&source, &bytes);
        let wallpapers = temp.path().join("wallpapers");

        let installed = install_image(&source, &wallpapers).unwrap();

        assert!(installed.join("image.jpg").is_file());
        assert!(!installed.join("image.png").exists());
        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(installed.join("octos.json")).unwrap())
                .unwrap();
        assert_eq!(config["image"], "image.jpg");
    }

    #[test]
    fn rejects_oversized_images() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("huge.png");
        let mut file = File::create(&source).unwrap();
        file.write_all(&png_bytes()).unwrap();
        file.set_len(MAX_IMAGE_BYTES + 1).unwrap();
        let wallpapers = temp.path().join("wallpapers");
        assert!(matches!(
            install_image(&source, &wallpapers),
            Err(InstallError::ImageTooLarge { .. })
        ));
    }
}
