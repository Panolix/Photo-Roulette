//! Finding photos and videos on disk and turning images into pixels.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui::ColorImage;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader};

/// Photos larger than this are scaled down before they are uploaded as a
/// texture. It keeps huge camera files from eating a silly amount of VRAM.
const MAX_TEXTURE_DIMENSION: u32 = 3000;

/// Maximum folder depth to walk while looking for pictures.
const MAX_SCAN_DEPTH: usize = 12;

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "webp", "bmp", "tif", "tiff", "ico", "tga", "qoi",
    "hdr", "exr", "avif", "heic", "heif",
];

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "wmv", "avi", "mkv", "webm", "flv", "mpg", "mpeg", "3gp", "3g2", "ts",
    "mts", "m2ts", "ogv", "vob",
];

pub struct LoadedPhoto {
    pub image: ColorImage,
}

pub struct MediaLibrary {
    pub media: Vec<PathBuf>,
    pub label: String,
    pub skipped: usize,
}

/// Scan folders (recursively) and media files into a de-duplicated library.
pub fn collect_media(roots: &[PathBuf]) -> Result<MediaLibrary, String> {
    let mut found = Vec::new();
    let mut skipped = 0usize;
    let mut label = None;
    let mut root_exists = false;

    for root in roots {
        if root.exists() {
            root_exists = true;
        }

        if root.is_dir() {
            if label.is_none() {
                label = folder_label(root);
            }
            walk(root, 0, &mut found, &mut skipped);
        } else if root.is_file() && is_media_file(root) {
            if label.is_none() {
                label = root.parent().and_then(folder_label);
            }
            if is_readable_media(root) {
                found.push(root.clone());
            } else {
                skipped += 1;
            }
        }
    }

    if !root_exists {
        return Err("That folder doesn't exist, or it can't be opened.".to_owned());
    }

    let mut seen = HashSet::new();
    let mut media = Vec::new();
    for path in found {
        let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.insert(key.clone()) {
            media.push(key);
        }
    }
    media.sort();

    if media.is_empty() {
        return Err(if skipped > 0 {
            "Found some media files, but couldn't read any of them.".to_owned()
        } else {
            "No photos or videos found. Pick a folder with JPG, PNG, HEIC, WebP, or video files."
                .to_owned()
        });
    }

    Ok(MediaLibrary {
        media,
        label: label.unwrap_or_else(|| "Your photos".to_owned()),
        skipped,
    })
}

/// Decode a photo (including EXIF rotation) and scale it down for display.
pub fn load_photo(path: &Path) -> Result<LoadedPhoto, String> {
    if needs_transcode(path) {
        #[cfg(target_os = "macos")]
        {
            let jpeg = convert_with_sips(path)?;
            let result = decode_image(&jpeg);
            let _ = fs::remove_file(&jpeg);
            return result;
        }

        #[cfg(not(target_os = "macos"))]
        {
            return Err("HEIC and AVIF files are only supported on macOS".to_owned());
        }
    }

    decode_image(path)
}

pub fn is_image_file(path: &Path) -> bool {
    extension_is_in(path, IMAGE_EXTENSIONS)
}

pub fn is_video_file(path: &Path) -> bool {
    extension_is_in(path, VIDEO_EXTENSIONS)
}

fn is_media_file(path: &Path) -> bool {
    is_image_file(path) || is_video_file(path)
}

fn extension_is_in(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extensions
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

/// Formats the image crate can't decode in this build, but macOS ImageIO can.
fn needs_transcode(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("heic")
                || extension.eq_ignore_ascii_case("heif")
                || extension.eq_ignore_ascii_case("avif")
        })
}

fn folder_label(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>, skipped: &mut usize) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();

        if file_type.is_dir() {
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".photoslibrary")
                || lower.ends_with(".app")
                || lower.ends_with(".bundle")
            {
                continue;
            }
            walk(&path, depth + 1, found, skipped);
        } else if file_type.is_file() && is_media_file(&path) {
            if is_readable_media(&path) {
                found.push(path);
            } else {
                *skipped += 1;
            }
        }
    }
}

