//! Thumbnails for imported image wallpapers.
//!
//! Cards used to decode the full-resolution source. A small JPEG keeps the
//! library cheap while the wallpaper page keeps using the original file.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::imageops::overlay;
use image::metadata::Orientation;
use image::{DynamicImage, GenericImageView, ImageDecoder as _, ImageReader, Rgba, RgbaImage};
use thiserror::Error;
use tracing::{info, warn};

/// Longest side of a generated thumbnail. Cards are ~150-250 CSS px wide and
/// the rotation list even smaller, so 512 covers 200% DPI.
pub const MAX_SIDE: u32 = 512;

const THUMBNAIL_NAME: &str = "thumbnail.jpg";
const JPEG_QUALITY: u8 = 80;
const BACKGROUND: [u8; 3] = [17, 17, 17];
const MAX_SOURCE_SIDE: u32 = 12_000;
const MAX_SOURCE_PIXELS: u64 = 40_000_000;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum ThumbnailError {
    #[error("could not read the image: {0}")]
    Image(#[from] image::ImageError),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("refusing to decode a {width}x{height} image")]
    TooLarge { width: u32, height: u32 },
    #[error("could not read the mod manifest: {0}")]
    Manifest(#[from] octos_core::manifest::ManifestError),
}

/// Existing thumbnail for a mod folder, if any.
pub fn thumbnail_path(mod_dir: &Path) -> Option<PathBuf> {
    [THUMBNAIL_NAME, "thumbnail.png"]
        .into_iter()
        .map(|name| mod_dir.join(name))
        .find(|path| {
            fs::metadata(path)
                .map(|metadata| metadata.is_file() && metadata.len() > 0)
                .unwrap_or(false)
        })
}

pub fn has_thumbnail(mod_dir: &Path) -> bool {
    thumbnail_path(mod_dir).is_some()
}

/// Decode `source`, resize it to at most [`MAX_SIDE`] and write a JPEG to
/// `destination`. The write goes through a temporary file so the catalog never
/// sees a half-written thumbnail.
pub fn write_thumbnail(source: &Path, destination: &Path) -> Result<(), ThumbnailError> {
    let (width, height) = ImageReader::open(source)?
        .with_guessed_format()?
        .into_dimensions()?;
    if width == 0
        || height == 0
        || width > MAX_SOURCE_SIDE
        || height > MAX_SOURCE_SIDE
        || u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS
    {
        return Err(ThumbnailError::TooLarge { width, height });
    }

    let reader = ImageReader::open(source)?.with_guessed_format()?;
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);

    let (width, height) = image.dimensions();
    let image = if width.max(height) > MAX_SIDE {
        image.thumbnail(MAX_SIDE, MAX_SIDE)
    } else {
        image
    };
    let rgb = flatten_to_rgb(image);

    let temp = destination.with_file_name(format!(
        ".thumbnail-{}-{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<(), ThumbnailError> {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
            fs::File::create(&temp)?,
            JPEG_QUALITY,
        );
        encoder.encode_image(&rgb)?;
        fs::rename(&temp, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn flatten_to_rgb(image: DynamicImage) -> DynamicImage {
    if image.color().has_alpha() {
        let mut canvas = RgbaImage::from_pixel(
            image.width(),
            image.height(),
            Rgba([BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255]),
        );
        overlay(&mut canvas, &image.to_rgba8(), 0, 0);
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(canvas).to_rgb8())
    } else {
        DynamicImage::ImageRgb8(image.to_rgb8())
    }
}

/// Generate `thumbnail.jpg` for a mod created by `install_image`, if it is
/// missing. Returns true when a file was written.
pub fn ensure_thumbnail(mod_dir: &Path) -> Result<bool, ThumbnailError> {
    let generated_folder = mod_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("image-"));
    if !generated_folder || has_thumbnail(mod_dir) {
        return Ok(false);
    }
    let manifest = octos_core::manifest::ModManifest::load(mod_dir)?;
    let Some(image) = manifest.image.as_deref() else {
        return Ok(false);
    };
    let generated = image
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("image."));
    if !generated {
        return Ok(false);
    }
    write_thumbnail(image, &mod_dir.join(THUMBNAIL_NAME))?;
    Ok(true)
}

/// Backfill thumbnails for every generated image mod under `wallpapers`.
/// Idempotent; used from a background thread.
pub fn ensure_all(wallpapers: &Path) -> usize {
    let Ok(entries) = fs::read_dir(wallpapers) else {
        return 0;
    };
    let mut generated = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        match ensure_thumbnail(&path) {
            Ok(true) => {
                generated += 1;
                info!("generated a thumbnail for {}", path.display());
            }
            Ok(false) => {}
            Err(err) => warn!("could not create a thumbnail for {}: {err}", path.display()),
        }
    }
    generated
}

#[cfg(test)]
mod tests {
    use image::{ImageBuffer, Rgba};
    use tempfile::tempdir;

    use super::*;

    fn write_png(path: &Path, width: u32, height: u32) {
        let image: RgbaImage = ImageBuffer::from_pixel(width, height, Rgba([200, 40, 40, 255]));
        image.save(path).unwrap();
    }

    fn write_transparent_png(path: &Path, width: u32, height: u32) {
        let image: RgbaImage = ImageBuffer::from_pixel(width, height, Rgba([255, 255, 255, 0]));
        image.save(path).unwrap();
    }

