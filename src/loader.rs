//! Finding photos and videos on disk and turning images into pixels.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

#[derive(Debug)]
pub struct MediaLibrary {
    pub media: Vec<PathBuf>,
    pub skipped: usize,
    pub skipped_transcoded: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    Cancelled,
    Message(String),
}

impl fmt::Display for ScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("Scan cancelled."),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

/// Scan folders (recursively) and media files into a de-duplicated library.
#[cfg(test)]
pub fn collect_media(roots: &[PathBuf]) -> Result<MediaLibrary, String> {
    let cancel = AtomicBool::new(false);
    let progress = AtomicUsize::new(0);
    collect_media_interruptible(roots, &cancel, &progress).map_err(|error| error.to_string())
}

/// Interruptible scan used by the UI so huge folders never freeze the window.
pub fn collect_media_interruptible(
    roots: &[PathBuf],
    cancel: &AtomicBool,
    progress: &AtomicUsize,
) -> Result<MediaLibrary, ScanError> {
    let mut found = Vec::new();
    let mut skipped = 0usize;
    let mut skipped_transcoded = 0usize;
    let mut root_exists = false;

    for root in roots {
        if cancel.load(Ordering::Relaxed) {
            return Err(ScanError::Cancelled);
        }
        if root.exists() {
            root_exists = true;
        }

        if root.is_dir() {
            walk(
                root,
                0,
                &mut found,
                &mut skipped,
                &mut skipped_transcoded,
                cancel,
                progress,
            );
        } else if root.is_file() && is_media_file(root) {
            if is_readable_media(root) {
                found.push(root.clone());
                progress.store(found.len(), Ordering::Relaxed);
            } else {
                skipped += 1;
                if needs_transcode(root) {
                    skipped_transcoded += 1;
                }
            }
        }
    }

    if cancel.load(Ordering::Relaxed) {
        return Err(ScanError::Cancelled);
    }

    if !root_exists {
        return Err(ScanError::Message(
            "That folder doesn't exist, or it can't be opened.".to_owned(),
        ));
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
        return Err(ScanError::Message(if skipped > 0 {
            "Found some media files, but couldn't read any of them.".to_owned()
        } else {
            "No photos or videos found. Pick a folder with JPG, PNG, HEIC, WebP, or video files."
                .to_owned()
        }));
    }

    Ok(MediaLibrary {
        media,
        skipped,
        skipped_transcoded,
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
            return decode_with_ffmpeg(path);
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

/// Formats decoded through macOS ImageIO or ffmpeg instead of the image crate.
fn needs_transcode(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("heic")
                || extension.eq_ignore_ascii_case("heif")
                || extension.eq_ignore_ascii_case("avif")
        })
}

fn walk(
    dir: &Path,
    depth: usize,
    found: &mut Vec<PathBuf>,
    skipped: &mut usize,
    skipped_transcoded: &mut usize,
    cancel: &AtomicBool,
    progress: &AtomicUsize,
) {
    if depth > MAX_SCAN_DEPTH || cancel.load(Ordering::Relaxed) {
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
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
            walk(
                &path,
                depth + 1,
                found,
                skipped,
                skipped_transcoded,
                cancel,
                progress,
            );
        } else if file_type.is_file() && is_media_file(&path) {
            if is_readable_media(&path) {
                found.push(path);
                progress.store(found.len(), Ordering::Relaxed);
            } else {
                *skipped += 1;
                if needs_transcode(&path) {
                    *skipped_transcoded += 1;
                }
            }
        }
    }
}

fn is_readable_media(path: &Path) -> bool {
    if is_video_file(path) {
        return path.metadata().is_ok_and(|metadata| metadata.len() > 0);
    }

    // HEIC/HEIF and AVIF go through ImageIO on macOS or ffmpeg elsewhere, so
    // the image crate can't inspect them here. Trust the extension and let the
    // loader deal with any errors.
    if needs_transcode(path) {
        #[cfg(target_os = "macos")]
        {
            return path.metadata().is_ok_and(|metadata| metadata.len() > 0);
        }
        #[cfg(not(target_os = "macos"))]
        {
            return crate::video::is_available()
                && path.metadata().is_ok_and(|metadata| metadata.len() > 0);
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
    finish_image(image)
}

fn finish_image(image: DynamicImage) -> Result<LoadedPhoto, String> {
    if image.width() == 0 || image.height() == 0 {
        return Err("image has no pixels".to_owned());
    }

    let image = if image.width().max(image.height()) > MAX_TEXTURE_DIMENSION {
        image.thumbnail(MAX_TEXTURE_DIMENSION, MAX_TEXTURE_DIMENSION)
    } else {
        image
    };

    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let image = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());

    Ok(LoadedPhoto { image })
}

#[cfg(not(target_os = "macos"))]
fn decode_with_ffmpeg(path: &Path) -> Result<LoadedPhoto, String> {
    use std::process::{Command, Stdio};

    let Some(ffmpeg) = crate::video::find_ffmpeg() else {
        return Err(
            "HEIC/AVIF on this platform needs ffmpeg. Install it, then reopen the folder."
                .to_owned(),
        );
    };

    let output = Command::new(ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("ffmpeg could not start: {error}"))?;

    if !output.status.success() || output.stdout.is_empty() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "ffmpeg couldn't decode this photo: {}",
            message.trim().lines().next().unwrap_or("unknown error")
        ));
    }

    let image = image::load_from_memory(&output.stdout)
        .map_err(|error| format!("ffmpeg output wasn't a readable image: {error}"))?;
    finish_image(image)
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
    use super::{
        ScanError, collect_media, collect_media_interruptible, is_image_file, is_video_file,
    };
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize};
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

    #[test]
    fn cancellation_stops_a_scan() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be sane")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "photo-roulette-cancel-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create test folder");
        let image = image::RgbImage::from_pixel(8, 8, image::Rgb([255, 0, 100]));
        image.save(root.join("one.png")).expect("save png");

        let cancel = AtomicBool::new(true);
        let progress = AtomicUsize::new(0);
        let result = collect_media_interruptible(std::slice::from_ref(&root), &cancel, &progress);
        assert_eq!(result.unwrap_err(), ScanError::Cancelled);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn progress_counts_found_media() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be sane")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "photo-roulette-progress-{}-{stamp}",
            std::process::id()
        ));
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).expect("create test folder");
        let image = image::RgbImage::from_pixel(8, 8, image::Rgb([0, 200, 100]));
        image.save(root.join("one.png")).expect("save png");
        image.save(nested.join("two.png")).expect("save png");

        let cancel = AtomicBool::new(false);
        let progress = AtomicUsize::new(0);
        let library = collect_media_interruptible(std::slice::from_ref(&root), &cancel, &progress)
            .expect("folder should scan");

        assert_eq!(library.media.len(), 2);
        assert_eq!(progress.load(std::sync::atomic::Ordering::Relaxed), 2);

        let _ = std::fs::remove_dir_all(root);
    }
}