fn is_readable_media(path: &Path) -> bool {
    if is_video_file(path) {
        return path.metadata().is_ok_and(|metadata| metadata.len() > 0);
    }

    // HEIC/HEIF and AVIF are decoded through ImageIO/sips on macOS, so the
    // image crate can't inspect them here. Trust the extension and let the
    // loader deal with any errors.
    if needs_transcode(path) {
        #[cfg(target_os = "macos")]
        {
            return path.metadata().is_ok_and(|metadata| metadata.len() > 0);
        }
        #[cfg(not(target_os = "macos"))]
        {
            return false;
        }
    }

    image::image_dimensions(path).is_ok()
}

fn decode_image(path: &Path) -> Result<LoadedPhoto, String> {
    if path.metadata().is_ok_and(|metadata| metadata.len() == 0) {
        return Err("file is empty".to_owned());
    }

    let reader = ImageReader::open(path)
        .map_err(|error| error.to_string())?
        .with_guessed_format()
        .map_err(|error| error.to_string())?;

    let mut decoder = reader.into_decoder().map_err(|error| error.to_string())?;

    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);

    let mut image = DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
    image.apply_orientation(orientation);

    if image.width() == 0 || image.height() == 0 {
        return Err("image has no pixels".to_owned());
    }

    if image.width().max(image.height()) > MAX_TEXTURE_DIMENSION {
        image = image.thumbnail(MAX_TEXTURE_DIMENSION, MAX_TEXTURE_DIMENSION);
    }

    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let image = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());

    Ok(LoadedPhoto { image })
}

#[cfg(target_os = "macos")]
fn convert_with_sips(path: &Path) -> Result<PathBuf, String> {
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let output = std::env::temp_dir().join(format!(
        "photo-roulette-{}-{unique}.jpg",
        std::process::id()
    ));
    let _ = fs::remove_file(&output);

    let result = std::process::Command::new("sips")
        .args(["-s", "format", "jpeg", "-s", "formatOptions", "92"])
        .arg(path)
        .arg("--out")
        .arg(&output)
        .output()
        .map_err(|error| format!("could not run sips: {error}"))?;

    if !result.status.success() || !output.is_file() {
        let _ = fs::remove_file(&output);
        let message = String::from_utf8_lossy(&result.stderr);
        let message = message.trim();
        return Err(if message.is_empty() {
            "macOS ImageIO couldn't convert this photo".to_owned()
        } else {
            format!("macOS ImageIO couldn't convert this photo: {message}")
        });
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{collect_media, is_image_file, is_video_file};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn recognizes_common_photo_extensions() {
        for name in [
            "a.jpg", "b.JPEG", "c.HEIC", "d.png", "e.webp", "f.gif", "g.tiff", "h.avif",
        ] {
            assert!(is_image_file(Path::new(name)), "{name} should be a photo");
        }
    }

    #[test]
    fn ignores_non_photos() {
        for name in ["a.txt", "b.mp4", "c.pdf", "d", "e.psd", "f.cr2"] {
            assert!(!is_image_file(Path::new(name)), "{name} should not match");
        }
    }

    #[test]
    fn recognizes_common_video_extensions() {
        for name in [
            "a.mp4", "b.MOV", "c.m4v", "d.wmv", "e.avi", "f.mkv", "g.webm", "h.3gp",
        ] {
            assert!(is_video_file(Path::new(name)), "{name} should be a video");
        }
        for name in ["a.jpg", "b.txt", "c.mov.png", "d"] {
            assert!(!is_video_file(Path::new(name)), "{name} should not match");
        }
    }

    #[test]
    fn scans_nested_folders_and_skips_junk() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be sane")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "photo-roulette-test-{}-{stamp}",
            std::process::id()
        ));
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).expect("create test folder");

        let image = image::RgbImage::from_pixel(8, 8, image::Rgb([255, 0, 100]));
        image.save(root.join("one.png")).expect("save png");
        image.save(nested.join("two.jpg")).expect("save jpg");
        std::fs::write(root.join("clip.mp4"), b"not really a video").expect("write video");
        std::fs::write(root.join("notes.txt"), "not a photo").expect("write text");
        std::fs::write(root.join("broken.jpg"), "not a jpeg").expect("write broken jpg");

        let library = collect_media(&[PathBuf::from(&root)]).expect("folder should scan");

        assert_eq!(library.media.len(), 3);
        assert_eq!(library.skipped, 1);
        assert!(library.media.iter().any(|path| path.ends_with("two.jpg")));
        assert!(library.media.iter().any(|path| path.ends_with("clip.mp4")));

        let _ = std::fs::remove_dir_all(root);
    }
}