    #[test]
    fn writes_a_bounded_jpeg() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source.png");
        let destination = temp.path().join("thumbnail.jpg");
        write_png(&source, 1600, 900);

        write_thumbnail(&source, &destination).unwrap();

        let thumb = image::open(&destination).unwrap();
        assert_eq!(thumb.width().max(thumb.height()), MAX_SIDE);
        assert!(destination.is_file());
    }

    #[test]
    fn small_images_are_not_upscaled() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source.png");
        let destination = temp.path().join("thumbnail.jpg");
        write_png(&source, 64, 32);

        write_thumbnail(&source, &destination).unwrap();

        let thumb = image::open(&destination).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (64, 32));
    }

    #[test]
    fn transparent_images_are_flattened() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source.png");
        let destination = temp.path().join("thumbnail.jpg");
        write_transparent_png(&source, 300, 300);

        write_thumbnail(&source, &destination).unwrap();

        let thumb = image::open(&destination).unwrap().to_rgb8();
        assert_eq!(thumb.get_pixel(10, 10).0, BACKGROUND);
    }

    #[test]
    fn rejects_broken_sources() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("broken.png");
        fs::write(&source, b"not an image").unwrap();
        let destination = temp.path().join("thumbnail.jpg");
        assert!(write_thumbnail(&source, &destination).is_err());
        assert!(!destination.exists());
    }

    #[test]
    fn gif_and_bmp_sources_are_supported() {
        let temp = tempdir().unwrap();
        for (name, format) in [
            ("source.gif", image::ImageFormat::Gif),
            ("source.bmp", image::ImageFormat::Bmp),
        ] {
            let source = temp.path().join(name);
            let image: RgbaImage = ImageBuffer::from_pixel(120, 80, Rgba([20, 120, 220, 255]));
            image.save_with_format(&source, format).unwrap();
            let destination = temp.path().join(format!("{name}.jpg"));
            write_thumbnail(&source, &destination).unwrap();
            assert!(destination.is_file(), "{name} should produce a thumbnail");
        }
    }

    #[test]
    fn rejects_oversized_sources() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("wide.png");
        let destination = temp.path().join("thumbnail.jpg");
        write_png(&source, MAX_SOURCE_SIDE + 1, 1);

        assert!(matches!(
            write_thumbnail(&source, &destination),
            Err(ThumbnailError::TooLarge { .. })
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn ensure_all_only_touches_generated_image_mods() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let generated = root.join("image-photo-1");
        fs::create_dir_all(&generated).unwrap();
        write_png(&generated.join("image.png"), 400, 300);
        fs::write(
            generated.join("octos.json"),
            r#"{"name":"Photo","image":"image.png","entry":"index.html"}"#,
        )
        .unwrap();
        fs::write(generated.join("index.html"), "<!doctype html>").unwrap();

        let other = root.join("image-photo-2");
        fs::create_dir_all(&other).unwrap();
        write_png(&other.join("preview.png"), 400, 300);
        fs::write(
            other.join("octos.json"),
            r#"{"name":"Photo 2","image":"preview.png","entry":"index.html"}"#,
        )
        .unwrap();
        fs::write(other.join("index.html"), "<!doctype html>").unwrap();

        let regular = root.join("regular-mod");
        fs::create_dir_all(&regular).unwrap();
        write_png(&regular.join("image.png"), 400, 300);
        fs::write(
            regular.join("octos.json"),
            r#"{"name":"Regular","image":"image.png","entry":"index.html"}"#,
        )
        .unwrap();

        assert_eq!(ensure_all(root), 1);
        assert!(generated.join("thumbnail.jpg").is_file());
        assert!(!other.join("thumbnail.jpg").exists());
        assert!(!regular.join("thumbnail.jpg").exists());
        assert_eq!(ensure_all(root), 0);
    }

    #[test]
    fn ensure_thumbnail_is_idempotent() {
        let temp = tempdir().unwrap();
        let mod_dir = temp.path().join("image-photo-1234");
        fs::create_dir_all(&mod_dir).unwrap();
        write_png(&mod_dir.join("image.png"), 800, 600);
        fs::write(
            mod_dir.join("octos.json"),
            r#"{"name":"Photo","image":"image.png","entry":"index.html"}"#,
        )
        .unwrap();
        fs::write(mod_dir.join("index.html"), "<!doctype html>").unwrap();

        assert!(ensure_thumbnail(&mod_dir).unwrap());
        assert!(!ensure_thumbnail(&mod_dir).unwrap());
        assert!(mod_dir.join("thumbnail.jpg").is_file());
    }

    #[test]
    fn skips_folders_that_are_not_generated_image_mods() {
        let temp = tempdir().unwrap();
        let mod_dir = temp.path().join("my-mod");
        fs::create_dir_all(&mod_dir).unwrap();
        write_png(&mod_dir.join("preview.png"), 800, 600);
        fs::write(
            mod_dir.join("octos.json"),
            r#"{"name":"Mod","image":"preview.png","entry":"index.html"}"#,
        )
        .unwrap();

        assert!(!ensure_thumbnail(&mod_dir).unwrap());
        assert!(!mod_dir.join("thumbnail.jpg").exists());
    }
}
